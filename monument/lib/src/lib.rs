//! Core library for Monument, a fast and flexible composing engine.

#![deny(clippy::all)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod graph;
pub mod layout;
pub mod music;
mod search;
pub mod utils;

use serde::Deserialize;
pub use utils::OptRange;

use itertools::Itertools;
use layout::{chunk_range::End, LinkIdx, StartIdx};
use music::Score;
use utils::{Counts, Rotation};

use std::{
    hash::Hash,
    ops::{Deref, Range},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::sync_channel,
        Arc, Mutex,
    },
};

use bellframe::{Mask, RowBuf, Stroke};
use graph::{optimise::Pass, Graph};

/// Information provided to Monument which specifies what compositions are generated.
///
/// Compare this to [`Config`], which determines _how_ those compositions are generated (and
/// therefore determines how quickly the results are generated).
#[derive(Debug, Clone)]
pub struct Query {
    pub layout: layout::Layout,

    pub len_range: Range<usize>,
    pub num_comps: usize,
    pub allow_false: bool,
    pub splice_style: SpliceStyle,

    pub calls: CallVec<CallType>,
    pub part_head: RowBuf,
    /// The `f32` is the weight given to every row in any course matching the given [`Mask`]
    pub ch_weights: Vec<(Mask, f32)>,
    pub splice_weight: f32,

    pub music_types: MusicTypeVec<music::MusicType>,
    pub music_displays: Vec<music::MusicDisplay>,
    pub start_stroke: Stroke,
    pub max_duffer_rows: Option<usize>,
}

impl Query {
    pub fn is_multipart(&self) -> bool {
        !self.part_head.is_rounds()
    }

    pub fn num_parts(&self) -> usize {
        self.part_head.order()
    }
}

/// The different styles of spliced that can be generated
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Deserialize)]
pub enum SpliceStyle {
    /// Splices could happen at any lead label
    #[serde(rename = "leads")]
    LeadLabels,
    /// Splice only happen whenever a call _could_ have happened
    #[serde(rename = "call locations")]
    CallLocations,
    /// Splices only happen when calls are actually made
    #[serde(rename = "calls")]
    Calls,
}

impl Default for SpliceStyle {
    fn default() -> Self {
        Self::LeadLabels
    }
}

/// A type of call (e.g. bob or single)
#[derive(Debug, Clone)]
pub struct CallType {
    pub debug_symbol: String,
    pub display_symbol: String,
    pub weight: f32,
}

impl From<layout::new::Call> for CallType {
    fn from(c: layout::new::Call) -> Self {
        Self {
            debug_symbol: c.debug_symbol,
            display_symbol: c.display_symbol,
            weight: c.weight,
        }
    }
}

/// Configuration parameters for Monument which **don't** change which compositions are emitted.
pub struct Config {
    /* General */
    /// Number of threads used to generate compositions.  If `None`, this uses the number of
    /// **physical** CPU cores (i.e. ignoring hyper-threading).
    pub num_threads: Option<usize>,

    /* Graph Generation */
    pub optimisation_passes: Vec<Mutex<Pass>>,
    /// The maximum graph size, in nodes.  If a search would produce a graph bigger than this, it
    /// is aborted.
    pub graph_size_limit: usize,
    pub split_by_start_chunk: bool,

    /* Search */
    pub queue_limit: usize,
    /// If `true`, the data structures used by searches will be leaked using [`std::mem::forget`].
    /// This massively improves the termination speed (because all individual allocations don't
    /// need to be freed), but only makes sense for the CLI, where Monument will do exactly one
    /// search run before terminating (thus returning the memory to the OS anyway).
    pub mem_forget_search_data: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            num_threads: None,

            graph_size_limit: 100_000,
            optimisation_passes: graph::optimise::passes::default(),
            split_by_start_chunk: false,

            queue_limit: 10_000_000,
            mem_forget_search_data: false,
        }
    }
}

/// A `Comp`osition generated by Monument.
#[derive(Debug, Clone)]
pub struct Comp {
    /// The [`Query`] from which this `Comp` was generated.  This is ignored when computing
    /// [`Eq`]uality and when [`Hash`]ing.
    // TODO: Remove this
    pub query: Arc<Query>,
    pub inner: CompInner,
}

/// The parts of `Comp` which can be easily `Hash`ed/`Eq`ed.
// TODO: find a cleaner way to implement `Hash`/`Eq`/etc. for `Comp`, making sure that `query` is
// calculated by memory location
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompInner {
    pub start_idx: StartIdx,
    pub start_chunk_label: String,
    pub links: Vec<(LinkIdx, String)>,
    pub end: End,

    pub rotation: Rotation,
    pub length: usize,
    /// The number of rows generated of each method
    pub method_counts: Counts,
    /// The number of counts generated of each [`MusicType`](music::MusicType)
    pub music_counts: Counts,
    /// The total [`Score`] of this composition, accumulated from music, calls, coursing patterns,
    /// etc.
    pub total_score: Score,
    /// Average [`Score`] generated by each row in the composition.   This is used to rank
    /// compositions to prevent the search algorithm being dominated by long compositions.
    pub avg_score: Score,
}

impl Comp {
    pub fn call_string(&self) -> String {
        let layout = &self.query.layout;
        let needs_brackets = layout.is_spliced() || layout.leadwise;

        let mut s = String::new();
        // Start
        s.push_str(&layout.starts[self.start_idx].label);
        // Chunks & links
        s.push_str(&self.start_chunk_label);
        for (link_idx, chunk_label) in &self.links {
            let link = &layout.links[*link_idx];
            if let Some(call_idx) = link.call_idx {
                let call = &self.query.calls[call_idx];
                s.push_str(if needs_brackets { "[" } else { "" });
                s.push_str(match layout.leadwise {
                    true => &call.debug_symbol, // use debug symbols for leadwise
                    false => &call.display_symbol,
                });
                s.push_str(&link.calling_position);
                s.push_str(if needs_brackets { "]" } else { "" });
            }
            s.push_str(chunk_label);
        }
        // End
        s.push_str(self.end.label(layout));
        s
    }

