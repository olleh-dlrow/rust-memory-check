use crate::core::cfg::ControlFlowGraph;
use crate::core::OpKind;
use crate::core::{CallInfo, GlobalLocalId};
use rustc_hir::def_id::DefId;
use rustc_middle::mir::Operand;
use rustc_middle::mir::TerminatorKind;
use rustc_middle::mir::{Place, PlaceElem};
use std::collections::{HashMap, HashSet};

use super::pfg::{PointerFlowGraph, ProjectionNode};
use super::{
    AnalysisOptions, CallerContext, CtxtSenCallId, CtxtSenSpanInfo, GlobalBasicBlockId,
    GlobalProjectionId, LocalId, RvalKind,
};
use crate::core::utils;
use std::collections::VecDeque;

pub fn alias_analysis(ctxt: AnalysisContext, entry: CtxtSenCallId) -> AnalysisContext {
    let mut ctxt = process_calls(ctxt, entry);

    while !ctxt.worklist.is_empty() {
        let pts = ctxt.worklist.pop_front().unwrap();
        let proj_node = ctxt.pfg.get_projection_node(pts.g_proj_id);
        let ptn = &proj_node.points_to;
        let delta = pts.points_to.difference(ptn).cloned().collect();
        ctxt = propagate(ctxt, pts.g_proj_id, delta);
    }

    log::debug!("reachable call: {:?}", ctxt.cs_reachable_calls);
    if utils::has_dbg(&ctxt.options, "pfg") {
        log::debug!("pfg: {:#?}", ctxt.pfg);
    }

    ctxt
}

pub struct AnalysisContext<'tcx> {
    pub options: AnalysisOptions,
    pub tcx: rustc_middle::ty::TyCtxt<'tcx>,
    pub cfgs: HashMap<DefId, ControlFlowGraph<'tcx>>,
    pub pfg: PointerFlowGraph<'tcx>,
    pub cs_reachable_calls: HashSet<CtxtSenCallId>,
    pub worklist: VecDeque<PointsTo>,
}

#[derive(Debug)]
pub struct PointsTo {
    pub g_proj_id: GlobalProjectionId,
    pub points_to: HashSet<GlobalProjectionId>,
}

impl PointsTo {
    pub fn new(g_proj_id: GlobalProjectionId, points_to: HashSet<GlobalProjectionId>) -> Self {
        PointsTo {
            g_proj_id,
            points_to,
        }
    }
}

fn add_reachable(ctxt: AnalysisContext, call_id: CtxtSenCallId) -> AnalysisContext {
    if ctxt.cs_reachable_calls.contains(&call_id) {
        ctxt
    } else {
        let mut ctxt = ctxt;

        let cfg = ctxt.cfgs.get(&call_id.def_id).unwrap();

        ctxt.cs_reachable_calls.insert(call_id.clone());

        for (bb_id, bb_info) in cfg.basic_block_infos.iter() {
            if let TerminatorKind::Drop { ref place, .. } = bb_info.terminator.kind {
                let span = bb_info.terminator.source_info.span;

                let cs_drop_span = CtxtSenSpanInfo::new(
                    call_id.def_id,
                    *bb_id,
                    span,
                    call_id.caller_context.clone(),
                );

                let g_proj_id = ctxt
                    .pfg
                    .add_or_update_node(&call_id, place, Some(cs_drop_span));

                // we assume that all drops of this place **in this context** refer to the same object, so we only add <c: x, {c: oi}> to WL once
                if ctxt.pfg.get_projection_node(g_proj_id).cs_drop_spans.len() == 1 {
                    let points_to = PointsTo::new(g_proj_id, Some(g_proj_id).into_iter().collect());
                    ctxt.worklist.push_back(points_to);
                }
            }

            for assignment in bb_info.assignment_infos.iter() {
                if let RvalKind::Addressed(place) = &assignment.rvalue {
                    let need_add_edge = match assignment.op {
                        OpKind::Move | OpKind::Ref | OpKind::AddressOf => true,
                        OpKind::Copy => {
                            let body = ctxt.tcx.optimized_mir(call_id.def_id);
                            let right_place_ty = place.ty(&body.local_decls, ctxt.tcx);
                            if right_place_ty.variant_index.is_some() {
                                log::debug!(
                                    "unhandled PlaceTy::variant_index: {:?}",
                                    right_place_ty.variant_index
                                );
                            }

                            if right_place_ty.ty.is_unsafe_ptr() || right_place_ty.ty.is_ref() {
                                true
                            } else {
                                log::debug!("ignored copy edge at: {:?} with op, lval, rval: {:?} {:?} {:?}", assignment.stat_span, assignment.op, assignment.lvalue, assignment.rvalue);
                                false
                            }
                        }
                    };

                    if need_add_edge {
                        let left_g_proj_id =
                            ctxt.pfg
                                .add_or_update_node(&call_id, &assignment.lvalue, None);
                        let right_g_proj_id = ctxt.pfg.add_or_update_node(&call_id, place, None);
                        add_edge(
                            &mut ctxt.pfg,
                            &mut ctxt.worklist,
                            right_g_proj_id,
                            left_g_proj_id,
                            CtxtSenSpanInfo::new(
                                call_id.def_id,
                                *bb_id,
                                assignment.stat_span,
                                call_id.caller_context.clone(),
                            ),
                        );
                    }
                }
            }
        }
        ctxt
    }
}

