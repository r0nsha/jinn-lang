use std::collections::HashSet;

use chili_ast::ast::{self, ModuleInfo};
use chili_span::Span;
use common::compiler_info::{self, IntrinsticModuleInfo};

pub(crate) fn insert_std_import(ast: &mut ast::Ast, imports: &mut HashSet<ModuleInfo>) {
    add_intrinsic_module(ast, imports, compiler_info::std_module_info())
}

pub(crate) fn add_intrinsic_module(
    ast: &mut ast::Ast,
    imports: &mut HashSet<ModuleInfo>,
    intrinsic_module_info: IntrinsticModuleInfo,
) {
    let intrinsic_module_info =
        ModuleInfo::new(intrinsic_module_info.name, intrinsic_module_info.file_path);

    ast.imports.push(ast::Import {
        binding_info_id: Default::default(),
        module_id: Default::default(),
        module_info: intrinsic_module_info,
        alias: intrinsic_module_info.name,
        import_path: vec![],
        visibility: ast::Visibility::Private,
        span: Span::unknown(),
    });

    imports.insert(intrinsic_module_info);
}
