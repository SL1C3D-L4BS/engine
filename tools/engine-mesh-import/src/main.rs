//! `engine-mesh-import` — glTF → EMSH/EMAT subprocess CLI (ADR-062).
//!
//! Build-time tool. Parses a glTF 2.0 file (`.gltf` or `.glb`), extracts
//! geometry primitives into [`engine_asset::MeshMeta`] (`EMSH`) blobs,
//! extracts materials into [`engine_asset::MaterialMeta`] (`EMAT`) blobs,
//! and emits a JSON manifest of what was produced.
//!
//! Owned-discipline:
//! - No third-party argument parser (matches `tools/engine-tex-compress/`).
//! - The third-party glTF parser is statically linked into this binary
//!   alone; the engine runtime never sees it. A CI grep guard in
//!   `.github/workflows/ci.yml` enforces the boundary (ADR-062 §2).
//! - The binary itself is the sandbox: editors invoke this CLI as a
//!   subprocess and consume the typed exit codes / JSON manifest.
//!   In-process seccomp-bpf filtering (ADR-019) is a future layer
//!   tracked alongside `engine_platform::sandbox`; today the
//!   subprocess boundary is the security property the editor relies on.
//!
//! ## Usage
//!
//! ```text
//! engine-mesh-import \
//!     --input <file.gltf|file.glb> \
//!     --out <output-dir> \
//!     [--material-shader-id <hex u32>]
//! ```
//!
//! Output:
//! - `<out>/mesh-<N>.emsh` per glTF primitive (`N` = primitive index in
//!   document order).
//! - `<out>/material-<M>.emat` per glTF material (`M` = material index).
//! - `<out>/manifest.json` listing the emitted artefacts with their
//!   relative paths.
//!
//! ## Out of scope (PR 1)
//!
//! - Texture extraction. EMAT records reference textures by content
//!   hash; the v1 importer leaves `texture_count = 0` and emits only
//!   the scalar PBR factors. A follow-up PR orchestrates
//!   `tools/engine-tex-compress/` to extract embedded glTF images and
//!   fill the texture-slot table.
//! - Skinning / animation. Bone weight/index semantics are defined in
//!   `engine_asset::VertexSemantic` but the v1 importer does not yet
//!   emit them.
//! - LODs and multi-LOD packing — Phase 11+ per ADR-061.

