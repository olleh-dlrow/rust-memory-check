use rustc_hir::def_id::DefId;
use rustc_middle::mir::terminator::TerminatorKind;
use rustc_middle::mir::Operand;
use rustc_middle::mir::Place;
use rustc_middle::mir::ProjectionElem;
use rustc_middle::mir::Rvalue;
use rustc_middle::mir::StatementKind;
use rustc_middle::ty;
use rustc_middle::ty::TyKind;
use rustc_span::Span;
use std::collections::{HashMap, HashSet};

use crate::core::utils;

use crate::core::AnalysisOptions;

use crate::core::BasicBlockId;
use crate::core::LocalId;

use crate::core::AssignmentInfo;

use super::BasicBlockInfo;
use super::CallInfo;
use super::LocalInfo;
use super::OpKind;
use super::RvalKind;

#[derive(Debug)]
pub struct ControlFlowGraph<'tcx> {
    pub options: AnalysisOptions,
    pub def_id: rustc_hir::def_id::DefId,
    pub local_infos: HashMap<LocalId, LocalInfo<'tcx>>,
    pub basic_block_infos: HashMap<BasicBlockId, BasicBlockInfo<'tcx>>,
    pub call_infos: HashMap<BasicBlockId, CallInfo<'tcx>>,
    pub has_ret: bool,
}

impl<'tcx> ControlFlowGraph<'tcx> {
    pub fn new(
        opts: &AnalysisOptions,
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
        def_id: rustc_hir::def_id::DefId,
    ) -> Self {
        let body: &rustc_middle::mir::Body = tcx.optimized_mir(def_id);
        if utils::has_dbg(opts, "body") {
            log::debug!("body of def id {:?}: \n{:#?}", def_id, body);
        }

        if utils::has_dbg(opts, "var-debug-info") {
            let infos = &body.var_debug_info;

            for info in infos {
                log::debug!("VarDebugInfo: {:?} {:?}", info.name, info.value);
            }

        }

        let has_ret = !body.local_decls.iter().next().unwrap().ty.is_unit();

        let mut local_infos = body
            .local_decls
            .iter_enumerated()
            .map(|(local, local_decl)| {
                let local_info = LocalInfo::new(
                    local,
                    local_decl.ty.needs_drop(tcx, tcx.param_env(def_id)),
                    local_decl.ty,
                    None,
                    local_decl.source_info.span,
                );
                (local, local_info)
            })
            .collect::<HashMap<_, _>>();


        // set var debug info
        for info in &body.var_debug_info {
            if let rustc_middle::mir::VarDebugInfoContents::Place(ref place) = info.value {
                if let Some(local_info) = local_infos.get_mut(&place.local) {
                    local_info.var_name = Some(info.name.to_string());
                }
            }
        }

        let call_infos = body
            .basic_blocks()
            .iter_enumerated()
            .map(|(bb, bb_data)| {
                let terminator = bb_data.terminator.as_ref().unwrap();
                if let TerminatorKind::Call {
                    ref func,
                    ref args,
                    ref destination,
                    ..
                } = &terminator.kind
                {
                    // match func {
                    //     Operand::Copy(_) => {log::debug!("func is copy: {:#?}", func)},
                    //     Operand::Move(_) => {log::debug!("func is move: {:#?}", func)},
                    //     Operand::Constant(_) => {log::debug!("func is constant: {:#?}", func)},
                    // };

                    if let Operand::Constant(ref constant) = func {
                        // log::debug!("call ty kind of func {:?}: {}", func, get_ty_kind_name(constant.literal.ty().kind()));

                        if let ty::FnDef(ref target_id, ..) = constant.literal.ty().kind() {
                            // if tcx.is_mir_available(*target_id) {
                            if utils::has_dbg(opts, "callee") {
                                log::debug!("mir available callee def id: {:?}", target_id);
                                // let target_body = tcx.optimized_mir(*target_id);
                                // log::debug!("target body: {:#?}", target_body);
                            }
                            let callee_def_id = *target_id;
                            let call_info = CallInfo::new(
                                callee_def_id,
                                bb,
                                func.clone(),
                                args.clone(),
                                destination.clone(),
                                terminator.source_info.span,
                            );
                            return Some((bb, call_info));
                            // }
                        }
                    }
                }
                return None;
            })
            .flatten()
            .collect::<HashMap<BasicBlockId, CallInfo>>();

        


        let basic_block_infos = body
            .basic_blocks()
            .iter_enumerated()
            .map(|(bb, bb_data)| {
                let successors =
                    get_basic_block_successors(&opts, &bb_data.terminator.as_ref().unwrap().kind);

                let assignment_infos: Vec<AssignmentInfo> = bb_data
                    .statements
                    .iter()
                    .map(|stat| match stat.kind {
                        StatementKind::Assign(ref assign) => {
                            if utils::has_dbg(opts, "assign") {
                                log::debug!("statement: {:?}", stat);
                                log::debug!("assign: {}", get_rvalue_name(&assign.1));
                                // handle left place ty
                                let left_ty = assign.0.ty(&body.local_decls, tcx).ty;
                                log::debug!("left type: {:?}", left_ty);
                                if left_ty.is_unsafe_ptr() || left_ty.is_ref() {
                                    log::debug!("left is unsafe ptr or ref");
                                }

                                // handle right place ty
                                let right_ty = assign.1.ty(&body.local_decls, tcx);
                                log::debug!("right type: {:?}", right_ty);
                                if right_ty.is_unsafe_ptr() || right_ty.is_ref() {
                                    log::debug!("right is unsafe ptr or ref");
                                }
                                log::debug!("");
                            }

                            if utils::has_dbg(opts, "span") {
                                log::debug!("statement: {:?}", stat);
                                let span = stat.source_info.span;
                                log::debug!(
                                    "statement span: lo: {:?} hi: {:?} ctxt: {:?}",
                                    span.lo(),
                                    span.hi(),
                                    span.ctxt()
                                );
                                log::debug!("span: {:?}", span);
                                log::debug!("");
                            }

                            get_assignment_infos(assign, stat.source_info.span)
                        }
                        _ => {
                            log::debug!("ignored non-assign statement: {:?}", stat);
                            vec![]
                        }
                    })
                    .flatten()
                    .collect();

                let bb_info = BasicBlockInfo::new(
                    bb,
                    bb_data.is_cleanup,
                    successors,
                    assignment_infos,
                    bb_data.terminator.clone().unwrap(),
                );
                (bb, bb_info)
            })
            .collect::<HashMap<_, _>>();


    

        Self {
            options: opts.clone(),
            def_id,
            local_infos,
            basic_block_infos,
            call_infos,
            has_ret,
        }
    }

}

