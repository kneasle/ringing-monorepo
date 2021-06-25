use std::{cell::Cell, cmp::Ordering, ops::Range, sync::Arc};

use shortlist::Shortlist;

use crate::{
    graph::{Graph, Node, NodeId},
    Engine,
};

/// The mutable data required to generate a composition.  Each worker thread will have their own
/// `EngineWorker` struct (but all share the same [`Engine`]).
#[derive(Debug)]
pub(crate) struct EngineWorker {
    thread_id: usize,
    len_range: Range<usize>,
    /// The in-memory [`Graph`] of [`Node`]s
    graph: Graph<NodePayload>,
    /// A `Shortlist` of discovered compositions
    shortlist: Shortlist<Comp>,
    /// Which links where chosen after each node.  These are indices into the `links` field on each
    /// `Segment`.  Therefore, this is cheap to track during the composing loop and reconstruction
    /// a human-friendly representation just requires a traversal of the node graph
    comp_prefix: Vec<usize>,
}

impl EngineWorker {
    /// Creates a new `EngineWorker`
    pub fn from_engine(engine: Arc<Engine>, thread_id: usize) -> Self {
        EngineWorker {
            thread_id,
            len_range: engine.len_range.clone(),
            graph: Graph::from_engine(&engine, |node_id| NodePayload::new(node_id, &engine)),
            shortlist: Shortlist::new(engine.config.num_comps),
            comp_prefix: Vec::new(),
        }
    }

    /// Run graph search over the node graph to find compositions.
    pub fn compose(&mut self) {
        for n in self.graph.start_nodes.clone() {
            self.expand_node(unsafe { n.as_ref() }.unwrap(), 0);
        }
    }

    fn expand_node(&mut self, node: &Node<NodePayload>, length: usize) {
        let payload = node.payload();

        /* ===== POTENTIALLY PRUNE THE NODE ===== */

        // If the node is false against anything in the comp prefix, then prune
        if payload.falseness_count.get() != 0 {
            return;
        }

        let len_after_this_node = length + payload.length;

        // If the node would make the comp too long then prune
        if len_after_this_node >= self.len_range.end {
            return;
        }

        /* ===== ADD THE NODE TO THE COMPOSITION ===== */

        // Sanity check that adding this node wouldn't make the comp false
        debug_assert_eq!(payload.falseness_count.get(), 0);

        // This node is false against itself, so by adding it to the composition we have to also
        // mark it as false (by adding one to its node count)
        payload
            .falseness_count
            .set(payload.falseness_count.get() + 1);
        // Since we are committing to ringing this node, we should register its falseness against
        // other nodes
        for &n in node.false_nodes() {
            let false_count_cell = &n.payload().falseness_count;
            false_count_cell.set(false_count_cell.get() + 1);
        }

        /* ===== EXPAND CHILD NODES ===== */

        for (i, &succ) in node.successors().iter().enumerate() {
            self.comp_prefix.push(i);
            // Add the new link to the composition
            self.expand_node(succ, len_after_this_node);
            self.comp_prefix.pop();
        }

        /* ===== REMOVE THIS NODE FROM THE COMPOSITION ===== */

        // Decrement own falseness count (because this node is no longer false against itself)
        payload
            .falseness_count
            .set(payload.falseness_count.get() - 1);
        // Decrement the falseness counters on all the nodes false against this one
        for &n in node.false_nodes() {
            let false_count_cell = &n.payload().falseness_count;
            false_count_cell.set(false_count_cell.get() - 1);
        }

        // Sanity check that the falseness count has been reset to 0
        debug_assert_eq!(payload.falseness_count.get(), 0);
    }
}

/// The payload stored in each [`Node`] in the [`Graph`]
#[derive(Debug, Clone)]
pub struct NodePayload {
    /// The number of rows in this node
    length: usize,
    /// The music score generated by this node
    score: f32,
    /// The number of nodes which are false against this one and are already in the composition.
    /// Nodes will only be expanded if this is 0
    falseness_count: Cell<u32>,
}

impl NodePayload {
    fn new(node_id: &NodeId, engine: &Engine) -> Self {
        let seg_table = engine.get_seg_table(node_id.central_id(&engine.segment_table).seg_id);
        Self {
            length: seg_table.length,
            score: match node_id {
                NodeId::Central(central_id) => seg_table.music.evaluate(&central_id.row),
                NodeId::End(_) | NodeId::Start(_) => todo!(),
            },
            falseness_count: Cell::new(0),
        }
    }
}

/// A completed composition
#[derive(Debug, Clone)]
pub struct Comp {
    pub starting_node: usize,
    pub calls: Vec<usize>,
    pub length: usize,
    pub score: f32,
}

impl Comp {
    #[allow(dead_code)]
    fn to_string(&self, engine: &Engine) -> String {
        let mut string = format!("(len: {}, score: {}) ", self.length, self.score);

        let mut current_seg_id = engine.segment_table.start_nodes[self.starting_node]
            .super_node
            .seg_id;
        for &link_ind in &self.calls {
            let link = &engine.get_seg_table(current_seg_id).links[link_ind];
            string.push_str(&link.display_name);
            current_seg_id = link.end_segment;
        }

        string
    }
}

impl PartialOrd for Comp {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.score.partial_cmp(&other.score)
    }
}

impl Ord for Comp {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

impl PartialEq for Comp {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score
    }
}

impl Eq for Comp {}
