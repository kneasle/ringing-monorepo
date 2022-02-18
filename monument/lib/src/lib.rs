//! Core library for Monument, a fast and flexible composing engine.

#![deny(clippy::all)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod graph;
pub mod layout;
pub mod music;
mod search;
pub mod utils;

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
        Arc,
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
    pub method_count_range: Range<usize>,

    pub part_head: RowBuf,
    /// The `f32` is the weight given to every row in any course matching the given [`Mask`]
    pub ch_weights: Vec<(Mask, f32)>,

    pub music_types: Vec<music::MusicType>,
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

/// Configuration parameters for Monument which **don't** change which compositions are emitted.
pub struct Config {
    /// Number of threads used to generate compositions.  If `None`, this uses the number of
    /// **physical** CPU cores (i.e. ignoring hyper-threading).
    pub num_threads: Option<usize>,
    pub queue_limit: usize,
    pub optimisation_passes: Vec<Pass>,
    pub split_by_start_chunk: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            num_threads: None,
            queue_limit: 10_000_000,
            optimisation_passes: graph::optimise::passes::default(),
            split_by_start_chunk: false,
        }
    }
}

/// A `Comp`osition generated by Monument.
#[derive(Debug, Clone)]
pub struct Comp {
    /// The [`Query`] from which this `Comp` was generated.  This is ignored when computing
    /// [`Eq`]uality and when [`Hash`]ing.
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

        let mut s = String::new();
        // Start
        s.push_str(&layout.starts[self.start_idx].label);
        // Chunks & links
        s.push_str(&self.start_chunk_label);
        for (link_idx, link_label) in &self.links {
            s.push_str(&layout.links[*link_idx].display_name);
            s.push_str(link_label);
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
            .map(|(count, music_type)| f32::from(music_type.weight()) * *count as f32)
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
    /// Creates an unoptimised [`Graph`] from which our compositions are generated
    pub fn unoptimised_graph(&self) -> Graph {
        log::debug!("Building `Graph`");
        graph::Graph::from_layout(
            &self.layout,
            &self.music_types,
            &self.ch_weights,
            // `- 1` makes sure that the length limit is an **inclusive** bound
            self.len_range.end - 1,
            &self.part_head,
            self.start_stroke,
            self.allow_false,
        )
    }

    /// Converts a single [`Graph`] into a set of [`Graph`]s which make tree search faster but
    /// generate the same overall set of compositions.
    pub fn optimise_graph(&self, graph: Graph, config: &mut Config) -> Vec<Graph> {
        log::debug!("Optimising graph(s)");
        let mut graphs = if config.split_by_start_chunk {
            graph.split_by_start_chunk()
        } else {
            vec![graph]
        };
        for g in &mut graphs {
            g.optimise(&mut config.optimisation_passes, self);
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
                let abort_flag = abort_flag.clone();
                let update_channel = update_tx.clone();
                std::thread::spawn(move || {
                    search::search(
                        &graph,
                        query.clone(),
                        queue_limit / num_threads,
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
}

#[derive(Debug)]
pub struct Progress {
    /// How many chunks have been expanded so far
    pub iter_count: usize,
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
