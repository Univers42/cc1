// frontend/sema/mod.rs — Semantic analysis for C89.
//
// Walks the AST, resolves types, builds the symbol table, and
// annotates each node's `ty` field with a TypeId.

use crate::ctx::*;
use crate::diagnostics::DiagEngine;
use crate::source::{InternId, Span};

/// Run semantic analysis on the AST rooted at `root`.
pub fn analyze(ctx: &mut Ctx, root: NodeId, diag: &DiagEngine) {
    let mut sema = Sema::new(ctx, diag);
    sema.visit(root);
}

/// Dump type information for --dump-types.
pub fn dump_types(ctx: &Ctx) {
    for (i, ty) in ctx.types.iter().enumerate() {
        eprintln!("  T{}: {:?}", i, ty);
    }
    for (i, sym) in ctx.symbols.iter().enumerate() {
        eprintln!(
            "  S{}: '{}' kind={:?} ty=T{}",
            i,
            ctx.get_str(sym.name),
            sym.kind,
            sym.ty.0
        );
    }
}

// ── Pre-interned basic types (pushed during Sema::new) ────────────────

struct BasicTypes {
    void: TypeId,
    char_ty: TypeId,
    schar: TypeId,
    uchar: TypeId,
    short: TypeId,
    ushort: TypeId,
    int: TypeId,
    uint: TypeId,
    long: TypeId,
    ulong: TypeId,
    float: TypeId,
    double: TypeId,
    long_double: TypeId,
    char_ptr: TypeId,
}

// ── Sema Pass ─────────────────────────────────────────────────────────

struct Sema<'a> {
    ctx: &'a mut Ctx,
    diag: &'a DiagEngine,
    bt: BasicTypes,
    loop_depth: u32,
    switch_depth: u32,
    current_return_type: TypeId,
}

impl<'a> Sema<'a> {
    fn new(ctx: &'a mut Ctx, diag: &'a DiagEngine) -> Self {
        let void = ctx.push_type(CType::Void);
        let char_ty = ctx.push_type(CType::Char);
        let schar = ctx.push_type(CType::SChar);
        let uchar = ctx.push_type(CType::UChar);
        let short = ctx.push_type(CType::Short);
        let ushort = ctx.push_type(CType::UShort);
        let int = ctx.push_type(CType::Int);
        let uint = ctx.push_type(CType::UInt);
        let long = ctx.push_type(CType::Long);
        let ulong = ctx.push_type(CType::ULong);
        let float = ctx.push_type(CType::Float);
        let double = ctx.push_type(CType::Double);
        let long_double = ctx.push_type(CType::LongDouble);
        let char_ptr = ctx.push_type(CType::Pointer { pointee: char_ty });

        Self {
            ctx,
            diag,
            bt: BasicTypes {
                void,
                char_ty,
                schar,
                uchar,
                short,
                ushort,
                int,
                uint,
                long,
                ulong,
                float,
                double,
                long_double,
                char_ptr,
            },
            loop_depth: 0,
            switch_depth: 0,
            current_return_type: TYPE_NONE,
        }
    }

    // ── Type Resolution ───────────────────────────────────────────────

    fn resolve_type_spec(&mut self, spec: &TypeSpecKind) -> TypeId {
        match spec {
            TypeSpecKind::Void => self.bt.void,
            TypeSpecKind::Char => self.bt.char_ty,
            TypeSpecKind::SignedChar => self.bt.schar,
            TypeSpecKind::UnsignedChar => self.bt.uchar,
            TypeSpecKind::Short => self.bt.short,
            TypeSpecKind::UnsignedShort => self.bt.ushort,
            TypeSpecKind::Int | TypeSpecKind::Signed => self.bt.int,
            TypeSpecKind::UnsignedInt | TypeSpecKind::Unsigned => self.bt.uint,
            TypeSpecKind::Long => self.bt.long,
            TypeSpecKind::UnsignedLong => self.bt.ulong,
            TypeSpecKind::Float => self.bt.float,
            TypeSpecKind::Double => self.bt.double,
            TypeSpecKind::LongDouble => self.bt.long_double,
            TypeSpecKind::Struct | TypeSpecKind::Union | TypeSpecKind::Enum => self.bt.int,
            TypeSpecKind::TypedefName => self.bt.int,
        }
    }

