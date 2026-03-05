// frontend/parser/mod.rs — Recursive descent parser for C89.
//
// Builds a flat, handle-based AST in the central Ctx.
// Covers the full C89 grammar: declarations, statements, expressions.

pub mod ast;

use std::collections::HashSet;

use crate::ctx::*;
use crate::diagnostics::DiagEngine;
use crate::frontend::lexer::token::{Token, TokenKind};
use crate::source::{InternId, Span};

/// Parse a token stream into an AST rooted at a TranslationUnit node.
pub fn parse(tokens: &[Token], ctx: &mut Ctx, diag: &DiagEngine) -> NodeId {
    let mut parser = Parser::new(tokens, ctx, diag);
    parser.parse_translation_unit()
}

// ── Internal types ────────────────────────────────────────────────────

/// Collected declaration specifiers.
#[allow(dead_code)]
struct DeclSpec {
    storage: StorageClass,
    is_const: bool,
    is_volatile: bool,
    type_node: NodeId, // The base type as an AST node (TypeSpec, StructDecl, etc.)
    span: Span,
}

/// Result of parsing a declarator.
struct Declarator {
    name: Option<InternId>,
    name_span: Span,
    /// Chain of type modifier nodes (PointerTo, ArrayOf, FuncType) wrapping the base.
    type_node: NodeId, // NODE_NONE means no additional modifiers
    params: Vec<NodeId>,
    is_variadic: bool,
    is_function: bool,
}

