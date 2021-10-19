use std::{cmp::Ordering, fmt::Debug, rc::Rc};

use bit_vec::BitVec;
use frontier::Frontier;
use monument_graph::{self as m_gr, layout::LinkIdx, music::Score, Data};

pub mod frontier;
mod graph;

use graph::{Graph, NodeIdx};

pub fn search<Ftr: Frontier<CompPrefix> + Debug>(graph: &m_gr::Graph, data: &Data) -> Vec<Comp> {
    // Lower the graph into a graph that's immutable but faster to traverse
    let lowered_graph = crate::graph::Graph::from(graph);
    search_lowered::<Ftr>(&lowered_graph, data)
}

fn search_lowered<Ftr: Frontier<CompPrefix> + Debug>(graph: &Graph, data: &Data) -> Vec<Comp> {
    // Initialise the frontier to just the start nodes
    let mut frontier = Ftr::default();
    for (i, (node_idx, _label)) in graph.starts.iter().enumerate() {
        let node = &graph.nodes[*node_idx];
        frontier.push(CompPrefix::new(
            CompPath::Start(i),
            *node_idx,
            node.falseness.clone(),
            node.score,
            node.length,
        ));
    }

    let mut comps = Vec::<Comp>::new();

    // Repeatedly choose the best prefix and expand it (i.e. add each way of extending it to the
    // frontier).
    let mut iter_count = 0;
    while let Some(prefix) = frontier.pop() {
        let CompPrefix {
            path,
            node_idx,
            unreachable_nodes,

            score,
            length,
            avg_score,
        } = prefix;
        let node = &graph.nodes[node_idx];

        // Check if the comp has come round
        if let Some(end_label) = &node.end_label {
            if data.len_range.contains(&length) {
                let (start_idx, links) = path.flatten(graph, data);
                let comp = Comp {
                    start_idx,
                    links,
                    end_label: end_label.to_owned(),

                    length,
                    score,
                    avg_score,
                };
                println!(
                    "q: {:>8}, len: {}, score: {:>6.2}, avg: {}, str: {}",
                    frontier.len(),
                    length,
                    score,
                    avg_score,
                    comp.display_string(graph, data, end_label)
                );
                comps.push(comp);

                if comps.len() == data.num_comps {
                    break; // Stop the search once we've got enough comps
                }
            }
            continue; // Don't expand comps after they've come round
        }

        // Expand this node
        let path = Rc::new(path);
        for &(link_idx, succ_idx) in &node.succs {
            let succ_node = &graph.nodes[succ_idx];

            let length = length + succ_node.length;
            let score = score + succ_node.score;

            if length >= data.len_range.end {
                continue; // Node would make comp too long
            }

            if unreachable_nodes.get(succ_idx.index()).unwrap() {
                continue; // Node is unreachable (i.e. false against something already in the comp)
            }

            // Compute which nodes are unreachable after this node has been added
            let mut new_unreachable_nodes = unreachable_nodes.clone();
            new_unreachable_nodes.or(&succ_node.falseness);

            frontier.push(CompPrefix::new(
                CompPath::Cons(path.clone(), link_idx),
                succ_idx,
                new_unreachable_nodes,
                score,
                length,
            ));
        }

        // If the queue gets too long, then reduce its size
        if frontier.len() >= data.queue_limit {
            println!("Truncating queue ({})...", frontier.len());
            frontier.truncate(data.queue_limit / 2);
            println!("done. ({})", frontier.len());
        }

        // Print stats every so often
        if iter_count % 1_000_000 == 0 {
            println!("{} iters, {} items in queue.", iter_count, frontier.len());
        }
        iter_count += 1;
    }

    comps
}

#[derive(Debug, Clone)]
pub struct Comp {
    pub start_idx: usize,
    pub links: Vec<LinkIdx>,
    pub end_label: String,

    pub length: usize,
    pub score: Score,
    pub avg_score: Score,
}

impl Comp {
    pub fn display_string(&self, graph: &Graph, data: &Data, end_label: &str) -> String {
        let mut s = String::new();
        // Start
        let (_node_idx, label) = &graph.starts[self.start_idx];
        s.push_str(label);
        // Links
        for &link_idx in &self.links {
            s.push_str(&data.layout.links[link_idx].display_name);
        }
        // End
        s.push_str(end_label);
        s
    }
}

/////////////////
// COMP PREFIX //
/////////////////

/// The prefix of a composition.  These are ordered by score.
#[derive(Debug, Clone)]
pub struct CompPrefix {
    /// The path traced to this node
    path: CompPath,

    node_idx: NodeIdx,
    unreachable_nodes: BitVec,

    /// Score refers to the **end** of the current node
    score: Score,
    /// Length refers to the **end** of the current node
    length: usize,

    /// Score per row in the composition
    avg_score: Score,
}

impl CompPrefix {
    fn new(
        path: CompPath,
        node_idx: NodeIdx,
        unreachable_nodes: BitVec,
        score: Score,
        length: usize,
    ) -> Self {
        Self {
            path,
            node_idx,
            unreachable_nodes,
            score,
            length,
            avg_score: score / length as f32,
        }
    }
}

impl PartialEq for CompPrefix {
    fn eq(&self, other: &Self) -> bool {
        self.avg_score == other.avg_score
    }
}

impl Eq for CompPrefix {}

impl PartialOrd for CompPrefix {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CompPrefix {
    fn cmp(&self, other: &Self) -> Ordering {
        self.avg_score.cmp(&other.avg_score)
    }
}

/// A route through the composition graph, stored as a reverse linked list.  This allows for
/// multiple compositions with the same prefix to share the data for that prefix.
#[derive(Debug, Clone)]
enum CompPath {
    /// The start of a composition, along with the index within `Graph::start_nodes` of this
    /// specific start
    Start(usize),
    /// The composition follows the sequence in the [`Rc`], followed by taking the `n`th successor
    /// to that node.
    Cons(Rc<Self>, LinkIdx),
}

impl CompPath {
    // TODO: Remove the dependence on `Graph`.  Ideally, we'd store comps in such a way that they
    // only need the `Layout`.
    fn flatten(&self, graph: &Graph, data: &Data) -> (usize, Vec<LinkIdx>) {
        let mut links = Vec::new();
        let start_idx = self.flatten_recursive(graph, data, &mut links);
        (start_idx, links)
    }

    // Recursively flatten `self`, returning the start idx
    fn flatten_recursive(&self, graph: &Graph, data: &Data, out: &mut Vec<LinkIdx>) -> usize {
        match self {
            Self::Start(idx) => *idx,
            Self::Cons(lhs, link) => {
                let start = lhs.flatten_recursive(graph, data, out);
                out.push(*link);
                start
            }
        }
    }
}