    fn resolve_type_node(&mut self, node_id: NodeId) -> TypeId {
        if node_id == NODE_NONE {
            return self.bt.int;
        }
        let kind = self.ctx.node(node_id).kind.clone();
        match &kind {
            NodeKind::TypeSpec { spec } => self.resolve_type_spec(spec),
            NodeKind::PointerTo { base, .. } => {
                let pointee = if *base == NODE_NONE {
                    self.bt.int
                } else {
                    self.resolve_type_node(*base)
                };
                self.ctx.push_type(CType::Pointer { pointee })
            }
            NodeKind::ArrayOf { base, size } => {
                let elem = if *base == NODE_NONE {
                    self.bt.int
                } else {
                    self.resolve_type_node(*base)
                };
                let len = if *size != NODE_NONE {
                    self.eval_const_expr(*size).map(|v| v as u64)
                } else {
                    None
                };
                self.ctx.push_type(CType::Array { elem, len })
            }
            NodeKind::StructDecl { tag, members } => {
                let tag = *tag;
                let members = members.clone();
                self.resolve_struct_type(tag, &members)
            }
            NodeKind::UnionDecl { tag, members } => {
                let tag = *tag;
                let members = members.clone();
                self.resolve_union_type(tag, &members)
            }
            NodeKind::EnumDecl { tag, enumerators } => {
                let tag = *tag;
                let enumerators = enumerators.clone();
                self.resolve_enum_type(tag, &enumerators)
            }
            _ => self.bt.int,
        }
    }

    fn resolve_struct_type(&mut self, tag: Option<InternId>, member_nodes: &[NodeId]) -> TypeId {
        let mut members = Vec::new();
        let mut offset = 0u32;
        let mut max_align = 1u32;

        for &m_id in member_nodes {
            let kind = self.ctx.node(m_id).kind.clone();
            if let NodeKind::MemberDecl {
                name,
                type_node,
                bitfield,
            } = kind
            {
                let ty = self.resolve_type_node(type_node);
                let align = self.ctx.type_align(ty);
                let size = self.ctx.type_size(ty);

                if align > 0 {
                    offset = (offset + align - 1) & !(align - 1);
                }
                members.push(MemberInfo {
                    name,
                    ty,
                    offset,
                    bitfield_width: if bitfield != NODE_NONE {
                        self.eval_const_expr(bitfield).map(|v| v as u32)
                    } else {
                        None
                    },
                });
                offset += size;
                if align > max_align {
                    max_align = align;
                }
            }
        }

        if max_align > 0 {
            offset = (offset + max_align - 1) & !(max_align - 1);
        }

        self.ctx.push_type(CType::Struct {
            tag,
            members,
            size: offset,
            align: max_align,
            complete: !member_nodes.is_empty(),
        })
    }

    fn resolve_union_type(&mut self, tag: Option<InternId>, member_nodes: &[NodeId]) -> TypeId {
        let mut members = Vec::new();
        let mut max_size = 0u32;
        let mut max_align = 1u32;

        for &m_id in member_nodes {
            let kind = self.ctx.node(m_id).kind.clone();
            if let NodeKind::MemberDecl {
                name,
                type_node,
                bitfield,
            } = kind
            {
                let ty = self.resolve_type_node(type_node);
                let size = self.ctx.type_size(ty);
                let align = self.ctx.type_align(ty);

                members.push(MemberInfo {
                    name,
                    ty,
                    offset: 0,
                    bitfield_width: if bitfield != NODE_NONE {
                        self.eval_const_expr(bitfield).map(|v| v as u32)
                    } else {
                        None
                    },
                });
                if size > max_size { max_size = size; }
                if align > max_align { max_align = align; }
            }
        }

        if max_align > 0 {
            max_size = (max_size + max_align - 1) & !(max_align - 1);
        }

        self.ctx.push_type(CType::Union {
            tag,
            members,
            size: max_size,
            align: max_align,
            complete: !member_nodes.is_empty(),
        })
    }

