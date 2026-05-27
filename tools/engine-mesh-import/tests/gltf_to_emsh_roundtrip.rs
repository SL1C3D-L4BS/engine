//! End-to-end import test: build a minimal glTF + .bin pair at
//! runtime, invoke the importer binary on it, and verify the EMSH /
//! EMAT outputs round-trip back through `engine_asset::decode`
//! (ADR-062 §Verification).

use engine_asset::{MaterialMeta, MeshMeta, VertexSemantic};
use std::path::{Path, PathBuf};
use std::process::Command;

const BINARY: &str = env!("CARGO_BIN_EXE_engine-mesh-import");

/// Per-test scratch directory under the workspace target/ tree.
/// Avoids `std::env::temp_dir` so collisions across parallel runs
/// are predictable + the artifacts are inspectable after a failure.
fn scratch(name: &str) -> PathBuf {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("create scratch dir");
    base
}

/// Write a minimal triangle `triangle.gltf` + `buffer.bin` into `dir`
/// and return the path to the .gltf. The triangle has 3 vertices
/// (positions only, no normals/uvs) and 3 indices, plus one PBR
/// material with non-default base color so the importer's factor
/// extraction has observable values to round-trip.
fn write_triangle_gltf(dir: &Path) -> PathBuf {
    // Vertex positions: three points spanning a triangle on the XY plane.
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let indices: [u16; 3] = [0, 1, 2];

    let mut buf = Vec::new();
    for p in &positions {
        for v in p {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    let positions_len = buf.len();
    for i in &indices {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    let total_len = buf.len();
    let indices_len = total_len - positions_len;
    let buf_path = dir.join("buffer.bin");
    std::fs::write(&buf_path, &buf).expect("write buffer.bin");

    // Hand-written glTF JSON. The buffer's URI is the relative
    // filename — gltf::import resolves it relative to the .gltf path.
    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [ {{ "nodes": [0] }} ],
  "nodes": [ {{ "mesh": 0 }} ],
  "meshes": [ {{
    "primitives": [ {{
      "attributes": {{ "POSITION": 0 }},
      "indices": 1,
      "material": 0
    }} ]
  }} ],
  "materials": [ {{
    "pbrMetallicRoughness": {{
      "baseColorFactor": [0.8, 0.4, 0.2, 1.0],
      "metallicFactor": 0.1,
      "roughnessFactor": 0.7
    }}
  }} ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
       "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] }},
    {{ "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": {positions_len} }},
    {{ "buffer": 0, "byteOffset": {positions_len}, "byteLength": {indices_len} }}
  ],
  "buffers": [
    {{ "uri": "buffer.bin", "byteLength": {total_len} }}
  ]
}}"#
    );
    let gltf_path = dir.join("triangle.gltf");
    std::fs::write(&gltf_path, json).expect("write triangle.gltf");
    gltf_path
}

#[test]
fn imports_minimal_triangle_to_emsh_and_emat() {
    let scratch = scratch("import-triangle");
    let gltf = write_triangle_gltf(&scratch);
    let out_dir = scratch.join("out");

    let status = Command::new(BINARY)
        .args(["--input"])
        .arg(&gltf)
        .args(["--out"])
        .arg(&out_dir)
        .args(["--material-shader-id", "0xcafef00d"])
        .status()
        .expect("spawn importer");
    assert!(status.success(), "importer exited with {status}");

    // EMSH file present + decodes + matches the expected layout.
    let emsh_path = out_dir.join("mesh-0.emsh");
    let emsh = std::fs::read(&emsh_path).expect("read mesh-0.emsh");
    let (meta, payload) = MeshMeta::decode(&emsh).expect("decode EMSH");
    assert_eq!(meta.vertex_count, 3);
    assert_eq!(meta.index_count, 3);
    assert!(meta.semantic_mask.contains(VertexSemantic::Position));
    assert!(!meta.semantic_mask.contains(VertexSemantic::Normal));
    assert!(!meta.semantic_mask.contains(VertexSemantic::Tangent));
    assert!(!meta.semantic_mask.contains(VertexSemantic::Uv0));
    assert_eq!(
        meta.vertex_stride as usize,
        VertexSemantic::Position.bytes()
    );
    assert_eq!(payload.len() as u64, meta.expected_payload_len());
    assert_eq!(meta.sub_mesh_count, 1);

    // EMAT file present + decodes + carries the PBR factors.
    let emat_path = out_dir.join("material-0.emat");
    let emat = std::fs::read(&emat_path).expect("read material-0.emat");
    let (mat, mat_payload) = MaterialMeta::decode(&emat).expect("decode EMAT");
    assert_eq!(mat.shader_id, 0xcafe_f00d);
    assert_eq!(mat.factor_count, 6);
    assert_eq!(mat.texture_count, 0);
    assert_eq!(mat_payload.len(), 6 * 4);
    let factor = |i: usize| {
        f32::from_le_bytes([
            mat_payload[i * 4],
            mat_payload[i * 4 + 1],
            mat_payload[i * 4 + 2],
            mat_payload[i * 4 + 3],
        ])
    };
    assert!((factor(0) - 0.8).abs() < 1e-6);
    assert!((factor(1) - 0.4).abs() < 1e-6);
    assert!((factor(2) - 0.2).abs() < 1e-6);
    assert!((factor(3) - 0.1).abs() < 1e-6);
    assert!((factor(4) - 0.7).abs() < 1e-6);

    // Manifest written + lists both artefacts.
    let manifest =
        std::fs::read_to_string(out_dir.join("manifest.json")).expect("read manifest.json");
    assert!(manifest.contains("\"mesh-0.emsh\""), "{manifest}");
    assert!(manifest.contains("\"material-0.emat\""), "{manifest}");
}

