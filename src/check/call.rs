use crate::{
    ast,
    check::intrinsics::{can_dispatch_intrinsic_at_comptime, dispatch_intrinsic},
    error::{
        diagnostic::{Diagnostic, Label},
        DiagnosticResult,
    },
    hir,
    infer::{
        coerce::{coerce_array_to_slice, OrCoerceIntoTy},
        display::{DisplayType, OrReportErr},
        normalize::Normalize,
        unify::UnifyType,
    },
    span::Span,
    sym,
    types::*,
};

use super::{env::Env, Check, CheckResult, CheckSess};

impl Check for ast::Call {
    fn check(&self, sess: &mut CheckSess, env: &mut Env, _expected_type: Option<TypeId>) -> CheckResult {
        let callee = self.callee.check(sess, env, None)?;

        match callee.ty().normalize(&sess.tcx) {
            Type::Function(function_type) => {
                let mut args: Vec<hir::Node> = vec![];

                enum Varargs {
                    Empty,
                    Individual(Vec<hir::Node>),
                    Spread(hir::Node),
                }

                let mut vararg_args = Varargs::Empty;

                // If the function was annotated by track_caller, its first argument
                // should be the inserted location parameter: track_caller@location
                let param_offset = match function_type.params.first() {
                    Some(param) if param.name == sym::TRACK_CALLER_LOCATION_PARAM => {
                        let ty = sess.location_type()?;

                        let arg = match sess.get_track_caller_location_param_id(env, self.span) {
                            Ok(id) => hir::Node::Id(hir::Id {
                                id,
                                ty,
                                span: self.span,
                            }),
                            Err(_) => {
                                let value = sess.build_location_value(env, self.span)?;

                                hir::Node::Const(hir::Const {
                                    value,
                                    ty,
                                    span: self.span,
                                })
                            }
                        };

                        args.push(arg);

                        1
                    }
                    _ => 0,
                };

                // Check the arguments passed against the function's parameter types
                for (index, arg) in self.args.iter().enumerate() {
                    if let Some(param) = function_type.params.get(index + param_offset) {
                        let param_type = sess.tcx.bound(param.ty.clone(), arg.value.span());
                        let mut node = arg.value.check(sess, env, Some(param_type))?;

                        node.ty()
                            .unify(&param_type, &mut sess.tcx)
                            .or_coerce_into_ty(&mut node, &param_type, &mut sess.tcx, sess.target_metrics.word_size)
                            .or_report_err(&sess.tcx, &param_type, None, &node.ty(), arg.value.span())?;

                        args.push(node);
                    } else if let Some(varargs) = &function_type.varargs {
                        // this is a variadic argument, meaning that the argument's
                        // index is greater than the function's param length
                        let mut node = arg.value.check(sess, env, None)?;

                        if let Some(vararg_type) = &varargs.ty {
                            let is_last = index == self.args.len() - 1;
                            match (arg.spread, is_last) {
                                (true, true) => {
                                    // This is a spreaded variadic argument
                                    match &vararg_args {
                                        Varargs::Individual(varargs) => {
                                            return Err(Diagnostic::error()
                                                .with_message(
                                                    "variadic arguments cannot be passed and spreaded at the same time",
                                                )
                                                .with_label(Label::primary(
                                                    arg.value.span(),
                                                    "cannot spread this argument",
                                                ))
                                                .with_label(Label::secondary(
                                                    varargs[0].span(),
                                                    "first variadic argument passed here",
                                                )))
                                        }
                                        Varargs::Spread(node) => {
                                            return Err(Diagnostic::error()
                                                .with_message("already spreaded variadic arguments")
                                                .with_label(Label::primary(
                                                    arg.value.span(),
                                                    "variadic arguments spreaded twice",
                                                ))
                                                .with_label(Label::secondary(node.span(), "first spread here")))
                                        }
                                        _ => {
                                            let ty = node.ty().normalize(&sess.tcx);

                                            match node.ty().normalize(&sess.tcx) {
                                                Type::Pointer(inner, _) => match inner.as_ref() {
                                                    Type::Slice(elem_type) => {
                                                        elem_type.unify(vararg_type, &mut sess.tcx).or_report_err(
                                                            &sess.tcx,
                                                            vararg_type,
                                                            None,
                                                            elem_type.as_ref(),
                                                            node.span(),
                                                        )?;

                                                        vararg_args = Varargs::Spread(node);
                                                    }
                                                    _ => {
                                                        return Err(Diagnostic::error()
                                                            .with_message(format!(
                                                                "cannot spread argument of type `{}`",
                                                                ty.display(&sess.tcx)
                                                            ))
                                                            .with_label(Label::primary(
                                                                arg.value.span(),
                                                                "invalid argument type",
                                                            )))
                                                    }
                                                },
                                                Type::Array(elem_type, _) => {
                                                    elem_type.unify(vararg_type, &mut sess.tcx).or_report_err(
                                                        &sess.tcx,
                                                        vararg_type,
                                                        None,
                                                        elem_type.as_ref(),
                                                        node.span(),
                                                    )?;

                                                    let (bound_node, rvalue_node) =
                                                        sess.build_rvalue_ref(env, node, false, self.span)?;

                                                    let slice_type = Type::slice_pointer(vararg_type.clone(), false);

                                                    let varargs_slice = coerce_array_to_slice(
                                                        &mut sess.tcx,
                                                        &rvalue_node,
                                                        slice_type.clone(),
                                                    );

                                                    let varargs_seq = hir::Node::Sequence(hir::Sequence {
                                                        statements: vec![bound_node, varargs_slice],
                                                        ty: sess.tcx.bound(slice_type, self.span),
                                                        span: self.span,
                                                        is_scope: false,
                                                    });

                                                    vararg_args = Varargs::Spread(varargs_seq);
                                                }
                                                _ => {
                                                    return Err(Diagnostic::error()
                                                        .with_message(format!(
                                                            "cannot spread argument of type `{}`",
                                                            ty.display(&sess.tcx)
                                                        ))
                                                        .with_label(Label::primary(
                                                            arg.value.span(),
                                                            "invalid argument type",
                                                        )))
                                                }
                                            }
                                        }
                                    }
                                }
                                (true, false) => {
                                    return Err(Diagnostic::error()
                                        .with_message("variadic argument spread must come last")
                                        .with_label(Label::primary(arg.value.span(), "invalid argument spread")))
                                }
                                _ => {
                                    // This is a regular variadic argument

                                    node.ty()
                                        .unify(vararg_type, &mut sess.tcx)
                                        .or_coerce_into_ty(
                                            &mut node,
                                            vararg_type,
                                            &mut sess.tcx,
                                            sess.target_metrics.word_size,
                                        )
                                        .or_report_err(&sess.tcx, vararg_type, None, &node.ty(), arg.value.span())?;

                                    match &mut vararg_args {
                                        Varargs::Individual(varargs) => {
                                            varargs.push(node);
                                        }
                                        Varargs::Spread(_) => unreachable!(),
                                        _ => {
                                            vararg_args = Varargs::Individual(vec![node]);
                                        }
                                    }
                                }
                            }
                        } else if arg.spread {
                            return Err(Diagnostic::error()
                                .with_message("cannot spread untyped variadic arguments")
                                .with_label(Label::primary(arg.value.span(), "cannot spread this argument")));
                        } else {
                            args.push(node);
                        }
                    } else {
                        return Err(arg_mismatch(sess, &function_type, self.args.len(), self.span));
                    }
                }

                if let Some(varargs) = &function_type.varargs {
                    if let Some(vararg_type) = &varargs.ty {
                        match vararg_args {
                            Varargs::Empty => (),
                            Varargs::Individual(vararg_args) => {
                                // Build a slice out of the passed variadic arguments
                                let varargs_array_literal =
                                    sess.array_literal_or_const(vararg_args, vararg_type.clone(), self.span);

                                let (bound_node, rvalue_node) =
                                    sess.build_rvalue_ref(env, varargs_array_literal, false, self.span)?;

                                let slice_type = Type::slice_pointer(vararg_type.clone(), false);

                                let varargs_slice =
                                    coerce_array_to_slice(&mut sess.tcx, &rvalue_node, slice_type.clone());

                                let varargs_seq = hir::Node::Sequence(hir::Sequence {
                                    statements: vec![bound_node, varargs_slice],
                                    ty: sess.tcx.bound(slice_type, self.span),
                                    span: self.span,
                                    is_scope: false,
                                });

                                args.push(varargs_seq);
                            }
                            Varargs::Spread(node) => args.push(node),
                        }
                    }
                }

                if args.len() < function_type.params.len() {
                    for param in function_type.params.iter().skip(args.len()) {
                        if let Some(default_value) = &param.default_value {
                            args.push(hir::Node::Const(hir::Const {
                                value: default_value.clone(),
                                ty: sess.tcx.bound(param.ty.clone(), self.span),
                                span: self.span,
                            }))
                        } else {
                            return Err(arg_mismatch(sess, &function_type, args.len(), self.span));
                        }
                    }
                }

                match &function_type.varargs {
                    Some(_) if args.len() < function_type.params.len() => {
                        return Err(arg_mismatch(sess, &function_type, args.len(), self.span))
                    }
                    None if args.len() != function_type.params.len() => {
                        return Err(arg_mismatch(sess, &function_type, args.len(), self.span))
                    }
                    _ => (),
                }

                validate_call_args(sess, &args)?;

                let ty = sess.tcx.bound(function_type.return_type.as_ref().clone(), self.span);

                if let Some(intrinsic) = can_dispatch_intrinsic_at_comptime(sess, &callee) {
                    dispatch_intrinsic(sess, env, &intrinsic, &args, ty, self.span)
                } else {
                    Ok(hir::Node::Call(hir::Call {
                        callee: Box::new(callee),
                        args,
                        ty,
                        span: self.span,
                    }))
                }
            }
            ty => {
                Err(Diagnostic::error()
                    .with_message(format!(
                        "expected a function or a struct, found `{}`",
                        ty.display(&sess.tcx)
                    ))
                    .with_label(Label::primary(callee.span(), "expression is not callable")))
                // // Try to infer this expression as a function
                // let args = self
                //     .args
                //     .iter()
                //     .map(|arg| arg.value.check(sess, env, None))
                //     .collect::<DiagnosticResult<Vec<_>>>()?;

                // let return_type = sess.tcx.var(self.span);

                // let inferred_function_type = Type::Function(FunctionType {
                //     params: args
                //         .iter()
                //         .map(|arg| FunctionTypeParam {
                //             name: ustr(""),
                //             ty: arg.ty().into(),
                //             default_value: None,
                //         })
                //         .collect(),
                //     return_type: Box::new(return_type.into()),
                //     varargs: None,
                //     kind: FunctionTypeKind::Orphan,
                // });

                // ty.unify(&inferred_function_type, &mut sess.tcx).or_report_err(
                //     &sess.tcx,
                //     &inferred_function_type,
                //     None,
                //     &ty,
                //     self.callee.span(),
                // )?;
                //
                // validate_call_args(sess, &args)?;

                // Ok(hir::Node::Call(hir::Call {
                //     callee: Box::new(callee),
                //     args,
                //     ty: return_type,
                //     span: self.span,
                // }))
            }
        }
    }
}

