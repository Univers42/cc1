// frontend/lexer/mod.rs — C89 Lexer: translation phases 1–3.

pub mod token;

use crate::diagnostics::DiagEngine;
use crate::source::{FileId, SourceMap, Span};
use self::token::{Token, TokenKind};

/// Lex a source file into a token stream.
/// Implements phases 1 (trigraphs), 2 (line splicing), 3 (tokenization).
pub fn lex(source_map: &SourceMap, file: FileId, diag: &DiagEngine) -> Vec<Token> {
    let src = source_map.file_content(file);

    // Phase 1: Trigraph replacement
    let phase1 = replace_trigraphs(src);

    // Phase 2: Backslash-newline splicing
    let phase2 = splice_lines(&phase1);

    // Phase 3: Tokenization
    let mut lexer = Lexer::new(file, &phase2, diag);
    lexer.lex_all()
}

// ── Phase 1: Trigraph Replacement ─────────────────────────────────────

/// Replace all 9 trigraph sequences per C89 §2.2.1.1.
fn replace_trigraphs(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 2 < bytes.len() && bytes[i] == b'?' && bytes[i + 1] == b'?' {
            if let Some(replacement) = trigraph_char(bytes[i + 2]) {
                out.push(replacement);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn trigraph_char(third: u8) -> Option<char> {
    match third {
        b'=' => Some('#'),
        b'/' => Some('\\'),
        b'\'' => Some('^'),
        b'(' => Some('['),
        b')' => Some(']'),
        b'!' => Some('|'),
        b'<' => Some('{'),
        b'>' => Some('}'),
        b'-' => Some('~'),
        _ => None,
    }
}

// ── Phase 2: Line Splicing ────────────────────────────────────────────

/// Delete backslash-newline sequences (join physical lines into logical lines).
fn splice_lines(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            // Skip the backslash-newline pair
            i += 2;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    // C89 §2.1.1.2: source file must end in a newline
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

// ── Phase 3: Lexer State Machine ──────────────────────────────────────

struct Lexer<'a> {
    file: FileId,
    src: &'a [u8],
    pos: usize,
    diag: &'a DiagEngine,
}

impl<'a> Lexer<'a> {
    fn new(file: FileId, src: &'a str, diag: &'a DiagEngine) -> Self {
        Self {
            file,
            src: src.as_bytes(),
            pos: 0,
            diag,
        }
    }

    fn lex_all(&mut self) -> Vec<Token> {
        let mut tokens = Vec::with_capacity(self.src.len() / 4);
        loop {
            self.skip_whitespace_and_comments();
            if self.pos >= self.src.len() {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: self.span(self.pos, self.pos),
                });
                break;
            }
            match self.lex_token() {
                Some(tok) => tokens.push(tok),
                None => {
                    // Skip unexpected character
                    let start = self.pos;
                    self.pos += 1;
                    self.diag.error(
                        self.span(start, self.pos),
                        format!("unexpected character '{}'", self.src[start] as char),
                    );
                }
            }
        }
        // Concatenate adjacent string literals
        concatenate_strings(&mut tokens);
        tokens
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> u8 {
        let b = self.src[self.pos];
        self.pos += 1;
        b
    }

    fn span(&self, lo: usize, hi: usize) -> Span {
        Span::new(self.file, lo as u32, hi as u32)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip whitespace
            while self.pos < self.src.len() && is_space(self.src[self.pos]) {
                self.pos += 1;
            }
            // Skip /* ... */ comments (C89 only has block comments)
            if self.pos + 1 < self.src.len()
                && self.src[self.pos] == b'/'
                && self.src[self.pos + 1] == b'*'
            {
                let start = self.pos;
                self.pos += 2;
                loop {
                    if self.pos + 1 >= self.src.len() {
                        self.diag.error(
                            self.span(start, self.pos),
                            "unterminated block comment",
                        );
                        return;
                    }
                    if self.src[self.pos] == b'*' && self.src[self.pos + 1] == b'/' {
                        self.pos += 2;
                        break;
                    }
                    self.pos += 1;
                }
                continue; // re-check for more whitespace/comments
            }
            break;
        }
    }

    fn lex_token(&mut self) -> Option<Token> {
        let start = self.pos;
        let b = self.peek()?;

        // String literal
        if b == b'"' {
            return Some(self.lex_string_literal(start));
        }

        // Character literal
        if b == b'\'' {
            return Some(self.lex_char_literal(start));
        }

        // Number (integer or float)
        if b.is_ascii_digit() || (b == b'.' && self.peek_at(1).map_or(false, |c| c.is_ascii_digit()))
        {
            return Some(self.lex_number(start));
        }

        // Identifier or keyword
        if is_ident_start(b) {
            return Some(self.lex_identifier(start));
        }

        // Operators and punctuators
        let result = self.lex_punctuator(start);
        if result.is_none() {
            // lex_punctuator advanced past the character but couldn't
            // match it — restore position so the caller can handle it.
            self.pos = start;
        }
        result
    }

    // ── String Literal Lexing ────────────────────────────────────────

    fn lex_string_literal(&mut self, start: usize) -> Token {
        self.advance(); // skip opening "
        let mut bytes = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    self.diag.error(self.span(start, self.pos), "unterminated string literal");
                    break;
                }
                Some(b'"') => {
                    self.advance();
                    break;
                }
                Some(b'\\') => {
                    let escaped = self.lex_escape_sequence();
                    bytes.push(escaped);
                }
                Some(c) => {
                    self.advance();
                    bytes.push(c);
                }
            }
        }
        bytes.push(0); // null terminator
        Token {
            kind: TokenKind::StringLiteral(bytes),
            span: self.span(start, self.pos),
        }
    }

    // ── Character Literal Lexing ─────────────────────────────────────

    fn lex_char_literal(&mut self, start: usize) -> Token {
        self.advance(); // skip opening '
        let value = match self.peek() {
            Some(b'\\') => self.lex_escape_sequence(),
            Some(b'\'') => {
                self.diag.error(self.span(start, self.pos + 1), "empty character constant");
                self.advance();
                0
            }
            Some(c) => {
                self.advance();
                c
            }
            None => {
                self.diag.error(self.span(start, self.pos), "unterminated character constant");
                0
            }
        };
        // Expect closing '
        if self.peek() == Some(b'\'') {
            self.advance();
        } else {
            // Multi-character constant or missing close
            while self.peek().map_or(false, |c| c != b'\'' && c != b'\n') {
                self.advance();
            }
            if self.peek() == Some(b'\'') {
                self.advance();
            }
            self.diag.warning(
                self.span(start, self.pos),
                "multi-character character constant",
            );
        }
        Token {
            kind: TokenKind::CharLiteral(value),
            span: self.span(start, self.pos),
        }
    }

    // ── Escape Sequence ──────────────────────────────────────────────

    fn lex_escape_sequence(&mut self) -> u8 {
        self.advance(); // skip backslash
        match self.peek() {
            Some(b'a') => { self.advance(); 0x07 }
            Some(b'b') => { self.advance(); 0x08 }
            Some(b'f') => { self.advance(); 0x0C }
            Some(b'n') => { self.advance(); 0x0A }
            Some(b'r') => { self.advance(); 0x0D }
            Some(b't') => { self.advance(); 0x09 }
            Some(b'v') => { self.advance(); 0x0B }
            Some(b'\\') => { self.advance(); b'\\' }
            Some(b'\'') => { self.advance(); b'\'' }
            Some(b'"') => { self.advance(); b'"' }
            Some(b'?') => { self.advance(); b'?' }
            Some(b'0'..=b'7') => self.lex_oct_escape(),
            Some(b'x') => {
                self.advance();
                self.lex_hex_escape()
            }
            Some(c) => {
                let start = self.pos - 1;
                self.advance();
                self.diag.warning(
                    self.span(start, self.pos),
                    format!("unknown escape sequence '\\{}'", c as char),
                );
                c
            }
            None => {
                self.diag.error(
                    self.span(self.pos - 1, self.pos),
                    "unexpected end of file in escape sequence",
                );
                0
            }
        }
    }

    fn lex_oct_escape(&mut self) -> u8 {
        let mut val: u32 = 0;
        for _ in 0..3 {
            match self.peek() {
                Some(c @ b'0'..=b'7') => {
                    val = val * 8 + (c - b'0') as u32;
                    self.advance();
                }
                _ => break,
            }
        }
        if val > 255 {
            self.diag.warning(
                self.span(self.pos - 3, self.pos),
                "octal escape sequence out of range",
            );
        }
        val as u8
    }

    fn lex_hex_escape(&mut self) -> u8 {
        let start = self.pos;
        let mut val: u32 = 0;
        let mut count = 0;
        while let Some(c) = self.peek() {
            if let Some(digit) = hex_digit(c) {
                val = val * 16 + digit as u32;
                self.advance();
                count += 1;
            } else {
                break;
            }
        }
        if count == 0 {
            self.diag.error(
                self.span(start - 2, self.pos),
                "\\x used with no following hex digits",
            );
            return 0;
        }
        if val > 255 {
            self.diag.warning(
                self.span(start - 2, self.pos),
                "hex escape sequence out of range",
            );
        }
        val as u8
    }

    // ── Number Lexing ────────────────────────────────────────────────

    fn lex_number(&mut self, start: usize) -> Token {
        // Detect base: 0x (hex), 0 (octal), or decimal
        let first = self.advance();
        let mut is_float = first == b'.';

        if first == b'0' {
            if self.peek() == Some(b'x') || self.peek() == Some(b'X') {
                // Hex integer
                self.advance(); // skip x/X
                let hex_start = self.pos;
                while self.peek().map_or(false, |c| c.is_ascii_hexdigit()) {
                    self.advance();
                }
                if self.pos == hex_start {
                    self.diag.error(self.span(start, self.pos), "invalid hex constant");
                    return Token {
                        kind: TokenKind::IntLiteral(0, token::IntSuffix::None),
                        span: self.span(start, self.pos),
                    };
                }
                let hex_str = std::str::from_utf8(&self.src[hex_start..self.pos]).unwrap_or("0");
                let value = u64::from_str_radix(hex_str, 16).unwrap_or_else(|_| {
                    self.diag.warning(
                        self.span(start, self.pos),
                        "integer constant is too large",
                    );
                    0
                });
                let suffix = self.lex_int_suffix();
                return Token {
                    kind: TokenKind::IntLiteral(value, suffix),
                    span: self.span(start, self.pos),
                };
            }
            // Could be octal or just `0`, or `0.xxx` float
        }

        // Continue consuming digits (decimal or octal digits for now)
        while self.peek().map_or(false, |c| c.is_ascii_digit()) {
            self.advance();
        }

        // Check for float: '.', 'e', 'E'
        if self.peek() == Some(b'.') {
            is_float = true;
            self.advance();
            while self.peek().map_or(false, |c| c.is_ascii_digit()) {
                self.advance();
            }
        }

        if self.peek() == Some(b'e') || self.peek() == Some(b'E') {
            is_float = true;
            self.advance();
            if self.peek() == Some(b'+') || self.peek() == Some(b'-') {
                self.advance();
            }
            let exp_start = self.pos;
            while self.peek().map_or(false, |c| c.is_ascii_digit()) {
                self.advance();
            }
            if self.pos == exp_start {
                self.diag.error(
                    self.span(start, self.pos),
                    "exponent has no digits",
                );
            }
        }

        if is_float {
            let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("0.0");
            let value = text.parse::<f64>().unwrap_or(0.0);
            let suffix = self.lex_float_suffix();
            return Token {
                kind: TokenKind::FloatLiteral(value, suffix),
                span: self.span(start, self.pos),
            };
        }

        // Integer: parse as decimal or octal
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("0");
        let value = if first == b'0' && text.len() > 1 {
            // Octal
            let mut val: u64 = 0;
            let mut had_error = false;
            for &b in &self.src[start + 1..self.pos] {
                if b >= b'0' && b <= b'7' {
                    val = val.wrapping_mul(8).wrapping_add((b - b'0') as u64);
                } else if b.is_ascii_digit() {
                    if !had_error {
                        self.diag.error(
                            self.span(start, self.pos),
                            "invalid digit in octal constant",
                        );
                        had_error = true;
                    }
                }
            }
            val
        } else {
            text.parse::<u64>().unwrap_or_else(|_| {
                self.diag.warning(self.span(start, self.pos), "integer constant is too large");
                0
            })
        };

        let suffix = self.lex_int_suffix();
        Token {
            kind: TokenKind::IntLiteral(value, suffix),
            span: self.span(start, self.pos),
        }
    }

    fn lex_int_suffix(&mut self) -> token::IntSuffix {
        let mut unsigned = false;
        let mut long_count = 0u8;

        loop {
            match self.peek() {
                Some(b'u') | Some(b'U') => {
                    if unsigned {
                        break;
                    }
                    unsigned = true;
                    self.advance();
                }
                Some(b'l') | Some(b'L') => {
                    if long_count >= 2 {
                        break;
                    }
                    long_count += 1;
                    self.advance();
                }
                _ => break,
            }
        }

        match (unsigned, long_count) {
            (false, 0) => token::IntSuffix::None,
            (true, 0) => token::IntSuffix::U,
            (false, 1) => token::IntSuffix::L,
            (true, 1) => token::IntSuffix::UL,
            (false, 2) => token::IntSuffix::LL,
            (true, 2) => token::IntSuffix::ULL,
            _ => token::IntSuffix::None,
        }
    }

    fn lex_float_suffix(&mut self) -> token::FloatSuffix {
        match self.peek() {
            Some(b'f') | Some(b'F') => {
                self.advance();
                token::FloatSuffix::F
            }
            Some(b'l') | Some(b'L') => {
                self.advance();
                token::FloatSuffix::L
            }
            _ => token::FloatSuffix::None,
        }
    }

    // ── Identifier / Keyword Lexing ──────────────────────────────────

    fn lex_identifier(&mut self, start: usize) -> Token {
        while self.peek().map_or(false, |c| is_ident_cont(c)) {
            self.advance();
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        let kind = match keyword_lookup(text) {
            Some(kw) => kw,
            None => TokenKind::Identifier(text.to_string()),
        };
        Token {
            kind,
            span: self.span(start, self.pos),
        }
    }

    // ── Punctuator Lexing (maximal munch) ────────────────────────────

    fn lex_punctuator(&mut self, start: usize) -> Option<Token> {
        let b = self.advance();
        let kind = match b {
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b';' => TokenKind::Semicolon,
            b',' => TokenKind::Comma,
            b'~' => TokenKind::Tilde,
            b'?' => TokenKind::Question,
            b':' => TokenKind::Colon,

            b'+' => match self.peek() {
                Some(b'+') => { self.advance(); TokenKind::PlusPlus }
                Some(b'=') => { self.advance(); TokenKind::PlusEq }
                _ => TokenKind::Plus,
            },
            b'-' => match self.peek() {
                Some(b'-') => { self.advance(); TokenKind::MinusMinus }
                Some(b'=') => { self.advance(); TokenKind::MinusEq }
                Some(b'>') => { self.advance(); TokenKind::Arrow }
                _ => TokenKind::Minus,
            },
            b'*' => match self.peek() {
                Some(b'=') => { self.advance(); TokenKind::StarEq }
                _ => TokenKind::Star,
            },
            b'/' => match self.peek() {
                Some(b'=') => { self.advance(); TokenKind::SlashEq }
                _ => TokenKind::Slash,
            },
            b'%' => match self.peek() {
                Some(b'=') => { self.advance(); TokenKind::PercentEq }
                _ => TokenKind::Percent,
            },
            b'&' => match self.peek() {
                Some(b'&') => { self.advance(); TokenKind::AmpAmp }
                Some(b'=') => { self.advance(); TokenKind::AmpEq }
                _ => TokenKind::Amp,
            },
            b'|' => match self.peek() {
                Some(b'|') => { self.advance(); TokenKind::PipePipe }
                Some(b'=') => { self.advance(); TokenKind::PipeEq }
                _ => TokenKind::Pipe,
            },
            b'^' => match self.peek() {
                Some(b'=') => { self.advance(); TokenKind::CaretEq }
                _ => TokenKind::Caret,
            },
            b'!' => match self.peek() {
                Some(b'=') => { self.advance(); TokenKind::BangEq }
                _ => TokenKind::Bang,
            },
            b'=' => match self.peek() {
                Some(b'=') => { self.advance(); TokenKind::EqEq }
                _ => TokenKind::Eq,
            },
            b'<' => match self.peek() {
                Some(b'<') => {
                    self.advance();
                    match self.peek() {
                        Some(b'=') => { self.advance(); TokenKind::LtLtEq }
                        _ => TokenKind::LtLt,
                    }
                }
                Some(b'=') => { self.advance(); TokenKind::LtEq }
                _ => TokenKind::Lt,
            },
            b'>' => match self.peek() {
                Some(b'>') => {
                    self.advance();
                    match self.peek() {
                        Some(b'=') => { self.advance(); TokenKind::GtGtEq }
                        _ => TokenKind::GtGt,
                    }
                }
                Some(b'=') => { self.advance(); TokenKind::GtEq }
                _ => TokenKind::Gt,
            },
            b'.' => {
                if self.peek() == Some(b'.') && self.peek_at(1) == Some(b'.') {
                    self.advance();
                    self.advance();
                    TokenKind::Ellipsis
                } else {
                    TokenKind::Dot
                }
            }
            b'#' => match self.peek() {
                Some(b'#') => { self.advance(); TokenKind::HashHash }
                _ => TokenKind::Hash,
            },

            _ => return None,
        };

        Some(Token {
            kind,
            span: self.span(start, self.pos),
        })
    }
}

