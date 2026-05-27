//! Owned RON reader for the Phase-5 frame-pacing scene fixture
//! (`testbed/frame-pacing/scenes/v0.ron`, ADR-047 §1).
//!
//! Recognises the v0 schema only — a flat `FramePacingScene(...)`
//! struct with named fields, scalar number/string/ident values, and
//! `(u32, u32)` tuple extents. Anything outside that schema is a parse
//! error; schema growth requires an amendment PR.
//!
//! Owned discipline: no `ron`, no `serde`, no third-party parser. The
//! parser is a tiny recursive-descent matching the documented v0 form.

use blake3::Hasher;

/// Parsed scene parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// Master deterministic seed.
    pub seed: u64,
    /// Frame count (e.g. 3600 = 60 s at 60 FPS).
    pub frames: u32,
    /// Entity count for the GPU runner's full pipeline.
    pub entities: u32,
    /// Distinct meshes the entities draw.
    pub unique_meshes: u32,
    /// Directional / cascaded-shadow light count.
    pub directional_lights: u32,
    /// Point + spot lights placed in the cluster grid.
    pub point_spot_lights: u32,
    /// BLAKE3 seed for the day-night camera path.
    pub camera_seed: u64,
    /// Quality preset name (e.g. "rx-580").
    pub quality: String,
    /// Target FPS for budget calibration (60 Hz today).
    pub target_fps: u32,
    /// Internal render resolution `[w, h]`.
    pub internal_extent: [u32; 2],
    /// Display resolution `[w, h]`.
    pub output_extent: [u32; 2],
}

/// 32-byte BLAKE3 hash of `bytes`, as a hex string. The bench JSON
/// report records this in the `scene_hash` field (ADR-008
/// content-addressed scene fixture).
pub fn scene_hash_hex(bytes: &[u8]) -> String {
    let mut h = Hasher::new();
    h.update(bytes);
    h.finalize().to_hex().to_string()
}

/// Parse a v0 scene from its on-disk bytes.
pub fn parse(source: &str) -> Result<Scene, String> {
    let mut p = Parser::new(source);
    p.skip_trivia();
    p.expect_ident("FramePacingScene")?;
    p.skip_trivia();
    p.expect_char('(')?;
    let mut scene = Scene::empty();
    let mut seen = SceneFieldsSeen::default();
    loop {
        p.skip_trivia();
        if p.peek() == Some(')') {
            p.bump();
            break;
        }
        let key = p.read_ident()?;
        p.skip_trivia();
        p.expect_char(':')?;
        p.skip_trivia();
        match key.as_str() {
            "seed" => {
                scene.seed = p.read_u64()?;
                seen.seed = true;
            }
            "frames" => {
                scene.frames = p.read_u32()?;
                seen.frames = true;
            }
            "entities" => {
                scene.entities = p.read_u32()?;
                seen.entities = true;
            }
            "unique_meshes" => {
                scene.unique_meshes = p.read_u32()?;
                seen.unique_meshes = true;
            }
            "directional_lights" => {
                scene.directional_lights = p.read_u32()?;
                seen.directional_lights = true;
            }
            "point_spot_lights" => {
                scene.point_spot_lights = p.read_u32()?;
                seen.point_spot_lights = true;
            }
            "camera_seed" => {
                scene.camera_seed = p.read_u64()?;
                seen.camera_seed = true;
            }
            "quality" => {
                scene.quality = p.read_string()?;
                seen.quality = true;
            }
            "target_fps" => {
                scene.target_fps = p.read_u32()?;
                seen.target_fps = true;
            }
            "internal_extent" => {
                scene.internal_extent = p.read_extent()?;
                seen.internal_extent = true;
            }
            "output_extent" => {
                scene.output_extent = p.read_extent()?;
                seen.output_extent = true;
            }
            other => {
                return Err(format!("unknown scene field `{other}`"));
            }
        }
        p.skip_trivia();
        // Optional trailing comma.
        if p.peek() == Some(',') {
            p.bump();
        }
    }
    seen.into_result()?;
    Ok(scene)
}

#[derive(Default)]
struct SceneFieldsSeen {
    seed: bool,
    frames: bool,
    entities: bool,
    unique_meshes: bool,
    directional_lights: bool,
    point_spot_lights: bool,
    camera_seed: bool,
    quality: bool,
    target_fps: bool,
    internal_extent: bool,
    output_extent: bool,
}

impl SceneFieldsSeen {
    fn into_result(self) -> Result<(), String> {
        let missing = [
            ("seed", self.seed),
            ("frames", self.frames),
            ("entities", self.entities),
            ("unique_meshes", self.unique_meshes),
            ("directional_lights", self.directional_lights),
            ("point_spot_lights", self.point_spot_lights),
            ("camera_seed", self.camera_seed),
            ("quality", self.quality),
            ("target_fps", self.target_fps),
            ("internal_extent", self.internal_extent),
            ("output_extent", self.output_extent),
        ]
        .iter()
        .filter_map(|(n, ok)| if !*ok { Some(*n) } else { None })
        .collect::<Vec<_>>()
        .join(", ");
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!("scene is missing required field(s): {missing}"))
        }
    }
}

impl Scene {
    fn empty() -> Self {
        Self {
            seed: 0,
            frames: 0,
            entities: 0,
            unique_meshes: 0,
            directional_lights: 0,
            point_spot_lights: 0,
            camera_seed: 0,
            quality: String::new(),
            target_fps: 0,
            internal_extent: [0, 0],
            output_extent: [0, 0],
        }
    }
}