fn validate_call_args(sess: &mut CheckSess, args: &[hir::Node]) -> DiagnosticResult<()> {
    for arg in args.iter() {
        match arg.ty().normalize(&sess.tcx) {
            Type::Type(_) | Type::AnyType => {
                return Err(Diagnostic::error()
                    .with_message("types cannot be passed as function arguments")
                    .with_label(Label::primary(arg.span(), "cannot pass type")))
            }
            Type::Module(_) => {
                return Err(Diagnostic::error()
                    .with_message("modules cannot be passed as function arguments")
                    .with_label(Label::primary(arg.span(), "cannot pass module")))
            }
            _ => (),
        }
    }

    Ok(())
}

fn arg_mismatch(sess: &CheckSess, function_type: &FunctionType, arg_count: usize, span: Span) -> Diagnostic {
    let expected = function_type.params.len();
    let actual = arg_count;

    Diagnostic::error()
        .with_message(format!(
            "function expects {} argument{}, but {} {} supplied",
            expected,
            if expected == 0 || expected > 1 { "s" } else { "" },
            actual,
            if actual == 0 || actual > 1 { "were" } else { "was" },
        ))
        .with_label(Label::primary(
            span,
            format!(
                "expected {} argument{}, got {}",
                expected,
                if expected == 0 || expected > 1 { "s" } else { "" },
                actual
            ),
        ))
        .with_note(format!("function is of type `{}`", function_type.display(&sess.tcx)))
}
