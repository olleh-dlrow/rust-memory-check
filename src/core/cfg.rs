/*
 * @Author: Shuwen Chen
 * @Date: 2023-03-13 02:23:14
 * @Last Modified by: Shuwen Chen
 * @Last Modified time: 2023-03-14 02:54:11
 */

use rustc_middle::mir::ProjectionElem;
use rustc_middle::mir::terminator::Terminator;
use rustc_middle::mir::terminator::TerminatorKind;
use rustc_middle::mir::Operand;
use rustc_middle::mir::Place;
use rustc_middle::mir::Rvalue;
use rustc_middle::mir::StatementKind;
use rustc_span::Span;
use std::collections::{HashMap, HashSet};

use crate::core::utils;

use crate::core::utils::AnalysisOptions;

type LocalId = rustc_middle::mir::Local;
type BasicBlockId = rustc_middle::mir::BasicBlock;

#[derive(Debug)]
pub struct FieldInfo {}

#[derive(Debug)]
pub struct LocalInfo<'tcx> {
    pub id: LocalId,
    pub need_drop: bool,
    pub ty: rustc_middle::ty::Ty<'tcx>,
    pub alias: Vec<LocalId>,
    pub field_info: Option<FieldInfo>,
    pub var_name: Option<String>,
    pub decl_span: rustc_span::Span,
}

#[derive(Debug)]
pub struct BasicBlockInfo<'tcx> {
    pub id: BasicBlockId,
    pub is_cleanup: bool,
    pub successors: HashSet<BasicBlockId>,
    pub assignment_infos: Vec<AssignmentInfo<'tcx>>,
    pub terminator: Terminator<'tcx>,
}

#[derive(Debug)]
pub enum OpKind {
    Copy,
    Move,
    Ref,
    AddressOf,
}

#[derive(Debug)]
pub enum RvalKind<'tcx> {
    Constant,
    Addressed(Place<'tcx>),
}

#[derive(Debug)]
pub struct AssignmentInfo<'tcx> {
    pub lvalue: Place<'tcx>,
    pub rvalue: RvalKind<'tcx>,
    pub span: rustc_span::Span,
    pub op: OpKind,
}

#[derive(Debug)]
pub struct ControlFlowGraph<'tcx> {
    pub options: AnalysisOptions,
    pub def_id: rustc_hir::def_id::DefId,
    pub local_infos: HashMap<LocalId, LocalInfo<'tcx>>,
    pub basic_block_infos: HashMap<BasicBlockId, BasicBlockInfo<'tcx>>,
}

impl<'tcx> ControlFlowGraph<'tcx> {
    pub fn new(opts: &AnalysisOptions, tcx: rustc_middle::ty::TyCtxt<'tcx>, def_id: rustc_hir::def_id::DefId) -> Self {
        let body: &rustc_middle::mir::Body = tcx.optimized_mir(def_id);
        if utils::has_dbg(opts, "body") {
            log::debug!("body of def id {:?}: \n{:#?}", def_id, body);
        }

        let local_infos = body
            .local_decls
            .iter_enumerated()
            .map(|(local, local_decl)| {
                // TODO: handle VarDebugInfo
                let local_info = LocalInfo::new(
                    local,
                    local_decl.ty.needs_drop(tcx, tcx.param_env(def_id)),
                    local_decl.ty,
                    None,
                    None,
                    local_decl.source_info.span,
                );
                (local, local_info)
            })
            .collect::<HashMap<_, _>>();

        let basic_block_infos = body
            .basic_blocks()
            .iter_enumerated()
            .map(|(bb, bb_data)| {
                let successors =
                    get_basic_block_successors(&bb_data.terminator.as_ref().unwrap().kind);

                let assignment_infos: Vec<AssignmentInfo> = bb_data
                    .statements
                    .iter()
                    .map(|stat| {
                        match stat.kind {
                            StatementKind::Assign(ref assign) => {
                                if utils::has_dbg(opts, "assign") {
                                    log::debug!("statement: {:?}", stat);
                                    log::debug!("assign: {}", get_rvalue_name(&assign.1));
                                    log::debug!("");
                                }
                                get_assignment_infos(assign, stat.source_info.span)
                            }
                            _ => {
                                log::debug!("ignored non-assign statement: {:?}", stat);
                                vec![]
                            }
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
        }
    }
}

impl<'tcx> LocalInfo<'tcx> {
    pub fn new(
        id: LocalId,
        need_drop: bool,
        ty: rustc_middle::ty::Ty<'tcx>,
        field_info: Option<FieldInfo>,
        var_name: Option<String>,
        decl_span: rustc_span::Span,
    ) -> Self {
        Self {
            id,
            need_drop,
            ty,
            alias: Vec::default(),
            field_info,
            var_name,
            decl_span,
        }
    }
}

impl<'tcx> BasicBlockInfo<'tcx> {
    pub fn new(
        id: rustc_middle::mir::BasicBlock,
        is_cleanup: bool,
        successors: std::collections::HashSet<rustc_middle::mir::BasicBlock>,
        assignment_infos: Vec<AssignmentInfo<'tcx>>,
        terminator: Terminator<'tcx>,
    ) -> Self {
        Self {
            id,
            is_cleanup,
            successors,
            assignment_infos,
            terminator,
        }
    }
}

fn get_basic_block_successors(terminator_kind: &TerminatorKind) -> HashSet<BasicBlockId> {
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
            ref target,
            ref cleanup,
            ..
        } => (*target).into_iter().chain(*cleanup).collect(),
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

impl<'tcx> AssignmentInfo<'tcx> {
    pub fn new(
        lvalue: Place<'tcx>,
        rvalue: RvalKind<'tcx>,
        span: rustc_span::Span,
        op: OpKind,
    ) -> Self {
        Self {
            lvalue,
            rvalue,
            span,
            op,
        }
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
            Operand::Copy(ref rvalue) => format!("use copy({:?}) proj: {}", rvalue, get_place_projection_name(rvalue)),
            Operand::Move(ref rvalue) => format!("use move({:?}) proj: {}", rvalue, get_place_projection_name(rvalue)), 
            Operand::Constant(ref constant) => format!("use constant {:?}", constant),
        },
        Rvalue::Repeat(ref op, ref _count) => format!("repeat({:?})", op),
        Rvalue::Ref(_, _, ref place) => format!("ref({:?})", place),
        Rvalue::Len(ref place) => format!("len({:?})", place),
        Rvalue::Cast(_, ref op, ref _ty) => match op {
            Operand::Copy(ref rvalue) => format!("cast copy({:?}) proj: {}", rvalue, get_place_projection_name(rvalue)),             
            Operand::Move(ref rvalue) => format!("cast move({:?}) proj: {}", rvalue, get_place_projection_name(rvalue)),            
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
            // _5 = (_4.0: i32)
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
        // TODO: handle assignment ShallowInitBox
        Rvalue::ShallowInitBox(ref _op, _) => {
            log::debug!("unhandled assign: {:?} in span {:?}", assign, span);
            vec![]
        }
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
