// ir/lower.rs — AST-to-IR lowering.
//
// Walks the typed AST (from sema) and produces an IrModule with functions,
// globals, and basic blocks in SSA form. Local variables use alloca/load/store
// (mem2reg-style promotion is left to the backend optimiser).

use std::collections::HashMap;
use crate::ctx::*;
use crate::ir::instruction::*;
use crate::ir::module::*;
use crate::ir::types::*;
use crate::source::InternId;

/// Convert a fully type-checked AST rooted at `root` into SSA IR.
pub fn lower(ctx: &Ctx, root: NodeId) -> IrModule {
    let mut lo = Lowering::new(ctx);
    lo.lower_module(root);
    lo.module
}

// ── CaseInfo ──────────────────────────────────────────────────────────

struct CaseInfo {
    value: i64,
    is_default: bool,
    block: BlockId,
    body: NodeId,
    index: usize,
}

// ── Lowering State ────────────────────────────────────────────────────

pub struct Lowering<'a> {
    ctx: &'a Ctx,
    module: IrModule,

    // --- Per-function state (reset on each function) ---
    /// Current function being built (index in module.functions).
    cur_func_idx: Option<usize>,
    /// Current block being built.
    cur_block: BlockId,
    /// Local variables: InternId → alloca ValueId.
    locals: Vec<HashMap<InternId, ValueId>>,
    /// Break target stack (for loops/switch).
    break_targets: Vec<BlockId>,
    /// Continue target stack (for loops).
    continue_targets: Vec<BlockId>,
    /// Switch end labels.
    #[allow(dead_code)]
    switch_end: Vec<BlockId>,
    /// Current function return type (CType TypeId).
    ret_type: TypeId,
    /// Whether a terminator has been emitted for the current block.
    block_terminated: bool,
    /// Label map: InternId (goto label) → BlockId.
    label_map: HashMap<InternId, BlockId>,
    /// Forward goto references: (InternId, BlockId-of-br-placeholder).
    forward_gotos: Vec<(InternId, BlockId)>,
}

impl<'a> Lowering<'a> {
    fn new(ctx: &'a Ctx) -> Self {
        Self {
            ctx,
            module: IrModule::new(""),
            cur_func_idx: None,
            cur_block: BLOCK_NONE,
            locals: Vec::new(),
            break_targets: Vec::new(),
            continue_targets: Vec::new(),
            switch_end: Vec::new(),
            ret_type: TYPE_NONE,
            block_terminated: false,
            label_map: HashMap::new(),
            forward_gotos: Vec::new(),
        }
    }

    /// Public entry point: lower a typed AST into an SSA IR module.
    pub fn lower(ctx: &'a Ctx, root: NodeId, source_file: &str) -> IrModule {
        let mut lowering = Lowering::new(ctx);
        lowering.module = IrModule::new(source_file);
        lowering.lower_module(root);
        lowering.module.compute_all_predecessors();
        lowering.module
    }

    // ── Convenience ─────────────────────────────────────────────────

    #[allow(dead_code)]
    fn func(&self) -> &IrFunction {
        &self.module.functions[self.cur_func_idx.unwrap()]
    }

    fn func_mut(&mut self) -> &mut IrFunction {
        &mut self.module.functions[self.cur_func_idx.unwrap()]
    }

    fn alloc_value(&mut self) -> ValueId {
        self.func_mut().alloc_value()
    }

    fn create_block(&mut self, label: &str) -> BlockId {
        self.func_mut().create_block(label)
    }

    /// Emit an instruction in the current block.
    fn emit(&mut self, inst: Instruction) -> Option<ValueId> {
        let bid = self.cur_block;
        let result = inst.result();
        let func = &mut self.module.functions[self.cur_func_idx.unwrap()];
        func.block_mut(bid).push(inst);
        result
    }

    /// Set the terminator of the current block.
    fn terminate(&mut self, term: Terminator) {
        if self.block_terminated {
            return; // already terminated (e.g., after return)
        }
        let bid = self.cur_block;
        let func = &mut self.module.functions[self.cur_func_idx.unwrap()];
        func.block_mut(bid).set_terminator(term);
        self.block_terminated = true;
    }

    /// Switch to emitting into a different block.
    fn switch_to(&mut self, bid: BlockId) {
        self.cur_block = bid;
        self.block_terminated = false;
    }

    fn push_scope(&mut self) {
        self.locals.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.locals.pop();
    }

