// backend/codegen/mod.rs — LLVM IR code generation for C89.
//
// Generates LLVM IR in textual form (.ll) from the typed AST.
// Uses SSA form with explicit alloca/load/store for local variables.

use std::collections::HashMap;

use crate::ctx::*;
use crate::opts::Opts;
use crate::source::InternId;

/// Generate LLVM IR for the translation unit rooted at `root`.
pub fn generate(ctx: &Ctx, root: NodeId, opts: &Opts) -> String {
    let mut cg = CodeGen::new(ctx, opts);
    cg.emit_module(root);
    cg.finish()
}

// ── Value representation ──────────────────────────────────────────────

/// An SSA value in the IR: either an unnamed register (%N) or a constant.
#[derive(Clone, Debug)]
enum Val {
    Reg(u32),           // %N
    Global(String),     // @name
    IntConst(i64),      // immediate integer
    FloatConst(f64),    // immediate float
    None,               // void / no value
}

impl Val {
    fn to_operand(&self) -> String {
        match self {
            Val::Reg(n) => format!("%{}", n),
            Val::Global(s) => format!("@{}", s),
            Val::IntConst(v) => format!("{}", v),
            Val::FloatConst(v) => format!("{:e}", v),
            Val::None => "void".into(),
        }
    }
}

/// Local variable info: alloca register and type.
struct LocalVar {
    alloca_reg: u32,
    ty: TypeId,
}

// ── Code Generator ────────────────────────────────────────────────────

struct CodeGen<'a> {
    ctx: &'a Ctx,
    #[allow(dead_code)]
    opts: &'a Opts,

    // Output buffers
    globals: String,
    func_buf: String,

    // SSA state
    next_reg: u32,
    next_label: u32,
    next_string: u32,

    // Local variables: name → LocalVar
    locals: Vec<HashMap<InternId, LocalVar>>,

    // String literal pool: content → global name
    string_pool: Vec<(Vec<u8>, String)>,

    // Current function's return type
    current_ret_type: TypeId,

    // Break/continue label stacks
    break_labels: Vec<String>,
    continue_labels: Vec<String>,

    // Switch state
    switch_end_labels: Vec<String>,
}

impl<'a> CodeGen<'a> {
    fn new(ctx: &'a Ctx, opts: &'a Opts) -> Self {
        Self {
            ctx,
            opts,
            globals: String::with_capacity(4096),
            func_buf: String::with_capacity(8192),
            next_reg: 0,
            next_label: 0,
            next_string: 0,
            locals: Vec::new(),
            string_pool: Vec::new(),
            current_ret_type: TYPE_NONE,
            break_labels: Vec::new(),
            continue_labels: Vec::new(),
            switch_end_labels: Vec::new(),
        }
    }

    fn finish(self) -> String {
        let mut out = String::with_capacity(self.globals.len() + self.func_buf.len() + 512);

        // Module header
        let (datalayout, triple) = match self.ctx.target {
            crate::target::Target::I386 => (
                "e-m:e-p:32:32-p270:32:32-p271:32:32-p272:64:64-f64:32:64-f80:32-n8:16:32-S128",
                "i386-pc-linux-gnu",
            ),
            crate::target::Target::X86_64 => (
                "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
                "x86_64-pc-linux-gnu",
            ),
        };

        out.push_str(&format!(
            "; ModuleID = '{}'\n\
             target datalayout = \"{}\"\n\
             target triple = \"{}\"\n\n",
            self.opts.input, datalayout, triple
        ));

        // String constants
        for (bytes, name) in &self.string_pool {
            let escaped = escape_llvm_string(bytes);
            out.push_str(&format!(
                "@{} = private unnamed_addr constant [{} x i8] c\"{}\", align 1\n",
                name,
                bytes.len(),
                escaped
            ));
        }
        if !self.string_pool.is_empty() {
            out.push('\n');
        }

        // Global variables
        out.push_str(&self.globals);

        // Functions
        out.push_str(&self.func_buf);

        out
    }

    // ── Helpers ───────────────────────────────────────────────────────

    fn fresh_reg(&mut self) -> u32 {
        let r = self.next_reg;
        self.next_reg += 1;
        r
    }

    fn fresh_label(&mut self, prefix: &str) -> String {
        let n = self.next_label;
        self.next_label += 1;
        format!("{}{}", prefix, n)
    }

    fn emit(&mut self, line: &str) {
        self.func_buf.push_str("  ");
        self.func_buf.push_str(line);
        self.func_buf.push('\n');
    }

    fn emit_label(&mut self, label: &str) {
        self.func_buf.push_str(label);
        self.func_buf.push_str(":\n");
    }

    fn llvm_type(&self, ty: TypeId) -> String {
        if ty == TYPE_NONE {
            return "i32".into();
        }
        self.ctx.llvm_type(ty)
    }

    #[allow(dead_code)]
    fn node_type(&self, id: NodeId) -> TypeId {
        self.ctx.node(id).ty
    }

    fn intern_string(&mut self, bytes: &[u8]) -> (String, usize) {
        // Check if already interned
        for (existing, name) in &self.string_pool {
            if existing == bytes {
                return (name.clone(), bytes.len());
            }
        }
        let name = format!(".str.{}", self.next_string);
        self.next_string += 1;
        self.string_pool.push((bytes.to_vec(), name.clone()));
        (name, bytes.len())
    }

    fn push_scope(&mut self) {
        self.locals.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.locals.pop();
    }

