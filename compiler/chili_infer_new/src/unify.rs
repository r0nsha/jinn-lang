use crate::{
    normalize::NormalizeTy,
    tycx::{TyBinding, TyCtx},
};
use chili_ast::{ty::*, workspace::Workspace};

pub(crate) type TyUnifyResult = Result<(), TyUnifyErr>;

#[derive(Debug)]
pub(crate) enum TyUnifyErr {
    Mismatch,
    Occurs,
}

pub(crate) trait Unify<T>
where
    Self: Sized,
    T: Sized,
{
    fn unify(&self, other: &T, tycx: &mut TyCtx, workspace: &Workspace) -> TyUnifyResult;
}

impl Unify<Ty> for Ty {
    fn unify(&self, other: &Ty, tycx: &mut TyCtx, workspace: &Workspace) -> TyUnifyResult {
        let t1 = TyKind::Var(*self);
        let t2 = TyKind::Var(*other);
        t1.unify(&t2, tycx, workspace)
    }
}

impl Unify<TyKind> for Ty {
    fn unify(&self, other: &TyKind, tycx: &mut TyCtx, workspace: &Workspace) -> TyUnifyResult {
        let ty = TyKind::Var(*self);
        ty.unify(other, tycx, workspace)
    }
}

impl Unify<Ty> for TyKind {
    fn unify(&self, other: &Ty, tycx: &mut TyCtx, workspace: &Workspace) -> TyUnifyResult {
        let other = TyKind::Var(*other);
        self.unify(&other, tycx, workspace)
    }
}

impl Unify<TyKind> for TyKind {
    fn unify(&self, other: &TyKind, tycx: &mut TyCtx, workspace: &Workspace) -> TyUnifyResult {
        match (self, other) {
            (TyKind::Unit, TyKind::Unit) => Ok(()),
            (TyKind::Bool, TyKind::Bool) => Ok(()),

            (TyKind::Int(t1), TyKind::Int(t2)) if t1 == t2 => Ok(()),
            (TyKind::UInt(t1), TyKind::UInt(t2)) if t1 == t2 => Ok(()),
            (TyKind::Float(t1), TyKind::Float(t2)) if t1 == t2 => Ok(()),

            // int/int
            (TyKind::AnyInt(var), ty @ TyKind::AnyInt(_))
            | (TyKind::AnyInt(var), ty @ TyKind::Int(_))
            | (ty @ TyKind::Int(_), TyKind::AnyInt(var))
            | (TyKind::AnyInt(var), ty @ TyKind::UInt(_))
            | (ty @ TyKind::UInt(_), TyKind::AnyInt(var))
            // int/float
            | (TyKind::AnyInt(var), ty @ TyKind::AnyFloat(_))
            | (ty @ TyKind::AnyFloat(_), TyKind::AnyInt(var))
            | (TyKind::AnyInt(var), ty @ TyKind::Float(_))
            | (ty @ TyKind::Float(_), TyKind::AnyInt(var))
            // float/float
            | (TyKind::AnyFloat(var), ty @ TyKind::AnyFloat(_))
            | (TyKind::AnyFloat(var), ty @ TyKind::Float(_))
            | (ty @ TyKind::Float(_), TyKind::AnyFloat(var)) => {
                tycx.bind(*var, ty.clone());
                Ok(())
            }

            (TyKind::Pointer(t1, a1),TyKind::Pointer(t2, a2))
            | (TyKind::MultiPointer(t1, a1),TyKind::MultiPointer(t2, a2)) 
            | (TyKind::Slice(t1, a1),TyKind::Slice(t2, a2)) => {
                if !can_coerce_mut(*a1,*a2) {
                    Err(TyUnifyErr::Mismatch)
                } else {
                    t1.unify(t2.as_ref(), tycx, workspace)?;
                    Ok(())
                }
            }

            (TyKind::Fn(f1),TyKind::Fn(f2)) => {
                if f1.params.len() != f2.params.len() || f1.variadic != f2.variadic {
                    Err(TyUnifyErr::Mismatch)
                } else {
                    for (p1, p2) in f1.params.iter().zip(f2.params.iter()) {
                        p1.ty.unify(&p2.ty, tycx, workspace)?;
                    }
                    f1.ret.unify(f2.ret.as_ref(), tycx, workspace)?;
                    Ok(())
                }
            }

            (TyKind::Array(t1, s1),TyKind::Array(t2, s2)) => {
                if *s1 != *s2 {
                    Err(TyUnifyErr::Mismatch)
                } else {
                    t1.unify(t2.as_ref(), tycx, workspace)?;
                    Ok(())
                }
            }

            (TyKind::Tuple(t1),TyKind::Tuple(t2)) => {
                if t1.len() != t2.len() {
                    Err(TyUnifyErr::Mismatch)
                } else {
                    for (t1, t2) in t1.iter().zip(t2.iter()) {
                        t1.unify(t2, tycx, workspace)?;
                    }
                    Ok(())
                }
            }

            (TyKind::Struct(t1),TyKind::Struct(t2)) => {
                if t1.binding_info_id == t2.binding_info_id {
                    Ok(())
                } else if t1.fields.len() != t2.fields.len() || t1.kind != t2.kind {
                    Err(TyUnifyErr::Mismatch)
                } else {
                    for (f1, f2) in t1.fields.iter().zip(t2.fields.iter()) {
                        f1.ty.unify(&f2.ty, tycx, workspace)?;
                    }
                    Ok(())
                }
            }
            
            (TyKind::Type(t1), TyKind::Type(t2)) => t1.unify(t2.as_ref(), tycx, workspace),
            (TyKind::Type(t1), t2) => t1.unify(t2, tycx, workspace),
            (t1, TyKind::Type(t2)) => t1.unify(t2.as_ref(), tycx, workspace),

            (TyKind::Var(var), _) => unify_var_ty(*var, other, tycx, workspace),
            (_, TyKind::Var(var)) => unify_var_ty(*var, self, tycx, workspace),

            (TyKind::Never, _) | (_, TyKind::Never) => Ok(()),

            _ => {
                // println!("{} <=> {}", self, other);
                Err(TyUnifyErr::Mismatch)},
        }
    }
}

