//! A plugin-able system of passes made over the composition [`Graph`], in order to modify or
//! optimise the graph.

use std::{fmt::Debug, ops::Not};

use crate::{layout::LinkIdx, Graph, Node, NodeId};

use self::Direction::{Backward, Forward};

pub type SinglePass = Box<dyn FnMut(&mut Graph)>;
/// A [`Pass`] which can be run both [`Forward`] and [`Backward`] over a [`Graph`].  For example,
/// computing distances to/from rounds (removing unreachable nodes).
pub type DirectionalPass = Box<dyn FnMut(&mut DirectionalView<'_>)>;

/// A pass which modifies a [`Graph`].  Passes are generally intended to perform optimisations -
/// they preserve the _semantic_ meaning of a [`Graph`] (i.e. the set of true compositions which it
/// generates), whilst modifying the [`Graph`] to make tree search faster.
pub enum Pass {
    /// Run a single non-directional pass
    Single(SinglePass),
    /// Apply a `DirectionalPass` but only in one [`Direction`]
    OneDirection(DirectionalPass, Direction),
    /// Apply a `DirectionalPass` twice, [`Forward`] first
    BothDirections(DirectionalPass),
    /// Apply a `DirectionalPass` twice, [`Backward`] first
    BothDirectionsRev(DirectionalPass),
}

impl Pass {
    /// Apply the effect of this [`Pass`] to a [`Graph`]
    pub fn run(&mut self, graph: &mut Graph) {
        match self {
            Pass::Single(pass) => pass(graph),
            Pass::OneDirection(pass, direction) => run_in_direction(pass, graph, *direction),
            Pass::BothDirections(pass) => {
                run_in_direction(pass, graph, Forward);
                run_in_direction(pass, graph, Backward);
            }
            Pass::BothDirectionsRev(pass) => {
                run_in_direction(pass, graph, Backward);
                run_in_direction(pass, graph, Forward);
            }
        }
    }
}

////////////////////////
// DIRECTIONAL PASSES //
////////////////////////

fn run_in_direction(pass: &mut DirectionalPass, graph: &mut Graph, direction: Direction) {
    pass(&mut DirectionalView::new(graph, direction))
}

/// A `Direction` in which a [`DirectionalPass`] can be run
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Copy)]
pub enum Direction {
    /// The direction that a composition will be rung (i.e. start -> end)
    Forward,
    /// The reverse of the direction that a composition will be rung (i.e. end -> start)
    Backward,
}

impl Not for Direction {
    type Output = Direction;

    fn not(self) -> Self::Output {
        match self {
            Forward => Backward,
            Backward => Forward,
        }
    }
}

/// The view of a [`Graph`] where 'start'/'end' and 'successors'/'predecessors' are defined in a
/// given [`Direction`].
#[derive(Debug)]
pub struct DirectionalView<'graph> {
    graph: &'graph mut Graph,
    direction: Direction,
}

impl<'graph> DirectionalView<'graph> {
    pub fn new(graph: &'graph mut Graph, direction: Direction) -> Self {
        Self { graph, direction }
    }

    pub fn start_nodes(&self) -> impl Iterator<Item = &NodeId> {
        match self.direction {
            Forward => self.graph.start_nodes().iter(),
            Backward => self.graph.end_nodes().iter(),
        }
    }

    pub fn end_nodes(&self) -> impl Iterator<Item = &NodeId> {
        match self.direction {
            Forward => self.graph.end_nodes().iter(),
            Backward => self.graph.start_nodes().iter(),
        }
    }

    pub fn get_node(&'graph self, id: &NodeId) -> Option<NodeView<'graph>> {
        self.graph.get_node(id).map(|node| NodeView {
            node,
            direction: self.direction,
        })
    }

    pub fn get_node_mut(&'graph mut self, id: &NodeId) -> Option<NodeViewMut<'graph>> {
        let direction = self.direction;
        self.graph
            .get_node_mut(id)
            .map(|node| NodeViewMut { node, direction })
    }
}

/// Immutable view of a [`Node`], facing in a given [`Direction`] (i.e. a [`Backward`] view will
/// swap the successors/predecessors).
#[derive(Clone, Copy)]
pub struct NodeView<'graph> {
    pub node: &'graph Node,
    pub direction: Direction,
}

impl<'graph> NodeView<'graph> {
    pub fn successors(&self) -> &[(LinkIdx, NodeId)] {
        match self.direction {
            Forward => self.node.successors(),
            Backward => self.node.predecessors(),
        }
    }

    pub fn predecessors(&self) -> &[(LinkIdx, NodeId)] {
        match self.direction {
            Forward => self.node.predecessors(),
            Backward => self.node.successors(),
        }
    }
}

/// Mutable view of a [`Node`], facing in a given [`Direction`] (i.e. a [`Backward`] view will
/// swap the successors/predecessors).
pub struct NodeViewMut<'graph> {
    pub node: &'graph mut Node,
    direction: Direction,
}

impl<'graph> NodeViewMut<'graph> {
    pub fn successors_mut(&mut self) -> &mut Vec<(LinkIdx, NodeId)> {
        match self.direction {
            Forward => self.node.successors_mut(),
            Backward => self.node.predecessors_mut(),
        }
    }

    pub fn predecessors_mut(&mut self) -> &mut Vec<(LinkIdx, NodeId)> {
        match self.direction {
            Forward => self.node.predecessors_mut(),
            Backward => self.node.successors_mut(),
        }
    }
}

////////////////////
// BUILTIN PASSES //
////////////////////

mod distance; // Compute node distances
mod strip_refs; // Strip references to non-existent nodes

// Re-exported as `crate::passes`
pub mod passes {
    use super::Pass;

    /// Creates a [`Pass`] which recomputes the distances to and from rounds for every node,
    /// removing any which can't reach rounds in either direction.
    pub fn strip_refs() -> Pass {
        Pass::Single(Box::new(super::strip_refs::strip_refs))
    }

    /// Creates a [`Pass`] which recomputes the distances to and from rounds for every node,
    /// removing any which can't reach rounds in either direction.
    pub fn compute_distances() -> Pass {
        Pass::BothDirections(Box::new(super::distance::compute_distances))
    }
}
