use std::collections::HashSet;

use rustc_hir::def_id::DefId;
use rustc_middle::mir::{Operand, Place, Terminator};

pub type LocalId = rustc_middle::mir::Local;
pub type BasicBlockId = rustc_middle::mir::BasicBlock;
pub type ProjectionId = u32;

pub mod analysis;
pub mod cfg;
pub mod check;
pub mod utils;
pub mod pfg;

#[derive(Clone, Debug)]
pub struct AnalysisOptions {
    pub debug_opts: Vec<String>,
    pub entries: Vec<String>,
    pub open_dbg: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Copy)]
pub struct DropObjectId {
    id: GlobalProjectionId,
}

impl From<GlobalProjectionId> for DropObjectId {
    fn from(id: GlobalProjectionId) -> Self {
        DropObjectId { id }
    }
}

impl Into<GlobalProjectionId> for DropObjectId {
    fn into(self) -> GlobalProjectionId {
        self.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallerContext {
    pub g_bb_ids: Vec<GlobalBasicBlockId>,
}

impl CallerContext {
    pub fn new(g_bb_ids: Vec<GlobalBasicBlockId>) -> Self {
        CallerContext { g_bb_ids }
    }

    pub fn is_same(&self, other: &CallerContext) -> bool {
        // log::debug!("left caller context: {:?}", self);
        // log::debug!("right caller context: {:?}", other);
        if self.g_bb_ids.len() != other.g_bb_ids.len() {
            return false;
        }

        for i in 0..self.g_bb_ids.len() {
            if self.g_bb_ids[i] != other.g_bb_ids[i] {
                return false;
            }
        }

        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CtxtSenCallId {
    pub def_id: DefId,
    pub caller_context: CallerContext,
}

impl CtxtSenCallId {
    pub fn new(def_id: DefId, caller_context: CallerContext) -> Self {
        CtxtSenCallId { def_id, caller_context }
    }
}

// #[derive(Clone, Debug, PartialEq, Eq, Hash, Copy)]
// pub struct CtxtSenGlobalProjectionId {
//     pub g_projection_id: GlobalProjectionId,
//     pub caller_context: CallerContext,
// }

// impl CtxtSenGlobalProjectionId {
//     pub fn new(g_projection_id: GlobalProjectionId, caller_context: CallerContext) -> Self {
//         CtxtSenGlobalProjectionId { g_projection_id, caller_context }
//     }
// }


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CtxtSenSpanInfo {
    pub def_id: DefId,
    pub basic_block_id: BasicBlockId,
    pub span: rustc_span::Span,
    pub caller_context: CallerContext,
}

impl CtxtSenSpanInfo {
    pub fn new(def_id: DefId, basic_block_id: BasicBlockId, span: rustc_span::Span, caller_context: CallerContext) -> Self {
        CtxtSenSpanInfo { def_id, basic_block_id, span, caller_context }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy)]
pub struct SpanInfo {
    pub def_id: DefId,
    pub basic_block_id: BasicBlockId,
    pub span: rustc_span::Span,
}

impl SpanInfo {
    pub fn new(def_id: DefId, basic_block_id: BasicBlockId, span: rustc_span::Span) -> Self {
        SpanInfo { def_id, basic_block_id, span }
    }
}

impl From<CtxtSenSpanInfo> for SpanInfo {
    fn from(info: CtxtSenSpanInfo) -> Self {
        SpanInfo::new(info.def_id, info.basic_block_id, info.span)
    }
}

impl Into<CtxtSenSpanInfo> for SpanInfo {
    fn into(self) -> CtxtSenSpanInfo {
        CtxtSenSpanInfo::new(self.def_id, self.basic_block_id, self.span, CallerContext::new(vec![]))
    }
}


#[derive(Clone, Debug, PartialEq, Eq, Hash, Copy)]
pub struct GlobalLocalId {
    pub def_id: DefId,
    pub local_id: LocalId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlobalProjectionId {
    pub g_local_id: GlobalLocalId,
    pub projection_id: ProjectionId,
}

impl GlobalProjectionId {
    pub fn new(g_local_id: GlobalLocalId, projection_id: ProjectionId) -> Self {
        GlobalProjectionId { g_local_id, projection_id }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Copy)]
pub struct GlobalBasicBlockId {
    pub def_id: DefId,
    pub bb_id: BasicBlockId,
}

#[derive(Debug)]
pub struct CallInfo<'tcx> {
    pub callee_def_id: DefId,
    pub caller_bb_id: BasicBlockId,
    pub func: Operand<'tcx>,
    pub args: Vec<Operand<'tcx>>,
    pub destination: Place<'tcx>,
    pub span: rustc_span::Span,
}

impl<'tcx> CallInfo<'tcx> {
    pub fn new(
        callee_def_id: DefId,
        caller_bb_id: BasicBlockId,
        func: Operand<'tcx>,
        args: Vec<Operand<'tcx>>,
        destination: Place<'tcx>,
        span: rustc_span::Span,
    ) -> Self {
        Self {
            callee_def_id,
            caller_bb_id,
            func,
            args,
            destination,
            span,
        }
    }
}

#[derive(Debug)]
pub struct FieldInfo {}

#[derive(Debug)]
pub struct LocalInfo<'tcx> {
    pub id: LocalId,
    pub need_drop: bool,
    pub ty: rustc_middle::ty::Ty<'tcx>,
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
    pub stat_span: rustc_span::Span,
    pub op: OpKind,
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
            stat_span: span,
            op,
        }
    }
}






impl GlobalLocalId {
    pub fn new(def_id: DefId, local_id: LocalId) -> Self {
        Self { def_id, local_id }
    }
}


impl GlobalBasicBlockId {
    pub fn new(def_id: DefId, bb_id: BasicBlockId) -> Self {
        Self { def_id, bb_id }
    }
}


impl<'tcx> LocalInfo<'tcx> {
    pub fn new(
        id: LocalId,
        need_drop: bool,
        ty: rustc_middle::ty::Ty<'tcx>,
        var_name: Option<String>,
        decl_span: rustc_span::Span,
    ) -> Self {
        Self {
            id,
            need_drop,
            ty,
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