fn unify_var_ty(var: Ty, other: &TyKind, tycx: &mut TyCtx, workspace: &Workspace) -> TyUnifyResult {
    match tycx.get_binding(var) {
        TyBinding::Bound(kind) => kind.clone().unify(other, tycx, workspace),
        TyBinding::Unbound => {
            let other_norm = other.normalize(tycx);

            if TyKind::Var(var) != other_norm {
                if occurs(var, &other_norm, tycx, workspace) {
                    Err(TyUnifyErr::Occurs)
                } else {
                    tycx.bind(var, other_norm);
                    Ok(())
                }
            } else {
                Ok(())
            }
        }
    }
}

fn occurs(var: Ty, kind: &TyKind, tycx: &TyCtx, workspace: &Workspace) -> bool {
    match kind {
        TyKind::Var(other) => match tycx.get_binding(*other) {
            TyBinding::Bound(ty) => occurs(var, &ty, tycx, workspace),
            TyBinding::Unbound => var == *other,
        },
        TyKind::Fn(f) => {
            f.params.iter().any(|p| occurs(var, &p.ty, tycx, workspace))
                || occurs(var, &f.ret, tycx, workspace)
        }
        TyKind::Pointer(ty, _)
        | TyKind::MultiPointer(ty, _)
        | TyKind::Array(ty, _)
        | TyKind::Slice(ty, _) => occurs(var, ty, tycx, workspace),
        TyKind::Tuple(tys) => tys.iter().any(|ty| occurs(var, ty, tycx, workspace)),
        TyKind::Struct(st) => st
            .fields
            .iter()
            .any(|f| occurs(var, &f.ty, tycx, workspace)),
        _ => false,
    }
}

// NOTE (Ron): checks that mutability rules are equal
pub(crate) fn can_coerce_mut(from: bool, to: bool) -> bool {
    from == to || (!from && to)
}