// ── String Literal Concatenation ──────────────────────────────────────

/// Concatenate adjacent string literals into a single token (C89 §3.1.4).
fn concatenate_strings(tokens: &mut Vec<Token>) {
    let mut i = 0;
    while i < tokens.len() {
        if let TokenKind::StringLiteral(_) = &tokens[i].kind {
            let start_span = tokens[i].span;
            let mut combined = if let TokenKind::StringLiteral(ref bytes) = tokens[i].kind {
                // Remove the null terminator before concat
                let mut b = bytes.clone();
                b.pop(); // remove trailing \0
                b
            } else {
                unreachable!()
            };

            let mut j = i + 1;
            while j < tokens.len() {
                if let TokenKind::StringLiteral(ref bytes) = tokens[j].kind {
                    let mut b = bytes.clone();
                    b.pop(); // remove trailing \0
                    combined.extend_from_slice(&b);
                    j += 1;
                } else {
                    break;
                }
            }

            if j > i + 1 {
                combined.push(0); // re-add null terminator
                let end_span = tokens[j - 1].span;
                tokens[i] = Token {
                    kind: TokenKind::StringLiteral(combined),
                    span: start_span.merge(end_span),
                };
                tokens.drain(i + 1..j);
            }
        }
        i += 1;
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0B' | b'\x0C')
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Look up a C89 keyword. Returns None if the identifier is not a keyword.
fn keyword_lookup(s: &str) -> Option<TokenKind> {
    match s {
        "auto" => Some(TokenKind::KwAuto),
        "break" => Some(TokenKind::KwBreak),
        "case" => Some(TokenKind::KwCase),
        "char" => Some(TokenKind::KwChar),
        "const" => Some(TokenKind::KwConst),
        "continue" => Some(TokenKind::KwContinue),
        "default" => Some(TokenKind::KwDefault),
        "do" => Some(TokenKind::KwDo),
        "double" => Some(TokenKind::KwDouble),
        "else" => Some(TokenKind::KwElse),
        "enum" => Some(TokenKind::KwEnum),
        "extern" => Some(TokenKind::KwExtern),
        "float" => Some(TokenKind::KwFloat),
        "for" => Some(TokenKind::KwFor),
        "goto" => Some(TokenKind::KwGoto),
        "if" => Some(TokenKind::KwIf),
        "int" => Some(TokenKind::KwInt),
        "long" => Some(TokenKind::KwLong),
        "register" => Some(TokenKind::KwRegister),
        "return" => Some(TokenKind::KwReturn),
        "short" => Some(TokenKind::KwShort),
        "signed" => Some(TokenKind::KwSigned),
        "sizeof" => Some(TokenKind::KwSizeof),
        "static" => Some(TokenKind::KwStatic),
        "struct" => Some(TokenKind::KwStruct),
        "switch" => Some(TokenKind::KwSwitch),
        "typedef" => Some(TokenKind::KwTypedef),
        "union" => Some(TokenKind::KwUnion),
        "unsigned" => Some(TokenKind::KwUnsigned),
        "void" => Some(TokenKind::KwVoid),
        "volatile" => Some(TokenKind::KwVolatile),
        "while" => Some(TokenKind::KwWhile),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn lex_str(src: &str) -> Vec<Token> {
        let mut sm = SourceMap::new();
        let fid = sm.add_file("test.c".into(), src.into());
        let diag = DiagEngine::new();
        let tokens = lex(&sm, fid, &diag);
        assert!(!diag.has_errors(), "lexer produced errors");
        tokens
    }

    fn lex_str_allow_errors(src: &str) -> (Vec<Token>, DiagEngine) {
        let mut sm = SourceMap::new();
        let fid = sm.add_file("test.c".into(), src.into());
        let diag = DiagEngine::new();
        let tokens = lex(&sm, fid, &diag);
        (tokens, diag)
    }

    #[test]
    fn test_empty_source() {
        let tokens = lex_str("");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(tokens[0].kind, TokenKind::Eof));
    }

    #[test]
    fn test_keywords() {
        let tokens = lex_str("int return void if else while for");
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::KwInt));
        assert!(matches!(kinds[1], TokenKind::KwReturn));
        assert!(matches!(kinds[2], TokenKind::KwVoid));
        assert!(matches!(kinds[3], TokenKind::KwIf));
        assert!(matches!(kinds[4], TokenKind::KwElse));
        assert!(matches!(kinds[5], TokenKind::KwWhile));
        assert!(matches!(kinds[6], TokenKind::KwFor));
    }

    #[test]
    fn test_all_32_keywords() {
        let src = "auto break case char const continue default do double else \
                   enum extern float for goto if int long register return \
                   short signed sizeof static struct switch typedef union \
                   unsigned void volatile while";
        let tokens = lex_str(src);
        // 32 keywords + EOF
        assert_eq!(tokens.len(), 33);
        for t in &tokens[..32] {
            assert!(
                t.kind.is_keyword(),
                "{:?} should be a keyword",
                t.kind
            );
        }
    }

    #[test]
    fn test_identifiers() {
        let tokens = lex_str("foo bar_baz _x123");
        assert!(matches!(&tokens[0].kind, TokenKind::Identifier(s) if s == "foo"));
        assert!(matches!(&tokens[1].kind, TokenKind::Identifier(s) if s == "bar_baz"));
        assert!(matches!(&tokens[2].kind, TokenKind::Identifier(s) if s == "_x123"));
    }

    #[test]
    fn test_integer_literals() {
        let tokens = lex_str("42 0 0x1F 077");
        assert!(matches!(tokens[0].kind, TokenKind::IntLiteral(42, _)));
        assert!(matches!(tokens[1].kind, TokenKind::IntLiteral(0, _)));
        assert!(matches!(tokens[2].kind, TokenKind::IntLiteral(31, _))); // 0x1F
        assert!(matches!(tokens[3].kind, TokenKind::IntLiteral(63, _))); // 077 octal
    }

    #[test]
    fn test_integer_suffixes() {
        let tokens = lex_str("42u 42L 42UL 42ll 42ull");
        assert!(matches!(tokens[0].kind, TokenKind::IntLiteral(42, token::IntSuffix::U)));
        assert!(matches!(tokens[1].kind, TokenKind::IntLiteral(42, token::IntSuffix::L)));
        assert!(matches!(tokens[2].kind, TokenKind::IntLiteral(42, token::IntSuffix::UL)));
        assert!(matches!(tokens[3].kind, TokenKind::IntLiteral(42, token::IntSuffix::LL)));
        assert!(matches!(tokens[4].kind, TokenKind::IntLiteral(42, token::IntSuffix::ULL)));
    }

    #[test]
    fn test_float_literals() {
        let tokens = lex_str("3.14 .5 1e10 1.5e-3 3.14f 1.0L");
        assert!(matches!(tokens[0].kind, TokenKind::FloatLiteral(v, token::FloatSuffix::None) if (v - 3.14).abs() < 1e-9));
        assert!(matches!(tokens[1].kind, TokenKind::FloatLiteral(v, _) if (v - 0.5).abs() < 1e-9));
        assert!(matches!(tokens[2].kind, TokenKind::FloatLiteral(_, token::FloatSuffix::None)));
        assert!(matches!(tokens[3].kind, TokenKind::FloatLiteral(_, _)));
        assert!(matches!(tokens[4].kind, TokenKind::FloatLiteral(_, token::FloatSuffix::F)));
        assert!(matches!(tokens[5].kind, TokenKind::FloatLiteral(_, token::FloatSuffix::L)));
    }

    #[test]
    fn test_string_literal() {
        let tokens = lex_str(r#""hello, world\n""#);
        match &tokens[0].kind {
            TokenKind::StringLiteral(bytes) => {
                assert_eq!(
                    bytes,
                    &[b'h', b'e', b'l', b'l', b'o', b',', b' ', b'w', b'o', b'r', b'l', b'd', 0x0A, 0]
                );
            }
            _ => panic!("expected string literal"),
        }
    }

    #[test]
    fn test_string_concatenation() {
        let tokens = lex_str(r#""hello" " world""#);
        match &tokens[0].kind {
            TokenKind::StringLiteral(bytes) => {
                // "hello world\0"
                assert_eq!(
                    bytes,
                    &[b'h', b'e', b'l', b'l', b'o', b' ', b'w', b'o', b'r', b'l', b'd', 0]
                );
            }
            _ => panic!("expected concatenated string literal"),
        }
        // Should be only 1 string + EOF
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn test_char_literal() {
        let tokens = lex_str("'a' '\\n' '\\x41' '\\0'");
        assert!(matches!(tokens[0].kind, TokenKind::CharLiteral(b'a')));
        assert!(matches!(tokens[1].kind, TokenKind::CharLiteral(0x0A)));
        assert!(matches!(tokens[2].kind, TokenKind::CharLiteral(0x41)));
        assert!(matches!(tokens[3].kind, TokenKind::CharLiteral(0)));
    }

    #[test]
    fn test_operators() {
        let tokens = lex_str("+ - * / % = == != < > <= >= && || !");
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::Plus));
        assert!(matches!(kinds[1], TokenKind::Minus));
        assert!(matches!(kinds[2], TokenKind::Star));
        assert!(matches!(kinds[3], TokenKind::Slash));
        assert!(matches!(kinds[4], TokenKind::Percent));
        assert!(matches!(kinds[5], TokenKind::Eq));
        assert!(matches!(kinds[6], TokenKind::EqEq));
        assert!(matches!(kinds[7], TokenKind::BangEq));
        assert!(matches!(kinds[8], TokenKind::Lt));
        assert!(matches!(kinds[9], TokenKind::Gt));
        assert!(matches!(kinds[10], TokenKind::LtEq));
        assert!(matches!(kinds[11], TokenKind::GtEq));
        assert!(matches!(kinds[12], TokenKind::AmpAmp));
        assert!(matches!(kinds[13], TokenKind::PipePipe));
        assert!(matches!(kinds[14], TokenKind::Bang));
    }

    #[test]
    fn test_compound_operators() {
        let tokens = lex_str("+= -= *= /= %= <<= >>= &= ^= |=");
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::PlusEq));
        assert!(matches!(kinds[1], TokenKind::MinusEq));
        assert!(matches!(kinds[2], TokenKind::StarEq));
        assert!(matches!(kinds[3], TokenKind::SlashEq));
        assert!(matches!(kinds[4], TokenKind::PercentEq));
        assert!(matches!(kinds[5], TokenKind::LtLtEq));
        assert!(matches!(kinds[6], TokenKind::GtGtEq));
        assert!(matches!(kinds[7], TokenKind::AmpEq));
        assert!(matches!(kinds[8], TokenKind::CaretEq));
        assert!(matches!(kinds[9], TokenKind::PipeEq));
    }

    #[test]
    fn test_bitwise_shift_operators() {
        let tokens = lex_str("<< >> & | ^ ~");
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::LtLt));
        assert!(matches!(kinds[1], TokenKind::GtGt));
        assert!(matches!(kinds[2], TokenKind::Amp));
        assert!(matches!(kinds[3], TokenKind::Pipe));
        assert!(matches!(kinds[4], TokenKind::Caret));
        assert!(matches!(kinds[5], TokenKind::Tilde));
    }

    #[test]
    fn test_increment_decrement_arrow() {
        let tokens = lex_str("++ -- ->");
        assert!(matches!(tokens[0].kind, TokenKind::PlusPlus));
        assert!(matches!(tokens[1].kind, TokenKind::MinusMinus));
        assert!(matches!(tokens[2].kind, TokenKind::Arrow));
    }

    #[test]
    fn test_punctuators() {
        let tokens = lex_str("( ) [ ] { } ; , . ...");
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::LParen));
        assert!(matches!(kinds[1], TokenKind::RParen));
        assert!(matches!(kinds[2], TokenKind::LBracket));
        assert!(matches!(kinds[3], TokenKind::RBracket));
        assert!(matches!(kinds[4], TokenKind::LBrace));
        assert!(matches!(kinds[5], TokenKind::RBrace));
        assert!(matches!(kinds[6], TokenKind::Semicolon));
        assert!(matches!(kinds[7], TokenKind::Comma));
        assert!(matches!(kinds[8], TokenKind::Dot));
        assert!(matches!(kinds[9], TokenKind::Ellipsis));
    }

    #[test]
    fn test_trigraphs() {
        // ??= → #, ??( → [, ??) → ]
        let result = replace_trigraphs("??=include ??( ??)");
        assert_eq!(result, "#include [ ]");
    }

    #[test]
    fn test_line_splicing() {
        let result = splice_lines("hel\\\nlo");
        assert_eq!(result, "hello\n");
    }

    #[test]
    fn test_block_comment() {
        let tokens = lex_str("int /* comment */ x;");
        assert!(matches!(tokens[0].kind, TokenKind::KwInt));
        assert!(matches!(&tokens[1].kind, TokenKind::Identifier(s) if s == "x"));
        assert!(matches!(tokens[2].kind, TokenKind::Semicolon));
    }

    #[test]
    fn test_maximal_munch_plusplus() {
        // x+++++y → x ++ ++ + y (maximal munch)
        let tokens = lex_str("x+++++y");
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::Identifier(_)));
        assert!(matches!(kinds[1], TokenKind::PlusPlus));
        assert!(matches!(kinds[2], TokenKind::PlusPlus));
        assert!(matches!(kinds[3], TokenKind::Plus));
        assert!(matches!(kinds[4], TokenKind::Identifier(_)));
    }

    #[test]
    fn test_spans_are_correct() {
        let tokens = lex_str("int x");
        assert_eq!(tokens[0].span.lo, 0);
        assert_eq!(tokens[0].span.hi, 3);
        assert_eq!(tokens[1].span.lo, 4);
        assert_eq!(tokens[1].span.hi, 5);
    }

    #[test]
    fn test_unterminated_comment_error() {
        let (_tokens, diag) = lex_str_allow_errors("int /* unterminated");
        assert!(diag.has_errors());
    }

    #[test]
    fn test_unterminated_string_error() {
        let (_tokens, diag) = lex_str_allow_errors("\"unterminated");
        assert!(diag.has_errors());
    }

    #[test]
    fn test_escape_sequences() {
        let tokens = lex_str(r#""\a\b\f\n\r\t\v\\\"\'""#);
        match &tokens[0].kind {
            TokenKind::StringLiteral(bytes) => {
                assert_eq!(bytes[0], 0x07); // \a
                assert_eq!(bytes[1], 0x08); // \b
                assert_eq!(bytes[2], 0x0C); // \f
                assert_eq!(bytes[3], 0x0A); // \n
                assert_eq!(bytes[4], 0x0D); // \r
                assert_eq!(bytes[5], 0x09); // \t
                assert_eq!(bytes[6], 0x0B); // \v
                assert_eq!(bytes[7], b'\\');
                assert_eq!(bytes[8], b'"');
                assert_eq!(bytes[9], b'\'');
                assert_eq!(bytes[10], 0);   // null terminator
            }
            _ => panic!("expected string literal"),
        }
    }

    #[test]
    fn test_octal_escape() {
        let tokens = lex_str(r#""\101""#); // \101 = 'A' = 65
        match &tokens[0].kind {
            TokenKind::StringLiteral(bytes) => {
                assert_eq!(bytes[0], 65);
            }
            _ => panic!("expected string literal"),
        }
    }

    #[test]
    fn test_hex_escape() {
        let tokens = lex_str(r#""\x41""#); // \x41 = 'A' = 65
        match &tokens[0].kind {
            TokenKind::StringLiteral(bytes) => {
                assert_eq!(bytes[0], 65);
            }
            _ => panic!("expected string literal"),
        }
    }

    #[test]
    fn test_hash_tokens() {
        let tokens = lex_str("# ##");
        assert!(matches!(tokens[0].kind, TokenKind::Hash));
        assert!(matches!(tokens[1].kind, TokenKind::HashHash));
    }

    #[test]
    fn test_ternary_colon() {
        let tokens = lex_str("? :");
        assert!(matches!(tokens[0].kind, TokenKind::Question));
        assert!(matches!(tokens[1].kind, TokenKind::Colon));
    }

    #[test]
    fn test_real_c_function() {
        let src = r#"
int main() {
    return 0;
}
"#;
        let tokens = lex_str(src);
        // int main ( ) { return 0 ; } EOF
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::KwInt));
        assert!(matches!(kinds[1], TokenKind::Identifier(_)));
        assert!(matches!(kinds[2], TokenKind::LParen));
        assert!(matches!(kinds[3], TokenKind::RParen));
        assert!(matches!(kinds[4], TokenKind::LBrace));
        assert!(matches!(kinds[5], TokenKind::KwReturn));
        assert!(matches!(kinds[6], TokenKind::IntLiteral(0, _)));
        assert!(matches!(kinds[7], TokenKind::Semicolon));
        assert!(matches!(kinds[8], TokenKind::RBrace));
        assert!(matches!(kinds[9], TokenKind::Eof));
    }

    #[test]
    fn test_hello_world_program() {
        let src = r#"int main() { printf("hello, world\n"); }"#;
        let tokens = lex_str(src);
        // Should lex without errors
        assert!(tokens.len() > 5);
        assert!(matches!(tokens.last().unwrap().kind, TokenKind::Eof));
    }
}