    fn resolve_enum_type(&mut self, tag: Option<InternId>, enumerators: &[(InternId, NodeId)]) -> TypeId {
        let mut value = 0i64;
        for &(name, expr_id) in enumerators {
            if expr_id != NODE_NONE {
                if let Some(v) = self.eval_const_expr(expr_id) {
                    value = v;
                }
            }
            let span = if expr_id != NODE_NONE {
                self.ctx.node(expr_id).span
            } else {
                Span::dummy()
            };
            self.ctx.add_symbol(Symbol {
                name,
                ty: self.bt.int,
                kind: SymbolKind::EnumConstant(value),
                span,
                scope: self.ctx.current_scope,
            });
            value += 1;
        }
        self.ctx.push_type(CType::Enum { tag })
    }

    // ── Constant Expression Evaluation ────────────────────────────────

    fn eval_const_expr(&self, id: NodeId) -> Option<i64> {
        let kind = &self.ctx.node(id).kind;
        match kind {
            NodeKind::IntLiteral { value, .. } => Some(*value as i64),
            NodeKind::CharLiteral { value } => Some(*value as i64),
            NodeKind::UnaryOp { op, operand } => {
                let v = self.eval_const_expr(*operand)?;
                match op {
                    UnaryOp::Neg => Some(-v),
                    UnaryOp::BitNot => Some(!v),
                    UnaryOp::LogNot => Some(if v == 0 { 1 } else { 0 }),
                    UnaryOp::Plus => Some(v),
                    _ => None,
                }
            }
            NodeKind::BinaryOp { op, lhs, rhs } => {
                let l = self.eval_const_expr(*lhs)?;
                let r = self.eval_const_expr(*rhs)?;
                match op {
                    BinOp::Add => Some(l.wrapping_add(r)),
                    BinOp::Sub => Some(l.wrapping_sub(r)),
                    BinOp::Mul => Some(l.wrapping_mul(r)),
                    BinOp::Div => if r == 0 { None } else { Some(l / r) },
                    BinOp::Mod => if r == 0 { None } else { Some(l % r) },
                    BinOp::Shl => Some(l << (r & 63)),
                    BinOp::Shr => Some(l >> (r & 63)),
                    BinOp::BitAnd => Some(l & r),
                    BinOp::BitOr => Some(l | r),
                    BinOp::BitXor => Some(l ^ r),
                    BinOp::Eq => Some(if l == r { 1 } else { 0 }),
                    BinOp::Ne => Some(if l != r { 1 } else { 0 }),
                    BinOp::Lt => Some(if l < r { 1 } else { 0 }),
                    BinOp::Gt => Some(if l > r { 1 } else { 0 }),
                    BinOp::Le => Some(if l <= r { 1 } else { 0 }),
                    BinOp::Ge => Some(if l >= r { 1 } else { 0 }),
                    BinOp::LogAnd => Some(if l != 0 && r != 0 { 1 } else { 0 }),
                    BinOp::LogOr => Some(if l != 0 || r != 0 { 1 } else { 0 }),
                }
            }
            NodeKind::Ident { name } => {
                if let Some(sym_id) = self.ctx.lookup_symbol(*name) {
                    let sym = self.ctx.get_symbol(sym_id);
                    if let SymbolKind::EnumConstant(v) = sym.kind {
                        return Some(v);
                    }
                }
                None
            }
            _ => None,
        }
    }

    // ── Main Visitor ──────────────────────────────────────────────────

