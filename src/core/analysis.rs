use crate::core::cfg::ControlFlowGraph;
use crate::core::OpKind;
use crate::core::{CallInfo};
use rustc_hir::def_id::DefId;
use rustc_middle::mir::Operand;
use rustc_middle::mir::TerminatorKind;
use rustc_middle::mir::{Place, PlaceElem};
use std::collections::{HashMap, HashSet};
use super::pfg::{PointerFlowGraph, DerefEdgeInfo};
use super::{
    AnalysisOptions, CallerContext, CtxtSenCallId, CtxtSenSpanInfo, DropObjectId,
    GlobalBasicBlockId, GlobalProjectionId, LocalId, RvalKind, cfg
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

    if utils::has_dbg(&ctxt.options, "RM") {
        log::debug!("reachable call: {:#?}", ctxt.cs_reachable_calls);
    }
    if utils::has_dbg(&ctxt.options, "pfg") {
        log::debug!("pfg: {:#?}", ctxt.pfg);
    }

    if utils::has_dbg(&ctxt.options, "pfgid") {
        ctxt.pfg.debug_proj(|proj_node| {
            log::debug!("projection: {:#?}", proj_node.projection);
            log::debug!("points to: {:#?}", proj_node.points_to);
            log::debug!("neighbors: {:#?}", proj_node.neighbors);
        }, |def_name| {
            def_name.ends_with("main") || def_name.ends_with("from")
        }, |_local_id| {
            // local_id.as_usize() == 1
            true
        }, |_global_proj_id| {
            true
        });
    }

    if utils::has_dbg(&ctxt.options, "pfg-paths") {
        ctxt.pfg.debug_paths(("main".to_owned(), LocalId::from_usize(1), 0));
    }

    ctxt
}

pub struct AnalysisContext<'tcx> {
    pub options: AnalysisOptions,
    pub tcx: rustc_middle::ty::TyCtxt<'tcx>,
    pub cfgs: HashMap<DefId, ControlFlowGraph<'tcx>>,
    pub called_infos: HashMap<DefId, HashSet<GlobalBasicBlockId>>,
    pub pfg: PointerFlowGraph<'tcx>,
    pub cs_reachable_calls: HashSet<CtxtSenCallId>,
    pub worklist: VecDeque<PointsTo>,
}

#[derive(Debug)]
pub struct PointsTo {
    pub g_proj_id: GlobalProjectionId,
    pub points_to: HashSet<DropObjectId>,
}

