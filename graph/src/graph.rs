//! A mutable graph of nodes.  Compositions are represented as paths through this node graph.

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
};

use itertools::Itertools;
use monument_utils::Frontier;

use crate::{
    falseness::FalsenessTable,
    layout::{Layout, LinkIdx, NodeId, Segment},
    music::{Breakdown, MusicType},
};

/// The number of rows required to get from the start of a node to/from rounds.
type Distance = usize;

/// A 'prototype' node graph that is (relatively) inefficient to traverse but easy to modify.  This
/// is usually used to build and optimise the node graph before being converted into an efficient
/// graph representation for use in tree search.
#[derive(Debug, Clone)]
pub struct Graph {
    // NOTE: References between nodes don't have to be valid (i.e. they can point to a [`Node`]
    // that isn't actually in the graph - in this case they will be ignored or discarded during the
    // optimisation process).
    nodes: HashMap<NodeId, Node>,
    /// **Invariant**: If `start_nodes` points to a node, it **must** be a start node (i.e. not
    /// have any predecessors, and have `start_label` set)
    start_nodes: Vec<NodeId>,
    /// **Invariant**: If `start_nodes` points to a node, it **must** be a end node (i.e. not have
    /// any successors, and have `end_nodes` set)
    end_nodes: Vec<NodeId>,
}

/// A `Node` in a node [`Graph`].  This is an indivisible chunk of ringing which cannot be split up
/// by calls or splices.
#[derive(Debug, Clone)]
pub struct Node {
    /// If this `Node` is a 'start' (i.e. it can be the first node in a composition), then this is
    /// `Some(label)` where `label` should be appended to the front of the human-friendly
    /// composition string.
    start_label: Option<String>,
    /// If this `Node` is an 'end' (i.e. adding it will complete a composition), then this is
    /// `Some(label)` where `label` should be appended to the human-friendly composition string.
    end_label: Option<String>,

    /// The number of rows in this node
    length: usize,
    /// The music generated by this node in the composition
    music: Breakdown,
    /// The nodes which share rows with `self`, including `self` (because all nodes are false
    /// against themselves).
    false_nodes: Vec<NodeId>,

    /// A lower bound on the number of rows required to go from any rounds to the first row of
    /// `self`
    lb_distance_from_rounds: Distance,
    /// A lower bound on the number of rows required to go from the first row **after** `self` to
    /// rounds.
    lb_distance_to_rounds: Distance,

    successors: Vec<(LinkIdx, NodeId)>,
    predecessors: Vec<(LinkIdx, NodeId)>,
}

///////////////////////////
// GETTERS AND ITERATORS //
///////////////////////////

impl Graph {
    // Getters

    pub fn get_node<'graph>(&'graph self, id: &NodeId) -> Option<&'graph Node> {
        self.nodes.get(id)
    }

    pub fn get_node_mut<'graph>(&'graph mut self, id: &NodeId) -> Option<&'graph mut Node> {
        self.nodes.get_mut(id)
    }

    pub fn start_nodes(&self) -> &[NodeId] {
        &self.start_nodes
    }

    pub fn end_nodes(&self) -> &[NodeId] {
        &self.end_nodes
    }

    pub fn node_map(&self) -> &HashMap<NodeId, Node> {
        &self.nodes
    }

    // Iterators

    /// An [`Iterator`] over the [`NodeId`] of every [`Node`] in this `Graph`
    pub fn ids(&self) -> impl Iterator<Item = &NodeId> {
        self.nodes.keys()
    }

    /// An [`Iterator`] over every [`Node`] in this `Graph` (including its [`NodeId`])
    pub fn nodes(&self) -> impl Iterator<Item = (&NodeId, &Node)> {
        self.nodes.iter()
    }

    /// A mutable [`Iterator`] over the [`NodeId`] of every [`Node`] in this `Graph`
    pub fn nodes_mut(&mut self) -> impl Iterator<Item = (&NodeId, &mut Node)> {
        self.nodes.iter_mut()
    }
}

impl Node {
    // Getters

    pub fn successors(&self) -> &[(LinkIdx, NodeId)] {
        self.successors.as_slice()
    }

    pub fn successors_mut(&mut self) -> &mut Vec<(LinkIdx, NodeId)> {
        &mut self.successors
    }

    pub fn predecessors(&self) -> &[(LinkIdx, NodeId)] {
        self.predecessors.as_slice()
    }

    pub fn predecessors_mut(&mut self) -> &mut Vec<(LinkIdx, NodeId)> {
        &mut self.predecessors
    }

    pub fn false_nodes(&self) -> &[NodeId] {
        self.false_nodes.as_slice()
    }

    pub fn false_nodes_mut(&mut self) -> &mut Vec<NodeId> {
        &mut self.false_nodes
    }
}

//////////////////////////
// OPTIMISATION HELPERS //
//////////////////////////

impl Graph {
    /// Removes all nodes for whom `pred` returns `false`
    pub fn retain_nodes(&mut self, pred: impl FnMut(&NodeId, &mut Node) -> bool) {
        self.nodes.retain(pred);
    }

    /// Remove elements from [`Self::start_nodes`] for which a predicate returns `false`.
    pub fn retain_start_nodes(&mut self, pred: impl FnMut(&NodeId) -> bool) {
        self.start_nodes.retain(pred);
    }

    /// Remove elements from [`Self::end_nodes`] for which a predicate returns `false`.
    pub fn retain_end_nodes(&mut self, pred: impl FnMut(&NodeId) -> bool) {
        self.end_nodes.retain(pred);
    }

