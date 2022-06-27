use crate::{
    ast::workspace::Workspace,
    hir,
    infer::{display::DisplayTy, normalize::Normalize, ty_ctx::TyCtx},
};
use itertools::Itertools;
use std::{fs::OpenOptions, io::Write, path::Path};

const INDENT: u16 = 2;

#[allow(unused)]
pub fn print(cache: &hir::Cache, workspace: &Workspace, tycx: &TyCtx) {
    if let Ok(file) = &OpenOptions::new()
        .read(false)
        .write(true)
        .create(true)
        .truncate(true)
        .append(false)
        .open(Path::new("hir.pretty.chili"))
    {
        let mut printer = Printer::new(workspace, tycx, file);
        cache.print(&mut printer, true);
    }
}

struct Printer<'a, W: Write> {
    workspace: &'a Workspace,
    tycx: &'a TyCtx,
    writer: W,
    identation: u16,
}

impl<'a, W: Write> Printer<'a, W> {
    fn new(workspace: &'a Workspace, tycx: &'a TyCtx, writer: W) -> Self {
        Self {
            workspace,
            tycx,
            writer,
            identation: 0,
        }
    }

    fn indent(&mut self) {
        self.identation += INDENT;
    }

    fn dedent(&mut self) {
        self.identation -= INDENT;
    }

    fn write(&mut self, s: &str) {
        self.writer.write_all(s.as_bytes()).unwrap();
    }

    fn write_indented(&mut self, s: &str, is_line_start: bool) {
        if is_line_start && self.identation > 0 {
            self.write(&(0..=self.identation).map(|_| " ").collect::<String>());
        }
        self.write(s);
    }

    fn write_comment(&mut self, s: &str, is_line_start: bool) {
        self.write_indented(&format!("// {}", s), is_line_start)
    }
}

trait Print<'a, W: Write> {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool);
}