    fn visit(&mut self, id: NodeId) {
        if id == NODE_NONE { return; }
        let kind = self.ctx.node(id).kind.clone();
        match &kind {
            NodeKind::TranslationUnit { decls } => {
                let decls = decls.clone();
                for d in &decls { self.visit(*d); }
            }
            NodeKind::FuncDef {
                return_type, name, params, body, is_variadic, ..
            } => {
                let ret_type = self.resolve_type_node(*return_type);
                let name = *name;
                let params = params.clone();
                let body = *body;
                let is_variadic = *is_variadic;

                let mut param_types = Vec::new();
                for &p_id in &params {
                    let pk = self.ctx.node(p_id).kind.clone();
                    if let NodeKind::ParamDecl { name: pname, type_node } = pk {
                        let pty = self.resolve_type_node(type_node);
                        param_types.push((pname, pty));
                        self.ctx.node_mut(p_id).ty = pty;
                    }
                }

                let func_ty = self.ctx.push_type(CType::Function {
                    ret: ret_type,
                    params: param_types.clone(),
                    variadic: is_variadic,
                });

                let span = self.ctx.node(id).span;
                self.ctx.add_symbol(Symbol {
                    name,
                    ty: func_ty,
                    kind: SymbolKind::Function,
                    span,
                    scope: self.ctx.current_scope,
                });
                self.ctx.node_mut(id).ty = func_ty;

                let prev_ret = self.current_return_type;
                self.current_return_type = ret_type;
                self.ctx.push_scope(ScopeKind::Function);

                for (pname, pty) in &param_types {
                    self.ctx.add_symbol(Symbol {
                        name: *pname,
                        ty: *pty,
                        kind: SymbolKind::Parameter,
                        span,
                        scope: self.ctx.current_scope,
                    });
                }

                self.visit(body);
                self.ctx.pop_scope();
                self.current_return_type = prev_ret;
            }
            NodeKind::VarDecl { name, type_node, init, .. } => {
                let ty = self.resolve_type_node(*type_node);
                let name = *name;
                let init = *init;
                let span = self.ctx.node(id).span;

                self.ctx.add_symbol(Symbol {
                    name,
                    ty,
                    kind: SymbolKind::Variable,
                    span,
                    scope: self.ctx.current_scope,
                });
                self.ctx.node_mut(id).ty = ty;

                if init != NODE_NONE {
                    let _ = self.visit_expr(init);
                }
            }
            NodeKind::TypedefDecl { name, type_node } => {
                let ty = self.resolve_type_node(*type_node);
                let name = *name;
                let span = self.ctx.node(id).span;
                self.ctx.add_symbol(Symbol {
                    name,
                    ty,
                    kind: SymbolKind::Typedef,
                    span,
                    scope: self.ctx.current_scope,
                });
                self.ctx.node_mut(id).ty = ty;
            }
            NodeKind::StructDecl { tag, members } => {
                let tag = *tag;
                let members = members.clone();
                let ty = self.resolve_struct_type(tag, &members);
                self.ctx.node_mut(id).ty = ty;
            }
            NodeKind::UnionDecl { tag, members } => {
                let tag = *tag;
                let members = members.clone();
                let ty = self.resolve_union_type(tag, &members);
                self.ctx.node_mut(id).ty = ty;
            }
            NodeKind::EnumDecl { tag, enumerators } => {
                let tag = *tag;
                let enumerators = enumerators.clone();
                let ty = self.resolve_enum_type(tag, &enumerators);
                self.ctx.node_mut(id).ty = ty;
            }
            NodeKind::CompoundStmt { stmts } => {
                let stmts = stmts.clone();
                self.ctx.push_scope(ScopeKind::Block);
                for s in &stmts { self.visit(*s); }
                self.ctx.pop_scope();
            }
            NodeKind::IfStmt { cond, then_br, else_br } => {
                let (c, t, e) = (*cond, *then_br, *else_br);
                let _ = self.visit_expr(c);
                self.visit(t);
                self.visit(e);
            }
            NodeKind::WhileStmt { cond, body } => {
                let (c, b) = (*cond, *body);
                let _ = self.visit_expr(c);
                self.loop_depth += 1;
                self.visit(b);
                self.loop_depth -= 1;
            }
            NodeKind::DoWhileStmt { body, cond } => {
                let (b, c) = (*body, *cond);
                self.loop_depth += 1;
                self.visit(b);
                self.loop_depth -= 1;
                let _ = self.visit_expr(c);
            }
            NodeKind::ForStmt { init, cond, incr, body } => {
                let (i, c, inc, b) = (*init, *cond, *incr, *body);
                self.visit(i);
                if c != NODE_NONE { let _ = self.visit_expr(c); }
                if inc != NODE_NONE { let _ = self.visit_expr(inc); }
                self.loop_depth += 1;
                self.visit(b);
                self.loop_depth -= 1;
            }
            NodeKind::SwitchStmt { expr, body } => {
                let (e, b) = (*expr, *body);
                let _ = self.visit_expr(e);
                self.switch_depth += 1;
                self.visit(b);
                self.switch_depth -= 1;
            }
            NodeKind::CaseStmt { expr, body } => {
                let (e, b) = (*expr, *body);
                if self.switch_depth == 0 {
                    self.diag.error(self.ctx.node(id).span, "'case' statement outside of switch");
                }
                let _ = self.visit_expr(e);
                self.visit(b);
            }
            NodeKind::DefaultStmt { body } => {
                let b = *body;
                if self.switch_depth == 0 {
                    self.diag.error(self.ctx.node(id).span, "'default' statement outside of switch");
                }
                self.visit(b);
            }
            NodeKind::ReturnStmt { expr } => {
                let e = *expr;
                if e != NODE_NONE { let _ = self.visit_expr(e); }
                self.ctx.node_mut(id).ty = self.current_return_type;
            }
            NodeKind::BreakStmt => {
                if self.loop_depth == 0 && self.switch_depth == 0 {
                    self.diag.error(self.ctx.node(id).span, "'break' statement not within loop or switch");
                }
            }
            NodeKind::ContinueStmt => {
                if self.loop_depth == 0 {
                    self.diag.error(self.ctx.node(id).span, "'continue' statement not within loop");
                }
            }
            NodeKind::GotoStmt { .. } => {}
            NodeKind::LabelStmt { stmt, .. } => {
                let s = *stmt;
                self.visit(s);
            }
            NodeKind::ExprStmt { expr } => {
                let e = *expr;
                let _ = self.visit_expr(e);
            }
            NodeKind::NullStmt => {}
            _ => { let _ = self.visit_expr(id); }
        }
    }