    fn add_local(&mut self, name: InternId, var: LocalVar) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name, var);
        }
    }

    fn lookup_local(&self, name: InternId) -> Option<&LocalVar> {
        for scope in self.locals.iter().rev() {
            if let Some(v) = scope.get(&name) {
                return Some(v);
            }
        }
        None
    }

    // ── Module-level emission ─────────────────────────────────────────

    fn emit_module(&mut self, root: NodeId) {
        let kind = self.ctx.node(root).kind.clone();
        if let NodeKind::TranslationUnit { decls } = kind {
            for d in &decls {
                self.emit_top_level(*d);
            }
        }
    }

    fn emit_top_level(&mut self, id: NodeId) {
        let kind = self.ctx.node(id).kind.clone();
        match kind {
            NodeKind::FuncDef {
                name,
                params,
                body,
                is_variadic,
                storage_class,
                ..
            } => {
                self.emit_func_def(id, name, &params, body, is_variadic, storage_class);
            }
            NodeKind::VarDecl {
                name,
                init,
                storage_class,
                ..
            } => {
                self.emit_global_var(id, name, init, storage_class);
            }
            // Forward declarations of functions
            NodeKind::StructDecl { .. }
            | NodeKind::UnionDecl { .. }
            | NodeKind::EnumDecl { .. }
            | NodeKind::TypedefDecl { .. } => {
                // Type declarations don't generate IR
            }
            _ => {}
        }
    }

    fn emit_global_var(&mut self, id: NodeId, name: InternId, init: NodeId, storage: StorageClass) {
        let ty = self.ctx.node(id).ty;
        let llty = self.llvm_type(ty);
        let name_str = self.ctx.get_str(name);
        let linkage = if storage == StorageClass::Static {
            "internal"
        } else {
            "dso_local"
        };

        let init_str = if init != NODE_NONE {
            // Try to evaluate as a constant
            match &self.ctx.node(init).kind {
                NodeKind::IntLiteral { value, .. } => format!("{}", value),
                NodeKind::FloatLiteral { value, .. } => format!("{:e}", value),
                _ => format!("zeroinitializer"),
            }
        } else {
            "zeroinitializer".into()
        };

        self.globals.push_str(&format!(
            "@{} = {} global {} {}, align {}\n",
            name_str,
            linkage,
            llty,
            init_str,
            self.ctx.type_align(ty).max(1)
        ));
    }

    // ── Function Definition ───────────────────────────────────────────

    fn emit_func_def(
        &mut self,
        id: NodeId,
        name: InternId,
        _params: &[NodeId],
        body: NodeId,
        is_variadic: bool,
        storage_class: StorageClass,
    ) {
        let func_ty = self.ctx.node(id).ty;
        let (ret_ty, param_types) = match self.ctx.get_type(func_ty) {
            CType::Function { ret, params, .. } => (*ret, params.clone()),
            _ => (TYPE_NONE, vec![]),
        };

        self.current_ret_type = ret_ty;
        self.next_reg = 0;
        self.next_label = 0;
        self.locals.clear();
        self.push_scope();

        let name_str = self.ctx.get_str(name).to_string();
        let ret_llty = self.llvm_type(ret_ty);
        let linkage = if storage_class == StorageClass::Static {
            "internal "
        } else {
            ""
        };

        // Build parameter list
        let mut param_strs = Vec::new();
        let mut param_regs = Vec::new();
        for (pname, pty) in &param_types {
            let reg = self.fresh_reg();
            param_strs.push(format!("{} %{}", self.llvm_type(*pty), reg));
            param_regs.push((reg, *pname, *pty));
        }
        if is_variadic {
            param_strs.push("...".into());
        }

        self.func_buf.push_str(&format!(
            "define {}{}@{}({}) {{\n",
            linkage,
            if ret_llty == "void" { "void " } else { &format!("{} ", ret_llty) },
            name_str,
            param_strs.join(", ")
        ));

        // Entry label
        let entry = self.fresh_label("entry");
        self.emit_label(&entry);

        // Allocate and store parameters
        for (arg_reg, pname, pty) in &param_regs {
            let llty = self.llvm_type(*pty);
            let alloca_reg = self.fresh_reg();
            self.emit(&format!("%{} = alloca {}, align {}", alloca_reg, llty, self.ctx.type_align(*pty).max(1)));
            self.emit(&format!("store {} %{}, ptr %{}, align {}", llty, arg_reg, alloca_reg, self.ctx.type_align(*pty).max(1)));
            self.add_local(*pname, LocalVar {
                alloca_reg,
                ty: *pty,
            });
        }

        // Emit function body
        self.emit_stmt(body);

        // Add implicit return if needed
        if matches!(self.ctx.get_type(ret_ty), CType::Void) {
            self.emit("ret void");
        } else {
            // Implicit return 0 for main, undefined for others
            self.emit(&format!("ret {} 0", ret_llty));
        }

        self.func_buf.push_str("}\n\n");
        self.pop_scope();
    }

    // ── Statement Emission ────────────────────────────────────────────

    fn emit_stmt(&mut self, id: NodeId) {
        if id == NODE_NONE {
            return;
        }
        let kind = self.ctx.node(id).kind.clone();
        match kind {
            NodeKind::CompoundStmt { stmts } => {
                self.push_scope();
                for s in &stmts {
                    self.emit_stmt(*s);
                }
                self.pop_scope();
            }
            NodeKind::VarDecl {
                name,
                init,
                ..
            } => {
                let ty = self.ctx.node(id).ty;
                let llty = self.llvm_type(ty);
                let alloca_reg = self.fresh_reg();
                let align = self.ctx.type_align(ty).max(1);
                self.emit(&format!("%{} = alloca {}, align {}", alloca_reg, llty, align));
                self.add_local(name, LocalVar {
                    alloca_reg,
                    ty,
                });

                if init != NODE_NONE {
                    let val = self.emit_expr(init);
                    let val_op = val.to_operand();
                    self.emit(&format!("store {} {}, ptr %{}, align {}", llty, val_op, alloca_reg, align));
                }
            }
            NodeKind::ReturnStmt { expr } => {
                if expr != NODE_NONE {
                    let val = self.emit_expr(expr);
                    let ret_llty = self.llvm_type(self.current_ret_type);
                    self.emit(&format!("ret {} {}", ret_llty, val.to_operand()));
                } else {
                    self.emit("ret void");
                }
                // Emit unreachable block for any code after return
                let after = self.fresh_label("after_ret");
                self.emit_label(&after);
            }
            NodeKind::IfStmt {
                cond,
                then_br,
                else_br,
            } => {
                let cond_val = self.emit_expr(cond);
                let cond_op = cond_val.to_operand();

                // Ensure condition is i1
                let cond_ty = self.ctx.node(cond).ty;
                let cond_reg = self.to_i1(cond_op, cond_ty);

                let then_label = self.fresh_label("if.then");
                let else_label = self.fresh_label("if.else");
                let end_label = self.fresh_label("if.end");

                if else_br != NODE_NONE {
                    self.emit(&format!("br i1 {}, label %{}, label %{}", cond_reg, then_label, else_label));
                    self.emit_label(&then_label);
                    self.emit_stmt(then_br);
                    self.emit(&format!("br label %{}", end_label));
                    self.emit_label(&else_label);
                    self.emit_stmt(else_br);
                    self.emit(&format!("br label %{}", end_label));
                } else {
                    self.emit(&format!("br i1 {}, label %{}, label %{}", cond_reg, then_label, end_label));
                    self.emit_label(&then_label);
                    self.emit_stmt(then_br);
                    self.emit(&format!("br label %{}", end_label));
                }
                self.emit_label(&end_label);
            }
            NodeKind::WhileStmt { cond, body } => {
                let cond_label = self.fresh_label("while.cond");
                let body_label = self.fresh_label("while.body");
                let end_label = self.fresh_label("while.end");

                self.break_labels.push(end_label.clone());
                self.continue_labels.push(cond_label.clone());

                self.emit(&format!("br label %{}", cond_label));
                self.emit_label(&cond_label);
                let cond_val = self.emit_expr(cond);
                let cond_ty = self.ctx.node(cond).ty;
                let cond_i1 = self.to_i1(cond_val.to_operand(), cond_ty);
                self.emit(&format!("br i1 {}, label %{}, label %{}", cond_i1, body_label, end_label));

                self.emit_label(&body_label);
                self.emit_stmt(body);
                self.emit(&format!("br label %{}", cond_label));

                self.emit_label(&end_label);
                self.break_labels.pop();
                self.continue_labels.pop();
            }
            NodeKind::DoWhileStmt { body, cond } => {
                let body_label = self.fresh_label("do.body");
                let cond_label = self.fresh_label("do.cond");
                let end_label = self.fresh_label("do.end");

                self.break_labels.push(end_label.clone());
                self.continue_labels.push(cond_label.clone());

                self.emit(&format!("br label %{}", body_label));
                self.emit_label(&body_label);
                self.emit_stmt(body);
                self.emit(&format!("br label %{}", cond_label));

                self.emit_label(&cond_label);
                let cond_val = self.emit_expr(cond);
                let cond_ty = self.ctx.node(cond).ty;
                let cond_i1 = self.to_i1(cond_val.to_operand(), cond_ty);
                self.emit(&format!("br i1 {}, label %{}, label %{}", cond_i1, body_label, end_label));

                self.emit_label(&end_label);
                self.break_labels.pop();
                self.continue_labels.pop();
            }
            NodeKind::ForStmt {
                init,
                cond,
                incr,
                body,
            } => {
                let cond_label = self.fresh_label("for.cond");
                let body_label = self.fresh_label("for.body");
                let incr_label = self.fresh_label("for.incr");
                let end_label = self.fresh_label("for.end");

                // Init
                self.emit_stmt(init);

                self.break_labels.push(end_label.clone());
                self.continue_labels.push(incr_label.clone());

                self.emit(&format!("br label %{}", cond_label));
                self.emit_label(&cond_label);

                if cond != NODE_NONE {
                    let cond_val = self.emit_expr(cond);
                    let cond_ty = self.ctx.node(cond).ty;
                    let cond_i1 = self.to_i1(cond_val.to_operand(), cond_ty);
                    self.emit(&format!("br i1 {}, label %{}, label %{}", cond_i1, body_label, end_label));
                } else {
                    self.emit(&format!("br label %{}", body_label));
                }

                self.emit_label(&body_label);
                self.emit_stmt(body);
                self.emit(&format!("br label %{}", incr_label));

                self.emit_label(&incr_label);
                if incr != NODE_NONE {
                    let _ = self.emit_expr(incr);
                }
                self.emit(&format!("br label %{}", cond_label));

                self.emit_label(&end_label);
                self.break_labels.pop();
                self.continue_labels.pop();
            }
            NodeKind::SwitchStmt { expr, body } => {
                // Simplified switch: emit as if-else chain
                // (Full LLVM switch instruction requires collecting all cases first)
                let switch_val = self.emit_expr(expr);
                let switch_ty = self.ctx.node(expr).ty;
                let end_label = self.fresh_label("switch.end");

                self.break_labels.push(end_label.clone());
                self.switch_end_labels.push(end_label.clone());

                // Store switch value for case comparisons
                let llty = self.llvm_type(switch_ty);
                let switch_alloca = self.fresh_reg();
                self.emit(&format!("%{} = alloca {}, align 4", switch_alloca, llty));
                self.emit(&format!("store {} {}, ptr %{}, align 4", llty, switch_val.to_operand(), switch_alloca));

                // For now, emit body directly (cases will use switch value)
                self.emit_stmt(body);
                self.emit(&format!("br label %{}", end_label));

                self.emit_label(&end_label);
                self.break_labels.pop();
                self.switch_end_labels.pop();
            }
            NodeKind::CaseStmt { expr: _, body } => {
                let case_label = self.fresh_label("case");
                self.emit(&format!("br label %{}", case_label));
                self.emit_label(&case_label);
                self.emit_stmt(body);
            }
            NodeKind::DefaultStmt { body } => {
                let default_label = self.fresh_label("default");
                self.emit(&format!("br label %{}", default_label));
                self.emit_label(&default_label);
                self.emit_stmt(body);
            }
            NodeKind::BreakStmt => {
                if let Some(label) = self.break_labels.last().cloned() {
                    self.emit(&format!("br label %{}", label));
                    let after = self.fresh_label("after_break");
                    self.emit_label(&after);
                }
            }
            NodeKind::ContinueStmt => {
                if let Some(label) = self.continue_labels.last().cloned() {
                    self.emit(&format!("br label %{}", label));
                    let after = self.fresh_label("after_continue");
                    self.emit_label(&after);
                }
            }
            NodeKind::GotoStmt { label } => {
                let label_str = self.ctx.get_str(label).to_string();
                self.emit(&format!("br label %label.{}", label_str));
                let after = self.fresh_label("after_goto");
                self.emit_label(&after);
            }
            NodeKind::LabelStmt { label, stmt } => {
                let label_str = self.ctx.get_str(label).to_string();
                self.emit(&format!("br label %label.{}", label_str));
                self.emit_label(&format!("label.{}", label_str));
                self.emit_stmt(stmt);
            }
            NodeKind::ExprStmt { expr } => {
                let _ = self.emit_expr(expr);
            }
            NodeKind::NullStmt => {}
            // Type declarations at block scope
            NodeKind::StructDecl { .. }
            | NodeKind::UnionDecl { .. }
            | NodeKind::EnumDecl { .. }
            | NodeKind::TypedefDecl { .. } => {}
            // Any expression used as a statement (e.g. for-init assign, bare calls)
            _ => {
                let _ = self.emit_expr(id);
            }
        }
    }

    // ── Expression Emission ───────────────────────────────────────────

    fn emit_expr(&mut self, id: NodeId) -> Val {
        if id == NODE_NONE {
            return Val::None;
        }
        let kind = self.ctx.node(id).kind.clone();
        match kind {
            NodeKind::IntLiteral { value, .. } => Val::IntConst(value as i64),
            NodeKind::FloatLiteral { value, .. } => Val::FloatConst(value),
            NodeKind::CharLiteral { value } => Val::IntConst(value as i64),
            NodeKind::StringLiteral { bytes } => {
                let (name, len) = self.intern_string(&bytes);
                let reg = self.fresh_reg();
                self.emit(&format!(
                    "%{} = getelementptr inbounds [{} x i8], ptr @{}, i32 0, i32 0",
                    reg, len, name
                ));
                Val::Reg(reg)
            }
            NodeKind::Ident { name } => {
                // Load the variable
                if let Some(local) = self.lookup_local(name) {
                    let alloca = local.alloca_reg;
                    let ty = local.ty;
                    let llty = self.llvm_type(ty);
                    let align = self.ctx.type_align(ty).max(1);
                    let reg = self.fresh_reg();
                    self.emit(&format!("%{} = load {}, ptr %{}, align {}", reg, llty, alloca, align));
                    Val::Reg(reg)
                } else {
                    // Could be a global or function — check if it's an enum constant
                    let name_str = self.ctx.get_str(name).to_string();
                    if let Some(sym_id) = self.ctx.lookup_symbol(name) {
                        let sym = self.ctx.get_symbol(sym_id);
                        match &sym.kind {
                            SymbolKind::EnumConstant(v) => Val::IntConst(*v),
                            SymbolKind::Function => Val::Global(name_str),
                            SymbolKind::Variable => {
                                // Global variable
                                let ty = sym.ty;
                                let llty = self.llvm_type(ty);
                                let align = self.ctx.type_align(ty).max(1);
                                let reg = self.fresh_reg();
                                self.emit(&format!("%{} = load {}, ptr @{}, align {}", reg, llty, name_str, align));
                                Val::Reg(reg)
                            }
                            _ => Val::IntConst(0),
                        }
                    } else {
                        Val::Global(name_str)
                    }
                }
            }
            NodeKind::BinaryOp { op, lhs, rhs } => {
                let lval = self.emit_expr(lhs);
                let rval = self.emit_expr(rhs);
                let result_ty = self.ctx.node(id).ty;
                self.emit_binop(op, lval, rval, result_ty)
            }
            NodeKind::UnaryOp { op, operand } => {
                match op {
                    UnaryOp::Neg => {
                        let val = self.emit_expr(operand);
                        let ty = self.ctx.node(id).ty;
                        let llty = self.llvm_type(ty);
                        let reg = self.fresh_reg();
                        if self.ctx.is_float_type(ty) {
                            self.emit(&format!("%{} = fneg {} {}", reg, llty, val.to_operand()));
                        } else {
                            self.emit(&format!("%{} = sub {} 0, {}", reg, llty, val.to_operand()));
                        }
                        Val::Reg(reg)
                    }
                    UnaryOp::Plus => self.emit_expr(operand),
                    UnaryOp::BitNot => {
                        let val = self.emit_expr(operand);
                        let ty = self.ctx.node(id).ty;
                        let llty = self.llvm_type(ty);
                        let reg = self.fresh_reg();
                        self.emit(&format!("%{} = xor {} {}, -1", reg, llty, val.to_operand()));
                        Val::Reg(reg)
                    }
                    UnaryOp::LogNot => {
                        let val = self.emit_expr(operand);
                        let ty = self.ctx.node(operand).ty;
                        let cmp_reg = self.to_i1(val.to_operand(), ty);
                        let not_reg = self.fresh_reg();
                        self.emit(&format!("%{} = xor i1 {}, true", not_reg, cmp_reg));
                        let ext_reg = self.fresh_reg();
                        self.emit(&format!("%{} = zext i1 %{} to i32", ext_reg, not_reg));
                        Val::Reg(ext_reg)
                    }
                    UnaryOp::PreInc | UnaryOp::PreDec => {
                        // ++x: load, add 1, store, return new value
                        let (alloca, ty) = self.get_lvalue_addr(operand);
                        let llty = self.llvm_type(ty);
                        let align = self.ctx.type_align(ty).max(1);
                        let load_reg = self.fresh_reg();
                        self.emit(&format!("%{} = load {}, ptr {}, align {}", load_reg, llty, alloca, align));
                        let result = self.fresh_reg();
                        let delta = if op == UnaryOp::PreInc { "add" } else { "sub" };
                        self.emit(&format!("%{} = {} {} %{}, 1", result, delta, llty, load_reg));
                        self.emit(&format!("store {} %{}, ptr {}, align {}", llty, result, alloca, align));
                        Val::Reg(result)
                    }
                }
            }
            NodeKind::PostfixOp { op, operand } => {
                let (alloca, ty) = self.get_lvalue_addr(operand);
                let llty = self.llvm_type(ty);
                let align = self.ctx.type_align(ty).max(1);
                let old_reg = self.fresh_reg();
                self.emit(&format!("%{} = load {}, ptr {}, align {}", old_reg, llty, alloca, align));
                let new_reg = self.fresh_reg();
                let delta = if op == PostfixOp::PostInc { "add" } else { "sub" };
                self.emit(&format!("%{} = {} {} %{}, 1", new_reg, delta, llty, old_reg));
                self.emit(&format!("store {} %{}, ptr {}, align {}", llty, new_reg, alloca, align));
                Val::Reg(old_reg)
            }
            NodeKind::Assign { op, lhs, rhs } => {
                let rval = self.emit_expr(rhs);
                let (alloca, ty) = self.get_lvalue_addr(lhs);
                let llty = self.llvm_type(ty);
                let align = self.ctx.type_align(ty).max(1);

                let store_val = if op == AssignOp::Assign {
                    rval
                } else {
                    // Compound assignment: load, operate, store
                    let load_reg = self.fresh_reg();
                    self.emit(&format!("%{} = load {}, ptr {}, align {}", load_reg, llty, alloca, align));
                    let binop = match op {
                        AssignOp::AddAssign => BinOp::Add,
                        AssignOp::SubAssign => BinOp::Sub,
                        AssignOp::MulAssign => BinOp::Mul,
                        AssignOp::DivAssign => BinOp::Div,
                        AssignOp::ModAssign => BinOp::Mod,
                        AssignOp::ShlAssign => BinOp::Shl,
                        AssignOp::ShrAssign => BinOp::Shr,
                        AssignOp::AndAssign => BinOp::BitAnd,
                        AssignOp::XorAssign => BinOp::BitXor,
                        AssignOp::OrAssign => BinOp::BitOr,
                        _ => unreachable!(),
                    };
                    self.emit_binop(binop, Val::Reg(load_reg), rval, ty)
                };

                self.emit(&format!("store {} {}, ptr {}, align {}", llty, store_val.to_operand(), alloca, align));
                store_val
            }
            NodeKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                let cond_val = self.emit_expr(cond);
                let cond_ty = self.ctx.node(cond).ty;
                let cond_i1 = self.to_i1(cond_val.to_operand(), cond_ty);

                let then_label = self.fresh_label("ternary.then");
                let else_label = self.fresh_label("ternary.else");
                let end_label = self.fresh_label("ternary.end");

                self.emit(&format!("br i1 {}, label %{}, label %{}", cond_i1, then_label, else_label));

                self.emit_label(&then_label);
                let then_val = self.emit_expr(then_expr);
                let then_op = then_val.to_operand();
                let _then_exit = self.fresh_label("ternary.then.exit");
                self.emit(&format!("br label %{}", end_label));

                self.emit_label(&else_label);
                let else_val = self.emit_expr(else_expr);
                let else_op = else_val.to_operand();
                self.emit(&format!("br label %{}", end_label));

                self.emit_label(&end_label);
                let result_ty = self.ctx.node(id).ty;
                let llty = self.llvm_type(result_ty);
                let phi_reg = self.fresh_reg();
                self.emit(&format!(
                    "%{} = phi {} [ {}, %{} ], [ {}, %{} ]",
                    phi_reg, llty, then_op, then_label, else_op, else_label
                ));
                Val::Reg(phi_reg)
            }
            NodeKind::Call { callee, args } => {
                // Emit arguments
                let mut arg_vals = Vec::new();
                for &a in &args {
                    let val = self.emit_expr(a);
                    let aty = self.ctx.node(a).ty;
                    arg_vals.push((val, aty));
                }

                // Get callee
                let callee_name = match &self.ctx.node(callee).kind {
                    NodeKind::Ident { name } => {
                        let s = self.ctx.get_str(*name).to_string();
                        s
                    }
                    _ => {
                        let v = self.emit_expr(callee);
                        v.to_operand()
                    }
                };

                let result_ty = self.ctx.node(id).ty;
                let ret_llty = self.llvm_type(result_ty);

                let mut arg_strs = Vec::new();
                for (val, aty) in &arg_vals {
                    let llty = self.llvm_type(*aty);
                    arg_strs.push(format!("{} {}", llty, val.to_operand()));
                }

                if ret_llty == "void" {
                    self.emit(&format!(
                        "call void @{}({})",
                        callee_name,
                        arg_strs.join(", ")
                    ));
                    Val::None
                } else {
                    let reg = self.fresh_reg();
                    self.emit(&format!(
                        "%{} = call {} @{}({})",
                        reg, ret_llty, callee_name,
                        arg_strs.join(", ")
                    ));
                    Val::Reg(reg)
                }
            }
            NodeKind::Cast { expr, .. } => {
                let val = self.emit_expr(expr);
                let src_ty = self.ctx.node(expr).ty;
                let dst_ty = self.ctx.node(id).ty;
                self.emit_cast(val, src_ty, dst_ty)
            }
            NodeKind::SizeofType { type_node } => {
                // Resolve type and return its size
                let ty = self.ctx.node(type_node).ty;
                let size = if ty != TYPE_NONE {
                    self.ctx.type_size(ty)
                } else {
                    4 // default
                };
                Val::IntConst(size as i64)
            }
            NodeKind::SizeofExpr { expr } => {
                let ty = self.ctx.node(expr).ty;
                let size = if ty != TYPE_NONE {
                    self.ctx.type_size(ty)
                } else {
                    4
                };
                Val::IntConst(size as i64)
            }
            NodeKind::AddrOf { expr } => {
                let (addr, _) = self.get_lvalue_addr(expr);
                // addr is already a pointer (ptr %N or ptr @name)
                // Return it as a register value
                match addr.strip_prefix('%') {
                    Some(n) => {
                        if let Ok(r) = n.parse::<u32>() {
                            Val::Reg(r)
                        } else {
                            Val::Global(addr)
                        }
                    }
                    None => Val::Global(addr.trim_start_matches('@').to_string()),
                }
            }
            NodeKind::Deref { expr } => {
                let ptr = self.emit_expr(expr);
                let result_ty = self.ctx.node(id).ty;
                let llty = self.llvm_type(result_ty);
                let align = self.ctx.type_align(result_ty).max(1);
                let reg = self.fresh_reg();
                self.emit(&format!("%{} = load {}, ptr {}, align {}", reg, llty, ptr.to_operand(), align));
                Val::Reg(reg)
            }
            NodeKind::ArraySubscript { expr, index } => {
                let base = self.emit_expr(expr);
                let idx = self.emit_expr(index);
                let result_ty = self.ctx.node(id).ty;
                let llty = self.llvm_type(result_ty);
                let gep_reg = self.fresh_reg();
                self.emit(&format!(
                    "%{} = getelementptr inbounds {}, ptr {}, i32 {}",
                    gep_reg, llty, base.to_operand(), idx.to_operand()
                ));
                let load_reg = self.fresh_reg();
                let align = self.ctx.type_align(result_ty).max(1);
                self.emit(&format!("%{} = load {}, ptr %{}, align {}", load_reg, llty, gep_reg, align));
                Val::Reg(load_reg)
            }
            NodeKind::MemberAccess {
                expr,
                member,
                is_arrow,
            } => {
                // Simplified: calculate GEP offset
                let base = if is_arrow {
                    self.emit_expr(expr)
                } else {
                    let (addr, _) = self.get_lvalue_addr(expr);
                    match addr.strip_prefix('%') {
                        Some(n) => {
                            if let Ok(r) = n.parse::<u32>() {
                                Val::Reg(r)
                            } else {
                                Val::Global(addr)
                            }
                        }
                        None => Val::Global(addr.trim_start_matches('@').to_string()),
                    }
                };

                let base_ty = if is_arrow {
                    match self.ctx.get_type(self.ctx.node(expr).ty) {
                        CType::Pointer { pointee } => *pointee,
                        _ => self.ctx.node(expr).ty,
                    }
                } else {
                    self.ctx.node(expr).ty
                };

                // Find member offset
                let (member_offset, member_ty) = self.find_member_offset(base_ty, member);
                let result_llty = self.llvm_type(member_ty);

                // GEP to member
                let gep_reg = self.fresh_reg();
                self.emit(&format!(
                    "%{} = getelementptr inbounds i8, ptr {}, i32 {}",
                    gep_reg, base.to_operand(), member_offset
                ));
                let load_reg = self.fresh_reg();
                let align = self.ctx.type_align(member_ty).max(1);
                self.emit(&format!(
                    "%{} = load {}, ptr %{}, align {}",
                    load_reg, result_llty, gep_reg, align
                ));
                Val::Reg(load_reg)
            }
            NodeKind::Comma { lhs, rhs } => {
                let _ = self.emit_expr(lhs);
                self.emit_expr(rhs)
            }
            _ => Val::IntConst(0),
        }
    }

    // ── Binary operation emission ─────────────────────────────────────

    fn emit_binop(&mut self, op: BinOp, lval: Val, rval: Val, result_ty: TypeId) -> Val {
        let llty = self.llvm_type(result_ty);
        let l = lval.to_operand();
        let r = rval.to_operand();
        let is_float = self.ctx.is_float_type(result_ty);
        let is_unsigned = self.ctx.is_unsigned(result_ty);

        match op {
            BinOp::Add => {
                let reg = self.fresh_reg();
                if is_float {
                    self.emit(&format!("%{} = fadd {} {}, {}", reg, llty, l, r));
                } else {
                    self.emit(&format!("%{} = add {} {}, {}", reg, llty, l, r));
                }
                Val::Reg(reg)
            }
            BinOp::Sub => {
                let reg = self.fresh_reg();
                if is_float {
                    self.emit(&format!("%{} = fsub {} {}, {}", reg, llty, l, r));
                } else {
                    self.emit(&format!("%{} = sub {} {}, {}", reg, llty, l, r));
                }
                Val::Reg(reg)
            }
            BinOp::Mul => {
                let reg = self.fresh_reg();
                if is_float {
                    self.emit(&format!("%{} = fmul {} {}, {}", reg, llty, l, r));
                } else {
                    self.emit(&format!("%{} = mul {} {}, {}", reg, llty, l, r));
                }
                Val::Reg(reg)
            }
            BinOp::Div => {
                let reg = self.fresh_reg();
                if is_float {
                    self.emit(&format!("%{} = fdiv {} {}, {}", reg, llty, l, r));
                } else if is_unsigned {
                    self.emit(&format!("%{} = udiv {} {}, {}", reg, llty, l, r));
                } else {
                    self.emit(&format!("%{} = sdiv {} {}, {}", reg, llty, l, r));
                }
                Val::Reg(reg)
            }
            BinOp::Mod => {
                let reg = self.fresh_reg();
                if is_unsigned {
                    self.emit(&format!("%{} = urem {} {}, {}", reg, llty, l, r));
                } else {
                    self.emit(&format!("%{} = srem {} {}, {}", reg, llty, l, r));
                }
                Val::Reg(reg)
            }
            BinOp::Shl => {
                let reg = self.fresh_reg();
                self.emit(&format!("%{} = shl {} {}, {}", reg, llty, l, r));
                Val::Reg(reg)
            }
            BinOp::Shr => {
                let reg = self.fresh_reg();
                if is_unsigned {
                    self.emit(&format!("%{} = lshr {} {}, {}", reg, llty, l, r));
                } else {
                    self.emit(&format!("%{} = ashr {} {}, {}", reg, llty, l, r));
                }
                Val::Reg(reg)
            }
            BinOp::BitAnd => {
                let reg = self.fresh_reg();
                self.emit(&format!("%{} = and {} {}, {}", reg, llty, l, r));
                Val::Reg(reg)
            }
            BinOp::BitOr => {
                let reg = self.fresh_reg();
                self.emit(&format!("%{} = or {} {}, {}", reg, llty, l, r));
                Val::Reg(reg)
            }
            BinOp::BitXor => {
                let reg = self.fresh_reg();
                self.emit(&format!("%{} = xor {} {}, {}", reg, llty, l, r));
                Val::Reg(reg)
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                // Compare, then zext to i32
                let cmp_op = if is_float {
                    match op {
                        BinOp::Eq => "fcmp oeq",
                        BinOp::Ne => "fcmp one",
                        BinOp::Lt => "fcmp olt",
                        BinOp::Gt => "fcmp ogt",
                        BinOp::Le => "fcmp ole",
                        BinOp::Ge => "fcmp oge",
                        _ => unreachable!(),
                    }
                } else if is_unsigned {
                    match op {
                        BinOp::Eq => "icmp eq",
                        BinOp::Ne => "icmp ne",
                        BinOp::Lt => "icmp ult",
                        BinOp::Gt => "icmp ugt",
                        BinOp::Le => "icmp ule",
                        BinOp::Ge => "icmp uge",
                        _ => unreachable!(),
                    }
                } else {
                    match op {
                        BinOp::Eq => "icmp eq",
                        BinOp::Ne => "icmp ne",
                        BinOp::Lt => "icmp slt",
                        BinOp::Gt => "icmp sgt",
                        BinOp::Le => "icmp sle",
                        BinOp::Ge => "icmp sge",
                        _ => unreachable!(),
                    }
                };

                let cmp_reg = self.fresh_reg();
                self.emit(&format!("%{} = {} {} {}, {}", cmp_reg, cmp_op, llty, l, r));
                let zext_reg = self.fresh_reg();
                self.emit(&format!("%{} = zext i1 %{} to i32", zext_reg, cmp_reg));
                Val::Reg(zext_reg)
            }
            BinOp::LogAnd => {
                // Short-circuit: a && b
                let lhs_i1 = self.to_i1(l.clone(), result_ty);
                let entry_label = self.fresh_label("land.entry");
                let rhs_label = self.fresh_label("land.rhs");
                let end_label = self.fresh_label("land.end");

                // We need the current block label for phi — emit an explicit branch
                self.emit(&format!("br label %{}", entry_label));
                self.emit_label(&entry_label);
                self.emit(&format!("br i1 {}, label %{}, label %{}", lhs_i1, rhs_label, end_label));

                self.emit_label(&rhs_label);
                let rhs_i1 = self.to_i1(r.clone(), result_ty);
                self.emit(&format!("br label %{}", end_label));

                self.emit_label(&end_label);
                let phi = self.fresh_reg();
                self.emit(&format!(
                    "%{} = phi i1 [ false, %{} ], [ {}, %{} ]",
                    phi, entry_label, rhs_i1, rhs_label
                ));
                let zext_reg = self.fresh_reg();
                self.emit(&format!("%{} = zext i1 %{} to i32", zext_reg, phi));
                Val::Reg(zext_reg)
            }
            BinOp::LogOr => {
                // Short-circuit: a || b
                let lhs_i1 = self.to_i1(l.clone(), result_ty);
                let entry_label = self.fresh_label("lor.entry");
                let rhs_label = self.fresh_label("lor.rhs");
                let end_label = self.fresh_label("lor.end");

                self.emit(&format!("br label %{}", entry_label));
                self.emit_label(&entry_label);
                self.emit(&format!("br i1 {}, label %{}, label %{}", lhs_i1, end_label, rhs_label));

                self.emit_label(&rhs_label);
                let rhs_i1 = self.to_i1(r.clone(), result_ty);
                self.emit(&format!("br label %{}", end_label));

                self.emit_label(&end_label);
                let phi = self.fresh_reg();
                self.emit(&format!(
                    "%{} = phi i1 [ true, %{} ], [ {}, %{} ]",
                    phi, entry_label, rhs_i1, rhs_label
                ));
                let zext_reg = self.fresh_reg();
                self.emit(&format!("%{} = zext i1 %{} to i32", zext_reg, phi));
                Val::Reg(zext_reg)
            }
        }
    }

    // ── Type casting ──────────────────────────────────────────────────

    fn emit_cast(&mut self, val: Val, src_ty: TypeId, dst_ty: TypeId) -> Val {
        if src_ty == dst_ty || src_ty == TYPE_NONE || dst_ty == TYPE_NONE {
            return val;
        }

        let src_llty = self.llvm_type(src_ty);
        let dst_llty = self.llvm_type(dst_ty);

        if src_llty == dst_llty {
            return val;
        }

        let src_is_float = self.ctx.is_float_type(src_ty);
        let dst_is_float = self.ctx.is_float_type(dst_ty);
        let src_is_ptr = self.ctx.is_pointer_type(src_ty);
        let dst_is_ptr = self.ctx.is_pointer_type(dst_ty);

        let reg = self.fresh_reg();

        if src_is_float && dst_is_float {
            // Float-to-float conversion
            let src_size = self.ctx.type_size(src_ty);
            let dst_size = self.ctx.type_size(dst_ty);
            if dst_size > src_size {
                self.emit(&format!("%{} = fpext {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
            } else {
                self.emit(&format!("%{} = fptrunc {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
            }
        } else if src_is_float && !dst_is_float {
            // Float-to-int
            if self.ctx.is_unsigned(dst_ty) {
                self.emit(&format!("%{} = fptoui {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
            } else {
                self.emit(&format!("%{} = fptosi {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
            }
        } else if !src_is_float && dst_is_float {
            // Int-to-float
            if self.ctx.is_unsigned(src_ty) {
                self.emit(&format!("%{} = uitofp {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
            } else {
                self.emit(&format!("%{} = sitofp {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
            }
        } else if src_is_ptr && !dst_is_ptr {
            // Pointer to int
            self.emit(&format!("%{} = ptrtoint {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
        } else if !src_is_ptr && dst_is_ptr {
            // Int to pointer
            self.emit(&format!("%{} = inttoptr {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
        } else {
            // Integer-to-integer
            let src_size = self.ctx.type_size(src_ty);
            let dst_size = self.ctx.type_size(dst_ty);
            if dst_size > src_size {
                if self.ctx.is_unsigned(src_ty) {
                    self.emit(&format!("%{} = zext {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
                } else {
                    self.emit(&format!("%{} = sext {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
                }
            } else if dst_size < src_size {
                self.emit(&format!("%{} = trunc {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
            } else {
                // Same size, different signedness: bitcast
                self.emit(&format!("%{} = bitcast {} {} to {}", reg, src_llty, val.to_operand(), dst_llty));
            }
        }

        Val::Reg(reg)
    }

    // ── Helpers ───────────────────────────────────────────────────────

    /// Convert a value to i1 (boolean). For use in branch conditions.
    fn to_i1(&mut self, operand: String, ty: TypeId) -> String {
        let llty = self.llvm_type(ty);
        let reg = self.fresh_reg();
        if self.ctx.is_float_type(ty) {
            self.emit(&format!("%{} = fcmp one {} {}, 0.0", reg, llty, operand));
        } else if self.ctx.is_pointer_type(ty) {
            self.emit(&format!("%{} = icmp ne {} {}, null", reg, llty, operand));
        } else {
            self.emit(&format!("%{} = icmp ne {} {}, 0", reg, llty, operand));
        }
        format!("%{}", reg)
    }

    /// Get the address (as an operand string) and type of an lvalue.
    fn get_lvalue_addr(&mut self, id: NodeId) -> (String, TypeId) {
        let kind = self.ctx.node(id).kind.clone();
        match kind {
            NodeKind::Ident { name } => {
                if let Some(local) = self.lookup_local(name) {
                    (format!("%{}", local.alloca_reg), local.ty)
                } else {
                    let name_str = self.ctx.get_str(name).to_string();
                    let ty = self.ctx.node(id).ty;
                    (format!("@{}", name_str), ty)
                }
            }
            NodeKind::Deref { expr } => {
                let ptr = self.emit_expr(expr);
                let result_ty = self.ctx.node(id).ty;
                (ptr.to_operand(), result_ty)
            }
            NodeKind::ArraySubscript { expr, index } => {
                let base = self.emit_expr(expr);
                let idx = self.emit_expr(index);
                let elem_ty = self.ctx.node(id).ty;
                let llty = self.llvm_type(elem_ty);
                let gep_reg = self.fresh_reg();
                self.emit(&format!(
                    "%{} = getelementptr inbounds {}, ptr {}, i32 {}",
                    gep_reg, llty, base.to_operand(), idx.to_operand()
                ));
                (format!("%{}", gep_reg), elem_ty)
            }
            NodeKind::MemberAccess {
                expr,
                member,
                is_arrow,
            } => {
                let base = if is_arrow {
                    self.emit_expr(expr)
                } else {
                    let (addr, _) = self.get_lvalue_addr(expr);
                    match addr.strip_prefix('%') {
                        Some(n) => {
                            if let Ok(r) = n.parse::<u32>() {
                                Val::Reg(r)
                            } else {
                                Val::Global(addr)
                            }
                        }
                        None => Val::Global(addr.trim_start_matches('@').to_string()),
                    }
                };

                let base_ty = if is_arrow {
                    match self.ctx.get_type(self.ctx.node(expr).ty) {
                        CType::Pointer { pointee } => *pointee,
                        _ => self.ctx.node(expr).ty,
                    }
                } else {
                    self.ctx.node(expr).ty
                };

                let (offset, member_ty) = self.find_member_offset(base_ty, member);
                let gep_reg = self.fresh_reg();
                self.emit(&format!(
                    "%{} = getelementptr inbounds i8, ptr {}, i32 {}",
                    gep_reg, base.to_operand(), offset
                ));
                (format!("%{}", gep_reg), member_ty)
            }
            _ => {
                // Fallback: emit as value and hope it's an address
                let val = self.emit_expr(id);
                let ty = self.ctx.node(id).ty;
                (val.to_operand(), ty)
            }
        }
    }

    fn find_member_offset(&self, ty: TypeId, member: InternId) -> (u32, TypeId) {
        if ty == TYPE_NONE {
            return (0, TYPE_NONE);
        }
        match self.ctx.get_type(ty) {
            CType::Struct { members, .. } | CType::Union { members, .. } => {
                for m in members {
                    if m.name == member {
                        return (m.offset, m.ty);
                    }
                }
                (0, TYPE_NONE)
            }
            _ => (0, TYPE_NONE),
        }
    }
}

// ── String escaping for LLVM IR ───────────────────────────────────────

fn escape_llvm_string(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if b == b'\\' {
            out.push_str("\\5C");
        } else if b == b'"' {
            out.push_str("\\22");
        } else if b == 0 {
            out.push_str("\\00");
        } else if b == b'\n' {
            out.push_str("\\0A");
        } else if b == b'\t' {
            out.push_str("\\09");
        } else if b == b'\r' {
            out.push_str("\\0D");
        } else if b >= 0x20 && b < 0x7f {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\{:02X}", b));
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::DiagEngine;
    use crate::frontend::lexer;
    use crate::source::SourceMap;
    use crate::target::Target;

    fn compile_str(src: &str) -> String {
        let mut sm = SourceMap::new();
        let fid = sm.add_file("test.c".into(), src.into());
        let diag = DiagEngine::new();
        let tokens = lexer::lex(&sm, fid, &diag);
        let mut ctx = Ctx::new(Target::I386);
        let root = crate::frontend::parser::parse(&tokens, &mut ctx, &diag);
        assert!(!diag.has_errors(), "parse errors");
        crate::frontend::sema::analyze(&mut ctx, root, &diag);
        assert!(!diag.has_errors(), "sema errors");
        let opts = Opts {
            input: "test.c".into(),
            output: None,
            target: Target::I386,
            emit_mode: crate::opts::EmitMode::LlvmIr,
            emit_debug: false,
            dump_tokens: false,
            dump_ast: false,
            dump_types: false,
            dump_ir: false,
            dump_ssa_ir: false,
            preprocess_only: false,
        };
        generate(&ctx, root, &opts)
    }

    #[test]
    fn test_codegen_return_42() {
        let ir = compile_str("int main() { return 42; }");
        assert!(ir.contains("define"), "should define a function");
        assert!(ir.contains("@main"), "should define main");
        assert!(ir.contains("ret i32 42"), "should return 42");
    }

    #[test]
    fn test_codegen_addition() {
        let ir = compile_str("int main() { return 1 + 2; }");
        assert!(ir.contains("add i32"));
    }

    #[test]
    fn test_codegen_local_var() {
        let ir = compile_str("int main() { int x = 42; return x; }");
        assert!(ir.contains("alloca i32"), "should allocate local");
        assert!(ir.contains("store i32 42"), "should store value");
        assert!(ir.contains("load i32"), "should load value");
    }

    #[test]
    fn test_codegen_if_else() {
        let ir = compile_str("int main() { if (1) return 42; else return 0; }");
        assert!(ir.contains("br i1"), "should have conditional branch");
        assert!(ir.contains("if.then"), "should have then label");
        assert!(ir.contains("if.else"), "should have else label");
    }

    #[test]
    fn test_codegen_while_loop() {
        let ir = compile_str("int main() { int i = 0; while (i < 10) { i = i + 1; } return i; }");
        assert!(ir.contains("while.cond"), "should have while condition");
        assert!(ir.contains("while.body"), "should have while body");
    }

    #[test]
    fn test_codegen_string_literal() {
        let ir = compile_str("int main() { char *s = \"hello\"; return 0; }");
        assert!(ir.contains(".str."), "should have string constant");
        assert!(ir.contains("hello"), "should contain hello");
    }

    #[test]
    fn test_codegen_function_call() {
        let ir = compile_str("int add(int a, int b) { return a + b; } int main() { return add(1, 2); }");
        assert!(ir.contains("@add"), "should define add");
        assert!(ir.contains("call i32 @add"), "should call add");
    }

    #[test]
    fn test_codegen_for_loop() {
        let ir = compile_str("int main() { int s = 0; int i; for (i = 0; i < 5; i = i + 1) s = s + i; return s; }");
        assert!(ir.contains("for.cond"), "should have for condition");
        assert!(ir.contains("for.body"), "should have for body");
        assert!(ir.contains("for.incr"), "should have for increment");
    }

    #[test]
    fn test_codegen_module_header() {
        let ir = compile_str("int main() { return 0; }");
        assert!(ir.contains("target datalayout"), "should have datalayout");
        assert!(ir.contains("target triple"), "should have triple");
        assert!(ir.contains("i386"), "should target i386");
    }
}
