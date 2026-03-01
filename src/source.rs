// source.rs — SourceMap, FileId, string interning.
//
// The SourceMap owns all source text. FileId is a handle into the file table.
// Span references a byte range within a file. SourceMap resolves Span→line:col.

use std::collections::HashMap;
use std::fs;
use std::io;

/// Handle to a source file in the SourceMap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

/// Byte-range span within a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn new(file: FileId, lo: u32, hi: u32) -> Self {
        Self { file, lo, hi }
    }

    /// A dummy span for compiler-generated constructs.
    pub fn dummy() -> Self {
        Self {
            file: FileId(u32::MAX),
            lo: 0,
            hi: 0,
        }
    }

    pub fn is_dummy(self) -> bool {
        self.file.0 == u32::MAX
    }

    /// Merge two spans into one covering both.
    pub fn merge(self, other: Span) -> Span {
        debug_assert_eq!(self.file, other.file);
        Span {
            file: self.file,
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }
}

/// Metadata for a single source file.
#[derive(Debug)]
struct FileInfo {
    name: String,
    content: String,
    /// Byte offsets of each line start (0-indexed into content).
    line_starts: Vec<u32>,
}

impl FileInfo {
    fn new(name: String, content: String) -> Self {
        let line_starts = compute_line_starts(&content);
        Self {
            name,
            content,
            line_starts,
        }
    }
}

/// Central source file registry. Owns all source text; provides span→location lookup.
pub struct SourceMap {
    files: Vec<FileInfo>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Load a file from disk and register it. Returns a FileId handle.
    pub fn load_file(&mut self, path: &str) -> io::Result<FileId> {
        let content = fs::read_to_string(path)?;
        Ok(self.add_file(path.to_string(), content))
    }

    /// Register a file from a string (useful for tests and preprocessor).
    pub fn add_file(&mut self, name: String, content: String) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(FileInfo::new(name, content));
        id
    }

    /// Get the file name for a FileId.
    pub fn file_name(&self, id: FileId) -> &str {
        &self.files[id.0 as usize].name
    }

    /// Get the full source content for a FileId.
    pub fn file_content(&self, id: FileId) -> &str {
        &self.files[id.0 as usize].content
    }

    /// Resolve a byte offset within a file to (1-based line, 1-based column).
    pub fn offset_to_line_col(&self, file: FileId, offset: u32) -> (u32, u32) {
        let info = &self.files[file.0 as usize];
        let line_idx = match info.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line = (line_idx + 1) as u32;
        let col = (offset - info.line_starts[line_idx] + 1) as u32;
        (line, col)
    }

    /// Resolve a Span to (filename, line, col) for the start of the span.
    pub fn span_to_location(&self, span: Span) -> (&str, u32, u32) {
        if span.is_dummy() {
            return ("<unknown>", 0, 0);
        }
        let (line, col) = self.offset_to_line_col(span.file, span.lo);
        (self.file_name(span.file), line, col)
    }

    /// Get the source text covered by a span.
    pub fn span_text(&self, span: Span) -> &str {
        if span.is_dummy() {
            return "";
        }
        let content = self.file_content(span.file);
        &content[span.lo as usize..span.hi as usize]
    }

    /// Get the full line of source containing the given byte offset.
    pub fn line_at_offset(&self, file: FileId, offset: u32) -> &str {
        let info = &self.files[file.0 as usize];
        let line_idx = match info.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let start = info.line_starts[line_idx] as usize;
        let end = if line_idx + 1 < info.line_starts.len() {
            info.line_starts[line_idx + 1] as usize
        } else {
            info.content.len()
        };
        info.content[start..end].trim_end_matches('\n')
    }
}

/// Compute byte offsets of each line start in the source text.
fn compute_line_starts(content: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            starts.push((i + 1) as u32);
        }
    }
    starts
}

/// String interning table. Maps strings to `InternId` handles.
/// All interned strings are stored in a single contiguous buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InternId(pub u32);

pub struct StringInterner {
    /// Contiguous buffer holding all interned strings, separated by null bytes.
    buffer: Vec<u8>,
    /// Maps (offset, len) for each InternId.
    entries: Vec<(u32, u32)>,
    /// Deduplication map: string content → InternId.
    map: HashMap<Vec<u8>, InternId>,
}