    // ── Expression Type Checking ──────────────────────────────────────

    fn visit_expr(&mut self, id: NodeId) -> TypeId {
        if id == NODE_NONE { return self.bt.void; }
        let kind = self.ctx.node(id).kind.clone();
        let ty = match &kind {
            NodeKind::IntLiteral { value, suffix } => {
                match suffix {
                    IntSuffix::None => {
                        if *value <= i32::MAX as u64 { self.bt.int }
                        else if *value <= u32::MAX as u64 { self.bt.uint }
                        else { self.bt.long }
                    }
                    IntSuffix::U => {
                        if *value <= u32::MAX as u64 { self.bt.uint } else { self.bt.ulong }
                    }
                    IntSuffix::L | IntSuffix::LL => self.bt.long,
                    IntSuffix::UL | IntSuffix::ULL => self.bt.ulong,
                }
            }
            NodeKind::FloatLiteral { suffix, .. } => match suffix {
                FloatSuffix::None => self.bt.double,
                FloatSuffix::F => self.bt.float,
                FloatSuffix::L => self.bt.long_double,
            },
            NodeKind::CharLiteral { .. } => self.bt.int,
            NodeKind::StringLiteral { .. } => self.bt.char_ptr,
            NodeKind::Ident { name } => {
                let name = *name;
                if let Some(sym_id) = self.ctx.lookup_symbol(name) {
                    self.ctx.get_symbol(sym_id).ty
                } else {
                    // Implicit function declaration (C89)
                    let func_ty = self.ctx.push_type(CType::Function {
                        ret: self.bt.int,
                        params: vec![],
                        variadic: true,
                    });
                    let span = self.ctx.node(id).span;
                    self.ctx.add_symbol(Symbol {
                        name,
                        ty: func_ty,
                        kind: SymbolKind::Function,
                        span,
                        scope: ScopeId(0),
                    });
                    func_ty
                }
            }
            NodeKind::BinaryOp { op, lhs, rhs } => {
                let (op, l, r) = (*op, *lhs, *rhs);
                let lty = self.visit_expr(l);
                let rty = self.visit_expr(r);
                self.check_binary_op(op, lty, rty)
            }
            NodeKind::UnaryOp { op, operand } => {
                let (op, o) = (*op, *operand);
                let oty = self.visit_expr(o);
                match op {
                    UnaryOp::Neg | UnaryOp::Plus | UnaryOp::BitNot => self.integer_promote(oty),
                    UnaryOp::LogNot => self.bt.int,
                    UnaryOp::PreInc | UnaryOp::PreDec => oty,
                }
            }
            NodeKind::PostfixOp { operand, .. } => {
                let o = *operand;
                self.visit_expr(o)
            }
            NodeKind::Assign { lhs, rhs, .. } => {
                let (l, r) = (*lhs, *rhs);
                let lty = self.visit_expr(l);
                let _ = self.visit_expr(r);
                lty
            }
            NodeKind::Ternary { cond, then_expr, else_expr } => {
                let (c, t, e) = (*cond, *then_expr, *else_expr);
                let _ = self.visit_expr(c);
                let tty = self.visit_expr(t);
                let ety = self.visit_expr(e);
                self.usual_arithmetic_conversion(tty, ety)
            }
            NodeKind::Call { callee, args } => {
                let callee = *callee;
                let args = args.clone();
                let callee_ty = self.visit_expr(callee);
                for a in &args { let _ = self.visit_expr(*a); }
                if callee_ty != TYPE_NONE {
                    match self.ctx.get_type(callee_ty).clone() {
                        CType::Function { ret, .. } => ret,
                        CType::Pointer { pointee } => {
                            match self.ctx.get_type(pointee).clone() {
                                CType::Function { ret, .. } => ret,
                                _ => self.bt.int,
                            }
                        }
                        _ => self.bt.int,
                    }
                } else { self.bt.int }
            }
            NodeKind::Cast { type_node, expr } => {
                let (tn, e) = (*type_node, *expr);
                let _ = self.visit_expr(e);
                self.resolve_type_node(tn)
            }
            NodeKind::SizeofType { .. } | NodeKind::SizeofExpr { .. } => self.bt.uint,
            NodeKind::MemberAccess { expr, member, is_arrow } => {
                let (e, m, a) = (*expr, *member, *is_arrow);
                let base_ty = self.visit_expr(e);
                self.resolve_member_type(base_ty, m, a)
            }
            NodeKind::ArraySubscript { expr, index } => {
                let (e, i) = (*expr, *index);
                let base_ty = self.visit_expr(e);
                let _ = self.visit_expr(i);
                match self.ctx.get_type(base_ty).clone() {
                    CType::Array { elem, .. } => elem,
                    CType::Pointer { pointee } => pointee,
                    _ => self.bt.int,
                }
            }
            NodeKind::AddrOf { expr } => {
                let e = *expr;
                let ety = self.visit_expr(e);
                self.ctx.push_type(CType::Pointer { pointee: ety })
            }
            NodeKind::Deref { expr } => {
                let e = *expr;
                let ety = self.visit_expr(e);
                match self.ctx.get_type(ety).clone() {
                    CType::Pointer { pointee } => pointee,
                    _ => self.bt.int,
                }
            }
            NodeKind::Comma { lhs, rhs } => {
                let (l, r) = (*lhs, *rhs);
                let _ = self.visit_expr(l);
                self.visit_expr(r)
            }
            NodeKind::InitList { values } => {
                let values = values.clone();
                for v in &values { let _ = self.visit_expr(*v); }
                self.bt.int
            }
            _ => self.bt.int,
        };

        self.ctx.node_mut(id).ty = ty;
        ty
    }