fn get_basic_block_successors(
    opts: &AnalysisOptions,
    terminator_kind: &TerminatorKind,
) -> HashSet<BasicBlockId> {
    match terminator_kind {
        TerminatorKind::Goto { ref target } => [*target].into_iter().collect(),
        TerminatorKind::SwitchInt { ref targets, .. } => targets
            .iter()
            .map(|(_, target)| target)
            .chain([targets.otherwise()])
            .collect(),
        TerminatorKind::Resume
        | TerminatorKind::Return
        | TerminatorKind::Abort
        | TerminatorKind::GeneratorDrop
        | TerminatorKind::Unreachable => HashSet::new(),
        TerminatorKind::Drop {
            ref target,
            ref unwind,
            ..
        } => Some(*target).into_iter().chain(*unwind).collect(),
        TerminatorKind::DropAndReplace {
            ref target,
            ref unwind,
            ..
        } => Some(*target).into_iter().chain(*unwind).collect(),
        TerminatorKind::Call {
            ref func,
            ref args,
            ref destination,
            ref target,
            ref cleanup,
            ..
        } => {
            if utils::has_dbg(opts, "func") {
                log::debug!("func: {:?}", func);
                log::debug!("args: {:?}", args);
                log::debug!("destination: {:?}", destination);
                log::debug!("target: {:?}", target);
                log::debug!("cleanup: {:?}", cleanup);
                log::debug!("");
            }
            (*target).into_iter().chain(*cleanup).collect()
        }
        TerminatorKind::Assert {
            ref target,
            ref cleanup,
            ..
        } => Some(*target).into_iter().chain(*cleanup).collect(),
        TerminatorKind::Yield {
            ref resume,
            ref drop,
            ..
        } => Some(*resume).into_iter().chain(*drop).collect(),
        TerminatorKind::FalseEdge {
            ref real_target, ..
        } => Some(*real_target).into_iter().collect(),
        TerminatorKind::FalseUnwind {
            ref real_target, ..
        } => Some(*real_target).into_iter().collect(),
        TerminatorKind::InlineAsm {
            ref destination,
            ref cleanup,
            ..
        } => (*destination).into_iter().chain(*cleanup).collect(),
    }
}