    /// For each start node in `self`, creates a copy of `self` with _only_ that start node.  This
    /// partitions the comps generated across these graphs, but allows for better optimisations
    /// because more is known about each `Graph`.
    pub fn split_by_start_node(&self) -> Vec<Graph> {
        self.start_nodes
            .iter()
            .cloned()
            .map(|start_id| {
                let mut new_self = self.clone();
                new_self.start_nodes = vec![start_id];
                new_self
            })
            .collect_vec()
    }
}

impl Node {
    /// A lower bound on the length of a composition which passes through this node.
    pub fn min_comp_length(&self) -> usize {
        self.lb_distance_from_rounds + self.length + self.lb_distance_to_rounds
    }
}

////////////////////////////
// CONVERSION FROM LAYOUT //
////////////////////////////

impl Graph {
    /// Generate a graph of all nodes which are reachable within a given length constraint.
    pub fn from_layout(layout: &Layout, music_types: &[MusicType], max_length: usize) -> Self {
        // The set of reachable nodes and whether or not they are a start node (each mapping to a
        // distance from rounds)
        let mut expanded_nodes: HashMap<NodeId, (Segment, Distance)> = HashMap::new();

        let mut end_nodes = Vec::new();

        // Unexplored nodes, ordered by distance from rounds (i.e. the minimum number of rows required
        // to reach them from rounds)
        let mut frontier: BinaryHeap<Reverse<Frontier<NodeId>>> = BinaryHeap::new();

        /* Run Dijkstra's algorithm using comp length as edge weights */

        // Populate the frontier with all the possible start nodes, each with distance 0
        let start_node_ids = layout
            .starts
            .iter()
            .map(|start| NodeId::new(start.course_head.to_owned(), start.row_idx, true))
            .collect_vec();
        frontier.extend(
            start_node_ids
                .iter()
                .cloned()
                .map(|node_id| Frontier {
                    item: node_id,
                    distance: 0,
                })
                .map(Reverse),
        );

        // Consume nodes from the frontier until the frontier is empty
        while let Some(Reverse(Frontier {
            item: node_id,
            distance,
        })) = frontier.pop()
        {
            // Don't expand nodes multiple times (Dijkstra's algorithm makes sure that the first time
            // it is expanded will be have the shortest distance)
            if expanded_nodes.get(&node_id).is_some() {
                continue;
            }
            // If the node hasn't been expanded yet, then add its reachable nodes to the frontier
            let segment = layout
                .get_segment(&node_id)
                .expect("Infinite segment found");

            // If the shortest composition including this node is longer the length limit, then don't
            // include it in the node graph
            let new_dist = distance + segment.length;
            if new_dist > max_length {
                continue;
            }
            if segment.is_end() {
                end_nodes.push(node_id.clone());
            }
            // Expand the node by adding its successors to the frontier
            for (_link_idx, id_after_link) in &segment.links {
                // Add the new node to the frontier
                frontier.push(Reverse(Frontier {
                    item: id_after_link.to_owned(),
                    distance: new_dist,
                }));
            }
            // Mark this node as expanded
            expanded_nodes.insert(node_id, (segment, distance));
        }

        // Once Dijkstra's finishes, `expanded_nodes` contains every node reachable from rounds
        // within the length of the composition.  However, we're still not done because we have to
        // build a graph over these IDs (which requires computing falseness, music, connections,
        // etc.).
        let mut nodes: HashMap<NodeId, Node> = expanded_nodes
            .iter()
            .map(|(node_id, (segment, distance))| {
                let music = Breakdown::from_rows(
                    segment.untransposed_rows(layout),
                    &node_id.course_head,
                    music_types,
                );

                let new_node = Node {
                    length: segment.length,
                    music,

                    start_label: segment.end_label.to_owned(),
                    end_label: segment.end_label.to_owned(),

                    lb_distance_from_rounds: *distance,
                    // Distances to rounds are computed later.  However, the distance is an lower
                    // bound, so we can set it to 0 without breaking any invariants.
                    lb_distance_to_rounds: 0,

                    successors: segment.links.to_owned(),
                    // These are populated in separate passes over the graph
                    false_nodes: Vec::new(),
                    predecessors: Vec::new(),
                };
                (node_id.clone(), new_node)
            })
            .collect();

        // We need to clone the `NodeId`s, because otherwise they would borrow from `nodes` whilst
        // the loop is modifying the contents (i.e. breaking reference aliasing)
        let node_ids_and_lengths = nodes
            .iter()
            .map(|(id, node)| (id.to_owned(), node.length))
            .collect_vec();

        // Compute falseness between the nodes
        let table = FalsenessTable::from_layout(layout, &node_ids_and_lengths);
        for (id, node) in nodes.iter_mut() {
            node.false_nodes = node_ids_and_lengths
                .iter()
                .filter(|(id2, length2)| table.are_false(id, node.length, id2, *length2))
                .map(|(id2, _)| id2.to_owned())
                .collect_vec();
        }

        // Add predecessor references
        for (id, _dist) in expanded_nodes {
            for (link_idx, succ_id) in nodes.get(&id).unwrap().successors.clone() {
                if let Some(node) = nodes.get_mut(&succ_id) {
                    node.predecessors.push((link_idx, id.clone()));
                }
            }
        }

        Self {
            nodes,
            start_nodes: start_node_ids,
            end_nodes,
        }
    }
}