/// we don't care about the caller context, so this should be vec![]
fn add_edge(
    pfg: &mut PointerFlowGraph,
    worklist: &mut VecDeque<PointsTo>,
    from: GlobalProjectionId,
    to: GlobalProjectionId,
    span_info: CtxtSenSpanInfo,
) {
    if pfg.has_edge(from, to) {
        return;
    } else {
        pfg.add_edge(from, to, span_info);
        let from_node = pfg.get_projection_node(from);

        if !from_node.points_to.is_empty() {
            let points_to = PointsTo::new(to, from_node.points_to.clone());
            worklist.push_back(points_to);
        }
    }
}

/// add all reachable calls
/// add drop and single assignment(eg. x = move y) to pfg, flag drop
/// add edge from caller arg to callee param
/// add edge from callee return to caller receiver
fn process_calls(ctxt: AnalysisContext, entry: CtxtSenCallId) -> AnalysisContext {
    let mut WL = VecDeque::new();
    WL.push_back(entry);

    let mut ctxt = ctxt;

    while !WL.is_empty() {
        let caller = WL.pop_front().unwrap();
        if !ctxt.cs_reachable_calls.contains(&caller) {
            ctxt = add_reachable(ctxt, caller.clone());

            let caller_cfg = ctxt.cfgs.get(&caller.def_id).unwrap();
            for (bb_id, call_info) in caller_cfg.call_infos.iter() {
                // select target context
                let target_context =
                    CallerContext::new(vec![GlobalBasicBlockId::new(caller.def_id, *bb_id)]);

                let span_info = CtxtSenSpanInfo::new(
                    caller.def_id,
                    *bb_id,
                    call_info.span,
                    CallerContext::new(vec![]),
                );
                // call is in the local crate
                if ctxt.cfgs.contains_key(&call_info.callee_def_id) {
                    let callee_id =
                        CtxtSenCallId::new(call_info.callee_def_id, target_context.clone());

                    // add callee to worklist
                    WL.push_back(callee_id.clone());

                    // add edge from caller arg to callee parameter
                    for (i, arg) in call_info.args.iter().enumerate() {
                        let i = i + 1;

                        let arg_place = match arg {
                            Operand::Move(ref place) => Some(place),
                            Operand::Copy(ref place) => Some(place),
                            Operand::Constant(_) => None,
                        };

                        if let Some(arg_place) = arg_place {
                            let need_add_edge = match arg {
                                Operand::Move(_) => true,
                                Operand::Copy(_) => {
                                    let arg_ty = utils::get_ty_from_place(
                                        ctxt.tcx,
                                        caller.def_id,
                                        arg_place,
                                    );

                                    if arg_ty.is_unsafe_ptr() || arg_ty.is_ref() {
                                        true
                                    } else {
                                        log::debug!("ignored copy edge at: {:?} with arg {:?}", call_info.span, arg);
                                        false
                                    }
                                }
                                Operand::Constant(_) => false,
                            };
                            if need_add_edge {
                                let arg_id = ctxt.pfg.add_or_update_node(&caller, arg_place, None);
                                let param_id = ctxt.pfg.add_or_update_node(
                                    &callee_id,
                                    &Place::from(LocalId::from_usize(i)),
                                    None,
                                );
                                add_edge(
                                    &mut ctxt.pfg,
                                    &mut ctxt.worklist,
                                    arg_id,
                                    param_id,
                                    span_info.clone(),
                                );
                            } else {
                                log::debug!(
                                    "ignored arg at caller {:?} callee: {:?}: {:?}",
                                    caller,
                                    call_info.callee_def_id,
                                    arg
                                );
                            }
                        }
                    }

                    // add edge from callee ret to caller ret
                    let dest_local_info = caller_cfg
                        .local_infos
                        .get(&call_info.destination.local)
                        .unwrap();
                    if !dest_local_info.ty.is_unit() {
                        let callee_ret_id = ctxt.pfg.add_or_update_node(
                            &callee_id,
                            &Place::from(LocalId::from_usize(0)),
                            None,
                        );
                        let ret_id =
                            ctxt.pfg
                                .add_or_update_node(&caller, &call_info.destination, None);
                        add_edge(
                            &mut ctxt.pfg,
                            &mut ctxt.worklist,
                            callee_ret_id,
                            ret_id,
                            span_info.clone(),
                        );
                    }
                } else {
                    // call is in an external crate
                    // log::debug!("external crate call: {:?}", call_info.callee_def_id);
                    let dest_local_info = caller_cfg
                        .local_infos
                        .get(&call_info.destination.local)
                        .unwrap();
                    // add edge from caller arg to caller ret
                    if !dest_local_info.ty.is_unit() {
                        let ret_id =
                            ctxt.pfg
                                .add_or_update_node(&caller, &call_info.destination, None);

                        // log::debug!("external crate caller ret id: {:?}", ret_id);
                        for arg in call_info.args.iter() {
                            let arg_place = match arg {
                                Operand::Move(ref place) => Some(place),
                                Operand::Copy(ref place) => Some(place),
                                Operand::Constant(_) => None,
                            };

                            if let Some(arg_place) = arg_place {
                                let need_add_edge = match arg {
                                    Operand::Move(_) => true,
                                    Operand::Copy(_) => {
                                        let arg_ty = utils::get_ty_from_place(
                                            ctxt.tcx,
                                            caller.def_id,
                                            arg_place,
                                        );

                                        if arg_ty.is_unsafe_ptr() || arg_ty.is_ref() {
                                            true
                                        } else {
                                            log::debug!("ignored copy edge at: {:?} with arg {:?} to ret {:?}", call_info.span, arg, call_info.destination);
                                            false
                                        }
                                    }
                                    Operand::Constant(_) => false,
                                };
                                if need_add_edge {
                                    let arg_id =
                                        ctxt.pfg.add_or_update_node(&caller, arg_place, None);

                                    add_edge(
                                        &mut ctxt.pfg,
                                        &mut ctxt.worklist,
                                        arg_id,
                                        ret_id,
                                        span_info.clone(),
                                    );
                                } else {
                                    log::debug!(
                                        "ignored arg at caller {:?} callee: {:?}: {:?}",
                                        caller,
                                        call_info.callee_def_id,
                                        arg
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    AnalysisContext {
        options: ctxt.options,
        tcx: ctxt.tcx,
        cfgs: ctxt.cfgs,
        pfg: ctxt.pfg,
        cs_reachable_calls: ctxt.cs_reachable_calls,
        worklist: ctxt.worklist,
    }
}

fn propagate(
    ctxt: AnalysisContext,
    g_proj_id: GlobalProjectionId,
    points_to: HashSet<GlobalProjectionId>,
) -> AnalysisContext {
    if !points_to.is_empty() {
        let mut ctxt = ctxt;

        // union pts
        let proj_node = ctxt.pfg.get_projection_node_mut(g_proj_id);
        proj_node.points_to.extend(points_to.clone());

        for (neighbor, _) in ctxt.pfg.get_projection_node(g_proj_id).neighbors.iter() {
            let pfg_node = ctxt.pfg.get_node(neighbor.g_local_id);
            let suffix_proj_node = ctxt.pfg.get_projection_node(*neighbor); // c': s

            // drop will propagate to all projections of the same caller context
            for (proj_id, proj_node) in pfg_node.projection_nodes.iter() {
                if suffix_proj_node
                    .caller_context
                    .is_same(&proj_node.caller_context)
                    && suffix_proj_node.is_suffix_of(&proj_node.projection)
                {
                    let points_to = PointsTo::new(
                        GlobalProjectionId::new(neighbor.g_local_id, *proj_id),
                        points_to.clone(),
                    );
                    ctxt.worklist.push_back(points_to);
                }
            }

            let points_to = PointsTo::new(*neighbor, points_to.clone());
            ctxt.worklist.push_back(points_to);
        }
        ctxt
    } else {
        ctxt
    }
}