use engine_asset::{
    AABB_BYTES, IndexFormat, MaterialMeta, MeshMeta, SUB_MESH_BYTES, SemanticMask, SubMesh,
    VertexSemantic, encode_aabb, encode_sub_mesh,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match parse(&argv) {
        Ok(Mode::Import(opts)) => match import(opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(ImporterError::ParserCrash(msg)) => {
                eprintln!("engine-mesh-import: parser crashed: {msg}");
                ExitCode::from(EXIT_PARSER_CRASH)
            }
            Err(ImporterError::SchemaInvalid(msg)) => {
                eprintln!("engine-mesh-import: schema invalid: {msg}");
                ExitCode::from(EXIT_SCHEMA_INVALID)
            }
            Err(ImporterError::Unsupported(msg)) => {
                eprintln!("engine-mesh-import: unsupported: {msg}");
                ExitCode::from(EXIT_UNSUPPORTED)
            }
            Err(ImporterError::Io(msg)) => {
                eprintln!("engine-mesh-import: io: {msg}");
                ExitCode::from(EXIT_IO)
            }
        },
        Ok(Mode::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("engine-mesh-import: {e}");
            eprintln!();
            print_help();
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Exit code for typed schema-validation failures.
const EXIT_SCHEMA_INVALID: u8 = 2;
/// Exit code for unsupported glTF features.
const EXIT_UNSUPPORTED: u8 = 3;
/// Exit code for I/O failures (input read, output write).
const EXIT_IO: u8 = 4;
/// Exit code reserved for typed parser-crash reports. (The actual
/// signal-handler case would surface as a non-zero exit via the
/// editor's subprocess wrapper rather than reaching this constant —
/// kept as documentation of the intended discrimination.)
const EXIT_PARSER_CRASH: u8 = 5;
/// Exit code for argument-parsing failures.
const EXIT_USAGE: u8 = 64;

enum Mode {
    Import(ImportOpts),
    Help,
}

struct ImportOpts {
    input: PathBuf,
    out_dir: PathBuf,
    material_shader_id: u32,
}

#[derive(Debug)]
enum ImporterError {
    /// Reserved for future use: the editor's subprocess wrapper will
    /// surface signals (SIGSEGV, SIGABRT) as this variant once the
    /// wrapper API lives in `engine_platform::subprocess`. Today the
    /// signal case escapes as a non-zero exit and the editor maps it
    /// itself.
    #[allow(dead_code)]
    ParserCrash(String),
    SchemaInvalid(String),
    Unsupported(String),
    Io(String),
}

impl From<std::io::Error> for ImporterError {
    fn from(e: std::io::Error) -> Self {
        ImporterError::Io(e.to_string())
    }
}

fn parse(argv: &[String]) -> Result<Mode, String> {
    if argv.is_empty() || matches!(argv[0].as_str(), "-h" | "--help") {
        return Ok(Mode::Help);
    }

    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut material_shader_id: u32 = 0;

    let mut i = 0;
    while i < argv.len() {
        let key = argv[i].as_str();
        let val = argv
            .get(i + 1)
            .ok_or_else(|| format!("flag {key} needs a value"))?;
        match key {
            "--input" => input = Some(PathBuf::from(val)),
            "--out" => out_dir = Some(PathBuf::from(val)),
            "--material-shader-id" => {
                let trimmed = val.strip_prefix("0x").unwrap_or(val);
                material_shader_id = u32::from_str_radix(trimmed, 16)
                    .map_err(|_| format!("--material-shader-id must be hex u32, got {val}"))?;
            }
            other => return Err(format!("unknown flag {other}")),
        }
        i += 2;
    }

    Ok(Mode::Import(ImportOpts {
        input: input.ok_or_else(|| "--input is required".to_string())?,
        out_dir: out_dir.ok_or_else(|| "--out is required".to_string())?,
        material_shader_id,
    }))
}

fn import(opts: ImportOpts) -> Result<(), ImporterError> {
    std::fs::create_dir_all(&opts.out_dir)?;

    let (document, buffers, _images) = gltf::import(&opts.input).map_err(map_gltf_error)?;

    let mut manifest = ManifestBuilder::new(&opts.input);

    // Materials first — meshes reference them by index.
    for material in document.materials() {
        let index = material.index().unwrap_or(0) as u32;
        let (meta, payload) = build_material(&material, opts.material_shader_id);
        let filename = format!("material-{index}.emat");
        let path = opts.out_dir.join(&filename);
        write_blob(&path, &meta.encode(payload.len() as u32), &payload)?;
        manifest.add_material(index, filename);
    }

    // Then meshes / primitives.
    let mut emitted_primitive_index: u32 = 0;
    for mesh in document.meshes() {
        let mesh_name = mesh.name().map(str::to_owned);
        for primitive in mesh.primitives() {
            let (meta, payload) = build_mesh_primitive(&primitive, &buffers)?;
            let filename = format!("mesh-{emitted_primitive_index}.emsh");
            let path = opts.out_dir.join(&filename);
            write_blob(&path, &meta.encode(payload.len() as u32), &payload)?;
            let material_idx = primitive
                .material()
                .index()
                .map(|i| i as u32)
                .unwrap_or(u32::MAX);
            manifest.add_mesh(
                emitted_primitive_index,
                filename,
                mesh_name.as_deref(),
                material_idx,
            );
            emitted_primitive_index += 1;
        }
    }

    // Manifest goes on stdout AND to <out>/manifest.json.
    let manifest_json = manifest.serialize();
    println!("{manifest_json}");
    let manifest_path = opts.out_dir.join("manifest.json");
    std::fs::write(&manifest_path, &manifest_json)?;

    Ok(())
}

fn map_gltf_error(e: gltf::Error) -> ImporterError {
    // gltf::Error has feature-gated variants we don't enumerate here;
    // the `Io` arm picks up filesystem failures explicitly and the
    // catch-all routes everything else through SchemaInvalid so
    // adversarial inputs surface as typed exit codes rather than
    // panics.
    match e {
        gltf::Error::Io(io) => ImporterError::Io(io.to_string()),
        other => ImporterError::SchemaInvalid(other.to_string()),
    }
}

fn build_mesh_primitive(
    primitive: &gltf::Primitive<'_>,
    buffers: &[gltf::buffer::Data],
) -> Result<(MeshMeta, Vec<u8>), ImporterError> {
    let reader = primitive.reader(|buf| buffers.get(buf.index()).map(|d| d.0.as_slice()));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| ImporterError::SchemaInvalid("primitive missing POSITION".into()))?
        .collect();
    if positions.is_empty() {
        return Err(ImporterError::SchemaInvalid(
            "primitive has zero vertices".into(),
        ));
    }

    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|it| it.collect());
    let tangents: Option<Vec<[f32; 4]>> = reader.read_tangents().map(|it| it.collect());
    let uv0: Option<Vec<[f32; 2]>> = reader.read_tex_coords(0).map(|tc| tc.into_f32().collect());

    let vertex_count = positions.len() as u32;

    // Build the semantic mask from the present attributes.
    let mut mask = SemanticMask::EMPTY.with(VertexSemantic::Position);
    if normals.as_ref().is_some_and(|n| n.len() == positions.len()) {
        mask = mask.with(VertexSemantic::Normal);
    }
    if tangents
        .as_ref()
        .is_some_and(|t| t.len() == positions.len())
    {
        mask = mask.with(VertexSemantic::Tangent);
    }
    if uv0.as_ref().is_some_and(|u| u.len() == positions.len()) {
        mask = mask.with(VertexSemantic::Uv0);
    }

    let vertex_stride = mask.vertex_stride();
    if vertex_stride > u8::MAX as usize {
        return Err(ImporterError::Unsupported(format!(
            "vertex stride {vertex_stride} exceeds u8::MAX"
        )));
    }

    // Indices.
    let (index_format, index_bytes, index_count) = match reader.read_indices() {
        Some(read) => {
            let u32s: Vec<u32> = read.into_u32().collect();
            let max_index = u32s.iter().copied().max().unwrap_or(0);
            let count = u32s.len() as u32;
            if max_index < u16::MAX as u32 {
                let mut bytes = Vec::with_capacity(u32s.len() * 2);
                for v in &u32s {
                    bytes.extend_from_slice(&(*v as u16).to_le_bytes());
                }
                (IndexFormat::U16, bytes, count)
            } else {
                let mut bytes = Vec::with_capacity(u32s.len() * 4);
                for v in &u32s {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                (IndexFormat::U32, bytes, count)
            }
        }
        None => {
            // Un-indexed: emit a trivial 0..N index buffer for layout regularity.
            let count = vertex_count;
            if vertex_count <= u16::MAX as u32 {
                let mut bytes = Vec::with_capacity(count as usize * 2);
                for i in 0..count {
                    bytes.extend_from_slice(&(i as u16).to_le_bytes());
                }
                (IndexFormat::U16, bytes, count)
            } else {
                let mut bytes = Vec::with_capacity(count as usize * 4);
                for i in 0..count {
                    bytes.extend_from_slice(&i.to_le_bytes());
                }
                (IndexFormat::U32, bytes, count)
            }
        }
    };

    // Build the vertex payload by interleaving the present attributes.
    let mut vertex_payload = Vec::with_capacity(positions.len() * vertex_stride);
    for (v, position) in positions.iter().enumerate() {
        for s in mask.iter() {
            match s {
                VertexSemantic::Position => {
                    for c in position {
                        vertex_payload.extend_from_slice(&c.to_le_bytes());
                    }
                }
                VertexSemantic::Normal => {
                    let n = normals.as_ref().unwrap()[v];
                    for c in n {
                        vertex_payload.extend_from_slice(&c.to_le_bytes());
                    }
                }
                VertexSemantic::Tangent => {
                    let t = tangents.as_ref().unwrap()[v];
                    for c in t {
                        vertex_payload.extend_from_slice(&c.to_le_bytes());
                    }
                }
                VertexSemantic::Uv0 => {
                    let u = uv0.as_ref().unwrap()[v];
                    for c in u {
                        vertex_payload.extend_from_slice(&c.to_le_bytes());
                    }
                }
                _ => unreachable!("only Position/Normal/Tangent/Uv0 considered in v1 importer"),
            }
        }
    }

    // AABB from positions.
    let mut min = positions[0];
    let mut max = positions[0];
    for p in &positions[1..] {
        for i in 0..3 {
            if p[i] < min[i] {
                min[i] = p[i];
            }
            if p[i] > max[i] {
                max[i] = p[i];
            }
        }
    }

    // SubMesh table: one sub-mesh per primitive in v1.
    let material_index = primitive
        .material()
        .index()
        .and_then(|i| u16::try_from(i).ok())
        .unwrap_or(0);
    let sub_mesh = SubMesh {
        first_index: 0,
        index_count,
        material_index,
        flags: 0,
    };

    let mut payload =
        Vec::with_capacity(vertex_payload.len() + index_bytes.len() + SUB_MESH_BYTES + AABB_BYTES);
    payload.extend_from_slice(&vertex_payload);
    payload.extend_from_slice(&index_bytes);
    payload.extend_from_slice(&encode_sub_mesh(sub_mesh));
    payload.extend_from_slice(&encode_aabb(min, max));

    let meta = MeshMeta {
        vertex_count,
        index_count,
        vertex_stride: vertex_stride as u8,
        index_format,
        semantic_mask: mask,
        sub_mesh_count: 1,
    };
    Ok((meta, payload))
}

fn build_material(material: &gltf::Material<'_>, shader_id: u32) -> (MaterialMeta, Vec<u8>) {
    let pbr = material.pbr_metallic_roughness();
    let base_color = pbr.base_color_factor();
    let metallic = pbr.metallic_factor();
    let roughness = pbr.roughness_factor();

    // 6 factors: base_color.rgb, metallic, roughness, alpha cutoff.
    let cutoff = material.alpha_cutoff().unwrap_or(0.5);
    let factors: [f32; 6] = [
        base_color[0],
        base_color[1],
        base_color[2],
        metallic,
        roughness,
        cutoff,
    ];

    let meta = MaterialMeta {
        shader_id,
        texture_count: 0, // texture extraction is a PR 1.5 follow-up
        factor_count: factors.len() as u8,
    };

    let mut payload = Vec::with_capacity(factors.len() * 4);
    for v in factors {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    (meta, payload)
}

fn write_blob(path: &Path, header: &[u8], payload: &[u8]) -> Result<(), ImporterError> {
    let mut out = Vec::with_capacity(header.len() + payload.len());
    out.extend_from_slice(header);
    out.extend_from_slice(payload);
    std::fs::write(path, &out)
        .map_err(|e| ImporterError::Io(format!("writing {}: {e}", path.display())))?;
    Ok(())
}

/// Owned JSON writer for the import manifest. Same discipline as the
/// frame-pacing bench's owned report writer — no serde, no third-party
/// JSON, deterministic key order via the underlying `BTreeMap`s.
struct ManifestBuilder {
    source: String,
    meshes: BTreeMap<u32, MeshEntry>,
    materials: BTreeMap<u32, String>,
}

struct MeshEntry {
    filename: String,
    name: Option<String>,
    material_index: u32,
}

impl ManifestBuilder {
    fn new(input: &Path) -> Self {
        Self {
            source: input.display().to_string(),
            meshes: BTreeMap::new(),
            materials: BTreeMap::new(),
        }
    }

    fn add_mesh(&mut self, index: u32, filename: String, name: Option<&str>, material_index: u32) {
        self.meshes.insert(
            index,
            MeshEntry {
                filename,
                name: name.map(str::to_owned),
                material_index,
            },
        );
    }

    fn add_material(&mut self, index: u32, filename: String) {
        self.materials.insert(index, filename);
    }

    fn serialize(&self) -> String {
        // Owned JSON. Keys ordered (BTreeMap); values escaped for the
        // small subset of characters that appear in our outputs (just
        // quote + backslash; filenames have neither).
        let mut s = String::new();
        s.push_str("{\n");
        s.push_str("  \"source\": ");
        push_json_string(&mut s, &self.source);
        s.push_str(",\n");
        s.push_str("  \"meshes\": [\n");
        let mut first = true;
        for (idx, entry) in &self.meshes {
            if !first {
                s.push_str(",\n");
            }
            first = false;
            s.push_str("    {");
            s.push_str(&format!(" \"index\": {idx},"));
            s.push_str(" \"filename\": ");
            push_json_string(&mut s, &entry.filename);
            s.push(',');
            s.push_str(" \"material_index\": ");
            if entry.material_index == u32::MAX {
                s.push_str("null");
            } else {
                s.push_str(&entry.material_index.to_string());
            }
            if let Some(n) = &entry.name {
                s.push_str(", \"name\": ");
                push_json_string(&mut s, n);
            }
            s.push_str(" }");
        }
        s.push_str("\n  ],\n");
        s.push_str("  \"materials\": [\n");
        let mut first = true;
        for (idx, filename) in &self.materials {
            if !first {
                s.push_str(",\n");
            }
            first = false;
            s.push_str("    {");
            s.push_str(&format!(" \"index\": {idx},"));
            s.push_str(" \"filename\": ");
            push_json_string(&mut s, filename);
            s.push_str(" }");
        }
        s.push_str("\n  ]\n");
        s.push('}');
        s.push('\n');
        s
    }
}

fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn print_help() {
    eprintln!(
        "engine-mesh-import — glTF → EMSH/EMAT subprocess CLI (ADR-062)\n\
         \n\
         USAGE:\n  \
             engine-mesh-import --input <PATH> --out <DIR> [--material-shader-id <HEX>]\n\
         \n\
         FLAGS:\n  \
             --input <PATH>                .gltf or .glb file to import\n  \
             --out <DIR>                   directory to write artefacts into\n  \
             --material-shader-id <HEX>    truncated BLAKE3 shader id for EMAT (default: 0)\n  \
             -h, --help                    show this message\n\
         \n\
         OUTPUTS:\n  \
             <out>/mesh-<N>.emsh           one per glTF primitive\n  \
             <out>/material-<M>.emat       one per glTF material\n  \
             <out>/manifest.json           list of emitted artefacts\n\
         \n\
         EXIT CODES:\n  \
             0   success\n  \
             2   schema invalid (malformed glTF)\n  \
             3   unsupported glTF feature\n  \
             4   io failure\n  \
             5   parser crash (reserved)\n  \
             64  usage error\n"
    );
}