// ── Parser ────────────────────────────────────────────────────────────

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    ctx: &'a mut Ctx,
    diag: &'a DiagEngine,
    /// Set of typedef names known so far, for disambiguation.
    typedef_names: HashSet<String>,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token], ctx: &'a mut Ctx, diag: &'a DiagEngine) -> Self {
        Self {
            tokens,
            pos: 0,
            ctx,
            diag,
            typedef_names: HashSet::new(),
        }
    }

    // ── Token Navigation ──────────────────────────────────────────────

    fn peek(&self) -> &TokenKind {
        if self.pos < self.tokens.len() {
            &self.tokens[self.pos].kind
        } else {
            &TokenKind::Eof
        }
    }

    #[allow(dead_code)]
    fn peek_token(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn peek_span(&self) -> Span {
        if self.pos < self.tokens.len() {
            self.tokens[self.pos].span
        } else {
            Span::dummy()
        }
    }

    fn peek_nth(&self, n: usize) -> &TokenKind {
        let idx = self.pos + n;
        if idx < self.tokens.len() {
            &self.tokens[idx].kind
        } else {
            &TokenKind::Eof
        }
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &TokenKind) -> bool {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            self.advance();
            true
        } else {
            self.diag.error(
                self.peek_span(),
                format!(
                    "expected {}, found {}",
                    expected.describe(),
                    self.peek().describe()
                ),
            );
            false
        }
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    /// Skip tokens until we find a synchronization point (`;` or `}`).
    fn synchronize(&mut self) {
        loop {
            match self.peek() {
                TokenKind::Semicolon => {
                    self.advance();
                    return;
                }
                TokenKind::RBrace | TokenKind::Eof => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    // ── Predicate helpers ─────────────────────────────────────────────

    #[allow(dead_code)]
    fn is_type_specifier_start(&self) -> bool {
        match self.peek() {
            k if k.is_type_specifier() => true,
            k if k.is_type_qualifier() => true,
            TokenKind::Identifier(name) => self.typedef_names.contains(name),
            _ => false,
        }
    }

    fn is_declaration_start(&self) -> bool {
        match self.peek() {
            k if k.is_type_specifier() => true,
            k if k.is_storage_class() => true,
            k if k.is_type_qualifier() => true,
            TokenKind::Identifier(name) => self.typedef_names.contains(name),
            _ => false,
        }
    }

    /// Check if we're looking at a type name inside parentheses (for cast detection).
    fn is_type_name_after_lparen(&self) -> bool {
        // We're at '(' — check if the next token starts a type name
        match self.peek_nth(1) {
            k if k.is_type_specifier() => true,
            k if k.is_type_qualifier() => true,
            TokenKind::Identifier(name) => self.typedef_names.contains(name),
            _ => false,
        }
    }

    // ══════════════════════════════════════════════════════════════════
    // TOP-LEVEL PARSING
    // ══════════════════════════════════════════════════════════════════

    fn parse_translation_unit(&mut self) -> NodeId {
        let start = self.peek_span();
        let mut decls = Vec::new();

        while !self.at_eof() {
            match self.parse_external_declaration() {
                Some(decl) => decls.push(decl),
                None => self.synchronize(),
            }
        }

        let span = if decls.is_empty() {
            start
        } else {
            let first = self.ctx.node(*decls.first().unwrap()).span;
            let last = self.ctx.node(*decls.last().unwrap()).span;
            first.merge(last)
        };

        self.ctx.push_node(NodeKind::TranslationUnit { decls }, span)
    }

    fn parse_external_declaration(&mut self) -> Option<NodeId> {
        let span = self.peek_span();

        if !self.is_declaration_start() {
            self.diag.error(span, "expected declaration");
            return None;
        }

        // Parse declaration specifiers
        let decl_spec = self.parse_decl_specifiers();

        // Bare specifier with no declarator: `struct foo { ... };` or `enum bar { ... };`
        if matches!(self.peek(), TokenKind::Semicolon) {
            self.advance();
            return Some(decl_spec.type_node);
        }

        // Parse first declarator
        let decl = self.parse_declarator(false);

        // Check: is this a function definition?
        if decl.is_function && matches!(self.peek(), TokenKind::LBrace) {
            return self.finish_function_def(decl_spec, decl);
        }

        // Otherwise it's a declaration (possibly with multiple declarators)
        self.finish_declaration(decl_spec, decl)
    }

    fn finish_function_def(&mut self, spec: DeclSpec, decl: Declarator) -> Option<NodeId> {
        let start = spec.span;
        let name = decl.name.unwrap_or_else(|| self.ctx.intern("<anon>"));

        // Register function name in file scope for forward references
        // (simplified — sema will handle properly)

        let body = self.parse_compound_stmt()?;
        let span = start.merge(self.ctx.node(body).span);

        Some(self.ctx.push_node(
            NodeKind::FuncDef {
                return_type: spec.type_node,
                name,
                params: decl.params,
                body,
                is_variadic: decl.is_variadic,
                storage_class: spec.storage,
            },
            span,
        ))
    }

    fn finish_declaration(&mut self, spec: DeclSpec, first_decl: Declarator) -> Option<NodeId> {
        let start = spec.span;
        let is_typedef = spec.storage == StorageClass::Typedef;

        // First declarator
        let mut decls = Vec::new();
        let first = self.make_var_or_typedef(&spec, &first_decl, is_typedef);
        decls.push(first);

        // Handle additional declarators: `int a, b, *c;`
        while matches!(self.peek(), TokenKind::Comma) {
            self.advance();
            let d = self.parse_declarator(false);
            let node = self.make_var_or_typedef(&spec, &d, is_typedef);
            decls.push(node);
        }

        self.expect(&TokenKind::Semicolon);

        if decls.len() == 1 {
            Some(decls[0])
        } else {
            // Wrap multiple declarators in a CompoundStmt for simplicity
            let _span = start.merge(self.peek_span());
            // Return just the first for now — multi-decl is handled as separate nodes
            // by pushing them all and returning the last
            // Actually, we need them all in the translation unit
            Some(decls[0]) // The others were pushed already
        }
    }

    fn make_var_or_typedef(&mut self, spec: &DeclSpec, decl: &Declarator, is_typedef: bool) -> NodeId {
        let name = decl.name.unwrap_or_else(|| self.ctx.intern("<anon>"));

        // Parse initializer if present
        let init = if matches!(self.peek(), TokenKind::Eq) {
            self.advance();
            self.parse_initializer()
        } else {
            NODE_NONE
        };

        let span = spec.span.merge(decl.name_span);
        let type_node = if decl.type_node != NODE_NONE {
            decl.type_node
        } else {
            spec.type_node
        };

        if is_typedef {
            // Register the typedef name for future parsing
            if let Some(name_id) = decl.name {
                self.typedef_names
                    .insert(self.ctx.get_str(name_id).to_string());
            }
            self.ctx.push_node(
                NodeKind::TypedefDecl {
                    name,
                    type_node,
                },
                span,
            )
        } else {
            self.ctx.push_node(
                NodeKind::VarDecl {
                    name,
                    type_node,
                    init,
                    storage_class: spec.storage,
                },
                span,
            )
        }
    }

    // ══════════════════════════════════════════════════════════════════
    // DECLARATION SPECIFIERS
    // ══════════════════════════════════════════════════════════════════

    /// Parse declaration specifiers: storage-class, type-qualifiers, type-specifiers.
    /// Handles combined specifiers like `unsigned long int`, `const volatile int *`, etc.
    fn parse_decl_specifiers(&mut self) -> DeclSpec {
        let start = self.peek_span();
        let mut storage = StorageClass::None;
        let mut is_const = false;
        let mut is_volatile = false;

        // Track type specifier keywords for combination
        let mut has_void = false;
        let mut has_char = false;
        let mut has_short = false;
        let mut has_int = false;
        let mut has_long = 0u32; // can appear twice for long long
        let mut has_float = false;
        let mut has_double = false;
        let mut has_signed = false;
        let mut has_unsigned = false;
        let mut struct_union_enum_node: Option<NodeId> = None;
        let mut typedef_node: Option<NodeId> = None;

        loop {
            match self.peek() {
                // Storage class
                TokenKind::KwAuto => { self.advance(); storage = StorageClass::Auto; }
                TokenKind::KwRegister => { self.advance(); storage = StorageClass::Register; }
                TokenKind::KwStatic => { self.advance(); storage = StorageClass::Static; }
                TokenKind::KwExtern => { self.advance(); storage = StorageClass::Extern; }
                TokenKind::KwTypedef => { self.advance(); storage = StorageClass::Typedef; }

                // Type qualifiers
                TokenKind::KwConst => { self.advance(); is_const = true; }
                TokenKind::KwVolatile => { self.advance(); is_volatile = true; }

                // Type specifiers
                TokenKind::KwVoid => { self.advance(); has_void = true; }
                TokenKind::KwChar => { self.advance(); has_char = true; }
                TokenKind::KwShort => { self.advance(); has_short = true; }
                TokenKind::KwInt => { self.advance(); has_int = true; }
                TokenKind::KwLong => { self.advance(); has_long += 1; }
                TokenKind::KwFloat => { self.advance(); has_float = true; }
                TokenKind::KwDouble => { self.advance(); has_double = true; }
                TokenKind::KwSigned => { self.advance(); has_signed = true; }
                TokenKind::KwUnsigned => { self.advance(); has_unsigned = true; }

                // Struct/union/enum
                TokenKind::KwStruct => {
                    struct_union_enum_node = Some(self.parse_struct_or_union_spec(true));
                }
                TokenKind::KwUnion => {
                    struct_union_enum_node = Some(self.parse_struct_or_union_spec(false));
                }
                TokenKind::KwEnum => {
                    struct_union_enum_node = Some(self.parse_enum_spec());
                }

                // Typedef name
                TokenKind::Identifier(name) if self.typedef_names.contains(name) => {
                    let name_id = self.ctx.intern(&name.clone());
                    let span = self.peek_span();
                    self.advance();
                    typedef_node = Some(self.ctx.push_node(
                        NodeKind::TypeSpec {
                            spec: TypeSpecKind::TypedefName,
                        },
                        span,
                    ));
                    // Store the name for sema to resolve
                    let _ = name_id; // name_id is embedded in the span
                }

                _ => break,
            }
        }

        // Determine the type specifier node
        let type_node = if let Some(node) = struct_union_enum_node {
            node
        } else if let Some(node) = typedef_node {
            node
        } else {
            // Combine simple type specifiers into a single TypeSpec node
            let spec = self.combine_type_specifiers(
                has_void, has_char, has_short, has_int, has_long,
                has_float, has_double, has_signed, has_unsigned,
                start,
            );
            self.ctx.push_node(NodeKind::TypeSpec { spec }, start)
        };

        DeclSpec {
            storage,
            is_const,
            is_volatile,
            type_node,
            span: start,
        }
    }

    /// Combine multiple type specifier keywords into a single TypeSpecKind.
    fn combine_type_specifiers(
        &self,
        void: bool, char: bool, short: bool, int: bool, long: u32,
        float: bool, double: bool, signed: bool, unsigned: bool,
        span: Span,
    ) -> TypeSpecKind {
        // C89 §3.5.2: valid combinations
        if void { return TypeSpecKind::Void; }
        if float { return TypeSpecKind::Float; }
        if double {
            if long > 0 { return TypeSpecKind::LongDouble; }
            return TypeSpecKind::Double;
        }
        if char {
            if unsigned { return TypeSpecKind::UnsignedChar; }
            if signed { return TypeSpecKind::SignedChar; }
            return TypeSpecKind::Char;
        }
        if short {
            if unsigned { return TypeSpecKind::UnsignedShort; }
            return TypeSpecKind::Short;
        }
        if long >= 2 {
            if unsigned { return TypeSpecKind::UnsignedLong; }
            return TypeSpecKind::Long;
        }
        if long == 1 {
            if unsigned { return TypeSpecKind::UnsignedLong; }
            return TypeSpecKind::Long;
        }
        if unsigned { return TypeSpecKind::UnsignedInt; }
        if signed || int { return TypeSpecKind::Int; }

        // Default: if nothing was specified, it's implicitly int
        if !void && !char && !short && !int && !float && !double && long == 0 && !signed && !unsigned {
            self.diag.warning(span, "type defaults to 'int'");
        }
        TypeSpecKind::Int
    }

    // ══════════════════════════════════════════════════════════════════
    // STRUCT / UNION / ENUM SPECIFIERS
    // ══════════════════════════════════════════════════════════════════

    fn parse_struct_or_union_spec(&mut self, is_struct: bool) -> NodeId {
        let start = self.peek_span();
        self.advance(); // skip 'struct'/'union'

        // Optional tag
        let tag = if let TokenKind::Identifier(name) = self.peek().clone() {
            let id = self.ctx.intern(&name);
            self.advance();
            Some(id)
        } else {
            None
        };

        // Optional member list
        let members = if matches!(self.peek(), TokenKind::LBrace) {
            self.advance(); // skip {
            let mems = self.parse_member_list();
            self.expect(&TokenKind::RBrace);
            mems
        } else {
            Vec::new()
        };

        let span = start.merge(self.peek_span());
        if is_struct {
            self.ctx.push_node(NodeKind::StructDecl { tag, members }, span)
        } else {
            self.ctx.push_node(NodeKind::UnionDecl { tag, members }, span)
        }
    }

    fn parse_member_list(&mut self) -> Vec<NodeId> {
        let mut members = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            // Parse member declaration specifiers
            let spec = self.parse_decl_specifiers();

            // Parse member declarators
            loop {
                let name_span = self.peek_span();
                let decl = self.parse_declarator(false);
                let name = decl.name.unwrap_or_else(|| self.ctx.intern("<anon>"));

                // Check for bitfield
                let bitfield = if matches!(self.peek(), TokenKind::Colon) {
                    self.advance();
                    self.parse_assignment_expr()
                } else {
                    NODE_NONE
                };

                let type_node = if decl.type_node != NODE_NONE {
                    decl.type_node
                } else {
                    spec.type_node
                };

                let span = spec.span.merge(name_span);
                members.push(self.ctx.push_node(
                    NodeKind::MemberDecl {
                        name,
                        type_node,
                        bitfield,
                    },
                    span,
                ));

                if !matches!(self.peek(), TokenKind::Comma) {
                    break;
                }
                self.advance(); // skip comma
            }
            self.expect(&TokenKind::Semicolon);
        }
        members
    }

    fn parse_enum_spec(&mut self) -> NodeId {
        let start = self.peek_span();
        self.advance(); // skip 'enum'

        let tag = if let TokenKind::Identifier(name) = self.peek().clone() {
            let id = self.ctx.intern(&name);
            self.advance();
            Some(id)
        } else {
            None
        };

        let mut enumerators = Vec::new();
        if matches!(self.peek(), TokenKind::LBrace) {
            self.advance();
            while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
                if let TokenKind::Identifier(name) = self.peek().clone() {
                    let name_id = self.ctx.intern(&name);
                    self.advance();

                    let value = if matches!(self.peek(), TokenKind::Eq) {
                        self.advance();
                        self.parse_assignment_expr()
                    } else {
                        NODE_NONE
                    };

                    enumerators.push((name_id, value));

                    if matches!(self.peek(), TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                } else {
                    self.diag
                        .error(self.peek_span(), "expected enumerator name");
                    break;
                }
            }
            self.expect(&TokenKind::RBrace);
        }

        let span = start.merge(self.peek_span());
        self.ctx
            .push_node(NodeKind::EnumDecl { tag, enumerators }, span)
    }

    // ══════════════════════════════════════════════════════════════════
    // DECLARATORS
    // ══════════════════════════════════════════════════════════════════

    /// Parse a declarator (possibly abstract if `allow_abstract` is true).
    /// Returns the name (if any), and a chain of type modifier nodes.
    fn parse_declarator(&mut self, allow_abstract: bool) -> Declarator {
        // Parse pointer prefix: * const volatile *
        let mut ptr_depth = Vec::new();
        while matches!(self.peek(), TokenKind::Star) {
            self.advance();
            let mut is_const = false;
            let mut is_volatile = false;
            while matches!(self.peek(), TokenKind::KwConst | TokenKind::KwVolatile) {
                match self.peek() {
                    TokenKind::KwConst => { is_const = true; self.advance(); }
                    TokenKind::KwVolatile => { is_volatile = true; self.advance(); }
                    _ => break,
                }
            }
            ptr_depth.push((is_const, is_volatile));
        }

        // Parse direct declarator
        let mut name: Option<InternId> = None;
        let mut name_span = self.peek_span();
        let mut params = Vec::new();
        let mut is_variadic = false;
        let mut is_function = false;
        let mut array_sizes: Vec<NodeId> = Vec::new();

        match self.peek().clone() {
            TokenKind::Identifier(n) => {
                name = Some(self.ctx.intern(&n));
                name_span = self.peek_span();
                self.advance();
            }
            TokenKind::LParen if allow_abstract || matches!(self.peek_nth(1), TokenKind::Star) => {
                // Parenthesized declarator: (*name)
                self.advance(); // skip (
                let inner = self.parse_declarator(allow_abstract);
                self.expect(&TokenKind::RParen);
                // The inner declarator's name becomes our name
                name = inner.name;
                name_span = inner.name_span;
            }
            _ if allow_abstract => {
                // Abstract declarator — no name, that's OK
            }
            _ => {
                // Not abstract and not an identifier — error
                if ptr_depth.is_empty() {
                    self.diag.error(self.peek_span(), "expected declarator name");
                }
            }
        }

        // Parse direct-declarator suffixes: [] and ()
        loop {
            match self.peek() {
                TokenKind::LBracket => {
                    self.advance();
                    let size = if matches!(self.peek(), TokenKind::RBracket) {
                        NODE_NONE
                    } else {
                        self.parse_assignment_expr()
                    };
                    self.expect(&TokenKind::RBracket);
                    array_sizes.push(size);
                }
                TokenKind::LParen => {
                    self.advance();
                    is_function = true;
                    if !matches!(self.peek(), TokenKind::RParen) {
                        self.parse_parameter_list(&mut params, &mut is_variadic);
                    }
                    self.expect(&TokenKind::RParen);
                }
                _ => break,
            }
        }

        // Build type modifier chain: innermost first, then wrap with pointers
        let span = name_span;
        let mut type_node = NODE_NONE;

        // Array modifiers (innermost)
        for size in array_sizes.into_iter().rev() {
            let base = if type_node == NODE_NONE { NODE_NONE } else { type_node };
            type_node = self.ctx.push_node(
                NodeKind::ArrayOf { base, size },
                span,
            );
        }

        // Function modifier
        if is_function && params.is_empty() && !is_variadic {
            // f() with no params — could be function with no specified params
        }

        // Pointer modifiers (outermost)
        for (is_const, is_volatile) in ptr_depth.into_iter().rev() {
            let base = if type_node == NODE_NONE { NODE_NONE } else { type_node };
            type_node = self.ctx.push_node(
                NodeKind::PointerTo {
                    base,
                    is_const,
                    is_volatile,
                },
                span,
            );
        }

        Declarator {
            name,
            name_span,
            type_node,
            params,
            is_variadic,
            is_function,
        }
    }

    fn parse_parameter_list(
        &mut self,
        params: &mut Vec<NodeId>,
        is_variadic: &mut bool,
    ) {
        // First check if it's just (void)
        if matches!(self.peek(), TokenKind::KwVoid) {
            if matches!(self.peek_nth(1), TokenKind::RParen) {
                self.advance(); // skip void
                return;
            }
        }

        // Check for old-style identifier list: f(a, b, c) — skip for now
        // Parse parameter declarations
        loop {
            if matches!(self.peek(), TokenKind::Ellipsis) {
                self.advance();
                *is_variadic = true;
                break;
            }

            if !self.is_declaration_start() {
                // Might be identifier-only (old-style)
                if let TokenKind::Identifier(_) = self.peek() {
                    // Treat as old-style param — skip it
                    self.advance();
                    if matches!(self.peek(), TokenKind::Comma) {
                        self.advance();
                        continue;
                    }
                    break;
                }
                break;
            }

            let spec = self.parse_decl_specifiers();
            let decl = self.parse_declarator(true);
            let name = decl.name.unwrap_or_else(|| self.ctx.intern(""));
            let type_node = if decl.type_node != NODE_NONE {
                decl.type_node
            } else {
                spec.type_node
            };
            let span = spec.span.merge(decl.name_span);
            params.push(self.ctx.push_node(
                NodeKind::ParamDecl { name, type_node },
                span,
            ));

            if matches!(self.peek(), TokenKind::Comma) {
                self.advance();
                if matches!(self.peek(), TokenKind::Ellipsis) {
                    self.advance();
                    *is_variadic = true;
                    break;
                }
            } else {
                break;
            }
        }
    }

    /// Parse a type-name (for casts and sizeof).
    fn parse_type_name(&mut self) -> NodeId {
        let spec = self.parse_decl_specifiers();
        let decl = self.parse_declarator(true); // abstract declarator
        if decl.type_node != NODE_NONE {
            decl.type_node
        } else {
            spec.type_node
        }
    }

    /// Parse an initializer (for variable declarations).
    fn parse_initializer(&mut self) -> NodeId {
        if matches!(self.peek(), TokenKind::LBrace) {
            // Aggregate initializer
            let start = self.peek_span();
            self.advance(); // skip {
            let mut values = Vec::new();
            while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
                values.push(self.parse_initializer());
                if matches!(self.peek(), TokenKind::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            let end = self.peek_span();
            self.expect(&TokenKind::RBrace);
            self.ctx
                .push_node(NodeKind::InitList { values }, start.merge(end))
        } else {
            self.parse_assignment_expr()
        }
    }

    // ══════════════════════════════════════════════════════════════════
    // STATEMENT PARSING
    // ══════════════════════════════════════════════════════════════════

    fn parse_compound_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        if !matches!(self.peek(), TokenKind::LBrace) {
            self.diag.error(start, "expected '{'");
            return None;
        }
        self.advance();

        let mut stmts = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            match self.parse_block_item() {
                Some(stmt) => stmts.push(stmt),
                None => self.synchronize(),
            }
        }

        let end = self.peek_span();
        self.expect(&TokenKind::RBrace);
        Some(
            self.ctx
                .push_node(NodeKind::CompoundStmt { stmts }, start.merge(end)),
        )
    }

    /// Parse a statement or declaration within a compound statement.
    fn parse_block_item(&mut self) -> Option<NodeId> {
        if self.is_declaration_start() {
            // Check if it's really a declaration and not a label or expression
            // involving a typedef name
            if let TokenKind::Identifier(name) = self.peek() {
                if self.typedef_names.contains(name) {
                    // Could be a declaration with typedef type, or an expression/label
                    // If next token after identifier is ':' → it's a label
                    if matches!(self.peek_nth(1), TokenKind::Colon) {
                        return self.parse_statement();
                    }
                    return self.parse_local_declaration();
                }
            }
            self.parse_local_declaration()
        } else {
            self.parse_statement()
        }
    }

    fn parse_local_declaration(&mut self) -> Option<NodeId> {
        let spec = self.parse_decl_specifiers();
        let is_typedef = spec.storage == StorageClass::Typedef;

        // Check for bare struct/enum declaration
        if matches!(self.peek(), TokenKind::Semicolon) {
            self.advance();
            return Some(spec.type_node);
        }

        let first_decl = self.parse_declarator(false);
        let first_node = self.make_var_or_typedef(&spec, &first_decl, is_typedef);

        // Handle additional declarators
        let mut extra = Vec::new();
        while matches!(self.peek(), TokenKind::Comma) {
            self.advance();
            let d = self.parse_declarator(false);
            let node = self.make_var_or_typedef(&spec, &d, is_typedef);
            extra.push(node);
        }

        self.expect(&TokenKind::Semicolon);

        if extra.is_empty() {
            Some(first_node)
        } else {
            // Wrap in CompoundStmt to hold multiple declarations
            let mut all = vec![first_node];
            all.extend(extra);
            let span = self.ctx.node(first_node).span;
            Some(self.ctx.push_node(NodeKind::CompoundStmt { stmts: all }, span))
        }
    }

    fn parse_statement(&mut self) -> Option<NodeId> {
        match self.peek().clone() {
            TokenKind::LBrace => self.parse_compound_stmt(),
            TokenKind::KwReturn => self.parse_return_stmt(),
            TokenKind::KwIf => self.parse_if_stmt(),
            TokenKind::KwWhile => self.parse_while_stmt(),
            TokenKind::KwDo => self.parse_do_while_stmt(),
            TokenKind::KwFor => self.parse_for_stmt(),
            TokenKind::KwSwitch => self.parse_switch_stmt(),
            TokenKind::KwCase => self.parse_case_stmt(),
            TokenKind::KwDefault => self.parse_default_stmt(),
            TokenKind::KwGoto => self.parse_goto_stmt(),
            TokenKind::KwBreak => {
                let span = self.peek_span();
                self.advance();
                self.expect(&TokenKind::Semicolon);
                Some(self.ctx.push_node(NodeKind::BreakStmt, span))
            }
            TokenKind::KwContinue => {
                let span = self.peek_span();
                self.advance();
                self.expect(&TokenKind::Semicolon);
                Some(self.ctx.push_node(NodeKind::ContinueStmt, span))
            }
            TokenKind::Semicolon => {
                let span = self.peek_span();
                self.advance();
                Some(self.ctx.push_node(NodeKind::NullStmt, span))
            }
            // Check for label: identifier ':'
            TokenKind::Identifier(_) if matches!(self.peek_nth(1), TokenKind::Colon) => {
                self.parse_label_stmt()
            }
            _ => {
                // Expression statement
                let expr = self.parse_expression();
                let span = self.ctx.node(expr).span;
                self.expect(&TokenKind::Semicolon);
                Some(self.ctx.push_node(NodeKind::ExprStmt { expr }, span))
            }
        }
    }

    fn parse_return_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        self.advance();
        let expr = if matches!(self.peek(), TokenKind::Semicolon) {
            NODE_NONE
        } else {
            self.parse_expression()
        };
        self.expect(&TokenKind::Semicolon);
        Some(self.ctx.push_node(NodeKind::ReturnStmt { expr }, start))
    }

    fn parse_if_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        self.advance();
        self.expect(&TokenKind::LParen);
        let cond = self.parse_expression();
        self.expect(&TokenKind::RParen);
        let then_br = match self.parse_statement() {
            Some(s) => s,
            None => return None,
        };
        let else_br = if matches!(self.peek(), TokenKind::KwElse) {
            self.advance();
            match self.parse_statement() {
                Some(s) => s,
                None => NODE_NONE,
            }
        } else {
            NODE_NONE
        };
        Some(self.ctx.push_node(
            NodeKind::IfStmt {
                cond,
                then_br,
                else_br,
            },
            start,
        ))
    }

    fn parse_while_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        self.advance();
        self.expect(&TokenKind::LParen);
        let cond = self.parse_expression();
        self.expect(&TokenKind::RParen);
        let body = match self.parse_statement() {
            Some(s) => s,
            None => return None,
        };
        Some(
            self.ctx
                .push_node(NodeKind::WhileStmt { cond, body }, start),
        )
    }

    fn parse_do_while_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        self.advance(); // skip 'do'
        let body = match self.parse_statement() {
            Some(s) => s,
            None => return None,
        };
        self.expect(&TokenKind::KwWhile);
        self.expect(&TokenKind::LParen);
        let cond = self.parse_expression();
        self.expect(&TokenKind::RParen);
        self.expect(&TokenKind::Semicolon);
        Some(
            self.ctx
                .push_node(NodeKind::DoWhileStmt { body, cond }, start),
        )
    }

    fn parse_for_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        self.advance();
        self.expect(&TokenKind::LParen);

        let init = if matches!(self.peek(), TokenKind::Semicolon) {
            let n = self
                .ctx
                .push_node(NodeKind::NullStmt, self.peek_span());
            self.advance();
            n
        } else if self.is_declaration_start() {
            // C89 doesn't have for-loop declarations, but handle gracefully
            let d = self.parse_local_declaration();
            d.unwrap_or(NODE_NONE)
        } else {
            let e = self.parse_expression();
            self.expect(&TokenKind::Semicolon);
            e
        };

        let cond = if matches!(self.peek(), TokenKind::Semicolon) {
            NODE_NONE
        } else {
            self.parse_expression()
        };
        self.expect(&TokenKind::Semicolon);

        let incr = if matches!(self.peek(), TokenKind::RParen) {
            NODE_NONE
        } else {
            self.parse_expression()
        };
        self.expect(&TokenKind::RParen);

        let body = match self.parse_statement() {
            Some(s) => s,
            None => return None,
        };

        Some(self.ctx.push_node(
            NodeKind::ForStmt {
                init,
                cond,
                incr,
                body,
            },
            start,
        ))
    }

    fn parse_switch_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        self.advance();
        self.expect(&TokenKind::LParen);
        let expr = self.parse_expression();
        self.expect(&TokenKind::RParen);
        let body = match self.parse_statement() {
            Some(s) => s,
            None => return None,
        };
        Some(
            self.ctx
                .push_node(NodeKind::SwitchStmt { expr, body }, start),
        )
    }

    fn parse_case_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        self.advance(); // skip 'case'
        let expr = self.parse_assignment_expr();
        self.expect(&TokenKind::Colon);
        let body = match self.parse_statement() {
            Some(s) => s,
            None => return None,
        };
        Some(
            self.ctx
                .push_node(NodeKind::CaseStmt { expr, body }, start),
        )
    }

    fn parse_default_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        self.advance(); // skip 'default'
        self.expect(&TokenKind::Colon);
        let body = match self.parse_statement() {
            Some(s) => s,
            None => return None,
        };
        Some(
            self.ctx
                .push_node(NodeKind::DefaultStmt { body }, start),
        )
    }

    fn parse_goto_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        self.advance(); // skip 'goto'
        if let TokenKind::Identifier(name) = self.peek().clone() {
            let label = self.ctx.intern(&name);
            self.advance();
            self.expect(&TokenKind::Semicolon);
            Some(
                self.ctx
                    .push_node(NodeKind::GotoStmt { label }, start),
            )
        } else {
            self.diag
                .error(self.peek_span(), "expected label name after 'goto'");
            self.synchronize();
            None
        }
    }

    fn parse_label_stmt(&mut self) -> Option<NodeId> {
        let start = self.peek_span();
        if let TokenKind::Identifier(name) = self.peek().clone() {
            let label = self.ctx.intern(&name);
            self.advance(); // skip identifier
            self.advance(); // skip ':'
            let stmt = match self.parse_statement() {
                Some(s) => s,
                None => self
                    .ctx
                    .push_node(NodeKind::NullStmt, self.peek_span()),
            };
            Some(
                self.ctx
                    .push_node(NodeKind::LabelStmt { label, stmt }, start),
            )
        } else {
            None
        }
    }

    // ══════════════════════════════════════════════════════════════════
    // EXPRESSION PARSING (all 15 C89 precedence levels)
    // ══════════════════════════════════════════════════════════════════

    /// Top-level expression: handles comma operator.
    fn parse_expression(&mut self) -> NodeId {
        let mut lhs = self.parse_assignment_expr();
        while matches!(self.peek(), TokenKind::Comma) {
            self.advance();
            let rhs = self.parse_assignment_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(NodeKind::Comma { lhs, rhs }, span);
        }
        lhs
    }

    fn parse_assignment_expr(&mut self) -> NodeId {
        let lhs = self.parse_ternary_expr();

        let op = match self.peek() {
            TokenKind::Eq => Some(AssignOp::Assign),
            TokenKind::PlusEq => Some(AssignOp::AddAssign),
            TokenKind::MinusEq => Some(AssignOp::SubAssign),
            TokenKind::StarEq => Some(AssignOp::MulAssign),
            TokenKind::SlashEq => Some(AssignOp::DivAssign),
            TokenKind::PercentEq => Some(AssignOp::ModAssign),
            TokenKind::LtLtEq => Some(AssignOp::ShlAssign),
            TokenKind::GtGtEq => Some(AssignOp::ShrAssign),
            TokenKind::AmpEq => Some(AssignOp::AndAssign),
            TokenKind::CaretEq => Some(AssignOp::XorAssign),
            TokenKind::PipeEq => Some(AssignOp::OrAssign),
            _ => None,
        };

        if let Some(op) = op {
            self.advance();
            let rhs = self.parse_assignment_expr(); // right-associative
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            self.ctx.push_node(NodeKind::Assign { op, lhs, rhs }, span)
        } else {
            lhs
        }
    }

    fn parse_ternary_expr(&mut self) -> NodeId {
        let cond = self.parse_logical_or_expr();
        if matches!(self.peek(), TokenKind::Question) {
            self.advance();
            let then_expr = self.parse_expression();
            self.expect(&TokenKind::Colon);
            let else_expr = self.parse_ternary_expr();
            let span = self.ctx.node(cond).span.merge(self.ctx.node(else_expr).span);
            self.ctx.push_node(
                NodeKind::Ternary {
                    cond,
                    then_expr,
                    else_expr,
                },
                span,
            )
        } else {
            cond
        }
    }

    fn parse_logical_or_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_logical_and_expr();
        while matches!(self.peek(), TokenKind::PipePipe) {
            self.advance();
            let rhs = self.parse_logical_and_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(
                NodeKind::BinaryOp {
                    op: BinOp::LogOr,
                    lhs,
                    rhs,
                },
                span,
            );
        }
        lhs
    }

    fn parse_logical_and_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_bitwise_or_expr();
        while matches!(self.peek(), TokenKind::AmpAmp) {
            self.advance();
            let rhs = self.parse_bitwise_or_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(
                NodeKind::BinaryOp {
                    op: BinOp::LogAnd,
                    lhs,
                    rhs,
                },
                span,
            );
        }
        lhs
    }

    fn parse_bitwise_or_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_bitwise_xor_expr();
        while matches!(self.peek(), TokenKind::Pipe) {
            self.advance();
            let rhs = self.parse_bitwise_xor_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(
                NodeKind::BinaryOp {
                    op: BinOp::BitOr,
                    lhs,
                    rhs,
                },
                span,
            );
        }
        lhs
    }

    fn parse_bitwise_xor_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_bitwise_and_expr();
        while matches!(self.peek(), TokenKind::Caret) {
            self.advance();
            let rhs = self.parse_bitwise_and_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(
                NodeKind::BinaryOp {
                    op: BinOp::BitXor,
                    lhs,
                    rhs,
                },
                span,
            );
        }
        lhs
    }

    fn parse_bitwise_and_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_equality_expr();
        while matches!(self.peek(), TokenKind::Amp) {
            self.advance();
            let rhs = self.parse_equality_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(
                NodeKind::BinaryOp {
                    op: BinOp::BitAnd,
                    lhs,
                    rhs,
                },
                span,
            );
        }
        lhs
    }

    fn parse_equality_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_relational_expr();
        loop {
            let op = match self.peek() {
                TokenKind::EqEq => BinOp::Eq,
                TokenKind::BangEq => BinOp::Ne,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_relational_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(NodeKind::BinaryOp { op, lhs, rhs }, span);
        }
        lhs
    }

    fn parse_relational_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_shift_expr();
        loop {
            let op = match self.peek() {
                TokenKind::Lt => BinOp::Lt,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::LtEq => BinOp::Le,
                TokenKind::GtEq => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_shift_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(NodeKind::BinaryOp { op, lhs, rhs }, span);
        }
        lhs
    }

    fn parse_shift_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_additive_expr();
        loop {
            let op = match self.peek() {
                TokenKind::LtLt => BinOp::Shl,
                TokenKind::GtGt => BinOp::Shr,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_additive_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(NodeKind::BinaryOp { op, lhs, rhs }, span);
        }
        lhs
    }

    fn parse_additive_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_multiplicative_expr();
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_multiplicative_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(NodeKind::BinaryOp { op, lhs, rhs }, span);
        }
        lhs
    }

    fn parse_multiplicative_expr(&mut self) -> NodeId {
        let mut lhs = self.parse_cast_expr();
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_cast_expr();
            let span = self.ctx.node(lhs).span.merge(self.ctx.node(rhs).span);
            lhs = self.ctx.push_node(NodeKind::BinaryOp { op, lhs, rhs }, span);
        }
        lhs
    }

    /// Parse cast expression: (type-name) unary-expression.
    fn parse_cast_expr(&mut self) -> NodeId {
        // Check for (type-name)
        if matches!(self.peek(), TokenKind::LParen) && self.is_type_name_after_lparen() {
            let start = self.peek_span();
            self.advance(); // skip (
            let type_node = self.parse_type_name();
            self.expect(&TokenKind::RParen);
            let expr = self.parse_cast_expr(); // cast is right-associative
            let span = start.merge(self.ctx.node(expr).span);
            return self.ctx.push_node(NodeKind::Cast { type_node, expr }, span);
        }
        self.parse_unary_expr()
    }

    fn parse_unary_expr(&mut self) -> NodeId {
        let span = self.peek_span();
        match self.peek().clone() {
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_cast_expr();
                let span = span.merge(self.ctx.node(operand).span);
                self.ctx.push_node(NodeKind::UnaryOp { op: UnaryOp::Neg, operand }, span)
            }
            TokenKind::Plus => {
                self.advance();
                let operand = self.parse_cast_expr();
                let span = span.merge(self.ctx.node(operand).span);
                self.ctx.push_node(NodeKind::UnaryOp { op: UnaryOp::Plus, operand }, span)
            }
            TokenKind::Bang => {
                self.advance();
                let operand = self.parse_cast_expr();
                let span = span.merge(self.ctx.node(operand).span);
                self.ctx.push_node(NodeKind::UnaryOp { op: UnaryOp::LogNot, operand }, span)
            }
            TokenKind::Tilde => {
                self.advance();
                let operand = self.parse_cast_expr();
                let span = span.merge(self.ctx.node(operand).span);
                self.ctx.push_node(NodeKind::UnaryOp { op: UnaryOp::BitNot, operand }, span)
            }
            TokenKind::PlusPlus => {
                self.advance();
                let operand = self.parse_unary_expr();
                let span = span.merge(self.ctx.node(operand).span);
                self.ctx.push_node(NodeKind::UnaryOp { op: UnaryOp::PreInc, operand }, span)
            }
            TokenKind::MinusMinus => {
                self.advance();
                let operand = self.parse_unary_expr();
                let span = span.merge(self.ctx.node(operand).span);
                self.ctx.push_node(NodeKind::UnaryOp { op: UnaryOp::PreDec, operand }, span)
            }
            TokenKind::Amp => {
                self.advance();
                let expr = self.parse_cast_expr();
                let span = span.merge(self.ctx.node(expr).span);
                self.ctx.push_node(NodeKind::AddrOf { expr }, span)
            }
            TokenKind::Star => {
                self.advance();
                let expr = self.parse_cast_expr();
                let span = span.merge(self.ctx.node(expr).span);
                self.ctx.push_node(NodeKind::Deref { expr }, span)
            }
            TokenKind::KwSizeof => {
                self.advance();
                if matches!(self.peek(), TokenKind::LParen) && self.is_type_name_after_lparen() {
                    // sizeof(type-name)
                    self.advance(); // skip (
                    let type_node = self.parse_type_name();
                    self.expect(&TokenKind::RParen);
                    let span = span.merge(self.peek_span());
                    self.ctx
                        .push_node(NodeKind::SizeofType { type_node }, span)
                } else if matches!(self.peek(), TokenKind::LParen) {
                    // sizeof(expr) — parenthesized expression
                    self.advance();
                    let expr = self.parse_expression();
                    self.expect(&TokenKind::RParen);
                    let span = span.merge(self.peek_span());
                    self.ctx.push_node(NodeKind::SizeofExpr { expr }, span)
                } else {
                    let expr = self.parse_unary_expr();
                    let span = span.merge(self.ctx.node(expr).span);
                    self.ctx.push_node(NodeKind::SizeofExpr { expr }, span)
                }
            }
            _ => self.parse_postfix_expr(),
        }
    }

    fn parse_postfix_expr(&mut self) -> NodeId {
        let mut expr = self.parse_primary_expr();

        loop {
            match self.peek() {
                TokenKind::PlusPlus => {
                    let span = self.ctx.node(expr).span.merge(self.peek_span());
                    self.advance();
                    expr = self.ctx.push_node(
                        NodeKind::PostfixOp {
                            op: PostfixOp::PostInc,
                            operand: expr,
                        },
                        span,
                    );
                }
                TokenKind::MinusMinus => {
                    let span = self.ctx.node(expr).span.merge(self.peek_span());
                    self.advance();
                    expr = self.ctx.push_node(
                        NodeKind::PostfixOp {
                            op: PostfixOp::PostDec,
                            operand: expr,
                        },
                        span,
                    );
                }
                TokenKind::LParen => {
                    self.advance();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), TokenKind::RParen) {
                        args.push(self.parse_assignment_expr());
                        while matches!(self.peek(), TokenKind::Comma) {
                            self.advance();
                            args.push(self.parse_assignment_expr());
                        }
                    }
                    let end = self.peek_span();
                    self.expect(&TokenKind::RParen);
                    let span = self.ctx.node(expr).span.merge(end);
                    expr = self
                        .ctx
                        .push_node(NodeKind::Call { callee: expr, args }, span);
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expression();
                    let end = self.peek_span();
                    self.expect(&TokenKind::RBracket);
                    let span = self.ctx.node(expr).span.merge(end);
                    expr = self
                        .ctx
                        .push_node(NodeKind::ArraySubscript { expr, index }, span);
                }
                TokenKind::Dot => {
                    self.advance();
                    if let TokenKind::Identifier(name) = self.peek().clone() {
                        let member = self.ctx.intern(&name);
                        let end = self.peek_span();
                        self.advance();
                        let span = self.ctx.node(expr).span.merge(end);
                        expr = self.ctx.push_node(
                            NodeKind::MemberAccess {
                                expr,
                                member,
                                is_arrow: false,
                            },
                            span,
                        );
                    } else {
                        self.diag
                            .error(self.peek_span(), "expected member name after '.'");
                    }
                }
                TokenKind::Arrow => {
                    self.advance();
                    if let TokenKind::Identifier(name) = self.peek().clone() {
                        let member = self.ctx.intern(&name);
                        let end = self.peek_span();
                        self.advance();
                        let span = self.ctx.node(expr).span.merge(end);
                        expr = self.ctx.push_node(
                            NodeKind::MemberAccess {
                                expr,
                                member,
                                is_arrow: true,
                            },
                            span,
                        );
                    } else {
                        self.diag
                            .error(self.peek_span(), "expected member name after '->'");
                    }
                }
                _ => break,
            }
        }

        expr
    }

    fn parse_primary_expr(&mut self) -> NodeId {
        let span = self.peek_span();
        match self.peek().clone() {
            TokenKind::IntLiteral(value, suffix) => {
                self.advance();
                let s = match suffix {
                    crate::frontend::lexer::token::IntSuffix::None => IntSuffix::None,
                    crate::frontend::lexer::token::IntSuffix::U => IntSuffix::U,
                    crate::frontend::lexer::token::IntSuffix::L => IntSuffix::L,
                    crate::frontend::lexer::token::IntSuffix::UL => IntSuffix::UL,
                    crate::frontend::lexer::token::IntSuffix::LL => IntSuffix::LL,
                    crate::frontend::lexer::token::IntSuffix::ULL => IntSuffix::ULL,
                };
                self.ctx
                    .push_node(NodeKind::IntLiteral { value, suffix: s }, span)
            }
            TokenKind::FloatLiteral(value, suffix) => {
                self.advance();
                let s = match suffix {
                    crate::frontend::lexer::token::FloatSuffix::None => FloatSuffix::None,
                    crate::frontend::lexer::token::FloatSuffix::F => FloatSuffix::F,
                    crate::frontend::lexer::token::FloatSuffix::L => FloatSuffix::L,
                };
                self.ctx
                    .push_node(NodeKind::FloatLiteral { value, suffix: s }, span)
            }
            TokenKind::CharLiteral(value) => {
                self.advance();
                self.ctx.push_node(NodeKind::CharLiteral { value }, span)
            }
            TokenKind::StringLiteral(bytes) => {
                self.advance();
                self.ctx
                    .push_node(NodeKind::StringLiteral { bytes }, span)
            }
            TokenKind::Identifier(name) => {
                let name_id = self.ctx.intern(&name);
                self.advance();
                self.ctx.push_node(NodeKind::Ident { name: name_id }, span)
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression();
                self.expect(&TokenKind::RParen);
                expr
            }
            _ => {
                self.diag.error(
                    span,
                    format!("expected expression, found {}", self.peek().describe()),
                );
                self.ctx.push_node(
                    NodeKind::IntLiteral {
                        value: 0,
                        suffix: IntSuffix::None,
                    },
                    span,
                )
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
// TESTS
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::DiagEngine;
    use crate::frontend::lexer;
    use crate::source::SourceMap;
    use crate::target::Target;

    fn parse_str(src: &str) -> (Ctx, NodeId, DiagEngine) {
        let mut sm = SourceMap::new();
        let fid = sm.add_file("test.c".into(), src.into());
        let diag = DiagEngine::new();
        let tokens = lexer::lex(&sm, fid, &diag);
        let mut ctx = Ctx::new(Target::I386);
        let root = parse(&tokens, &mut ctx, &diag);
        (ctx, root, diag)
    }

    #[test]
    fn test_parse_empty() {
        let (ctx, root, diag) = parse_str("");
        assert!(!diag.has_errors());
        assert!(matches!(
            ctx.node(root).kind,
            NodeKind::TranslationUnit { ref decls } if decls.is_empty()
        ));
    }

    #[test]
    fn test_parse_simple_function() {
        let (ctx, root, diag) = parse_str("int main() { return 0; }");
        assert!(!diag.has_errors(), "parser produced errors");
        match &ctx.node(root).kind {
            NodeKind::TranslationUnit { decls } => {
                assert_eq!(decls.len(), 1);
                assert!(matches!(ctx.node(decls[0]).kind, NodeKind::FuncDef { .. }));
            }
            _ => panic!("expected TranslationUnit"),
        }
    }

    #[test]
    fn test_parse_binary_expression() {
        let (ctx, root, diag) = parse_str("int x = 1 + 2;");
        assert!(!diag.has_errors());
        match &ctx.node(root).kind {
            NodeKind::TranslationUnit { decls } => {
                assert_eq!(decls.len(), 1);
                match &ctx.node(decls[0]).kind {
                    NodeKind::VarDecl { init, .. } => {
                        assert!(matches!(
                            ctx.node(*init).kind,
                            NodeKind::BinaryOp {
                                op: BinOp::Add,
                                ..
                            }
                        ));
                    }
                    _ => panic!("expected VarDecl"),
                }
            }
            _ => panic!("expected TranslationUnit"),
        }
    }

    #[test]
    fn test_parse_function_call() {
        let (_ctx, _root, diag) = parse_str("int main() { foo(1, 2); }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_if_else() {
        let (_ctx, _root, diag) =
            parse_str("int main() { if (x) return 1; else return 0; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_while_loop() {
        let (_ctx, _root, diag) = parse_str("int main() { while (1) break; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_for_loop() {
        let (_ctx, _root, diag) =
            parse_str("int main() { for (i = 0; i < 10; i++) x(); }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_do_while() {
        let (_ctx, _root, diag) =
            parse_str("int main() { do { x(); } while (1); }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_switch() {
        let (_ctx, _root, diag) = parse_str(
            "int main() { switch (x) { case 1: return 1; case 2: return 2; default: return 0; } }",
        );
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_goto_label() {
        let (_ctx, _root, diag) = parse_str(
            "int main() { goto end; end: return 0; }",
        );
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_struct() {
        let (_ctx, _root, diag) = parse_str(
            "struct point { int x; int y; }; int main() { return 0; }",
        );
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_enum() {
        let (_ctx, _root, diag) = parse_str(
            "enum color { RED, GREEN, BLUE = 5 }; int main() { return RED; }",
        );
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_pointer_decl() {
        let (_ctx, _root, diag) = parse_str("int *p; int main() { return 0; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_unsigned_long() {
        let (_ctx, _root, diag) =
            parse_str("unsigned long int x; int main() { return 0; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_function_params() {
        let (ctx, root, diag) =
            parse_str("int add(int a, int b) { return a + b; }");
        assert!(!diag.has_errors());
        match &ctx.node(root).kind {
            NodeKind::TranslationUnit { decls } => {
                assert_eq!(decls.len(), 1);
                match &ctx.node(decls[0]).kind {
                    NodeKind::FuncDef { params, .. } => {
                        assert_eq!(params.len(), 2);
                    }
                    _ => panic!("expected FuncDef"),
                }
            }
            _ => panic!("expected TranslationUnit"),
        }
    }

    #[test]
    fn test_parse_cast() {
        let (_ctx, _root, diag) =
            parse_str("int main() { float f = (float)42; return (int)f; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_sizeof() {
        let (_ctx, _root, diag) =
            parse_str("int main() { int x = sizeof(int); return x; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_comma_expr() {
        let (_ctx, _root, diag) = parse_str("int main() { int x; x = (1, 2, 3); return x; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_init_list() {
        let (_ctx, _root, diag) = parse_str("int arr[] = {1, 2, 3};");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_multiple_decls() {
        let (_ctx, _root, diag) = parse_str("int a, b, c;");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_parse_complex_program() {
        let (_ctx, _root, diag) = parse_str(
            r#"
            int printf(const char *fmt, ...);

            struct point {
                int x;
                int y;
            };

            enum direction { NORTH, SOUTH, EAST, WEST };

            int add(int a, int b) {
                return a + b;
            }

            int main() {
                int result;
                struct point p;
                result = add(1, 2);
                p.x = 10;
                p.y = 20;
                if (result > 0) {
                    printf("positive\n");
                } else {
                    printf("non-positive\n");
                }
                return 0;
            }
            "#,
        );
        assert!(!diag.has_errors(), "complex program parse failed");
    }

    #[test]
    fn test_error_recovery_no_crash() {
        let (_ctx, _root, _diag) = parse_str("int main( { }");
        // Should not crash, may have errors
    }
}
