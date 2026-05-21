//! Source files, the source map, and byte-offset spans.
//!
//! A [`SourceMap`] owns every [`Source`] the compiler has ingested; a
//! [`FileId`] is a stable cross-pass handle to one of them. A [`Span`] pairs
//! a file id with a half-open byte range — `lo` is inclusive, `hi` is
//! exclusive — over the file's text.

/// Stable handle to one source file inside a [`SourceMap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

/// A half-open byte range inside the file identified by `file`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// The file this span refers to.
    pub file: FileId,
    /// Inclusive byte offset of the first character.
    pub lo: u32,
    /// Exclusive byte offset just past the last character.
    pub hi: u32,
}

impl Span {
    /// Constructs a span. `lo <= hi` is required.
    pub fn new(file: FileId, lo: u32, hi: u32) -> Self {
        debug_assert!(lo <= hi);
        Self { file, lo, hi }
    }

    /// A zero-width span at `lo` in `file`. Used for synthesized nodes.
    pub fn point(file: FileId, lo: u32) -> Self {
        Self::new(file, lo, lo)
    }

    /// Widens `self` to cover both `self` and `other`. Both must refer to
    /// the same file; otherwise `self` is returned unchanged.
    pub fn join(self, other: Span) -> Span {
        if self.file != other.file {
            return self;
        }
        Span::new(self.file, self.lo.min(other.lo), self.hi.max(other.hi))
    }
}

/// One source file: a logical name and its text.
#[derive(Clone, Debug)]
pub struct Source {
    /// Logical path or name — used in diagnostics, not opened on disk.
    pub name: String,
    /// File text. Byte offsets in spans index into this string.
    pub text: String,
    /// Cumulative line-start offsets; index `i` is the byte offset of
    /// the start of line `i + 1` (line numbers are 1-based).
    line_starts: Vec<u32>,
}

impl Source {
    /// Constructs a [`Source`] and precomputes its line table.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        Self {
            name: name.into(),
            text,
            line_starts,
        }
    }

    /// The `(line, column)` 1-based location of `offset`.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let idx = self.line_starts.partition_point(|&s| s <= offset);
        let line = idx as u32; // partition_point returns count of <= matches
        let line_start = self
            .line_starts
            .get(idx.saturating_sub(1))
            .copied()
            .unwrap_or(0);
        let col = offset.saturating_sub(line_start) + 1;
        (line, col)
    }

    /// The full text of line `line` (1-based), without its trailing newline.
    pub fn line_text(&self, line: u32) -> Option<&str> {
        let idx = line.checked_sub(1)? as usize;
        let start = *self.line_starts.get(idx)? as usize;
        let end = self
            .line_starts
            .get(idx + 1)
            .map(|e| *e as usize - 1)
            .unwrap_or(self.text.len());
        self.text.get(start..end)
    }
}

/// Owns every source file the compiler has ingested.
#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<Source>,
}

impl SourceMap {
    /// An empty source map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingests a source, returning its stable id.
    pub fn add(&mut self, source: Source) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(source);
        id
    }

    /// Borrows a previously-added source by id.
    pub fn get(&self, id: FileId) -> &Source {
        &self.files[id.0 as usize]
    }

    /// The number of files in the map.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Borrows the text of `span`, panicking if it is out of bounds.
    pub fn text(&self, span: Span) -> &str {
        &self.get(span.file).text[span.lo as usize..span.hi as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_roundtrip() {
        let src = Source::new("test.sli", "fn main() {\n    return 0;\n}\n");
        assert_eq!(src.line_col(0), (1, 1));
        assert_eq!(src.line_col(12), (2, 1));
        assert_eq!(src.line_col(16), (2, 5));
        assert_eq!(src.line_text(1), Some("fn main() {"));
        assert_eq!(src.line_text(2), Some("    return 0;"));
    }

    #[test]
    fn span_join_same_file() {
        let f = FileId(0);
        let a = Span::new(f, 4, 7);
        let b = Span::new(f, 10, 12);
        assert_eq!(a.join(b), Span::new(f, 4, 12));
    }
}