impl PointsTo {
    pub fn new(g_proj_id: GlobalProjectionId, points_to: HashSet<DropObjectId>) -> Self {
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
            // handle drop object
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
                    let drop_object_id: DropObjectId = g_proj_id.into();
                    let points_to =
                        PointsTo::new(g_proj_id, Some(drop_object_id).into_iter().collect());
                    ctxt.worklist.push_back(points_to);
                }
            }

            // handle assignment
            for assignment in bb_info.assignment_infos.iter() {
                if let RvalKind::Addressed(place) = &assignment.rvalue {
                    let need_add_edge = match assignment.op {
                        OpKind::Move | OpKind::Ref | OpKind::AddressOf => true,
                        OpKind::Copy => {
                            if is_ptr_copy(ctxt.tcx, call_id.def_id, place) {
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
                                CallerContext::new(vec![]),
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
    assert!(span_info.caller_context.g_bb_ids.is_empty());
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
    let mut call_work_list = VecDeque::new();
    call_work_list.push_back(entry);

    let mut ctxt = ctxt;

    while !call_work_list.is_empty() {
        let caller = call_work_list.pop_front().unwrap();

        if !ctxt.cs_reachable_calls.contains(&caller) {
            // add caller with context to reachable calls
            ctxt = add_reachable(ctxt, caller.clone());

            // TODO: ensure all callee cfgs are in cfgs
            let mut new_cfg_list = vec![];
            for (_, call_info) in ctxt.cfgs.get(&caller.def_id).unwrap().call_infos.iter() {
                // if callee is not in cfgs, we need to create it
                if !ctxt.cfgs.contains_key(&call_info.callee_def_id) {
                    let def_name = utils::parse_def_id(call_info.callee_def_id).join("::");
                    // we ignore the CHA of some common pointer related functions
                    if ARG_TO_RET_DEF_NAMES.iter().any(|&s| def_name.ends_with(s)) {
                        continue;
                    }
                    // we ignore the edge of some clone functions
                    if IGNORE_DEF_NAMES.iter().any(|&s| def_name.ends_with(s)) {
                        continue;
                    }
                    // we ignore all standard library functions
                    if def_name.starts_with("std::") {
                        continue;
                    }

                    if def_name.starts_with("core::") {
                        continue;
                    }

                    if def_name.starts_with("alloc::") {
                        continue;
                    }

                    if let Some(callee_cfg) =
                        cfg::try_create_cfg(&ctxt.options, ctxt.tcx, call_info.callee_def_id, false)
                    {
                        cfg::add_called_info(&ctxt.options, &mut ctxt.called_infos, &callee_cfg);
                        new_cfg_list.push(callee_cfg);
                    }
                }
            }
            ctxt.cfgs
                .extend(new_cfg_list.into_iter().map(|cfg| (cfg.def_id, cfg)));

            // add edges from caller args to callee params
            let caller_cfg = ctxt.cfgs.get(&caller.def_id).unwrap();
            for (bb_id, call_info) in caller_cfg.call_infos.iter() {
                if ctxt.cfgs.contains_key(&call_info.callee_def_id) {
                    // select target context
                    let target_context =
                        CallerContext::new(vec![GlobalBasicBlockId::new(caller.def_id, *bb_id)]);
                    let callee_id =
                        CtxtSenCallId::new(call_info.callee_def_id, target_context.clone());

                    // add callee to worklist
                    call_work_list.push_back(callee_id.clone());

                    add_args_and_ret_edge(
                        &mut ctxt.pfg,
                        &mut ctxt.worklist,
                        ctxt.tcx,
                        caller_cfg,
                        &caller,
                        call_info,
                        &target_context,
                    );
                } else {
                    log::debug!(
                        "external unsolved crate call: {:?} in caller {:?} bb {:?}",
                        call_info.callee_def_id,
                        caller.def_id,
                        bb_id
                    );

                    let def_name = utils::parse_def_id(call_info.callee_def_id).join("::");
                    // we ignore the edge of some clone functions
                    if IGNORE_DEF_NAMES.iter().any(|&s| def_name.ends_with(s)) {
                        continue;
                    }
                    
                    add_args_to_ret_edge(
                        &mut ctxt.pfg,
                        &mut ctxt.worklist,
                        ctxt.tcx,
                        caller_cfg,
                        &caller,
                        call_info,
                    );
                }
            }
        }
    }

    AnalysisContext {
        options: ctxt.options,
        tcx: ctxt.tcx,
        cfgs: ctxt.cfgs,
        called_infos: ctxt.called_infos,
        pfg: ctxt.pfg,
        cs_reachable_calls: ctxt.cs_reachable_calls,
        worklist: ctxt.worklist,
    }
}

fn propagate(
    ctxt: AnalysisContext,
    cur_g_proj_id: GlobalProjectionId,
    points_to: HashSet<DropObjectId>,
) -> AnalysisContext {
    if !points_to.is_empty() {
        let mut ctxt = ctxt;

        // union pts
        let cur_proj_node = ctxt.pfg.get_projection_node_mut(cur_g_proj_id);
        cur_proj_node.points_to.extend(points_to.clone());

        // add multi drop object
        if !cur_proj_node.cs_drop_spans.is_empty()
            && !ctxt.pfg.multi_drop_objects.contains(&cur_g_proj_id.into())
            && ctxt.pfg.get_projection_node(cur_g_proj_id).points_to.len() > 1
        {
            ctxt.pfg.multi_drop_objects.insert(cur_g_proj_id.into());
        }

        let cur_proj_node = ctxt.pfg.get_projection_node(cur_g_proj_id);
        // diffuse to sub level
        for (proj_id, proj_node) in ctxt
            .pfg
            .get_node(cur_g_proj_id.g_local_id)
            .projection_nodes
            .iter()
        {
            if proj_id != &cur_g_proj_id.projection_id
                && cur_proj_node
                    .caller_context
                    .is_same(&proj_node.caller_context)
                && cur_proj_node.is_prefix_of(&proj_node.projection)
            {
                let points_to_set = PointsTo::new(
                    GlobalProjectionId::new(cur_g_proj_id.g_local_id, *proj_id),
                    points_to.clone(),
                );
                ctxt.worklist.push_back(points_to_set);
            }
        }

        // diffuse to same level
        let mut local_wl = vec![];
        for (_, proj_node) in ctxt
            .pfg
            .get_node(cur_g_proj_id.g_local_id)
            .projection_nodes
            .iter()
        {
            if cur_proj_node
                .caller_context
                .is_same(&proj_node.caller_context)
                && proj_node.is_prefix_of(&cur_proj_node.projection)
            {
                let suffix_projections =
                    cur_proj_node.projection[proj_node.projection.len()..].to_vec();

                for (neighbor, _) in proj_node.neighbors.iter() {
                    let neighbor_proj_node = ctxt.pfg.get_projection_node(*neighbor);
                    let projections = neighbor_proj_node
                        .projection
                        .iter()
                        .chain(suffix_projections.iter())
                        .cloned()
                        .collect::<Vec<_>>();

                    let call_id = CtxtSenCallId::new(
                        neighbor.g_local_id.def_id,
                        neighbor_proj_node.caller_context.clone(),
                    );

                    local_wl.push((call_id, neighbor.g_local_id, projections, points_to.clone()));
                }
            }
        }

        for (call_id, g_local_id, projections, points_to) in local_wl {
            let virtual_node_id = ctxt.pfg.add_or_update_virtual_node(
                &call_id,
                g_local_id.local_id,
                &projections,
                None,
            );
            let points_to_set = PointsTo::new(virtual_node_id, points_to.clone());
            ctxt.worklist.push_back(points_to_set);
        }

        ctxt
    } else {
        ctxt
    }
}

fn is_ptr_copy<'tcx>(
    tcx: rustc_middle::ty::TyCtxt<'tcx>,
    def_id: DefId,
    place: &Place<'tcx>,
) -> bool {
    let ty = utils::get_ty_from_place(tcx, def_id, place);

    if ty.is_unsafe_ptr() || ty.is_ref() {
        true
    } else if place.projection.contains(&PlaceElem::Deref) {
        true
    } else {
        false
    }
}

fn add_args_and_ret_edge<'tcx>(
    pfg: &mut PointerFlowGraph<'tcx>,
    worklist: &mut VecDeque<PointsTo>,
    tcx: rustc_middle::ty::TyCtxt<'tcx>,
    caller_cfg: &ControlFlowGraph,
    caller: &CtxtSenCallId,
    call_info: &CallInfo<'tcx>,
    target_context: &CallerContext,
) {
    let span_info = CtxtSenSpanInfo::new(
        caller.def_id,
        call_info.caller_bb_id,
        call_info.span,
        CallerContext::new(vec![]),
    );
    let callee_id = CtxtSenCallId::new(call_info.callee_def_id, target_context.clone());
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
                    if is_ptr_copy(tcx, caller.def_id, arg_place) {
                        true
                    } else {
                        log::debug!(
                            "ignored copy edge at: {:?} with arg {:?} ",
                            call_info.span,
                            arg
                        );
                        false
                    }
                }
                Operand::Constant(_) => false,
            };
            if need_add_edge {
                let arg_id = pfg.add_or_update_node(&caller, arg_place, None);
                let param_id =
                    pfg.add_or_update_node(&callee_id, &Place::from(LocalId::from_usize(i)), None);
                
                // arguments are seen as deref
                pfg.deref_edges.insert(DerefEdgeInfo::new(arg_id, param_id, (true, false)));

                add_edge(pfg, worklist, arg_id, param_id, span_info.clone());
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
        let callee_ret_id =
            pfg.add_or_update_node(&callee_id, &Place::from(LocalId::from_usize(0)), None);
        let ret_id = pfg.add_or_update_node(&caller, &call_info.destination, None);
        add_edge(pfg, worklist, callee_ret_id, ret_id, span_info.clone());
    }
}

