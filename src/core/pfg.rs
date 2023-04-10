use super::{CtxtSenCallId, CtxtSenSpanInfo};
use super::{ProjectionId, GlobalProjectionId};

use crate::core::{GlobalLocalId};
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{PlaceElem, Place};
use std::collections::{HashMap, HashSet};
use crate::core::CallerContext;

use super::{GlobalBasicBlockId, LocalId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectionNeighborInfo {
    pub neighbor_id: GlobalProjectionId,
    pub span_info: CtxtSenSpanInfo,    
}

impl ProjectionNeighborInfo {
    pub fn new(neighbor_id: GlobalProjectionId, span_info: CtxtSenSpanInfo) -> Self {
        ProjectionNeighborInfo { neighbor_id, span_info }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy)]
pub struct DerefEdgeInfo {
    pub from: GlobalProjectionId,
    pub to: GlobalProjectionId,
    pub is_deref: (bool, bool),
}

impl DerefEdgeInfo {
    pub fn new(from: GlobalProjectionId, to: GlobalProjectionId, is_deref: (bool, bool)) -> Self {
        DerefEdgeInfo { from, to, is_deref }
    }
}
#[derive(Debug)]
pub struct ProjectionNode<'tcx> {
    pub id: ProjectionId,
    pub caller_context: CallerContext,

    pub projection: Vec<PlaceElem<'tcx>>,
    pub points_to: HashSet<GlobalProjectionId>, 

    pub cs_drop_spans: Vec<CtxtSenSpanInfo>,

    pub neighbors: HashMap<GlobalProjectionId, ProjectionNeighborInfo>,
}

impl<'tcx> ProjectionNode<'tcx> {
    pub fn new(id: ProjectionId, projection: Vec<PlaceElem<'tcx>>, drop_spans: Vec<CtxtSenSpanInfo>, caller_context: CallerContext) -> Self {
        ProjectionNode { id, caller_context, projection, cs_drop_spans: drop_spans, points_to: HashSet::new(), neighbors: HashMap::new() }
    }

    pub fn add_neighbor(&mut self, neighbor: ProjectionNeighborInfo) {
        self.neighbors.insert(neighbor.neighbor_id, neighbor);
    }


    pub fn is_suffix_of(&self, proj: &Vec<PlaceElem<'tcx>>) -> bool {
        if self.projection.len() > proj.len() {
            return false;
        }

        for i in 0..self.projection.len() {
            if self.projection[i] != proj[i] {
                return false;
            }
        }

        true
    }

    pub fn is_same_projection(&self, proj: &Vec<PlaceElem<'tcx>>) -> bool {
        if self.projection.len() != proj.len() {
            return false;
        }

        for i in 0..self.projection.len() {
            if self.projection[i] != proj[i] {
                return false;
            }
        }

        true
    }

    pub fn add_drop_span(&mut self, drop_span: CtxtSenSpanInfo) {
        self.cs_drop_spans.push(drop_span);
    }

}

#[derive(Debug)]
pub struct PfgNode<'tcx> {
    pub gid: GlobalLocalId,
    pub projection_nodes: HashMap<ProjectionId, ProjectionNode<'tcx>>,
}

impl<'tcx> PfgNode<'tcx> {
    pub fn new(gid: GlobalLocalId) -> Self {
        PfgNode { gid, projection_nodes: HashMap::new() }
    }

    pub fn try_get_projection_node_mut(&mut self, proj: &Vec<PlaceElem<'tcx>>) -> Option<&mut ProjectionNode<'tcx>> {
        for (_, node) in self.projection_nodes.iter_mut() {
            if node.is_same_projection(proj) {
                return Some(node);
            }
        }

        None
    }

    pub fn try_get_projection_id(&self, proj: &Vec<PlaceElem<'tcx>>, caller_context: CallerContext) -> Option<ProjectionId> {
        for (id, node) in self.projection_nodes.iter() {
            if node.is_same_projection(proj) && node.caller_context == caller_context {
                return Some(*id);
            }
        }

        None
    }


    pub fn has_projection(&self, proj: &Vec<PlaceElem<'tcx>>) -> bool {
        for (_, node) in self.projection_nodes.iter() {
            if node.is_same_projection(proj) {
                return true;
            }
        }

        false
    }

    pub fn add_projection(&mut self, proj: Vec<PlaceElem<'tcx>>, caller_context: CallerContext) -> ProjectionId {
        let id = self.projection_nodes.len() as ProjectionId;
        let node = ProjectionNode::new(id, proj, vec![], caller_context);
        self.projection_nodes.insert(id, node);
        id
    }
}

#[derive(Debug)]
pub struct PointerFlowGraph<'tcx> {
    pub nodes: HashMap<GlobalLocalId, PfgNode<'tcx>>,
    pub deref_edges: HashSet<DerefEdgeInfo>,
}