fn get_place_projection_name(place: &Place<'_>) -> String {
    let mut name = String::new();
    for proj in place.projection.iter() {
        match proj {
            ProjectionElem::Deref => name.push_str("deref"),
            ProjectionElem::Field(field, _) => name.push_str(&format!("field{}", field.index())),
            ProjectionElem::Index(local) => name.push_str(&format!("index{}", local.index())),
            ProjectionElem::ConstantIndex { offset, .. } => {
                name.push_str(&format!("constant_index{}", offset))
            }
            ProjectionElem::Subslice { from, to, .. } => {
                name.push_str(&format!("subslice{}{}", from, to))
            }
            ProjectionElem::Downcast(_, variant) => {
                name.push_str(&format!("downcast{}", variant.index()))
            }
        }
    }
    name
}

fn get_rvalue_name<'tcx>(rvalue: &Rvalue<'tcx>) -> String {
    match rvalue {
        Rvalue::Use(ref op) => match op {
            Operand::Copy(ref rvalue) => format!(
                "use copy({:?}) proj: {}",
                rvalue,
                get_place_projection_name(rvalue)
            ),
            Operand::Move(ref rvalue) => format!(
                "use move({:?}) proj: {}",
                rvalue,
                get_place_projection_name(rvalue)
            ),
            Operand::Constant(ref constant) => format!("use constant {:?}", constant),
        },
        Rvalue::Repeat(ref op, ref _count) => format!("repeat({:?})", op),
        Rvalue::Ref(_, _, ref place) => format!("ref({:?})", place),
        Rvalue::Len(ref place) => format!("len({:?})", place),
        Rvalue::Cast(_, ref op, ref _ty) => match op {
            Operand::Copy(ref rvalue) => format!(
                "cast copy({:?}) proj: {}",
                rvalue,
                get_place_projection_name(rvalue)
            ),
            Operand::Move(ref rvalue) => format!(
                "cast move({:?}) proj: {}",
                rvalue,
                get_place_projection_name(rvalue)
            ),
            Operand::Constant(ref constant) => format!("cast constant {:?}", constant),
        },
        Rvalue::BinaryOp(..) => format!("binary_op"),
        Rvalue::CheckedBinaryOp(..) => {
            format!("checked_binary_op")
        }
        Rvalue::UnaryOp(_, ref op) => format!("unary_op({:?})", op),
        Rvalue::Discriminant(ref place) => format!("discriminant({:?})", place),
        Rvalue::NullaryOp(_, ref _ty) => "nullary_op".to_string(),
        Rvalue::Aggregate(_, ref ops) => format!("aggregate({:?})", ops),
        Rvalue::ThreadLocalRef(_) => format!("thread_local_ref"),
        Rvalue::AddressOf(_, _) => format!("address_of"),
        Rvalue::ShallowInitBox(_, _) => format!("shallow_init_box"),
    }
}

