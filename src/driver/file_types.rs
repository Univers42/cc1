// driver/file_types.rs — Input file classification.
//
// Classifies input files by extension and magic bytes: object/archive detection,
// C source detection, assembly source detection, explicit language override,
// line marker stripping for -P, and binary object probing.

use std::fs;
use std::io::Read;

// ── Object / Archive detection ─────────────────────────────────────────

/// Check if a file is an object file or archive by extension.
///
/// Recognises:
///   - Standard:     `.o`, `.a`, `.so`
///   - Non-standard: `.os`, `.od` (heatshrink), `.lo` (libtool), `.obj` (Windows)
///   - Versioned:    `.so.1`, `.so.1.2.3`, etc.
///   - Suffixed:     `.a.xyzzy` (skarnet.org build system, filename only)
pub fn is_object_or_archive(path: &str) -> bool {
    // Standard extensions
    if path.ends_with(".o")
        || path.ends_with(".a")
        || path.ends_with(".so")
    {
        return true;
    }

    // Non-standard extensions used by build systems
    if path.ends_with(".os")
        || path.ends_with(".od")
        || path.ends_with(".lo")
        || path.ends_with(".obj")
    {
        return true;
    }

    // Versioned shared libraries: .so.1, .so.1.2.3, etc.
    if let Some(pos) = path.rfind(".so.") {
        let rest = &path[pos + 4..];
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return true;
        }
    }

    // Suffixed static archives: .a.xyzzy (skarnet.org build system).
    // Only match in the filename component to avoid false positives in directory names.
    if let Some(fname) = std::path::Path::new(path).file_name().and_then(|f| f.to_str()) {
        if let Some(pos) = fname.find(".a.") {
            if pos > 0 {
                return true;
            }
        }
    }

    false
}

// ── C source detection ─────────────────────────────────────────────────

/// Check if a file is a C source file by extension.
pub fn is_c_source(path: &str) -> bool {
    path.ends_with(".c") || path.ends_with(".h") || path.ends_with(".i")
}

// ── Assembly source detection ──────────────────────────────────────────

/// Check if a file is an assembly source file by extension.
/// `.s` is pure assembly; `.S` needs the C preprocessor first.
pub fn is_assembly_source(path: &str) -> bool {
    path.ends_with(".s") || path.ends_with(".S")
}

/// Check if the assembly source file needs C preprocessing (`.S`).
pub fn is_assembly_with_cpp(path: &str) -> bool {
    path.ends_with(".S")
}

/// Check if the explicit -x language override indicates assembly.
pub fn is_explicit_assembly(lang: Option<&str>) -> bool {
    matches!(lang, Some("assembler") | Some("assembler-with-cpp"))
}

// ── Magic byte probing ─────────────────────────────────────────────────

/// Detect a binary object file or archive by reading the first 8 bytes.
///
/// Returns `true` for:
///   - `\x7fELF` → ELF object file
///   - `!<arch>\n` → ar archive
pub fn looks_like_binary_object(path: &str) -> bool {
    let mut buf = [0u8; 8];
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let Ok(n) = file.read(&mut buf) else {
        return false;
    };
    if n < 4 {
        return false;
    }
    // ELF magic
    if buf[..4] == *b"\x7fELF" {
        return true;
    }
    // ar archive magic
    if n >= 8 && buf[..8] == *b"!<arch>\n" {
        return true;
    }
    false
}

// ── Line marker stripping ──────────────────────────────────────────────

/// Strip `# <line> "file"` markers from preprocessed output (for the -P flag).
pub fn strip_line_markers(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("# ") {
            // Check if it looks like a line marker: # <number> "file"
            let rest = &trimmed[2..];
            if rest.starts_with(|c: char| c.is_ascii_digit()) {
                continue; // Skip line markers
            }
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_object_extensions() {
        assert!(is_object_or_archive("foo.o"));
        assert!(is_object_or_archive("path/to/libfoo.a"));
        assert!(is_object_or_archive("libfoo.so"));
    }

    #[test]
    fn test_nonstandard_object_extensions() {
        assert!(is_object_or_archive("foo.os"));
        assert!(is_object_or_archive("foo.od"));
        assert!(is_object_or_archive("foo.lo"));
        assert!(is_object_or_archive("foo.obj"));
    }

    #[test]
    fn test_versioned_shared_libraries() {
        assert!(is_object_or_archive("libfoo.so.1"));
        assert!(is_object_or_archive("libfoo.so.1.2.3"));
        assert!(is_object_or_archive("libfoo.so.42"));
    }

    #[test]
    fn test_suffixed_static_archives() {
        assert!(is_object_or_archive("libfoo.a.xyzzy"));
        assert!(is_object_or_archive("/path/to/lib.a.suffix"));
    }

    #[test]
    fn test_non_object_files() {
        assert!(!is_object_or_archive("foo.c"));
        assert!(!is_object_or_archive("foo.s"));
        assert!(!is_object_or_archive("foo.h"));
        assert!(!is_object_or_archive("foo.rs"));
    }

    #[test]
    fn test_c_source_detection() {
        assert!(is_c_source("foo.c"));
        assert!(is_c_source("path/to/bar.h"));
        assert!(is_c_source("baz.i"));
        assert!(!is_c_source("foo.s"));
        assert!(!is_c_source("foo.o"));
    }

    #[test]
    fn test_assembly_source_detection() {
        assert!(is_assembly_source("foo.s"));
        assert!(is_assembly_source("foo.S"));
        assert!(is_assembly_with_cpp("foo.S"));
        assert!(!is_assembly_with_cpp("foo.s"));
        assert!(!is_assembly_source("foo.c"));
    }

    #[test]
    fn test_explicit_assembly() {
        assert!(is_explicit_assembly(Some("assembler")));
        assert!(is_explicit_assembly(Some("assembler-with-cpp")));
        assert!(!is_explicit_assembly(Some("c")));
        assert!(!is_explicit_assembly(None));
    }

    #[test]
    fn test_strip_line_markers() {
        let input = "# 1 \"test.c\"\nint x;\n# 2 \"test.c\"\nint y;\n";
        let result = strip_line_markers(input);
        assert_eq!(result, "int x;\nint y;\n");
    }

    #[test]
    fn test_strip_line_markers_preserves_pragmas() {
        // # pragma should NOT be stripped (it doesn't start with a digit)
        let input = "# pragma once\nint x;\n# 1 \"test.c\"\nint y;\n";
        let result = strip_line_markers(input);
        assert!(result.contains("# pragma once"));
        assert!(!result.contains("# 1 \"test.c\""));
    }
}