impl StringInterner {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
            entries: Vec::with_capacity(256),
            map: HashMap::new(),
        }
    }

    /// Intern a string and return its handle. Identical strings return the same handle.
    pub fn intern(&mut self, s: &str) -> InternId {
        let bytes = s.as_bytes();
        if let Some(&id) = self.map.get(bytes) {
            return id;
        }
        let offset = self.buffer.len() as u32;
        let len = bytes.len() as u32;
        self.buffer.extend_from_slice(bytes);
        self.buffer.push(0); // null separator
        let id = InternId(self.entries.len() as u32);
        self.entries.push((offset, len));
        self.map.insert(bytes.to_vec(), id);
        id
    }

    /// Look up the string for an InternId.
    pub fn get(&self, id: InternId) -> &str {
        let (offset, len) = self.entries[id.0 as usize];
        let bytes = &self.buffer[offset as usize..(offset + len) as usize];
        // Safety: we only stored valid UTF-8 strings.
        unsafe { std::str::from_utf8_unchecked(bytes) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_map_add_file() {
        let mut sm = SourceMap::new();
        let id = sm.add_file("test.c".into(), "int main() {}\n".into());
        assert_eq!(sm.file_name(id), "test.c");
        assert_eq!(sm.file_content(id), "int main() {}\n");
    }

    #[test]
    fn test_offset_to_line_col() {
        let mut sm = SourceMap::new();
        let id = sm.add_file("test.c".into(), "abc\ndef\nghi\n".into());
        // Line 1, col 1
        assert_eq!(sm.offset_to_line_col(id, 0), (1, 1));
        // Line 1, col 3 ('c')
        assert_eq!(sm.offset_to_line_col(id, 2), (1, 3));
        // Line 2, col 1 ('d')
        assert_eq!(sm.offset_to_line_col(id, 4), (2, 1));
        // Line 3, col 2 ('h')
        assert_eq!(sm.offset_to_line_col(id, 9), (3, 2));
    }

    #[test]
    fn test_span_to_location() {
        let mut sm = SourceMap::new();
        let id = sm.add_file("main.c".into(), "int x;\nint y;\n".into());
        let span = Span::new(id, 7, 12); // "int y"
        let (file, line, col) = sm.span_to_location(span);
        assert_eq!(file, "main.c");
        assert_eq!(line, 2);
        assert_eq!(col, 1);
    }

    #[test]
    fn test_span_text() {
        let mut sm = SourceMap::new();
        let id = sm.add_file("test.c".into(), "hello world".into());
        let span = Span::new(id, 6, 11);
        assert_eq!(sm.span_text(span), "world");
    }

    #[test]
    fn test_dummy_span() {
        let span = Span::dummy();
        assert!(span.is_dummy());
    }

    #[test]
    fn test_span_merge() {
        let f = FileId(0);
        let a = Span::new(f, 5, 10);
        let b = Span::new(f, 8, 15);
        let merged = a.merge(b);
        assert_eq!(merged.lo, 5);
        assert_eq!(merged.hi, 15);
    }

    #[test]
    fn test_string_interner() {
        let mut interner = StringInterner::new();
        let a = interner.intern("hello");
        let b = interner.intern("world");
        let c = interner.intern("hello");
        assert_eq!(a, c); // same string → same id
        assert_ne!(a, b);
        assert_eq!(interner.get(a), "hello");
        assert_eq!(interner.get(b), "world");
    }

    #[test]
    fn test_string_interner_empty() {
        let mut interner = StringInterner::new();
        let id = interner.intern("");
        assert_eq!(interner.get(id), "");
    }

    #[test]
    fn test_line_at_offset() {
        let mut sm = SourceMap::new();
        let id = sm.add_file("t.c".into(), "line one\nline two\nline three\n".into());
        assert_eq!(sm.line_at_offset(id, 0), "line one");
        assert_eq!(sm.line_at_offset(id, 9), "line two");
        assert_eq!(sm.line_at_offset(id, 18), "line three");
    }
}