impl<'tcx> PointerFlowGraph<'tcx> {
    pub fn new() -> Self {
        PointerFlowGraph { nodes: HashMap::new(), deref_edges: HashSet::new() }
    }

    pub fn get_neighbor_info(&self, from: GlobalProjectionId, to: GlobalProjectionId) -> &ProjectionNeighborInfo {
        let from_node = self.get_projection_node(from);
        from_node.neighbors.get(&to).unwrap()
    }


    pub fn add_or_update_node(&mut self, call_id: &CtxtSenCallId, place: &Place<'tcx>, drop_span: Option<CtxtSenSpanInfo>) -> GlobalProjectionId {
        // add or update pfg node
        let local_id: LocalId = place.local;
        let g_local_id = GlobalLocalId { def_id: call_id.def_id, local_id };
        if !self.nodes.contains_key(&g_local_id) {
            self.nodes.insert(g_local_id, PfgNode::new(g_local_id));
        }
        let node = self.nodes.get_mut(&g_local_id).unwrap();

        // add or update projection node
        let projection = place.projection.to_vec();
        
        let proj_id: ProjectionId = match node.try_get_projection_id(&projection, call_id.caller_context.clone()) {
            Some(id) => id,
            None => node.add_projection(projection.clone(), call_id.caller_context.clone()),
        };

        // add drop span in this context
        let proj_node = node.projection_nodes.get_mut(&proj_id).unwrap();
        if let Some(drop_span) = drop_span {
            proj_node.add_drop_span(drop_span);
        }

        GlobalProjectionId::new(g_local_id, proj_id)
    }

    pub fn get_projection_node(&self, g_proj_id: GlobalProjectionId) -> &ProjectionNode<'tcx> {
        let node = self.nodes.get(&g_proj_id.g_local_id).unwrap();
        node.projection_nodes.get(&g_proj_id.projection_id).unwrap()
    }

    pub fn get_node(&self, g_local_id: GlobalLocalId) -> &PfgNode<'tcx> {
        self.nodes.get(&g_local_id).unwrap()
    }

    pub fn get_node_mut(&mut self, g_local_id: GlobalLocalId) -> &mut PfgNode<'tcx> {
        self.nodes.get_mut(&g_local_id).unwrap()
    }

    pub fn get_projection_node_mut(&mut self, g_proj_id: GlobalProjectionId) -> &mut ProjectionNode<'tcx> {
        let node = self.nodes.get_mut(&g_proj_id.g_local_id).unwrap();
        node.projection_nodes.get_mut(&g_proj_id.projection_id).unwrap()
    }

    pub fn has_edge(&self, from: GlobalProjectionId, to: GlobalProjectionId) -> bool {
        if !self.has_global_projection(from) {
            return false;
        }

        if !self.has_global_local(to.g_local_id) {
            return false;
        }

        let from_node = self.nodes.get(&from.g_local_id).unwrap();
        let from_node = from_node.projection_nodes.get(&from.projection_id).unwrap();

        from_node.neighbors.contains_key(&to)
    }

    pub fn has_global_local(&self, g_local_id: GlobalLocalId) -> bool {
        self.nodes.contains_key(&g_local_id)
    }

    pub fn has_global_projection(&self, g_proj_id: GlobalProjectionId) -> bool {
        if !self.nodes.contains_key(&g_proj_id.g_local_id) {
            return false;
        }

        let node = self.nodes.get(&g_proj_id.g_local_id).unwrap();
        node.projection_nodes.contains_key(&g_proj_id.projection_id)
    }

    pub fn add_edge(&mut self, from: GlobalProjectionId, to: GlobalProjectionId, span_info: CtxtSenSpanInfo) {
        let from_node = self.get_projection_node_mut(from);
        from_node.add_neighbor(ProjectionNeighborInfo::new(to, span_info));

        // add deref edge 
        let from_node = self.get_projection_node(from);
        let to_node = self.get_projection_node(to);

        let from_is_deref = from_node.projection.contains(&PlaceElem::Deref);
        let to_is_deref = to_node.projection.contains(&PlaceElem::Deref);

        if from_is_deref || to_is_deref {
            self.deref_edges.insert(DerefEdgeInfo::new(from, to, (from_is_deref, to_is_deref)));
        }
    }

}