impl<'a, W: Write> Print<'a, W> for hir::Cache {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        enum Item<'a> {
            Binding(&'a hir::Binding),
            Function(&'a hir::Function),
        }

        self.bindings
            .iter()
            .map(|(_, b)| Item::Binding(b))
            .chain(self.functions.iter().map(|(_, f)| Item::Function(f)))
            .group_by(|item| match item {
                Item::Binding(x) => x.module_id,
                Item::Function(x) => x.module_id,
            })
            .into_iter()
            .for_each(|(module_id, items)| {
                let module_info = p.workspace.module_infos.get(module_id).unwrap();

                p.write_comment(
                    &format!("{} ({})\n\n", module_info.name, module_info.file_path),
                    true,
                );

                for item in items {
                    match item {
                        Item::Binding(binding) => binding.print(p, true),
                        Item::Function(function) => function.print(p, true),
                    }

                    p.write(";\n\n");
                }
            });
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Node {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        match self {
            hir::Node::Const(x) => x.print(p, is_line_start),
            hir::Node::Binding(x) => x.print(p, is_line_start),
            hir::Node::Id(x) => x.print(p, is_line_start),
            hir::Node::Assignment(x) => x.print(p, is_line_start),
            hir::Node::MemberAccess(x) => x.print(p, is_line_start),
            hir::Node::Call(x) => x.print(p, is_line_start),
            hir::Node::Cast(x) => x.print(p, is_line_start),
            hir::Node::Sequence(x) => x.print(p, is_line_start),
            hir::Node::Control(x) => x.print(p, is_line_start),
            hir::Node::Builtin(x) => x.print(p, is_line_start),
            hir::Node::Literal(x) => x.print(p, is_line_start),
        }
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Binding {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        p.write_indented("let ", is_line_start);
        p.write(&self.name);
        // p.write(": ");
        // p.write(
        //     &p.workspace
        //         .binding_infos
        //         .get(self.id)
        //         .unwrap()
        //         .ty
        //         .display(p.tycx),
        // );
        p.write(" = ");
        self.value.print(p, false);
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Function {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        match &self.kind {
            hir::FunctionKind::Orphan { .. } => p.write_indented("fn ", is_line_start),
            hir::FunctionKind::Extern { lib } => {
                if let Some(lib) = lib {
                    p.write_indented(&format!("extern fn \"{}\" ", lib.path()), is_line_start)
                } else {
                    p.write_indented("extern fn ", is_line_start)
                }
            }
            hir::FunctionKind::Intrinsic(_) => p.write_indented("intrinsic fn ", is_line_start),
        }

        p.write(&self.name);

        let function_type = self.ty.normalize(p.tycx).into_function();

        p.write("(");
        for (index, param) in function_type.params.iter().enumerate() {
            p.write(&param.display(p.tycx));

            if index < function_type.params.len() - 1 {
                p.write(", ");
            }
        }
        p.write(") -> ");
        p.write(&function_type.return_type.display(p.tycx));
        p.write(" ");

        match &self.kind {
            hir::FunctionKind::Orphan { body } => body.as_ref().unwrap().print(p, false),
            hir::FunctionKind::Extern { .. } | hir::FunctionKind::Intrinsic(..) => (),
        }
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Const {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        p.write_indented(&self.value.display(p.tycx), is_line_start);
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Id {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        p.write_indented(
            &p.workspace.binding_infos.get(self.id).unwrap().name,
            is_line_start,
        );
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Assignment {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        self.lhs.print(p, is_line_start);
        p.write(" = ");
        self.rhs.print(p, false);
    }
}

impl<'a, W: Write> Print<'a, W> for hir::MemberAccess {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        self.value.print(p, is_line_start);
        p.write(".");
        p.write(&self.member);
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Call {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        self.callee.print(p, is_line_start);

        p.write("(");

        for (index, arg) in self.args.iter().enumerate() {
            arg.print(p, false);

            if index < self.args.len() - 1 {
                p.write(", ");
            }
        }

        p.write(")");
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Cast {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        self.value.print(p, is_line_start);
        p.write(" as ");
        p.write(&self.ty.display(p.tycx));
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Sequence {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        if self.is_block {
            p.write_indented("{\n", is_line_start);
            p.indent();
        }

        for (index, statement) in self.statements.iter().enumerate() {
            statement.print(p, true);
            if index < self.statements.len() - 1 {
                p.write(";\n");
            } else {
                p.write("\n");
            }
        }

        if self.is_block {
            p.dedent();
            p.write_indented("}", true);
        }
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Control {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        match self {
            hir::Control::If(if_) => {
                p.write_indented("if ", is_line_start);
                if_.condition.print(p, false);

                if_.then.print(p, false);

                if let Some(otherwise) = &if_.otherwise {
                    p.write(" else ");
                    otherwise.print(p, false);
                }
            }
            hir::Control::While(while_) => {
                p.write_indented("while ", is_line_start);
                while_.condition.print(p, false);
                while_.body.print(p, false);
            }
            hir::Control::Return(return_) => {
                p.write_indented("return ", is_line_start);
                return_.value.print(p, false);
            }
            hir::Control::Break(_) => p.write_indented("break", is_line_start),
            hir::Control::Continue(_) => p.write_indented("continue", is_line_start),
        }
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Builtin {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        fn write_binary<'a, W: Write>(
            op: &str,
            binary: &hir::Binary,
            p: &mut Printer<'a, W>,
            is_line_start: bool,
        ) {
            binary.lhs.print(p, is_line_start);
            p.write(" ");
            p.write(op);
            p.write(" ");
            binary.rhs.print(p, false);
        }

        match self {
            hir::Builtin::Add(binary) => write_binary("+", binary, p, is_line_start),
            hir::Builtin::Sub(binary) => write_binary("-", binary, p, is_line_start),
            hir::Builtin::Mul(binary) => write_binary("*", binary, p, is_line_start),
            hir::Builtin::Div(binary) => write_binary("/", binary, p, is_line_start),
            hir::Builtin::Rem(binary) => write_binary("%", binary, p, is_line_start),
            hir::Builtin::Shl(binary) => write_binary("<<", binary, p, is_line_start),
            hir::Builtin::Shr(binary) => write_binary(">>", binary, p, is_line_start),
            hir::Builtin::And(binary) => write_binary("&&", binary, p, is_line_start),
            hir::Builtin::Or(binary) => write_binary("||", binary, p, is_line_start),
            hir::Builtin::Lt(binary) => write_binary("<", binary, p, is_line_start),
            hir::Builtin::Le(binary) => write_binary("<=", binary, p, is_line_start),
            hir::Builtin::Gt(binary) => write_binary(">", binary, p, is_line_start),
            hir::Builtin::Ge(binary) => write_binary(">=", binary, p, is_line_start),
            hir::Builtin::Eq(binary) => write_binary("==", binary, p, is_line_start),
            hir::Builtin::Ne(binary) => write_binary("!=", binary, p, is_line_start),
            hir::Builtin::BitAnd(binary) => write_binary("&", binary, p, is_line_start),
            hir::Builtin::BitOr(binary) => write_binary("|", binary, p, is_line_start),
            hir::Builtin::BitXor(binary) => write_binary("^", binary, p, is_line_start),
            hir::Builtin::Not(unary) => {
                p.write_indented("!", is_line_start);
                unary.value.print(p, false);
            }
            hir::Builtin::Neg(unary) => {
                p.write_indented("-", is_line_start);
                unary.value.print(p, false);
            }
            hir::Builtin::Ref(unary) => {
                p.write_indented("&", is_line_start);
                unary.value.print(p, false);
            }
            hir::Builtin::Deref(unary) => {
                unary.value.print(p, is_line_start);
                p.write(".*");
            }
            hir::Builtin::Offset(offset) => {
                offset.value.print(p, is_line_start);
                p.write("[offset: ");
                offset.offset.print(p, false);
                p.write("]");
            }
            hir::Builtin::Slice(slice) => {
                slice.value.print(p, is_line_start);
                p.write("[");
                slice.low.print(p, false);
                p.write("..");
                slice.high.print(p, false);
                p.write("]");
            }
        }
    }
}

impl<'a, W: Write> Print<'a, W> for hir::Literal {
    fn print(&self, p: &mut Printer<'a, W>, is_line_start: bool) {
        todo!();
    }
}
