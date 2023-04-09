/*
 * @Author: Shuwen Chen
 * @Date: 2023-03-13 02:23:20
 * @Last Modified by: Shuwen Chen
 * @Last Modified time: 2023-04-09 20:33:31
 */
use crate::core::cfg::ControlFlowGraph;
use crate::core::OpKind;
use crate::core::{CallInfo, GlobalLocalId};
use rustc_hir::def_id::DefId;
use rustc_middle::mir::Operand;
use rustc_middle::mir::TerminatorKind;
use rustc_middle::mir::{Place, PlaceElem};
use std::collections::{HashMap, HashSet};

use crate::core::utils;
use super::pfg::{PointerFlowGraph, ProjectionNode};
use super::{GlobalBasicBlockId, GlobalProjectionId, LocalId, RvalKind, AnalysisOptions};
use std::collections::VecDeque;

pub fn alias_analysis(ctxt: AnalysisContext, entry: DefId) -> AnalysisContext {
    let mut ctxt = process_calls(ctxt, entry);

    while !ctxt.worklist.is_empty() {
        let pts = ctxt.worklist.pop_front().unwrap();
        let proj_node = ctxt.pfg.get_projection_node(pts.g_proj_id);
        let ptn = &proj_node.points_to;
        let delta = pts.points_to.difference(ptn).cloned().collect();
        ctxt = propagate(ctxt, pts.g_proj_id, delta);
    }

    log::debug!("reachable call: {:?}", ctxt.reachable_call);
    if utils::has_dbg(&ctxt.options, "pfg") {
        log::debug!("pfg: {:#?}", ctxt.pfg);
    }

    ctxt
}

pub struct AnalysisContext<'tcx> {
    pub options: AnalysisOptions,
    pub cfgs: HashMap<DefId, ControlFlowGraph<'tcx>>,
    pub pfg: PointerFlowGraph<'tcx>,
    pub reachable_call: HashSet<DefId>,
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

fn add_reachable(ctxt: AnalysisContext, def_id: DefId) -> AnalysisContext {
    if ctxt.reachable_call.contains(&def_id) {
        ctxt
    } else {
        let mut ctxt = ctxt;

        let cfg = ctxt.cfgs.get(&def_id).unwrap();

        ctxt.reachable_call.insert(def_id);

        for (bb_id, bb_info) in cfg.basic_block_infos.iter() {
            if let TerminatorKind::Drop { ref place, .. } = bb_info.terminator.kind {
                let g_proj_id = ctxt.pfg.add_or_update_node(
                    def_id,
                    place,
                    Some(GlobalBasicBlockId::new(def_id, *bb_id)),
                );

                if ctxt.pfg.get_projection_node(g_proj_id).drop_positions.len() == 1 {
                    let points_to = PointsTo::new(g_proj_id, Some(g_proj_id).into_iter().collect());
                    ctxt.worklist.push_back(points_to);
                }
            }

            for assignment in bb_info.assignment_infos.iter() {
                match assignment.op {
                    OpKind::Move | OpKind::Ref | OpKind::AddressOf => {
                        if let RvalKind::Addressed(place) = &assignment.rvalue {
                            let left_g_proj_id =
                                ctxt.pfg
                                    .add_or_update_node(def_id, &assignment.lvalue, None);
                            let right_g_proj_id = ctxt.pfg.add_or_update_node(def_id, place, None);
                            add_edge(
                                &mut ctxt.pfg,
                                &mut ctxt.worklist,
                                right_g_proj_id,
                                left_g_proj_id,
                            );
                        }
                    }
                    _ => {
                        log::debug!(
                            "ignored add edge at: {:?} with op, lval, rval: {:?} {:?} {:?}",
                            assignment.span,
                            assignment.op,
                            assignment.lvalue,
                            assignment.rvalue
                        );
                    }
                }
            }
        }
        ctxt
    }
}

fn add_edge(
    pfg: &mut PointerFlowGraph,
    worklist: &mut VecDeque<PointsTo>,
    from: GlobalProjectionId,
    to: GlobalProjectionId,
) {
    if pfg.has_edge(from, to) {
        return;
    } else {
        pfg.add_edge(from, to);
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
fn process_calls(ctxt: AnalysisContext, entry: DefId) -> AnalysisContext {
    let mut WL = VecDeque::new();
    WL.push_back(entry);

    let mut ctxt = ctxt;

    while !WL.is_empty() {
        let caller = WL.pop_front().unwrap();
        if !ctxt.reachable_call.contains(&caller) {
            ctxt = add_reachable(ctxt, caller);

            let caller_cfg = ctxt.cfgs.get(&caller).unwrap();
            for (bb_id, call_info) in caller_cfg.call_infos.iter() {
                // call is in the local crate
                if ctxt.cfgs.contains_key(&call_info.callee_def_id) {
                    // add callee to worklist
                    WL.push_back(call_info.callee_def_id);

                    // add edge from caller arg to callee parameter
                    for (i, arg) in call_info.args.iter().enumerate() {
                        let i = i + 1;
                        match arg {
                            Operand::Move(ref place) => {
                                let arg_id = ctxt.pfg.add_or_update_node(caller, place, None);
                                let param_id = ctxt.pfg.add_or_update_node(
                                    call_info.callee_def_id,
                                    &Place::from(LocalId::from_usize(i)),
                                    None,
                                );
                                add_edge(&mut ctxt.pfg, &mut ctxt.worklist, arg_id, param_id);
                            }
                            Operand::Constant(_) | Operand::Copy(_) => {
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
                            call_info.callee_def_id,
                            &Place::from(LocalId::from_usize(0)),
                            None,
                        );
                        let ret_id =
                            ctxt.pfg
                                .add_or_update_node(caller, &call_info.destination, None);
                        add_edge(&mut ctxt.pfg, &mut ctxt.worklist, callee_ret_id, ret_id);
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
                                .add_or_update_node(caller, &call_info.destination, None);
                        
                        // log::debug!("external crate caller ret id: {:?}", ret_id);
                        for arg in call_info.args.iter() {
                            match arg {
                                Operand::Move(ref place) => {
                                    let arg_id = ctxt.pfg.add_or_update_node(caller, place, None);

                                    add_edge(&mut ctxt.pfg, &mut ctxt.worklist, arg_id, ret_id);
                                }
                                Operand::Constant(_) | Operand::Copy(_) => {
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
        cfgs: ctxt.cfgs,
        pfg: ctxt.pfg,
        reachable_call: ctxt.reachable_call,
        worklist: ctxt.worklist,
    }
}


fn propagate(ctxt: AnalysisContext, g_proj_id: GlobalProjectionId, points_to: HashSet<GlobalProjectionId>) -> AnalysisContext {
    if !points_to.is_empty() {
        let mut ctxt = ctxt;
        let proj_node = ctxt.pfg.get_projection_node_mut(g_proj_id);
        proj_node.points_to.extend(points_to.clone());

        let suffix_proj_node = ctxt.pfg.get_projection_node(g_proj_id);

        for neighbor in suffix_proj_node.neighbors.iter() {

            let pfg_node = ctxt.pfg.get_node(neighbor.g_local_id);
            
            for (proj_id, proj_node) in pfg_node.projection_nodes.iter() {
                if suffix_proj_node.is_suffix_of(&proj_node.projection) {
                    let points_to = PointsTo::new(GlobalProjectionId::new(neighbor.g_local_id, *proj_id), points_to.clone());
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