    // ── Type Arithmetic ───────────────────────────────────────────────

    fn integer_promote(&self, ty: TypeId) -> TypeId {
        if ty == TYPE_NONE { return self.bt.int; }
        match self.ctx.get_type(ty) {
            CType::Char | CType::SChar | CType::UChar | CType::Short | CType::UShort => self.bt.int,
            _ => ty,
        }
    }

    fn usual_arithmetic_conversion(&self, lty: TypeId, rty: TypeId) -> TypeId {
        if lty == TYPE_NONE || rty == TYPE_NONE { return self.bt.int; }
        if matches!(self.ctx.get_type(lty), CType::LongDouble) || matches!(self.ctx.get_type(rty), CType::LongDouble) { return self.bt.long_double; }
        if matches!(self.ctx.get_type(lty), CType::Double) || matches!(self.ctx.get_type(rty), CType::Double) { return self.bt.double; }
        if matches!(self.ctx.get_type(lty), CType::Float) || matches!(self.ctx.get_type(rty), CType::Float) { return self.bt.float; }

        let lp = self.integer_promote(lty);
        let rp = self.integer_promote(rty);
        if std::mem::discriminant(self.ctx.get_type(lp)) == std::mem::discriminant(self.ctx.get_type(rp)) { return lp; }

        let l_unsigned = self.ctx.is_unsigned(lp);
        let r_unsigned = self.ctx.is_unsigned(rp);
        if l_unsigned == r_unsigned {
            return if self.type_rank(lp) >= self.type_rank(rp) { lp } else { rp };
        }
        let (unsigned_ty, signed_ty) = if l_unsigned { (lp, rp) } else { (rp, lp) };
        if self.type_rank(unsigned_ty) >= self.type_rank(signed_ty) { unsigned_ty } else { signed_ty }
    }