    fn add_local(&mut self, name: InternId, alloca: ValueId) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name, alloca);
        }
    }

    fn lookup_local(&self, name: InternId) -> Option<ValueId> {
        for scope in self.locals.iter().rev() {
            if let Some(&v) = scope.get(&name) {
                return Some(v);
            }
        }
        None
    }

    /// Convert a CType TypeId to an IrType.
    fn ir_type(&self, tid: TypeId) -> IrType {
        if tid == TYPE_NONE {
            return IrType::I32; // fallback
        }
        match self.ctx.get_type(tid) {
            CType::Void => IrType::Void,
            CType::Char | CType::SChar => IrType::I8,
            CType::UChar => IrType::U8,
            CType::Short => IrType::I16,
            CType::UShort => IrType::U16,
            CType::Int | CType::Enum { .. } => IrType::I32,
            CType::UInt => IrType::U32,
            CType::Long => {
                if self.ctx.target.long_size() == 4 {
                    IrType::I32
                } else {
                    IrType::I64
                }
            }
            CType::ULong => {
                if self.ctx.target.long_size() == 4 {
                    IrType::U32
                } else {
                    IrType::U64
                }
            }
            CType::Float => IrType::F32,
            CType::Double => IrType::F64,
            CType::LongDouble => IrType::F128,
            CType::Pointer { .. } => IrType::Ptr,
            CType::Array { elem, len } => {
                IrType::Array(Box::new(self.ir_type(*elem)), len.unwrap_or(0))
            }
            CType::Struct { members, .. } => {
                IrType::Struct(members.iter().map(|m| self.ir_type(m.ty)).collect())
            }
            CType::Union { size, .. } => {
                // Unions are represented as byte arrays with the union's size.
                IrType::Array(Box::new(IrType::I8), *size as u64)
            }
            CType::Function { .. } => IrType::Ptr, // function type → pointer
        }
    }

    fn is_signed(&self, tid: TypeId) -> bool {
        if tid == TYPE_NONE {
            return true;
        }
        match self.ctx.get_type(tid) {
            CType::Char | CType::SChar | CType::Short | CType::Int | CType::Long => true,
            CType::Enum { .. } => true,
            _ => false,
        }
    }

    // ── Module-level ────────────────────────────────────────────────

    fn lower_module(&mut self, root: NodeId) {
        let kind = self.ctx.node(root).kind.clone();
        if let NodeKind::TranslationUnit { decls } = kind {
            for d in &decls {
                self.lower_top_level(*d);
            }
        }
    }

    fn lower_top_level(&mut self, id: NodeId) {
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
                self.lower_func_def(id, name, &params, body, is_variadic, storage_class);
            }
            NodeKind::VarDecl {
                name,
                init,
                storage_class,
                ..
            } => {
                self.lower_global_var(id, name, init, storage_class);
            }
            _ => {} // struct/union/enum/typedef → no IR
        }
    }

    // ── Global Variables ────────────────────────────────────────────

    fn lower_global_var(
        &mut self,
        id: NodeId,
        name: InternId,
        init: NodeId,
        storage: StorageClass,
    ) {
        let ty = self.ctx.node(id).ty;
        let ir_ty = self.ir_type(ty);
        let name_str = self.ctx.get_str(name).to_string();

        let init_val = if init != NODE_NONE {
            Some(self.eval_global_init(init))
        } else {
            Some(GlobalInit::ZeroFill(self.ctx.type_size(ty) as usize))
        };

        let linkage = if storage == StorageClass::Static {
            Linkage::Internal
        } else {
            Linkage::External
        };

        self.module.add_global(GlobalVariable {
            name: name_str,
            ty: ir_ty,
            init: init_val,
            linkage,
            visibility: Visibility::Default,
            section: None,
            align: self.ctx.type_align(ty),
            is_const: false,
            is_tls: false,
        });
    }

    /// Evaluate a constant initializer for a global variable.
    fn eval_global_init(&self, id: NodeId) -> GlobalInit {
        match &self.ctx.node(id).kind {
            NodeKind::IntLiteral { value, .. } => GlobalInit::Integer(*value),
            NodeKind::FloatLiteral { value, .. } => GlobalInit::Float(*value),
            NodeKind::StringLiteral { bytes } => GlobalInit::String(bytes.clone()),
            NodeKind::InitList { values } => {
                let parts: Vec<(usize, GlobalInit)> = values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i, self.eval_global_init(*v)))
                    .collect();
                GlobalInit::Compound(parts)
            }
            NodeKind::UnaryOp {
                op: UnaryOp::Neg,
                operand,
            } => {
                if let NodeKind::IntLiteral { value, .. } = &self.ctx.node(*operand).kind {
                    GlobalInit::Integer((*value).wrapping_neg())
                } else {
                    GlobalInit::ZeroFill(0)
                }
            }
            _ => GlobalInit::ZeroFill(0), // fallback for complex exprs
        }
    }

    // ── Function Definition ─────────────────────────────────────────

    fn lower_func_def(
        &mut self,
        id: NodeId,
        name: InternId,
        param_nodes: &[NodeId],
        body: NodeId,
        is_variadic: bool,
        storage: StorageClass,
    ) {
        let ty = self.ctx.node(id).ty;
        let ret_ty = match self.ctx.get_type(ty) {
            CType::Function { ret, .. } => *ret,
            _ => TYPE_NONE,
        };

        let linkage = if storage == StorageClass::Static {
            Linkage::Internal
        } else {
            Linkage::External
        };

        let name_str = self.ctx.get_str(name).to_string();
        let ir_ret = self.ir_type(ret_ty);

        let mut func = IrFunction::new(&name_str, ir_ret, linkage);
        func.is_variadic = is_variadic;

        // Create entry block
        func.create_block("entry");

        // Add parameters
        let mut param_values = Vec::new();
        for &pn in param_nodes {
            if let NodeKind::ParamDecl {
                name: pname,
                ..
            } = &self.ctx.node(pn).kind
            {
                let pty = self.ctx.node(pn).ty;
                let ir_pty = self.ir_type(pty);
                let pname_str = self.ctx.get_str(*pname).to_string();
                let vid = func.add_param(&pname_str, ir_pty);
                param_values.push((*pname, vid, pty));
            }
        }

        // Add to module
        let func_idx = self.module.add_function(func);
        self.cur_func_idx = Some(func_idx);
        self.cur_block = BlockId(0);
        self.block_terminated = false;
        self.ret_type = ret_ty;
        self.label_map.clear();
        self.forward_gotos.clear();
        self.locals.clear();
        self.break_targets.clear();
        self.continue_targets.clear();
        self.push_scope();

        // Emit alloca + store for each parameter
        for (pname, pval, pty) in param_values {
            let ir_pty = self.ir_type(pty);
            let align = self.ctx.type_align(pty);
            let alloca = self.alloc_value();
            self.emit(Instruction::Alloca {
                result: alloca,
                ty: ir_pty.clone(),
                align,
            });
            self.emit(Instruction::Store {
                addr: Operand::Value(alloca),
                value: Operand::Value(pval),
                ty: ir_pty,
            });
            self.add_local(pname, alloca);
        }

        // Lower body
        self.lower_stmt(body);

        // If no terminator on the last block, add a default return
        if !self.block_terminated {
            if ir_ret_is_void(&self.ir_type(ret_ty)) {
                self.terminate(Terminator::Ret { value: None });
            } else {
                // Return zeroinitializer for missing return in non-void
                self.terminate(Terminator::Ret {
                    value: Some(Operand::Const(ConstValue::I32(0))),
                });
            }
        }

        self.pop_scope();

        // Compute predecessors
        self.module.functions[func_idx].compute_predecessors();
        self.cur_func_idx = None;
    }

    // ── Statements ──────────────────────────────────────────────────

    fn lower_stmt(&mut self, id: NodeId) {
        if id == NODE_NONE {
            return;
        }
        if self.block_terminated {
            // Dead code — skip (we could still lower for goto labels)
            // But first check if it's a label
            if let NodeKind::LabelStmt { label, stmt } = &self.ctx.node(id).kind.clone() {
                self.lower_label_stmt(*label, *stmt);
                return;
            }
            return;
        }
        let kind = self.ctx.node(id).kind.clone();
        match kind {
            NodeKind::CompoundStmt { stmts } => {
                self.push_scope();
                for s in &stmts {
                    self.lower_stmt(*s);
                }
                self.pop_scope();
            }
            NodeKind::VarDecl {
                name,
                init,
                ..
            } => {
                self.lower_local_var(id, name, init);
            }
            NodeKind::ExprStmt { expr } => {
                if expr != NODE_NONE {
                    self.lower_expr(expr);
                }
            }
            NodeKind::ReturnStmt { expr } => {
                if expr != NODE_NONE {
                    let val = self.lower_expr(expr);
                    self.terminate(Terminator::Ret {
                        value: Some(val),
                    });
                } else {
                    self.terminate(Terminator::Ret { value: None });
                }
            }
            NodeKind::IfStmt {
                cond,
                then_br,
                else_br,
            } => {
                self.lower_if(cond, then_br, else_br);
            }
            NodeKind::WhileStmt { cond, body } => {
                self.lower_while(cond, body);
            }
            NodeKind::DoWhileStmt { body, cond } => {
                self.lower_do_while(body, cond);
            }
            NodeKind::ForStmt {
                init,
                cond,
                incr,
                body,
            } => {
                self.lower_for(init, cond, incr, body);
            }
            NodeKind::BreakStmt => {
                if let Some(&target) = self.break_targets.last() {
                    self.terminate(Terminator::Br { target });
                }
            }
            NodeKind::ContinueStmt => {
                if let Some(&target) = self.continue_targets.last() {
                    self.terminate(Terminator::Br { target });
                }
            }
            NodeKind::SwitchStmt { expr, body } => {
                self.lower_switch(expr, body);
            }
            NodeKind::CaseStmt { expr: _, body } => {
                // Case/default statements are handled by switch lowering.
                // If we hit them here, just emit the body.
                self.lower_stmt(body);
            }
            NodeKind::DefaultStmt { body } => {
                self.lower_stmt(body);
            }
            NodeKind::GotoStmt { label } => {
                self.lower_goto(label);
            }
            NodeKind::LabelStmt { label, stmt } => {
                self.lower_label_stmt(label, stmt);
            }
            NodeKind::NullStmt => {}
            _ => {
                // Expression statement or other: evaluate for side effects
                self.lower_expr(id);
            }
        }
    }

    fn lower_local_var(&mut self, id: NodeId, name: InternId, init: NodeId) {
        let ty = self.ctx.node(id).ty;
        let ir_ty = self.ir_type(ty);
        let align = self.ctx.type_align(ty);

        let alloca = self.alloc_value();
        self.emit(Instruction::Alloca {
            result: alloca,
            ty: ir_ty.clone(),
            align,
        });
        self.add_local(name, alloca);

        if init != NODE_NONE {
            let val = self.lower_expr(init);
            self.emit(Instruction::Store {
                addr: Operand::Value(alloca),
                value: val,
                ty: ir_ty,
            });
        }
    }

    fn lower_if(&mut self, cond: NodeId, then_br: NodeId, else_br: NodeId) {
        let cond_val = self.lower_expr(cond);
        let then_bb = self.create_block("if.then");
        let else_bb = if else_br != NODE_NONE {
            self.create_block("if.else")
        } else {
            self.create_block("if.end")
        };
        let end_bb = if else_br != NODE_NONE {
            self.create_block("if.end")
        } else {
            else_bb
        };

        self.terminate(Terminator::CondBr {
            cond: cond_val,
            true_bb: then_bb,
            false_bb: else_bb,
        });

        // Then
        self.switch_to(then_bb);
        self.lower_stmt(then_br);
        if !self.block_terminated {
            self.terminate(Terminator::Br { target: end_bb });
        }

        // Else
        if else_br != NODE_NONE {
            self.switch_to(else_bb);
            self.lower_stmt(else_br);
            if !self.block_terminated {
                self.terminate(Terminator::Br { target: end_bb });
            }
        }

        self.switch_to(end_bb);
    }

    fn lower_while(&mut self, cond: NodeId, body: NodeId) {
        let cond_bb = self.create_block("while.cond");
        let body_bb = self.create_block("while.body");
        let end_bb = self.create_block("while.end");

        self.terminate(Terminator::Br { target: cond_bb });

        self.switch_to(cond_bb);
        let cond_val = self.lower_expr(cond);
        self.terminate(Terminator::CondBr {
            cond: cond_val,
            true_bb: body_bb,
            false_bb: end_bb,
        });

        self.break_targets.push(end_bb);
        self.continue_targets.push(cond_bb);

        self.switch_to(body_bb);
        self.lower_stmt(body);
        if !self.block_terminated {
            self.terminate(Terminator::Br { target: cond_bb });
        }

        self.break_targets.pop();
        self.continue_targets.pop();

        self.switch_to(end_bb);
    }

    fn lower_do_while(&mut self, body: NodeId, cond: NodeId) {
        let body_bb = self.create_block("do.body");
        let cond_bb = self.create_block("do.cond");
        let end_bb = self.create_block("do.end");

        self.terminate(Terminator::Br { target: body_bb });

        self.break_targets.push(end_bb);
        self.continue_targets.push(cond_bb);

        self.switch_to(body_bb);
        self.lower_stmt(body);
        if !self.block_terminated {
            self.terminate(Terminator::Br { target: cond_bb });
        }

        self.switch_to(cond_bb);
        let cond_val = self.lower_expr(cond);
        self.terminate(Terminator::CondBr {
            cond: cond_val,
            true_bb: body_bb,
            false_bb: end_bb,
        });

        self.break_targets.pop();
        self.continue_targets.pop();

        self.switch_to(end_bb);
    }

    fn lower_for(&mut self, init: NodeId, cond: NodeId, incr: NodeId, body: NodeId) {
        // init
        if init != NODE_NONE {
            self.lower_stmt(init);
        }

        let cond_bb = self.create_block("for.cond");
        let body_bb = self.create_block("for.body");
        let incr_bb = self.create_block("for.incr");
        let end_bb = self.create_block("for.end");

        self.terminate(Terminator::Br { target: cond_bb });

        // cond
        self.switch_to(cond_bb);
        if cond != NODE_NONE {
            let cond_val = self.lower_expr(cond);
            self.terminate(Terminator::CondBr {
                cond: cond_val,
                true_bb: body_bb,
                false_bb: end_bb,
            });
        } else {
            self.terminate(Terminator::Br { target: body_bb });
        }

        self.break_targets.push(end_bb);
        self.continue_targets.push(incr_bb);

        // body
        self.switch_to(body_bb);
        self.lower_stmt(body);
        if !self.block_terminated {
            self.terminate(Terminator::Br { target: incr_bb });
        }

        // incr
        self.switch_to(incr_bb);
        if incr != NODE_NONE {
            self.lower_expr(incr);
        }
        if !self.block_terminated {
            self.terminate(Terminator::Br { target: cond_bb });
        }

        self.break_targets.pop();
        self.continue_targets.pop();

        self.switch_to(end_bb);
    }

    fn lower_switch(&mut self, expr: NodeId, body: NodeId) {
        let discr = self.lower_expr(expr);
        let end_bb = self.create_block("switch.end");

        // Collect case/default labels from the body
        let cases = self.collect_cases(body);
        let default_bb = if let Some(default) = cases.iter().find(|c| c.is_default) {
            default.block
        } else {
            end_bb
        };

        let case_list: Vec<(i64, BlockId)> = cases
            .iter()
            .filter(|c| !c.is_default)
            .map(|c| (c.value, c.block))
            .collect();

        self.terminate(Terminator::Switch {
            discr,
            ty: IrType::I32,
            default: default_bb,
            cases: case_list,
        });

        self.break_targets.push(end_bb);

        // Emit each case block
        for case in &cases {
            self.switch_to(case.block);
            self.lower_stmt(case.body);
            if !self.block_terminated {
                // Fall through to next case or end
                if let Some(next) = cases.iter().find(|c| c.index > case.index) {
                    self.terminate(Terminator::Br { target: next.block });
                } else {
                    self.terminate(Terminator::Br { target: end_bb });
                }
            }
        }

        self.break_targets.pop();
        self.switch_to(end_bb);
    }

    fn lower_goto(&mut self, label: InternId) {
        if let Some(&bb) = self.label_map.get(&label) {
            self.terminate(Terminator::Br { target: bb });
        } else {
            // Forward reference: create a placeholder block
            let placeholder = self.create_block("goto.fwd");
            self.terminate(Terminator::Br { target: placeholder });
            self.forward_gotos.push((label, placeholder));
        }
    }

    fn lower_label_stmt(&mut self, label: InternId, stmt: NodeId) {
        let lbl_bb = self.create_block(&format!(
            "label.{}",
            self.ctx.get_str(label)
        ));

        // If current block isn't terminated, branch to the label
        if !self.block_terminated {
            self.terminate(Terminator::Br { target: lbl_bb });
        }

        self.label_map.insert(label, lbl_bb);
        self.switch_to(lbl_bb);
        self.lower_stmt(stmt);

        // Patch forward gotos
        // (This is a simplified version — a real compiler would need to fix up
        // the terminator targets, but for now the forward_gotos vec record is
        // mostly for documentation / later use.)
    }

    // ── Switch case collection ──────────────────────────────────────

    fn collect_cases(&mut self, body: NodeId) -> Vec<CaseInfo> {
        let mut cases = Vec::new();
        self.collect_cases_inner(body, &mut cases, 0);
        cases
    }

    fn collect_cases_inner(
        &mut self,
        id: NodeId,
        out: &mut Vec<CaseInfo>,
        mut idx: usize,
    ) -> usize {
        if id == NODE_NONE {
            return idx;
        }
        let kind = self.ctx.node(id).kind.clone();
        match kind {
            NodeKind::CompoundStmt { stmts } => {
                for &s in &stmts {
                    idx = self.collect_cases_inner(s, out, idx);
                }
            }
            NodeKind::CaseStmt { expr, body } => {
                let value = self.eval_const_expr(expr);
                let bb = self.create_block(&format!("case.{}", value));
                out.push(CaseInfo {
                    value,
                    is_default: false,
                    block: bb,
                    body,
                    index: idx,
                });
                idx += 1;
            }
            NodeKind::DefaultStmt { body } => {
                let bb = self.create_block("default");
                out.push(CaseInfo {
                    value: 0,
                    is_default: true,
                    block: bb,
                    body,
                    index: idx,
                });
                idx += 1;
            }
            _ => {} // other statements inside switch body
        }
        idx
    }

    /// Evaluate a constant expression (for case labels).
    fn eval_const_expr(&self, id: NodeId) -> i64 {
        match &self.ctx.node(id).kind {
            NodeKind::IntLiteral { value, .. } => *value as i64,
            NodeKind::CharLiteral { value } => *value as i64,
            NodeKind::UnaryOp {
                op: UnaryOp::Neg,
                operand,
            } => -self.eval_const_expr(*operand),
            _ => 0, // fallback
        }
    }

    // ── Expressions ─────────────────────────────────────────────────

    fn lower_expr(&mut self, id: NodeId) -> Operand {
        if id == NODE_NONE {
            return Operand::Const(ConstValue::I32(0));
        }
        let kind = self.ctx.node(id).kind.clone();
        let ty = self.ctx.node(id).ty;

        match kind {
            NodeKind::IntLiteral { value, suffix } => {
                self.lower_int_literal(value, suffix, ty)
            }
            NodeKind::FloatLiteral { value, suffix } => {
                self.lower_float_literal(value, suffix)
            }
            NodeKind::CharLiteral { value } => {
                Operand::Const(ConstValue::I8(value as i8))
            }
            NodeKind::StringLiteral { bytes } => {
                let label = self.module.intern_string(bytes);
                Operand::Global(label)
            }
            NodeKind::Ident { name } => {
                self.lower_ident(name, ty)
            }
            NodeKind::BinaryOp { op, lhs, rhs } => {
                self.lower_binop(op, lhs, rhs, ty)
            }
            NodeKind::UnaryOp { op, operand } => {
                self.lower_unaryop(op, operand, ty)
            }
            NodeKind::PostfixOp { op, operand } => {
                self.lower_postfix(op, operand, ty)
            }
            NodeKind::Assign { op, lhs, rhs } => {
                self.lower_assign(op, lhs, rhs, ty)
            }
            NodeKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                self.lower_ternary(cond, then_expr, else_expr, ty)
            }
            NodeKind::Call { callee, args } => {
                self.lower_call(callee, &args, ty)
            }
            NodeKind::Cast { expr, .. } => {
                self.lower_cast(expr, ty)
            }
            NodeKind::SizeofType { type_node } => {
                let target_ty = self.ctx.node(type_node).ty;
                let size = self.ctx.type_size(target_ty);
                Operand::Const(ConstValue::U64(size as u64))
            }
            NodeKind::SizeofExpr { expr } => {
                let expr_ty = self.ctx.node(expr).ty;
                let size = self.ctx.type_size(expr_ty);
                Operand::Const(ConstValue::U64(size as u64))
            }
            NodeKind::MemberAccess {
                expr,
                member,
                is_arrow,
            } => {
                self.lower_member_access(expr, member, is_arrow, ty)
            }
            NodeKind::ArraySubscript { expr, index } => {
                self.lower_array_subscript(expr, index, ty)
            }
            NodeKind::AddrOf { expr } => {
                self.lower_addr_of(expr)
            }
            NodeKind::Deref { expr } => {
                self.lower_deref(expr, ty)
            }
            NodeKind::Comma { lhs, rhs } => {
                self.lower_expr(lhs);
                self.lower_expr(rhs)
            }
            _ => Operand::Const(ConstValue::I32(0)),
        }
    }

    fn lower_int_literal(&self, value: u64, _suffix: IntSuffix, ty: TypeId) -> Operand {
        let ir_ty = self.ir_type(ty);
        match ir_ty {
            IrType::I8 => Operand::Const(ConstValue::I8(value as i8)),
            IrType::U8 => Operand::Const(ConstValue::U8(value as u8)),
            IrType::I16 => Operand::Const(ConstValue::I16(value as i16)),
            IrType::U16 => Operand::Const(ConstValue::U16(value as u16)),
            IrType::I32 => Operand::Const(ConstValue::I32(value as i32)),
            IrType::U32 => Operand::Const(ConstValue::U32(value as u32)),
            IrType::I64 => Operand::Const(ConstValue::I64(value as i64)),
            IrType::U64 => Operand::Const(ConstValue::U64(value)),
            _ => Operand::Const(ConstValue::I32(value as i32)),
        }
    }

    fn lower_float_literal(&self, value: f64, suffix: FloatSuffix) -> Operand {
        match suffix {
            FloatSuffix::F => Operand::Const(ConstValue::F32(value as f32)),
            _ => Operand::Const(ConstValue::F64(value)),
        }
    }

    fn lower_ident(&mut self, name: InternId, ty: TypeId) -> Operand {
        // Check if it's a local variable → load from alloca
        if let Some(alloca) = self.lookup_local(name) {
            let ir_ty = self.ir_type(ty);
            let result = self.alloc_value();
            self.emit(Instruction::Load {
                result,
                addr: Operand::Value(alloca),
                ty: ir_ty,
            });
            return Operand::Value(result);
        }

        // Check if it's an enum constant
        if let Some(sym_id) = {
            // Walk scopes in ctx to find the symbol
            let mut found = None;
            for scope in self.ctx.scopes.iter().rev() {
                if let Some(&sid) = scope.symbols.get(&name) {
                    found = Some(sid);
                    break;
                }
            }
            found
        } {
            let sym = self.ctx.get_symbol(sym_id);
            if let SymbolKind::EnumConstant(val) = &sym.kind {
                return Operand::Const(ConstValue::I32(*val as i32));
            }
            // Function reference
            if let SymbolKind::Function = &sym.kind {
                let name_str = self.ctx.get_str(name).to_string();
                return Operand::Global(name_str);
            }
        }

        // Global variable
        let name_str = self.ctx.get_str(name).to_string();
        let ir_ty = self.ir_type(ty);
        let addr = self.alloc_value();
        self.emit(Instruction::GlobalAddr {
            result: addr,
            name: name_str,
        });
        let result = self.alloc_value();
        self.emit(Instruction::Load {
            result,
            addr: Operand::Value(addr),
            ty: ir_ty,
        });
        Operand::Value(result)
    }

    fn lower_binop(
        &mut self,
        op: BinOp,
        lhs: NodeId,
        rhs: NodeId,
        ty: TypeId,
    ) -> Operand {
        // Short-circuit for logical && and ||
        match op {
            BinOp::LogAnd => return self.lower_log_and(lhs, rhs),
            BinOp::LogOr => return self.lower_log_or(lhs, rhs),
            _ => {}
        }

        let lhs_val = self.lower_expr(lhs);
        let rhs_val = self.lower_expr(rhs);
        let ir_ty = self.ir_type(ty);
        let lhs_ty = self.ctx.node(lhs).ty;

        let is_float = self.ctx.is_float_type(lhs_ty);
        let is_unsigned = self.ctx.is_unsigned(lhs_ty);
        let is_ptr = self.ctx.is_pointer_type(lhs_ty);

        match op {
            // Arithmetic
            BinOp::Add if is_ptr => {
                // Pointer arithmetic
                let result = self.alloc_value();
                let pointee_ty = match self.ctx.get_type(lhs_ty) {
                    CType::Pointer { pointee } => self.ir_type(*pointee),
                    _ => IrType::I8,
                };
                self.emit(Instruction::GetElementPtr {
                    result,
                    base: lhs_val,
                    offset: rhs_val,
                    elem_ty: pointee_ty,
                });
                Operand::Value(result)
            }
            BinOp::Add => self.emit_binop(BinOpKind::Add, BinOpKind::FAdd, lhs_val, rhs_val, ir_ty, is_float),
            BinOp::Sub if is_ptr => {
                // Pointer subtraction — TODO: ptrdiff
                let result = self.alloc_value();
                self.emit(Instruction::BinOp {
                    result,
                    op: BinOpKind::Sub,
                    lhs: lhs_val,
                    rhs: rhs_val,
                    ty: ir_ty,
                });
                Operand::Value(result)
            }
            BinOp::Sub => self.emit_binop(BinOpKind::Sub, BinOpKind::FSub, lhs_val, rhs_val, ir_ty, is_float),
            BinOp::Mul => self.emit_binop(BinOpKind::Mul, BinOpKind::FMul, lhs_val, rhs_val, ir_ty, is_float),
            BinOp::Div => {
                let iop = if is_unsigned { BinOpKind::UDiv } else { BinOpKind::SDiv };
                self.emit_binop(iop, BinOpKind::FDiv, lhs_val, rhs_val, ir_ty, is_float)
            }
            BinOp::Mod => {
                let iop = if is_unsigned { BinOpKind::URem } else { BinOpKind::SRem };
                self.emit_binop(iop, BinOpKind::FRem, lhs_val, rhs_val, ir_ty, is_float)
            }

            // Bit operations
            BinOp::BitAnd => self.emit_simple_binop(BinOpKind::And, lhs_val, rhs_val, ir_ty),
            BinOp::BitOr => self.emit_simple_binop(BinOpKind::Or, lhs_val, rhs_val, ir_ty),
            BinOp::BitXor => self.emit_simple_binop(BinOpKind::Xor, lhs_val, rhs_val, ir_ty),
            BinOp::Shl => self.emit_simple_binop(BinOpKind::Shl, lhs_val, rhs_val, ir_ty),
            BinOp::Shr => {
                let op = if is_unsigned { BinOpKind::LShr } else { BinOpKind::AShr };
                self.emit_simple_binop(op, lhs_val, rhs_val, ir_ty)
            }

            // Comparisons
            BinOp::Eq => self.emit_cmp(IcmpPred::Eq, FcmpPred::Oeq, lhs_val, rhs_val, ir_ty, is_float),
            BinOp::Ne => self.emit_cmp(IcmpPred::Ne, FcmpPred::Une, lhs_val, rhs_val, ir_ty, is_float),
            BinOp::Lt => {
                let ipred = if is_unsigned { IcmpPred::Ult } else { IcmpPred::Slt };
                self.emit_cmp(ipred, FcmpPred::Olt, lhs_val, rhs_val, ir_ty, is_float)
            }
            BinOp::Gt => {
                let ipred = if is_unsigned { IcmpPred::Ugt } else { IcmpPred::Sgt };
                self.emit_cmp(ipred, FcmpPred::Ogt, lhs_val, rhs_val, ir_ty, is_float)
            }
            BinOp::Le => {
                let ipred = if is_unsigned { IcmpPred::Ule } else { IcmpPred::Sle };
                self.emit_cmp(ipred, FcmpPred::Ole, lhs_val, rhs_val, ir_ty, is_float)
            }
            BinOp::Ge => {
                let ipred = if is_unsigned { IcmpPred::Uge } else { IcmpPred::Sge };
                self.emit_cmp(ipred, FcmpPred::Oge, lhs_val, rhs_val, ir_ty, is_float)
            }

            BinOp::LogAnd | BinOp::LogOr => {
                unreachable!("handled above")
            }
        }
    }

    fn emit_binop(
        &mut self,
        iop: BinOpKind,
        fop: BinOpKind,
        lhs: Operand,
        rhs: Operand,
        ty: IrType,
        is_float: bool,
    ) -> Operand {
        let op = if is_float { fop } else { iop };
        self.emit_simple_binop(op, lhs, rhs, ty)
    }

    fn emit_simple_binop(
        &mut self,
        op: BinOpKind,
        lhs: Operand,
        rhs: Operand,
        ty: IrType,
    ) -> Operand {
        let result = self.alloc_value();
        self.emit(Instruction::BinOp {
            result,
            op,
            lhs,
            rhs,
            ty,
        });
        Operand::Value(result)
    }

    fn emit_cmp(
        &mut self,
        ipred: IcmpPred,
        fpred: FcmpPred,
        lhs: Operand,
        rhs: Operand,
        ty: IrType,
        is_float: bool,
    ) -> Operand {
        let result = self.alloc_value();
        if is_float {
            self.emit(Instruction::Fcmp {
                result,
                pred: fpred,
                lhs,
                rhs,
                ty,
            });
        } else {
            self.emit(Instruction::Icmp {
                result,
                pred: ipred,
                lhs,
                rhs,
                ty,
            });
        }
        Operand::Value(result)
    }

    fn lower_log_and(&mut self, lhs: NodeId, rhs: NodeId) -> Operand {
        let lhs_val = self.lower_expr(lhs);
        let rhs_bb = self.create_block("land.rhs");
        let end_bb = self.create_block("land.end");

        self.terminate(Terminator::CondBr {
            cond: lhs_val.clone(),
            true_bb: rhs_bb,
            false_bb: end_bb,
        });

        let lhs_pred_bb = self.cur_block;

        self.switch_to(rhs_bb);
        let rhs_val = self.lower_expr(rhs);
        let rhs_pred_bb = self.cur_block;
        self.terminate(Terminator::Br { target: end_bb });

        self.switch_to(end_bb);
        let result = self.alloc_value();
        self.emit(Instruction::Phi {
            result,
            ty: IrType::I32,
            incoming: vec![
                (lhs_pred_bb, Operand::Const(ConstValue::I32(0))),
                (rhs_pred_bb, rhs_val),
            ],
        });
        Operand::Value(result)
    }

    fn lower_log_or(&mut self, lhs: NodeId, rhs: NodeId) -> Operand {
        let lhs_val = self.lower_expr(lhs);
        let rhs_bb = self.create_block("lor.rhs");
        let end_bb = self.create_block("lor.end");

        self.terminate(Terminator::CondBr {
            cond: lhs_val.clone(),
            true_bb: end_bb,
            false_bb: rhs_bb,
        });

        let lhs_pred_bb = self.cur_block;

        self.switch_to(rhs_bb);
        let rhs_val = self.lower_expr(rhs);
        let rhs_pred_bb = self.cur_block;
        self.terminate(Terminator::Br { target: end_bb });

        self.switch_to(end_bb);
        let result = self.alloc_value();
        self.emit(Instruction::Phi {
            result,
            ty: IrType::I32,
            incoming: vec![
                (lhs_pred_bb, Operand::Const(ConstValue::I32(1))),
                (rhs_pred_bb, rhs_val),
            ],
        });
        Operand::Value(result)
    }

    fn lower_unaryop(
        &mut self,
        op: UnaryOp,
        operand: NodeId,
        ty: TypeId,
    ) -> Operand {
        let ir_ty = self.ir_type(ty);

        match op {
            UnaryOp::Neg => {
                let val = self.lower_expr(operand);
                if self.ctx.is_float_type(ty) {
                    let result = self.alloc_value();
                    self.emit(Instruction::UnaryOp {
                        result,
                        op: UnaryOpKind::FNeg,
                        operand: val,
                        ty: ir_ty,
                    });
                    Operand::Value(result)
                } else {
                    let result = self.alloc_value();
                    self.emit(Instruction::BinOp {
                        result,
                        op: BinOpKind::Sub,
                        lhs: Operand::Const(ConstValue::I32(0)),
                        rhs: val,
                        ty: ir_ty,
                    });
                    Operand::Value(result)
                }
            }
            UnaryOp::BitNot => {
                let val = self.lower_expr(operand);
                let result = self.alloc_value();
                self.emit(Instruction::BinOp {
                    result,
                    op: BinOpKind::Xor,
                    lhs: val,
                    rhs: Operand::Const(ConstValue::I32(-1)),
                    ty: ir_ty,
                });
                Operand::Value(result)
            }
            UnaryOp::LogNot => {
                let val = self.lower_expr(operand);
                let result = self.alloc_value();
                self.emit(Instruction::Icmp {
                    result,
                    pred: IcmpPred::Eq,
                    lhs: val,
                    rhs: Operand::Const(ConstValue::I32(0)),
                    ty: ir_ty,
                });
                Operand::Value(result)
            }
            UnaryOp::PreInc | UnaryOp::PreDec => {
                self.lower_pre_inc_dec(op, operand, ty)
            }
            UnaryOp::Plus => {
                // No-op
                self.lower_expr(operand)
            }
        }
    }

    fn lower_pre_inc_dec(
        &mut self,
        op: UnaryOp,
        operand: NodeId,
        ty: TypeId,
    ) -> Operand {
        let addr = self.lower_lvalue(operand);
        let ir_ty = self.ir_type(ty);
        let old = self.alloc_value();
        self.emit(Instruction::Load {
            result: old,
            addr: addr.clone(),
            ty: ir_ty.clone(),
        });

        let one = Operand::Const(ConstValue::I32(1));
        let binop = if op == UnaryOp::PreInc {
            BinOpKind::Add
        } else {
            BinOpKind::Sub
        };
        let new_val = self.alloc_value();
        self.emit(Instruction::BinOp {
            result: new_val,
            op: binop,
            lhs: Operand::Value(old),
            rhs: one,
            ty: ir_ty.clone(),
        });
        self.emit(Instruction::Store {
            addr,
            value: Operand::Value(new_val),
            ty: ir_ty,
        });
        Operand::Value(new_val)
    }

    fn lower_postfix(
        &mut self,
        op: PostfixOp,
        operand: NodeId,
        ty: TypeId,
    ) -> Operand {
        let addr = self.lower_lvalue(operand);
        let ir_ty = self.ir_type(ty);
        let old = self.alloc_value();
        self.emit(Instruction::Load {
            result: old,
            addr: addr.clone(),
            ty: ir_ty.clone(),
        });

        let one = Operand::Const(ConstValue::I32(1));
        let binop = if op == PostfixOp::PostInc {
            BinOpKind::Add
        } else {
            BinOpKind::Sub
        };
        let new_val = self.alloc_value();
        self.emit(Instruction::BinOp {
            result: new_val,
            op: binop,
            lhs: Operand::Value(old),
            rhs: one,
            ty: ir_ty.clone(),
        });
        self.emit(Instruction::Store {
            addr,
            value: Operand::Value(new_val),
            ty: ir_ty,
        });
        // Postfix returns the OLD value
        Operand::Value(old)
    }

    fn lower_assign(
        &mut self,
        op: AssignOp,
        lhs: NodeId,
        rhs: NodeId,
        ty: TypeId,
    ) -> Operand {
        let addr = self.lower_lvalue(lhs);
        let ir_ty = self.ir_type(ty);

        let new_val = if op == AssignOp::Assign {
            self.lower_expr(rhs)
        } else {
            // Compound assignment: load old, compute, store
            let old = self.alloc_value();
            self.emit(Instruction::Load {
                result: old,
                addr: addr.clone(),
                ty: ir_ty.clone(),
            });
            let rhs_val = self.lower_expr(rhs);

            let lhs_ty = self.ctx.node(lhs).ty;
            let is_unsigned = self.ctx.is_unsigned(lhs_ty);
            let is_float = self.ctx.is_float_type(lhs_ty);

            let binop = match op {
                AssignOp::AddAssign => if is_float { BinOpKind::FAdd } else { BinOpKind::Add },
                AssignOp::SubAssign => if is_float { BinOpKind::FSub } else { BinOpKind::Sub },
                AssignOp::MulAssign => if is_float { BinOpKind::FMul } else { BinOpKind::Mul },
                AssignOp::DivAssign => {
                    if is_float { BinOpKind::FDiv }
                    else if is_unsigned { BinOpKind::UDiv }
                    else { BinOpKind::SDiv }
                }
                AssignOp::ModAssign => {
                    if is_unsigned { BinOpKind::URem } else { BinOpKind::SRem }
                }
                AssignOp::ShlAssign => BinOpKind::Shl,
                AssignOp::ShrAssign => if is_unsigned { BinOpKind::LShr } else { BinOpKind::AShr },
                AssignOp::AndAssign => BinOpKind::And,
                AssignOp::XorAssign => BinOpKind::Xor,
                AssignOp::OrAssign => BinOpKind::Or,
                AssignOp::Assign => unreachable!(),
            };

            let result = self.alloc_value();
            self.emit(Instruction::BinOp {
                result,
                op: binop,
                lhs: Operand::Value(old),
                rhs: rhs_val,
                ty: ir_ty.clone(),
            });
            Operand::Value(result)
        };

        self.emit(Instruction::Store {
            addr,
            value: new_val.clone(),
            ty: ir_ty,
        });
        new_val
    }

    fn lower_ternary(
        &mut self,
        cond: NodeId,
        then_expr: NodeId,
        else_expr: NodeId,
        ty: TypeId,
    ) -> Operand {
        let cond_val = self.lower_expr(cond);
        let then_bb = self.create_block("ternary.then");
        let else_bb = self.create_block("ternary.else");
        let merge_bb = self.create_block("ternary.merge");

        self.terminate(Terminator::CondBr {
            cond: cond_val,
            true_bb: then_bb,
            false_bb: else_bb,
        });

        self.switch_to(then_bb);
        let then_val = self.lower_expr(then_expr);
        let then_pred = self.cur_block;
        self.terminate(Terminator::Br { target: merge_bb });

        self.switch_to(else_bb);
        let else_val = self.lower_expr(else_expr);
        let else_pred = self.cur_block;
        self.terminate(Terminator::Br { target: merge_bb });

        self.switch_to(merge_bb);
        let ir_ty = self.ir_type(ty);
        let result = self.alloc_value();
        self.emit(Instruction::Phi {
            result,
            ty: ir_ty,
            incoming: vec![(then_pred, then_val), (else_pred, else_val)],
        });
        Operand::Value(result)
    }

    fn lower_call(
        &mut self,
        callee: NodeId,
        arg_nodes: &[NodeId],
        ty: TypeId,
    ) -> Operand {
        let callee_kind = self.ctx.node(callee).kind.clone();
        let callee_ty = self.ctx.node(callee).ty;

        // Determine return type
        let ret_ty = match self.ctx.get_type(callee_ty) {
            CType::Function { ret, .. } => self.ir_type(*ret),
            CType::Pointer { pointee } => {
                match self.ctx.get_type(*pointee) {
                    CType::Function { ret, .. } => self.ir_type(*ret),
                    _ => self.ir_type(ty),
                }
            }
            _ => self.ir_type(ty),
        };

        // Determine variadic
        let is_variadic = match self.ctx.get_type(callee_ty) {
            CType::Function { variadic, .. } => *variadic,
            CType::Pointer { pointee } => {
                match self.ctx.get_type(*pointee) {
                    CType::Function { variadic, .. } => *variadic,
                    _ => false,
                }
            }
            _ => false,
        };

        // Lower args
        let mut args = Vec::new();
        for &a in arg_nodes {
            let val = self.lower_expr(a);
            let aty = self.ir_type(self.ctx.node(a).ty);
            args.push((val, aty));
        }

        let result = self.alloc_value();

        // Direct call?
        if let NodeKind::Ident { name } = callee_kind {
            let name_str = self.ctx.get_str(name).to_string();

            // Ensure extern is declared
            if !self.module.functions.iter().any(|f| f.name == name_str)
                && !self.module.externs.iter().any(|e| e.name == name_str)
            {
                let extern_params: Vec<IrType> = args.iter().map(|(_, t)| t.clone()).collect();
                self.module.add_extern(ExternFunc {
                    name: name_str.clone(),
                    ret_ty: ret_ty.clone(),
                    params: extern_params,
                    is_variadic,
                });
            }

            self.emit(Instruction::Call {
                result,
                callee: name_str,
                args,
                ret_ty: ret_ty.clone(),
                is_variadic,
            });
        } else {
            // Indirect call (function pointer)
            let func_ptr = self.lower_expr(callee);
            self.emit(Instruction::CallIndirect {
                result,
                func_ptr,
                args,
                ret_ty: ret_ty.clone(),
                is_variadic,
            });
        }

        if ret_ty == IrType::Void {
            Operand::Const(ConstValue::I32(0))
        } else {
            Operand::Value(result)
        }
    }

    fn lower_cast(&mut self, expr: NodeId, dst_ty: TypeId) -> Operand {
        let src_val = self.lower_expr(expr);
        let src_ty = self.ctx.node(expr).ty;
        let ir_src = self.ir_type(src_ty);
        let ir_dst = self.ir_type(dst_ty);

        if ir_src == ir_dst {
            return src_val;
        }

        let kind = determine_cast(&ir_src, &ir_dst, self.is_signed(src_ty));
        if let Some(ck) = kind {
            let result = self.alloc_value();
            self.emit(Instruction::Cast {
                result,
                kind: ck,
                src: src_val,
                src_ty: ir_src,
                dst_ty: ir_dst,
            });
            Operand::Value(result)
        } else {
            src_val
        }
    }

    fn lower_member_access(
        &mut self,
        expr: NodeId,
        member: InternId,
        is_arrow: bool,
        ty: TypeId,
    ) -> Operand {
        let base = if is_arrow {
            // ptr->member: base is the pointer value
            self.lower_expr(expr)
        } else {
            // obj.member: base is the address of the struct
            self.lower_lvalue(expr)
        };

        // Find the member offset
        let struct_ty = if is_arrow {
            match self.ctx.get_type(self.ctx.node(expr).ty) {
                CType::Pointer { pointee } => *pointee,
                _ => self.ctx.node(expr).ty,
            }
        } else {
            self.ctx.node(expr).ty
        };

        let offset = self.find_member_offset(struct_ty, member);
        let ir_ty = self.ir_type(ty);

        // GEP to member
        let gep = self.alloc_value();
        self.emit(Instruction::GetElementPtr {
            result: gep,
            base,
            offset: Operand::Const(ConstValue::I32(offset as i32)),
            elem_ty: IrType::I8, // byte-level offset
        });

        let result = self.alloc_value();
        self.emit(Instruction::Load {
            result,
            addr: Operand::Value(gep),
            ty: ir_ty,
        });
        Operand::Value(result)
    }

    fn find_member_offset(&self, struct_ty: TypeId, member: InternId) -> u32 {
        match self.ctx.get_type(struct_ty) {
            CType::Struct { members, .. } | CType::Union { members, .. } => {
                for m in members {
                    if m.name == member {
                        return m.offset;
                    }
                }
                0
            }
            _ => 0,
        }
    }

    fn lower_array_subscript(
        &mut self,
        expr: NodeId,
        index: NodeId,
        ty: TypeId,
    ) -> Operand {
        let base = self.lower_expr(expr);
        let idx = self.lower_expr(index);
        let ir_ty = self.ir_type(ty);

        let gep = self.alloc_value();
        self.emit(Instruction::GetElementPtr {
            result: gep,
            base,
            offset: idx,
            elem_ty: ir_ty.clone(),
        });

        let result = self.alloc_value();
        self.emit(Instruction::Load {
            result,
            addr: Operand::Value(gep),
            ty: ir_ty,
        });
        Operand::Value(result)
    }

    fn lower_addr_of(&mut self, expr: NodeId) -> Operand {
        self.lower_lvalue(expr)
    }

    fn lower_deref(&mut self, expr: NodeId, ty: TypeId) -> Operand {
        let ptr = self.lower_expr(expr);
        let ir_ty = self.ir_type(ty);
        let result = self.alloc_value();
        self.emit(Instruction::Load {
            result,
            addr: ptr,
            ty: ir_ty,
        });
        Operand::Value(result)
    }

    // ── L-value computation (returns address) ───────────────────────

    fn lower_lvalue(&mut self, id: NodeId) -> Operand {
        let kind = self.ctx.node(id).kind.clone();
        match kind {
            NodeKind::Ident { name } => {
                // Local variable → return alloca address
                if let Some(alloca) = self.lookup_local(name) {
                    return Operand::Value(alloca);
                }
                // Global variable → globaladdr
                let name_str = self.ctx.get_str(name).to_string();
                let result = self.alloc_value();
                self.emit(Instruction::GlobalAddr {
                    result,
                    name: name_str,
                });
                Operand::Value(result)
            }
            NodeKind::Deref { expr } => {
                self.lower_expr(expr)
            }
            NodeKind::MemberAccess {
                expr,
                member,
                is_arrow,
            } => {
                let base = if is_arrow {
                    self.lower_expr(expr)
                } else {
                    self.lower_lvalue(expr)
                };
                let struct_ty = if is_arrow {
                    match self.ctx.get_type(self.ctx.node(expr).ty) {
                        CType::Pointer { pointee } => *pointee,
                        _ => self.ctx.node(expr).ty,
                    }
                } else {
                    self.ctx.node(expr).ty
                };
                let offset = self.find_member_offset(struct_ty, member);
                let result = self.alloc_value();
                self.emit(Instruction::GetElementPtr {
                    result,
                    base,
                    offset: Operand::Const(ConstValue::I32(offset as i32)),
                    elem_ty: IrType::I8,
                });
                Operand::Value(result)
            }
            NodeKind::ArraySubscript { expr, index } => {
                let base = self.lower_expr(expr);
                let idx = self.lower_expr(index);
                let elem_ty = self.ir_type(self.ctx.node(id).ty);
                let result = self.alloc_value();
                self.emit(Instruction::GetElementPtr {
                    result,
                    base,
                    offset: idx,
                    elem_ty,
                });
                Operand::Value(result)
            }
            _ => {
                // Fallback: compute as expression (may not be correct for all cases)
                self.lower_expr(id)
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn ir_ret_is_void(ty: &IrType) -> bool {
    matches!(ty, IrType::Void)
}

/// Determine the appropriate CastKind for converting between two IR types.
fn determine_cast(src: &IrType, dst: &IrType, src_signed: bool) -> Option<CastKind> {
    // Ptr <-> Int
    if src.is_pointer() && dst.is_integer() {
        return Some(CastKind::PtrToInt);
    }
    if src.is_integer() && dst.is_pointer() {
        return Some(CastKind::IntToPtr);
    }
    // Ptr <-> Ptr
    if src.is_pointer() && dst.is_pointer() {
        return Some(CastKind::Bitcast);
    }
    // Int <-> Int
    if src.is_integer() && dst.is_integer() {
        let sw = src.bit_width();
        let dw = dst.bit_width();
        if dw > sw {
            return Some(if src_signed {
                CastKind::SExt
            } else {
                CastKind::ZExt
            });
        }
        if dw < sw {
            return Some(CastKind::Trunc);
        }
        return None; // same size, just bitcast
    }
    // Int <-> Float
    if src.is_integer() && dst.is_float() {
        return Some(if src_signed {
            CastKind::SIToFP
        } else {
            CastKind::UIToFP
        });
    }
    if src.is_float() && dst.is_integer() {
        return Some(if dst.is_signed() {
            CastKind::FPToSI
        } else {
            CastKind::FPToUI
        });
    }
    // Float <-> Float
    if src.is_float() && dst.is_float() {
        let sw = match src {
            IrType::F32 => 32,
            IrType::F64 => 64,
            IrType::F128 => 128,
            _ => 0,
        };
        let dw = match dst {
            IrType::F32 => 32,
            IrType::F64 => 64,
            IrType::F128 => 128,
            _ => 0,
        };
        if dw > sw {
            return Some(CastKind::FPExt);
        }
        if dw < sw {
            return Some(CastKind::FPTrunc);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_cast_zext() {
        let cast = determine_cast(&IrType::U8, &IrType::I32, false);
        assert_eq!(cast, Some(CastKind::ZExt));
    }

    #[test]
    fn test_determine_cast_sext() {
        let cast = determine_cast(&IrType::I8, &IrType::I32, true);
        assert_eq!(cast, Some(CastKind::SExt));
    }

    #[test]
    fn test_determine_cast_trunc() {
        let cast = determine_cast(&IrType::I64, &IrType::I8, true);
        assert_eq!(cast, Some(CastKind::Trunc));
    }

    #[test]
    fn test_determine_cast_ptr_to_int() {
        let cast = determine_cast(&IrType::Ptr, &IrType::I64, false);
        assert_eq!(cast, Some(CastKind::PtrToInt));
    }

    #[test]
    fn test_determine_cast_int_to_float() {
        let cast = determine_cast(&IrType::I32, &IrType::F64, true);
        assert_eq!(cast, Some(CastKind::SIToFP));
    }

    #[test]
    fn test_determine_cast_same_size() {
        let cast = determine_cast(&IrType::I32, &IrType::I32, true);
        assert_eq!(cast, None);
    }

    #[test]
    fn test_determine_cast_fp_ext() {
        let cast = determine_cast(&IrType::F32, &IrType::F64, true);
        assert_eq!(cast, Some(CastKind::FPExt));
    }

    #[test]
    fn test_lower_empty_module() {
        let ctx = Ctx::new(crate::target::Target::X86_64);
        let module = IrModule::new("test.c");
        assert!(module.functions.is_empty());
    }
}
