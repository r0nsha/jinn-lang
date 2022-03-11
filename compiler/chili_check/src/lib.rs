mod check_assign;
mod check_binary;
mod check_binding;
mod check_call;
mod check_expr;
mod check_fn;
mod check_pattern;
mod check_unary;
mod lints;

use chili_ast::ty::Ty;
use chili_ast::workspace::ModuleIdx;
use chili_ast::{
    ast::Ast,
    workspace::{BindingInfoIdx, Workspace},
};
use chili_error::DiagnosticResult;
use chili_infer::infer::InferenceContext;
use chili_infer::substitute::{substitute_ty, Substitute};
use common::scopes::Scopes;

pub fn check<'w>(workspace: &mut Workspace<'w>, asts: &mut Vec<Ast>) -> DiagnosticResult<()> {
    let target_metrics = workspace.build_options.target_platform.metrics();
    let mut infcx = InferenceContext::new(target_metrics.word_size);

    // infer types
    {
        let mut sess = CheckSess::new(workspace, &mut infcx);

        sess.init_scopes.push_scope();

        for ast in asts.iter_mut() {
            for import in ast.imports.iter_mut() {
                sess.check_import(import)?;
            }

            for binding in ast.bindings.iter_mut() {
                sess.check_top_level_binding(binding, ast.module_idx, binding.pattern.span())?;
            }
        }

        sess.init_scopes.pop_scope();
    }

    // substitute type variables
    let table = infcx.get_table_mut();

    for ast in asts.iter_mut() {
        for binding in ast.bindings.iter_mut() {
            binding.substitute(table)?;
        }
    }

    for binding_info in workspace.binding_infos.iter_mut() {
        substitute_ty(&binding_info.ty, table, binding_info.span)?;
    }

    for binding in workspace.binding_infos.iter() {
        println!("{} -> {:?}", binding.symbol, binding.ty);
    }

    Ok(())
}

pub(crate) struct CheckSess<'w, 'a> {
    pub(crate) workspace: &'a mut Workspace<'w>,
    pub(crate) infcx: &'a mut InferenceContext,
    pub(crate) init_scopes: Scopes<BindingInfoIdx, InitState>,
}

impl<'w, 'a> CheckSess<'w, 'a> {
    pub(crate) fn new(workspace: &'a mut Workspace<'w>, infcx: &'a mut InferenceContext) -> Self {
        Self {
            workspace,
            infcx,
            init_scopes: Scopes::new(),
        }
    }

    pub(crate) fn update_binding_info_ty(&mut self, idx: BindingInfoIdx, ty: Ty) {
        self.workspace.get_binding_info_mut(idx).unwrap().ty = ty;
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum InitState {
    NotInit,
    Init,
}

impl InitState {
    pub(crate) fn is_not_init(&self) -> bool {
        match self {
            InitState::NotInit => true,
            _ => false,
        }
    }

    pub(crate) fn is_init(&self) -> bool {
        match self {
            InitState::Init => true,
            _ => false,
        }
    }
}

pub(crate) struct CheckFrame {
    pub(crate) depth: usize,
    pub(crate) loop_depth: usize,
    pub(crate) module_idx: ModuleIdx,
    pub(crate) expected_return_ty: Option<Ty>,
    pub(crate) self_types: Vec<Ty>,
}

impl CheckFrame {
    pub(crate) fn new(
        previous_depth: usize,
        module_idx: ModuleIdx,
        expected_return_ty: Option<Ty>,
    ) -> Self {
        Self {
            depth: previous_depth + 1,
            loop_depth: 0,
            module_idx,
            expected_return_ty,
            self_types: vec![],
        }
    }
}