fn add_args_to_ret_edge<'tcx>(
    pfg: &mut PointerFlowGraph<'tcx>,
    worklist: &mut VecDeque<PointsTo>,
    tcx: rustc_middle::ty::TyCtxt<'tcx>,
    caller_cfg: &ControlFlowGraph,
    caller: &CtxtSenCallId,
    call_info: &CallInfo<'tcx>,
) {
    let span_info = CtxtSenSpanInfo::new(
        caller.def_id,
        call_info.caller_bb_id,
        call_info.span,
        CallerContext::new(vec![]),
    );
    let dest_local_info = caller_cfg
        .local_infos
        .get(&call_info.destination.local)
        .unwrap();
    // add edge from caller arg to caller ret
    if !dest_local_info.ty.is_unit() {
        let ret_id = pfg.add_or_update_node(&caller, &call_info.destination, None);

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
                        if is_ptr_copy(tcx, caller.def_id, arg_place) {
                            true
                        } else {
                            log::debug!(
                                "ignored copy edge at: {:?} with arg {:?} to ret {:?}",
                                call_info.span,
                                arg,
                                call_info.destination
                            );
                            false
                        }
                    }
                    Operand::Constant(_) => false,
                };
                if need_add_edge {
                    let arg_id = pfg.add_or_update_node(&caller, arg_place, None);

                    // arguments are seen as deref
                    pfg.deref_edges.insert(DerefEdgeInfo::new(arg_id, ret_id, (true, false)));
                    add_edge(pfg, worklist, arg_id, ret_id, span_info.clone());
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


lazy_static! {

// args directy to ret
static ref ARG_TO_RET_DEF_NAMES: Vec<&'static str> = vec![
    // Box
    "from_raw",
    "into_raw",
    
    // ptr
    "as_ref",
    "as_mut",
    "borrow",
    "borrow_mut",
    "deref",
    "deref_mut",
    "borrow",
    "borrow_mut",
    "as_ptr",
    "as_mut_ptr",
    "from_raw_parts",
    "as_mut_slice",
    "as_slice",
    "get",
    "get_mut",

    // vec

    // string
    "as_bytes",
    "as_mut_str",
    "as_mut_vec",
    "as_str",
    "as_bytes_mut",
];

// ignore defs, eg. clone()
static ref IGNORE_DEF_NAMES: Vec<&'static str> = vec![
    "clone",
];
}


