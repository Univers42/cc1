// frontend/parser/ast.rs — AST pretty-printer for --dump-ast.

use crate::ctx::{Ctx, NodeId, NodeKind, NODE_NONE};

/// Dump the AST rooted at `root` to stderr.
pub fn dump(ctx: &Ctx, root: NodeId) {
    dump_node(ctx, root, 0);
}

fn dump_node(ctx: &Ctx, id: NodeId, indent: usize) {
    if id == NODE_NONE {
        return;
    }
    let node = ctx.node(id);
    let prefix = "  ".repeat(indent);

    match &node.kind {
        NodeKind::TranslationUnit { decls } => {
            eprintln!("{}TranslationUnit", prefix);
            for &d in decls {
                dump_node(ctx, d, indent + 1);
            }
        }
        NodeKind::FuncDef {
            name, params, body, ..
        } => {
            eprintln!("{}FuncDef '{}'", prefix, ctx.get_str(*name));
            for &p in params {
                dump_node(ctx, p, indent + 1);
            }
            dump_node(ctx, *body, indent + 1);
        }
        NodeKind::VarDecl { name, init, .. } => {
            eprintln!("{}VarDecl '{}'", prefix, ctx.get_str(*name));
            if *init != NODE_NONE {
                dump_node(ctx, *init, indent + 1);
            }
        }
        NodeKind::ParamDecl { name, .. } => {
            eprintln!("{}ParamDecl '{}'", prefix, ctx.get_str(*name));
        }
        NodeKind::TypedefDecl { name, type_node } => {
            eprintln!("{}TypedefDecl '{}'", prefix, ctx.get_str(*name));
            dump_node(ctx, *type_node, indent + 1);
        }
        NodeKind::StructDecl { tag, members } => {
            let tag_str = tag.map_or("<anon>".to_string(), |t| ctx.get_str(t).to_string());
            eprintln!("{}StructDecl '{}'", prefix, tag_str);
            for &m in members {
                dump_node(ctx, m, indent + 1);
            }
        }
        NodeKind::UnionDecl { tag, members } => {
            let tag_str = tag.map_or("<anon>".to_string(), |t| ctx.get_str(t).to_string());
            eprintln!("{}UnionDecl '{}'", prefix, tag_str);
            for &m in members {
                dump_node(ctx, m, indent + 1);
            }
        }
        NodeKind::EnumDecl { tag, enumerators } => {
            let tag_str = tag.map_or("<anon>".to_string(), |t| ctx.get_str(t).to_string());
            eprintln!("{}EnumDecl '{}'", prefix, tag_str);
            for (name, val) in enumerators {
                let pref2 = "  ".repeat(indent + 1);
                eprintln!("{}Enumerator '{}'", pref2, ctx.get_str(*name));
                dump_node(ctx, *val, indent + 2);
            }
        }
        NodeKind::MemberDecl { name, type_node, bitfield } => {
            eprintln!("{}MemberDecl '{}'", prefix, ctx.get_str(*name));
            dump_node(ctx, *type_node, indent + 1);
            if *bitfield != NODE_NONE {
                dump_node(ctx, *bitfield, indent + 1);
            }
        }
        NodeKind::TypeSpec { spec } => {
            eprintln!("{}TypeSpec {:?}", prefix, spec);
        }
        NodeKind::PointerTo { base, is_const, is_volatile } => {
            let quals = match (*is_const, *is_volatile) {
                (true, true) => " const volatile",
                (true, false) => " const",
                (false, true) => " volatile",
                (false, false) => "",
            };
            eprintln!("{}PointerTo{}", prefix, quals);
            dump_node(ctx, *base, indent + 1);
        }
        NodeKind::ArrayOf { base, size } => {
            eprintln!("{}ArrayOf", prefix);
            dump_node(ctx, *base, indent + 1);
            if *size != NODE_NONE {
                dump_node(ctx, *size, indent + 1);
            }
        }
        NodeKind::CompoundStmt { stmts } => {
            eprintln!("{}CompoundStmt", prefix);
            for &s in stmts {
                dump_node(ctx, s, indent + 1);
            }
        }
        NodeKind::ReturnStmt { expr } => {
            eprintln!("{}ReturnStmt", prefix);
            if *expr != NODE_NONE {
                dump_node(ctx, *expr, indent + 1);
            }
        }
        NodeKind::IfStmt {
            cond,
            then_br,
            else_br,
        } => {
            eprintln!("{}IfStmt", prefix);
            dump_node(ctx, *cond, indent + 1);
            dump_node(ctx, *then_br, indent + 1);
            if *else_br != NODE_NONE {
                dump_node(ctx, *else_br, indent + 1);
            }
        }
        NodeKind::WhileStmt { cond, body } => {
            eprintln!("{}WhileStmt", prefix);
            dump_node(ctx, *cond, indent + 1);
            dump_node(ctx, *body, indent + 1);
        }
        NodeKind::ForStmt {
            init,
            cond,
            incr,
            body,
        } => {
            eprintln!("{}ForStmt", prefix);
            dump_node(ctx, *init, indent + 1);
            dump_node(ctx, *cond, indent + 1);
            dump_node(ctx, *incr, indent + 1);
            dump_node(ctx, *body, indent + 1);
        }
        NodeKind::ExprStmt { expr } => {
            eprintln!("{}ExprStmt", prefix);
            dump_node(ctx, *expr, indent + 1);
        }
        NodeKind::DoWhileStmt { body, cond } => {
            eprintln!("{}DoWhileStmt", prefix);
            dump_node(ctx, *body, indent + 1);
            dump_node(ctx, *cond, indent + 1);
        }
        NodeKind::SwitchStmt { expr, body } => {
            eprintln!("{}SwitchStmt", prefix);
            dump_node(ctx, *expr, indent + 1);
            dump_node(ctx, *body, indent + 1);
        }
        NodeKind::CaseStmt { expr, body } => {
            eprintln!("{}CaseStmt", prefix);
            dump_node(ctx, *expr, indent + 1);
            dump_node(ctx, *body, indent + 1);
        }
        NodeKind::DefaultStmt { body } => {
            eprintln!("{}DefaultStmt", prefix);
            dump_node(ctx, *body, indent + 1);
        }
        NodeKind::GotoStmt { label } => {
            eprintln!("{}GotoStmt '{}'", prefix, ctx.get_str(*label));
        }
        NodeKind::LabelStmt { label, stmt } => {
            eprintln!("{}LabelStmt '{}'", prefix, ctx.get_str(*label));
            dump_node(ctx, *stmt, indent + 1);
        }
        NodeKind::BreakStmt => eprintln!("{}BreakStmt", prefix),
        NodeKind::ContinueStmt => eprintln!("{}ContinueStmt", prefix),
        NodeKind::NullStmt => eprintln!("{}NullStmt", prefix),
        NodeKind::IntLiteral { value, suffix } => {
            eprintln!("{}IntLiteral {} {:?}", prefix, value, suffix);
        }
        NodeKind::FloatLiteral { value, suffix } => {
            eprintln!("{}FloatLiteral {} {:?}", prefix, value, suffix);
        }
        NodeKind::CharLiteral { value } => {
            eprintln!("{}CharLiteral {}", prefix, value);
        }
        NodeKind::StringLiteral { bytes } => {
            let s: String = bytes
                .iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as char)
                .collect();
            eprintln!("{}StringLiteral \"{}\"", prefix, s);
        }
        NodeKind::Ident { name } => {
            eprintln!("{}Ident '{}'", prefix, ctx.get_str(*name));
        }
        NodeKind::BinaryOp { op, lhs, rhs } => {
            eprintln!("{}BinaryOp {:?}", prefix, op);
            dump_node(ctx, *lhs, indent + 1);
            dump_node(ctx, *rhs, indent + 1);
        }
        NodeKind::UnaryOp { op, operand } => {
            eprintln!("{}UnaryOp {:?}", prefix, op);
            dump_node(ctx, *operand, indent + 1);
        }
        NodeKind::PostfixOp { op, operand } => {
            eprintln!("{}PostfixOp {:?}", prefix, op);
            dump_node(ctx, *operand, indent + 1);
        }
        NodeKind::Assign { op, lhs, rhs } => {
            eprintln!("{}Assign {:?}", prefix, op);
            dump_node(ctx, *lhs, indent + 1);
            dump_node(ctx, *rhs, indent + 1);
        }
        NodeKind::Call { callee, args } => {
            eprintln!("{}Call", prefix);
            dump_node(ctx, *callee, indent + 1);
            for &a in args {
                dump_node(ctx, a, indent + 1);
            }
        }
        NodeKind::Ternary {
            cond,
            then_expr,
            else_expr,
        } => {
            eprintln!("{}Ternary", prefix);
            dump_node(ctx, *cond, indent + 1);
            dump_node(ctx, *then_expr, indent + 1);
            dump_node(ctx, *else_expr, indent + 1);
        }
        NodeKind::Cast { type_node, expr } => {
            eprintln!("{}Cast", prefix);
            dump_node(ctx, *type_node, indent + 1);
            dump_node(ctx, *expr, indent + 1);
        }
        NodeKind::SizeofType { type_node } => {
            eprintln!("{}SizeofType", prefix);
            dump_node(ctx, *type_node, indent + 1);
        }
        NodeKind::SizeofExpr { expr } => {
            eprintln!("{}SizeofExpr", prefix);
            dump_node(ctx, *expr, indent + 1);
        }
        NodeKind::MemberAccess {
            expr,
            member,
            is_arrow,
        } => {
            let op = if *is_arrow { "->" } else { "." };
            eprintln!("{}MemberAccess {} '{}'", prefix, op, ctx.get_str(*member));
            dump_node(ctx, *expr, indent + 1);
        }
        NodeKind::ArraySubscript { expr, index } => {
            eprintln!("{}ArraySubscript", prefix);
            dump_node(ctx, *expr, indent + 1);
            dump_node(ctx, *index, indent + 1);
        }
        NodeKind::AddrOf { expr } => {
            eprintln!("{}AddrOf", prefix);
            dump_node(ctx, *expr, indent + 1);
        }
        NodeKind::Deref { expr } => {
            eprintln!("{}Deref", prefix);
            dump_node(ctx, *expr, indent + 1);
        }
        NodeKind::Comma { lhs, rhs } => {
            eprintln!("{}Comma", prefix);
            dump_node(ctx, *lhs, indent + 1);
            dump_node(ctx, *rhs, indent + 1);
        }
        NodeKind::InitList { values } => {
            eprintln!("{}InitList", prefix);
            for &v in values {
                dump_node(ctx, v, indent + 1);
            }
        }
        _ => {
            eprintln!("{}Node {:?}", prefix, std::mem::discriminant(&node.kind));
        }
    }
}
