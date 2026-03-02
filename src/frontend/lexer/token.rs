// frontend/lexer/token.rs — Token types for C89 lexical analysis.

use crate::source::Span;

/// A single token produced by the lexer.
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Integer literal suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntSuffix {
    None,
    U,
    L,
    UL,
    LL,
    ULL,
}

/// Floating-point literal suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatSuffix {
    None,  // double
    F,     // float
    L,     // long double
}

/// Discriminated union of all C89 token kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // ── End of file ──
    Eof,

    // ── Identifiers and literals ──
    Identifier(String),
    IntLiteral(u64, IntSuffix),
    FloatLiteral(f64, FloatSuffix),
    CharLiteral(u8),
    StringLiteral(Vec<u8>), // includes null terminator

    // ── C89 Keywords (32 total) ──
    KwAuto,
    KwBreak,
    KwCase,
    KwChar,
    KwConst,
    KwContinue,
    KwDefault,
    KwDo,
    KwDouble,
    KwElse,
    KwEnum,
    KwExtern,
    KwFloat,
    KwFor,
    KwGoto,
    KwIf,
    KwInt,
    KwLong,
    KwRegister,
    KwReturn,
    KwShort,
    KwSigned,
    KwSizeof,
    KwStatic,
    KwStruct,
    KwSwitch,
    KwTypedef,
    KwUnion,
    KwUnsigned,
    KwVoid,
    KwVolatile,
    KwWhile,

    // ── Operators ──
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    Amp,        // &
    Pipe,       // |
    Caret,      // ^
    Tilde,      // ~
    Bang,       // !
    Eq,         // =
    Lt,         // <
    Gt,         // >

    PlusPlus,   // ++
    MinusMinus, // --
    Arrow,      // ->
    PlusEq,     // +=
    MinusEq,    // -=
    StarEq,     // *=
    SlashEq,    // /=
    PercentEq,  // %=
    AmpEq,      // &=
    PipeEq,     // |=
    CaretEq,    // ^=
    LtLtEq,     // <<=
    GtGtEq,     // >>=

    EqEq,       // ==
    BangEq,     // !=
    LtEq,       // <=
    GtEq,       // >=
    LtLt,       // <<
    GtGt,       // >>

    AmpAmp,     // &&
    PipePipe,   // ||

    // ── Punctuators ──
    LParen,     // (
    RParen,     // )
    LBracket,   // [
    RBracket,   // ]
    LBrace,     // {
    RBrace,     // }
    Semicolon,  // ;
    Comma,      // ,
    Dot,        // .
    Ellipsis,   // ...
    Question,   // ?
    Colon,      // :
    Hash,       // #
    HashHash,   // ##
}

impl TokenKind {
    /// Returns true if this token is a C89 keyword.
    pub fn is_keyword(&self) -> bool {
        matches!(
            self,
            TokenKind::KwAuto
                | TokenKind::KwBreak
                | TokenKind::KwCase
                | TokenKind::KwChar
                | TokenKind::KwConst
                | TokenKind::KwContinue
                | TokenKind::KwDefault
                | TokenKind::KwDo
                | TokenKind::KwDouble
                | TokenKind::KwElse
                | TokenKind::KwEnum
                | TokenKind::KwExtern
                | TokenKind::KwFloat
                | TokenKind::KwFor
                | TokenKind::KwGoto
                | TokenKind::KwIf
                | TokenKind::KwInt
                | TokenKind::KwLong
                | TokenKind::KwRegister
                | TokenKind::KwReturn
                | TokenKind::KwShort
                | TokenKind::KwSigned
                | TokenKind::KwSizeof
                | TokenKind::KwStatic
                | TokenKind::KwStruct
                | TokenKind::KwSwitch
                | TokenKind::KwTypedef
                | TokenKind::KwUnion
                | TokenKind::KwUnsigned
                | TokenKind::KwVoid
                | TokenKind::KwVolatile
                | TokenKind::KwWhile
        )
    }

    /// Returns true if this token is a type specifier keyword.
    pub fn is_type_specifier(&self) -> bool {
        matches!(
            self,
            TokenKind::KwVoid
                | TokenKind::KwChar
                | TokenKind::KwShort
                | TokenKind::KwInt
                | TokenKind::KwLong
                | TokenKind::KwFloat
                | TokenKind::KwDouble
                | TokenKind::KwSigned
                | TokenKind::KwUnsigned
                | TokenKind::KwStruct
                | TokenKind::KwUnion
                | TokenKind::KwEnum
        )
    }

    /// Returns true if this token is a storage class specifier.
    pub fn is_storage_class(&self) -> bool {
        matches!(
            self,
            TokenKind::KwAuto
                | TokenKind::KwRegister
                | TokenKind::KwStatic
                | TokenKind::KwExtern
                | TokenKind::KwTypedef
        )
    }

    /// Returns true if this is a type qualifier.
    pub fn is_type_qualifier(&self) -> bool {
        matches!(self, TokenKind::KwConst | TokenKind::KwVolatile)
    }