    fn type_rank(&self, ty: TypeId) -> u32 {
        match self.ctx.get_type(ty) {
            CType::Char | CType::SChar | CType::UChar => 1,
            CType::Short | CType::UShort => 2,
            CType::Int | CType::UInt | CType::Enum { .. } => 3,
            CType::Long | CType::ULong => 4,
            _ => 3,
        }
    }

    fn check_binary_op(&mut self, op: BinOp, lty: TypeId, rty: TypeId) -> TypeId {
        match op {
            BinOp::Add | BinOp::Sub => {
                if self.ctx.is_pointer_type(lty) { return lty; }
                if self.ctx.is_pointer_type(rty) && op == BinOp::Add { return rty; }
                self.usual_arithmetic_conversion(lty, rty)
            }
            BinOp::Mul | BinOp::Div | BinOp::Mod => self.usual_arithmetic_conversion(lty, rty),
            BinOp::Shl | BinOp::Shr => self.integer_promote(lty),
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => self.usual_arithmetic_conversion(lty, rty),
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::LogAnd | BinOp::LogOr => self.bt.int,
        }
    }

    fn resolve_member_type(&self, base_ty: TypeId, member: InternId, is_arrow: bool) -> TypeId {
        let ty = if is_arrow {
            match self.ctx.get_type(base_ty) {
                CType::Pointer { pointee } => *pointee,
                _ => return self.bt.int,
            }
        } else {
            base_ty
        };
        match self.ctx.get_type(ty) {
            CType::Struct { members, .. } | CType::Union { members, .. } => {
                for m in members { if m.name == member { return m.ty; } }
                self.bt.int
            }
            _ => self.bt.int,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::DiagEngine;
    use crate::frontend::lexer;
    use crate::source::SourceMap;
    use crate::target::Target;

    fn analyze_str(src: &str) -> (Ctx, NodeId, DiagEngine) {
        let mut sm = SourceMap::new();
        let fid = sm.add_file("test.c".into(), src.into());
        let diag = DiagEngine::new();
        let tokens = lexer::lex(&sm, fid, &diag);
        let mut ctx = Ctx::new(Target::I386);
        let root = crate::frontend::parser::parse(&tokens, &mut ctx, &diag);
        analyze(&mut ctx, root, &diag);
        (ctx, root, diag)
    }

    #[test]
    fn test_sema_simple_function() {
        let (ctx, root, diag) = analyze_str("int main() { return 0; }");
        assert!(!diag.has_errors());
        match &ctx.node(root).kind {
            NodeKind::TranslationUnit { decls } => {
                assert_ne!(ctx.node(decls[0]).ty, TYPE_NONE);
            }
            _ => panic!("expected TranslationUnit"),
        }
    }

    #[test]
    fn test_sema_variable_type() {
        let (_ctx, _root, diag) = analyze_str("int main() { int x; return x; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_sema_enum_constants() {
        let (_ctx, _root, diag) = analyze_str("enum color { RED, GREEN, BLUE }; int main() { return RED; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_sema_break_outside_loop() {
        let (_ctx, _root, diag) = analyze_str("int main() { break; }");
        assert!(diag.has_errors());
    }

    #[test]
    fn test_sema_break_inside_loop() {
        let (_ctx, _root, diag) = analyze_str("int main() { while(1) break; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_sema_arithmetic() {
        let (_ctx, _root, diag) = analyze_str("int main() { int x = 1 + 2; return x; }");
        assert!(!diag.has_errors());
    }

    #[test]
    fn test_sema_pointer() {
        let (_ctx, _root, diag) = analyze_str("int main() { int *p; int x = *p; return x; }");
        assert!(!diag.has_errors());
    }
}
