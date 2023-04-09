use super::{ProjectionId, GlobalProjectionId};

use crate::core::{GlobalLocalId};
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{PlaceElem, Place};
use std::collections::{HashMap, HashSet};

use super::{GlobalBasicBlockId, LocalId};


#[derive(Debug)]
pub struct ProjectionNode<'tcx> {
    pub id: ProjectionId,
    pub projection: Vec<PlaceElem<'tcx>>,
    pub points_to: HashSet<GlobalProjectionId>, 

    pub drop_positions: Vec<GlobalBasicBlockId>,

    pub neighbors: HashSet<GlobalProjectionId>,
}

impl<'tcx> ProjectionNode<'tcx> {
    pub fn new(id: ProjectionId, projection: Vec<PlaceElem<'tcx>>, drop_positions: Vec<GlobalBasicBlockId>) -> Self {
        ProjectionNode { id, projection, drop_positions, points_to: HashSet::new(), neighbors: HashSet::new() }
    }

    pub fn add_neighbor(&mut self, neighbor: GlobalProjectionId) {
        self.neighbors.insert(neighbor);
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

    pub fn add_drop_position(&mut self, drop_pos: GlobalBasicBlockId) {
        self.drop_positions.push(drop_pos);
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

    pub fn try_get_projection_id(&self, proj: &Vec<PlaceElem<'tcx>>) -> Option<ProjectionId> {
        for (id, node) in self.projection_nodes.iter() {
            if node.is_same_projection(proj) {
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

    pub fn add_projection(&mut self, proj: Vec<PlaceElem<'tcx>>) -> ProjectionId {
        let id = self.projection_nodes.len() as ProjectionId;
        let node = ProjectionNode::new(id, proj, vec![]);
        self.projection_nodes.insert(id, node);
        id
    }
}

#[derive(Debug)]
pub struct PointerFlowGraph<'tcx> {
    pub nodes: HashMap<GlobalLocalId, PfgNode<'tcx>>,
}

impl<'tcx> PointerFlowGraph<'tcx> {
    pub fn new() -> Self {
        PointerFlowGraph { nodes: HashMap::new() }
    }


    pub fn add_or_update_node(&mut self, def_id: DefId, place: &Place<'tcx>, drop_pos: Option<GlobalBasicBlockId>) -> GlobalProjectionId {
        let local_id: LocalId = place.local;
        let g_local_id = GlobalLocalId { def_id, local_id };
        if !self.nodes.contains_key(&g_local_id) {
            self.nodes.insert(g_local_id, PfgNode::new(g_local_id));
        }

        let node = self.nodes.get_mut(&g_local_id).unwrap();

        let projection = place.projection.to_vec();
        
        let proj_id = match node.try_get_projection_id(&projection) {
            Some(id) => id,
            None => node.add_projection(projection.clone()),
        };

        let proj_node = node.projection_nodes.get_mut(&proj_id).unwrap();
        if let Some(drop_pos) = drop_pos {
            proj_node.add_drop_position(drop_pos);
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

        from_node.neighbors.contains(&to)
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

    pub fn add_edge(&mut self, from: GlobalProjectionId, to: GlobalProjectionId) {
        let from_node = self.nodes.get_mut(&from.g_local_id).unwrap();
        let from_node = from_node.projection_nodes.get_mut(&from.projection_id).unwrap();
        from_node.add_neighbor(to);
    }

}