/// Recursive-descent parser over the v0 schema's byte stream.
struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).map(|b| *b as char)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    /// Skip whitespace and `// line comments`.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.src.get(self.pos + offset).map(|b| *b as char)
    }

    fn expect_char(&mut self, c: char) -> Result<(), String> {
        match self.peek() {
            Some(g) if g == c => {
                self.bump();
                Ok(())
            }
            Some(g) => Err(format!("expected `{c}`, found `{g}` at byte {}", self.pos)),
            None => Err(format!("expected `{c}`, found EOF at byte {}", self.pos)),
        }
    }

    fn expect_ident(&mut self, expected: &str) -> Result<(), String> {
        let got = self.read_ident()?;
        if got == expected {
            Ok(())
        } else {
            Err(format!("expected `{expected}`, found `{got}`"))
        }
    }

    fn read_ident(&mut self) -> Result<String, String> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                self.bump();
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(format!("expected identifier at byte {}", self.pos));
        }
        Ok(std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| "non-UTF-8 identifier".to_string())?
            .to_string())
    }

    fn read_number_token(&mut self) -> Result<String, String> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                self.bump();
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(format!("expected number at byte {}", self.pos));
        }
        Ok(std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| "non-UTF-8 number".to_string())?
            .replace('_', ""))
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let t = self.read_number_token()?;
        t.parse().map_err(|_| format!("bad u32: {t}"))
    }

    fn read_u64(&mut self) -> Result<u64, String> {
        let t = self.read_number_token()?;
        t.parse().map_err(|_| format!("bad u64: {t}"))
    }

    fn read_string(&mut self) -> Result<String, String> {
        self.expect_char('"')?;
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == '"' {
                let body = std::str::from_utf8(&self.src[start..self.pos])
                    .map_err(|_| "non-UTF-8 string".to_string())?
                    .to_string();
                self.bump();
                return Ok(body);
            }
            if c == '\\' {
                return Err("backslash escapes are not supported by v0".to_string());
            }
            self.bump();
        }
        Err("unterminated string".to_string())
    }

    fn read_extent(&mut self) -> Result<[u32; 2], String> {
        self.expect_char('(')?;
        self.skip_trivia();
        let w = self.read_u32()?;
        self.skip_trivia();
        self.expect_char(',')?;
        self.skip_trivia();
        let h = self.read_u32()?;
        self.skip_trivia();
        // Optional trailing comma inside the tuple.
        if self.peek() == Some(',') {
            self.bump();
            self.skip_trivia();
        }
        self.expect_char(')')?;
        Ok([w, h])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL: &str = include_str!("../../../testbed/frame-pacing/scenes/v0.ron");

    #[test]
    fn parse_canonical_v0_round_trips_all_fields() {
        let s = parse(CANONICAL).expect("canonical scene parses");
        assert_eq!(s.seed, 0);
        assert_eq!(s.frames, 3600);
        assert_eq!(s.entities, 10_000);
        assert_eq!(s.unique_meshes, 50);
        assert_eq!(s.directional_lights, 16);
        assert_eq!(s.point_spot_lights, 48);
        assert_eq!(s.camera_seed, 0);
        assert_eq!(s.quality, "rx-580");
        assert_eq!(s.target_fps, 60);
        assert_eq!(s.internal_extent, [1280, 720]);
        assert_eq!(s.output_extent, [2560, 1440]);
    }

    #[test]
    fn parse_rejects_missing_required_field() {
        let src = r#"FramePacingScene(
            seed: 0,
            frames: 60,
        )"#;
        let err = parse(src).unwrap_err();
        assert!(err.contains("missing required field"));
        assert!(err.contains("entities"));
    }

    #[test]
    fn parse_rejects_unknown_field() {
        let src = "FramePacingScene(unknown_field: 5)";
        let err = parse(src).unwrap_err();
        assert!(err.contains("unknown scene field"));
        assert!(err.contains("unknown_field"));
    }

    #[test]
    fn parse_handles_inline_line_comments() {
        let src = r#"
// preamble comment
FramePacingScene(
    seed: 0,                // single-line comment
    frames: 100,
    entities: 1,
    unique_meshes: 1,
    directional_lights: 0,
    point_spot_lights: 0,
    camera_seed: 0,
    quality: "test",
    target_fps: 30,
    internal_extent: (640, 360),
    output_extent: (1280, 720),
)
"#;
        let s = parse(src).expect("parses with comments");
        assert_eq!(s.frames, 100);
        assert_eq!(s.internal_extent, [640, 360]);
    }

    #[test]
    fn parse_rejects_a_missing_tuple_comma() {
        let src = r#"FramePacingScene(
    seed: 0,
    frames: 1,
    entities: 1,
    unique_meshes: 1,
    directional_lights: 0,
    point_spot_lights: 0,
    camera_seed: 0,
    quality: "x",
    target_fps: 60,
    internal_extent: (1280 720),
    output_extent: (2560, 1440),
)"#;
        assert!(parse(src).is_err());
    }

    #[test]
    fn scene_hash_hex_matches_blake3_of_bytes() {
        // Hand-computed: the empty input's hash.
        assert_eq!(
            scene_hash_hex(b""),
            blake3::hash(b"").to_hex().to_string()
        );
        // Trivial determinism: same input → same hex.
        let a = scene_hash_hex(CANONICAL.as_bytes());
        let b = scene_hash_hex(CANONICAL.as_bytes());
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "BLAKE3 32-byte hex is 64 chars");
    }
}