    /// Returns a human-readable name for error messages.
    pub fn describe(&self) -> &'static str {
        match self {
            TokenKind::Eof => "end of file",
            TokenKind::Identifier(_) => "identifier",
            TokenKind::IntLiteral(_, _) => "integer constant",
            TokenKind::FloatLiteral(_, _) => "floating constant",
            TokenKind::CharLiteral(_) => "character constant",
            TokenKind::StringLiteral(_) => "string literal",
            TokenKind::KwAuto => "'auto'",
            TokenKind::KwBreak => "'break'",
            TokenKind::KwCase => "'case'",
            TokenKind::KwChar => "'char'",
            TokenKind::KwConst => "'const'",
            TokenKind::KwContinue => "'continue'",
            TokenKind::KwDefault => "'default'",
            TokenKind::KwDo => "'do'",
            TokenKind::KwDouble => "'double'",
            TokenKind::KwElse => "'else'",
            TokenKind::KwEnum => "'enum'",
            TokenKind::KwExtern => "'extern'",
            TokenKind::KwFloat => "'float'",
            TokenKind::KwFor => "'for'",
            TokenKind::KwGoto => "'goto'",
            TokenKind::KwIf => "'if'",
            TokenKind::KwInt => "'int'",
            TokenKind::KwLong => "'long'",
            TokenKind::KwRegister => "'register'",
            TokenKind::KwReturn => "'return'",
            TokenKind::KwShort => "'short'",
            TokenKind::KwSigned => "'signed'",
            TokenKind::KwSizeof => "'sizeof'",
            TokenKind::KwStatic => "'static'",
            TokenKind::KwStruct => "'struct'",
            TokenKind::KwSwitch => "'switch'",
            TokenKind::KwTypedef => "'typedef'",
            TokenKind::KwUnion => "'union'",
            TokenKind::KwUnsigned => "'unsigned'",
            TokenKind::KwVoid => "'void'",
            TokenKind::KwVolatile => "'volatile'",
            TokenKind::KwWhile => "'while'",
            TokenKind::Plus => "'+'",
            TokenKind::Minus => "'-'",
            TokenKind::Star => "'*'",
            TokenKind::Slash => "'/'",
            TokenKind::Percent => "'%'",
            TokenKind::Amp => "'&'",
            TokenKind::Pipe => "'|'",
            TokenKind::Caret => "'^'",
            TokenKind::Tilde => "'~'",
            TokenKind::Bang => "'!'",
            TokenKind::Eq => "'='",
            TokenKind::Lt => "'<'",
            TokenKind::Gt => "'>'",
            TokenKind::PlusPlus => "'++'",
            TokenKind::MinusMinus => "'--'",
            TokenKind::Arrow => "'->'",
            TokenKind::PlusEq => "'+='",
            TokenKind::MinusEq => "'-='",
            TokenKind::StarEq => "'*='",
            TokenKind::SlashEq => "'/='",
            TokenKind::PercentEq => "'%='",
            TokenKind::AmpEq => "'&='",
            TokenKind::PipeEq => "'|='",
            TokenKind::CaretEq => "'^='",
            TokenKind::LtLtEq => "'<<='",
            TokenKind::GtGtEq => "'>>='",
            TokenKind::EqEq => "'=='",
            TokenKind::BangEq => "'!='",
            TokenKind::LtEq => "'<='",
            TokenKind::GtEq => "'>='",
            TokenKind::LtLt => "'<<'",
            TokenKind::GtGt => "'>>'",
            TokenKind::AmpAmp => "'&&'",
            TokenKind::PipePipe => "'||'",
            TokenKind::LParen => "'('",
            TokenKind::RParen => "')'",
            TokenKind::LBracket => "'['",
            TokenKind::RBracket => "']'",
            TokenKind::LBrace => "'{'",
            TokenKind::RBrace => "'}'",
            TokenKind::Semicolon => "';'",
            TokenKind::Comma => "','",
            TokenKind::Dot => "'.'",
            TokenKind::Ellipsis => "'...'",
            TokenKind::Question => "'?'",
            TokenKind::Colon => "':'",
            TokenKind::Hash => "'#'",
            TokenKind::HashHash => "'##'",
        }
    }
}

impl PartialEq<f64> for FloatSuffix {
    fn eq(&self, _other: &f64) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_keyword() {
        assert!(TokenKind::KwInt.is_keyword());
        assert!(TokenKind::KwReturn.is_keyword());
        assert!(!TokenKind::Plus.is_keyword());
        assert!(!TokenKind::Identifier("foo".into()).is_keyword());
    }

    #[test]
    fn test_is_type_specifier() {
        assert!(TokenKind::KwInt.is_type_specifier());
        assert!(TokenKind::KwVoid.is_type_specifier());
        assert!(TokenKind::KwStruct.is_type_specifier());
        assert!(!TokenKind::KwReturn.is_type_specifier());
    }

    #[test]
    fn test_is_storage_class() {
        assert!(TokenKind::KwStatic.is_storage_class());
        assert!(TokenKind::KwExtern.is_storage_class());
        assert!(!TokenKind::KwInt.is_storage_class());
    }

    #[test]
    fn test_describe() {
        assert_eq!(TokenKind::KwInt.describe(), "'int'");
        assert_eq!(TokenKind::Eof.describe(), "end of file");
        assert_eq!(TokenKind::Semicolon.describe(), "';'");
    }
}