#[test]
fn import_is_deterministic_across_runs() {
    // Identical input → identical output bytes (the content-addressed
    // property the asset pak relies on).
    let scratch_a = scratch("import-determinism-a");
    let scratch_b = scratch("import-determinism-b");
    let gltf_a = write_triangle_gltf(&scratch_a);
    let gltf_b = write_triangle_gltf(&scratch_b);
    let out_a = scratch_a.join("out");
    let out_b = scratch_b.join("out");

    for (gltf, out) in [(&gltf_a, &out_a), (&gltf_b, &out_b)] {
        let status = Command::new(BINARY)
            .args(["--input"])
            .arg(gltf)
            .args(["--out"])
            .arg(out)
            .args(["--material-shader-id", "0x12345678"])
            .status()
            .expect("spawn importer");
        assert!(status.success());
    }

    let a_emsh = std::fs::read(out_a.join("mesh-0.emsh")).unwrap();
    let b_emsh = std::fs::read(out_b.join("mesh-0.emsh")).unwrap();
    assert_eq!(a_emsh, b_emsh);
    let a_emat = std::fs::read(out_a.join("material-0.emat")).unwrap();
    let b_emat = std::fs::read(out_b.join("material-0.emat")).unwrap();
    assert_eq!(a_emat, b_emat);
}

#[test]
fn malformed_gltf_is_rejected_without_crashing_editor() {
    // ADR-062 §Verification gltf_red_team: a deliberately malformed
    // input should surface as a typed exit code, not a panic / signal.
    let scratch = scratch("import-red-team");
    let gltf_path = scratch.join("bogus.gltf");
    // Truncated JSON — missing closing braces, no asset section.
    std::fs::write(&gltf_path, b"{ \"asset\": { \"version\":").unwrap();
    let out_dir = scratch.join("out");

    let output = Command::new(BINARY)
        .args(["--input"])
        .arg(&gltf_path)
        .args(["--out"])
        .arg(&out_dir)
        .output()
        .expect("spawn importer");
    assert!(!output.status.success(), "expected non-zero exit");
    // Typed schema-invalid exit code per ADR-062 §6.
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn missing_input_reports_io_error() {
    let scratch = scratch("import-missing-input");
    let out_dir = scratch.join("out");
    let output = Command::new(BINARY)
        .args(["--input"])
        .arg(scratch.join("does-not-exist.gltf"))
        .args(["--out"])
        .arg(&out_dir)
        .output()
        .expect("spawn importer");
    assert!(!output.status.success());
    // gltf::Error::Io routes to ImporterError::Io → exit 4.
    assert_eq!(
        output.status.code(),
        Some(4),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn help_flag_exits_success() {
    let output = Command::new(BINARY)
        .arg("--help")
        .output()
        .expect("spawn importer");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("engine-mesh-import"));
    assert!(stderr.contains("--input"));
    assert!(stderr.contains("--out"));
}