    pub fn part_head(&self) -> RowBuf {
        self.query.part_head.pow_u(self.rotation as usize)
    }

    pub fn music_score(&self) -> f32 {
        self.music_counts
            .iter()
            .zip_eq(&self.query.music_types)
            .map(|(count, music_type)| f32::from(music_type.weight) * *count as f32)
            .sum::<f32>()
    }
}

impl std::fmt::Display for Comp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "len: {}, ", self.length)?;
        // Method counts for spliced
        if self.query.layout.is_spliced() {
            write!(f, "ms: {:>3?}, ", self.method_counts.as_slice())?;
        }
        // Part heads if multi-part with >2 parts (2-part compositions only have one possible part
        // head)
        if self.query.num_parts() > 2 {
            write!(f, "PH: {}, ", self.part_head())?;
        }
        write!(
            f,
            "music: {:>6.2?}, avg score: {:.6}, str: {}",
            self.music_score(),
            self.avg_score,
            self.call_string()
        )
    }
}

impl Deref for Comp {
    type Target = CompInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl PartialEq for Comp {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Comp {}

impl Hash for Comp {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

////////////
// SEARCH //
////////////

impl Query {
    pub fn unoptimised_graph(&self, config: &Config) -> Result<Graph, graph::BuildError> {
        graph::Graph::new_unoptimised(self, config)
    }

    /// Converts a single [`Graph`] into a set of [`Graph`]s which make tree search faster but
    /// generate the same overall set of compositions.
    pub fn optimise_graph(&self, graph: Graph, config: &Config) -> Vec<Graph> {
        log::debug!("Optimising graph(s)");
        let mut graphs = if config.split_by_start_chunk {
            graph.split_by_start_chunk()
        } else {
            vec![graph]
        };
        for g in &mut graphs {
            g.optimise(&config.optimisation_passes, self);
            log::debug!(
                "Optimised graph has {} chunks, {} starts, {} ends",
                g.chunk_map().len(),
                g.start_chunks().len(),
                g.end_chunks().len()
            );
        }
        graphs
    }

    /// Given a set of (optimised) graphs, run multi-threaded tree search to generate compositions.
    /// `update_fn` is run whenever a thread generates a [`QueryUpdate`].
    pub fn search(
        arc_self: Arc<Self>,
        graphs: Vec<Graph>,
        config: &Config,
        mut update_fn: impl FnMut(QueryUpdate) + Send + 'static,
        abort_flag: Arc<AtomicBool>,
    ) {
        // Make sure that `abort_flag` starts as false (so the search doesn't abort immediately).
        // We want this to be sequentially consistent to make sure that the worker threads don't
        // see the previous value (which could be 'true').
        abort_flag.store(false, Ordering::SeqCst);
        // Create a new thread which will handle the query updates
        let (update_tx, update_rx) = sync_channel::<QueryUpdate>(1_000);
        let update_thread = std::thread::spawn(move || {
            while let Ok(update) = update_rx.recv() {
                update_fn(update);
            }
        });
        // Run the search
        let num_threads = graphs.len();
        let handles = graphs
            .into_iter()
            .map(|graph| {
                let query = arc_self.clone();
                let queue_limit = config.queue_limit;
                let mem_forget_search_data = config.mem_forget_search_data;
                let abort_flag = abort_flag.clone();
                let update_channel = update_tx.clone();
                std::thread::spawn(move || {
                    search::search(
                        &graph,
                        query.clone(),
                        queue_limit / num_threads,
                        mem_forget_search_data,
                        update_channel,
                        abort_flag,
                    );
                })
            })
            .collect_vec();
        // `update_thread` will only terminate once **all** copies of `update_tx` are dropped.
        // Each worker thread has its own copy, but if we don't explicitly drop this one then
        // `update_thread` will never terminate (causing the worker thread to hang).
        drop(update_tx);

        // Wait for all search threads to terminate
        for h in handles {
            h.join().unwrap();
        }
        // Wait for `update_thread` to terminate
        update_thread.join().unwrap();
    }
}

/// Instances of this are emitted by the search as it's running
#[derive(Debug)]
pub enum QueryUpdate {
    /// A new composition has been found
    Comp(Comp),
    /// A thread is sending a status update
    Progress(Progress),
    /// The queue of prefixes has got too large and is being shortened
    TruncatingQueue,
    /// The search is being aborted
    Aborting,
}

#[derive(Debug)]
pub struct Progress {
    /// How many chunks have been expanded so far
    pub iter_count: usize,
    /// How many comps have been generated so far
    pub num_comps: usize,

    /// The current length of the A* queue
    pub queue_len: usize,
    /// The average length of a composition in the queue
    pub avg_length: f32,
    /// The length of the longest composition in the queue
    pub max_length: u32,
}

impl Progress {
    /// The [`Progress`] made by a search which hasn't started yet
    pub const START: Self = Self {
        iter_count: 0,
        num_comps: 0,

        queue_len: 0,
        avg_length: 0.0,
        max_length: 0,
    };
}

impl Default for Progress {
    fn default() -> Self {
        Self::START
    }
}

index_vec::define_index_type! { pub struct CallIdx = usize; }
index_vec::define_index_type! { pub struct MusicTypeIdx = usize; }
pub type CallVec<T> = index_vec::IndexVec<CallIdx, T>;
pub type MusicTypeVec<T> = index_vec::IndexVec<MusicTypeIdx, T>;