fn get_assignment_infos<'tcx>(
    assign: &Box<(Place<'tcx>, Rvalue<'tcx>)>,
    span: Span,
) -> Vec<AssignmentInfo<'tcx>> {
    match assign.1 {
        Rvalue::Use(ref op) => match op {
            // eg. _3: i32 = _2: i32,
            Operand::Copy(ref rvalue) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Copy,
                )]
            }
            // eg. _6 = move _4
            Operand::Move(ref rvalue) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Move,
                )]
            }
            // eg. _1 = const 1_i32, (_4.0: i32) = const 1_i32
            Operand::Constant(ref _constant) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Constant,
                    span,
                    OpKind::Copy,
                )]
            }
        },
        // eg. _2 = &_1
        Rvalue::Ref(_, _, ref rvalue) => {
            vec![AssignmentInfo::new(
                assign.0,
                RvalKind::Addressed(*rvalue),
                span,
                OpKind::Ref,
            )]
        }
        // eg. _14 = &raw const (*_15), let i = &x as *const i32
        Rvalue::AddressOf(_, ref rvalue) => {
            vec![AssignmentInfo::new(
                assign.0,
                RvalKind::Addressed(*rvalue),
                span,
                OpKind::AddressOf,
            )]
        }
        // eg. a = vec![1, 2]
        // _6 = alloc::alloc::exchange_malloc(..), _7 = ShallowInitBox(move _6)
        Rvalue::ShallowInitBox(ref op, _) => match op {
            Operand::Copy(ref rvalue) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Copy,
                )]
            }
            Operand::Move(ref rvalue) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Move,
                )]
            }
            Operand::Constant(ref _constant) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Constant,
                    span,
                    OpKind::Copy,
                )]
            }
            // log::debug!("unhandled assign: {:?} in span {:?}", assign, span);
        },
        Rvalue::Cast(_, ref op, _) => match op {
            Operand::Copy(ref rvalue) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Copy,
                )]
            }
            // eg. _16 = move _17 as *const i32, let j = x as *const i32
            Operand::Move(ref rvalue) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Move,
                )]
            }
            Operand::Constant(ref _constant) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Constant,
                    span,
                    OpKind::Copy,
                )]
            }
        },
        Rvalue::Aggregate(_, ref ops) => ops
            .iter()
            .map(|op| match op {
                Operand::Copy(ref rvalue) => Some(AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Copy,
                )),
                Operand::Move(ref rvalue) => Some(AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Move,
                )),
                Operand::Constant(ref _constant) => Some(AssignmentInfo::new(
                    assign.0,
                    RvalKind::Constant,
                    span,
                    OpKind::Copy,
                )),
            })
            .flatten()
            .collect(),
        Rvalue::Discriminant(ref rvalue) => {
            vec![AssignmentInfo::new(
                assign.0,
                RvalKind::Addressed(*rvalue),
                span,
                OpKind::Move,
            )]
        }
        // eg. _22 = [const 123_i32; 12]
        Rvalue::Repeat(ref op, _) => match op {
            Operand::Copy(ref rvalue) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Copy,
                )]
            }
            Operand::Move(ref rvalue) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Addressed(*rvalue),
                    span,
                    OpKind::Move,
                )]
            }
            Operand::Constant(ref _constant) => {
                vec![AssignmentInfo::new(
                    assign.0,
                    RvalKind::Constant,
                    span,
                    OpKind::Copy,
                )]
            }
        },
        Rvalue::ThreadLocalRef(_) => {
            log::debug!("unhandled assign: {:?} in span {:?}", assign, span);
            vec![]
        }
        Rvalue::Len(_) => {
            log::debug!("unhandled assign: {:?} in span {:?}", assign, span);
            vec![]
        }
        // eg. Gt(move _4, move _5)
        Rvalue::BinaryOp(_, _) => {
            log::debug!("unhandled assign: {:?} in span {:?}", assign, span);
            vec![]
        }
        // eg. CheckedAdd(_1, const 1i32)
        Rvalue::CheckedBinaryOp(_, _) => {
            log::debug!("unhandled assign: {:?} in span {:?}", assign, span);
            vec![]
        }
        Rvalue::NullaryOp(_, _) => {
            log::debug!("unhandled assign: {:?} in span {:?}", assign, span);
            vec![]
        }
        Rvalue::UnaryOp(_, _) => {
            log::debug!("unhandled assign: {:?} in span {:?}", assign, span);
            vec![]
        }
    }
}

fn get_ty_kind_name(ty: &TyKind<'_>) -> String {
    match ty {
        TyKind::Adt(adt, _) => format!("adt({:?})", adt),
        TyKind::Array(_, _) => format!("array"),
        TyKind::Bool => format!("bool"),
        TyKind::Char => format!("char"),
        TyKind::Float(_) => format!("float"),
        TyKind::FnDef(_, _) => format!("fn_def"),
        TyKind::FnPtr(_) => format!("fn_ptr"),
        TyKind::Foreign(_) => format!("foreign"),
        TyKind::Int(_) => format!("int"),
        TyKind::Never => format!("never"),
        TyKind::Param(_) => format!("param"),
        TyKind::Placeholder(_) => format!("placeholder"),
        TyKind::RawPtr(_) => format!("raw_ptr"),
        TyKind::Ref(_, _, _) => format!("ref"),
        TyKind::Slice(_) => format!("slice"),
        TyKind::Str => format!("str"),
        TyKind::Tuple(_) => format!("tuple"),
        TyKind::Uint(_) => format!("uint"),
        TyKind::Infer(_) => format!("infer"),
        TyKind::Error(_) => format!("error"),
        TyKind::Closure(_, _) => format!("closure"),
        TyKind::Generator(_, _, _) => format!("generator"),
        TyKind::GeneratorWitness(_) => format!("generator_witness"),
        TyKind::Dynamic(_, _) => format!("dynamic"),
        TyKind::Projection(_) => format!("projection"),
        TyKind::Opaque(_, _) => format!("opaque"),
        TyKind::Bound(_, _) => format!("bound"),
    }
}
