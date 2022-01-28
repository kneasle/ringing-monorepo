//! Creation and manipulation of composition graphs.  This implements routines for creating and
//! optimising such graphs, in preparation for performing tree search.

mod falseness;
pub mod optimise;

use std::{
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, HashMap, HashSet},
};

use bellframe::{Mask, Row, RowBuf, Stroke, Truth};
use itertools::Itertools;

use crate::{
    layout::{
        chunk_range::{ChunkRange, End, PerPartLength, RangeEnd, RangeFactory, TotalLength},
        ChunkId, Layout, LinkIdx, RowRange, StandardChunkId, StartIdx,
    },
    music::{Breakdown, MusicType, Score, StrokeSet},
    utils::{FrontierItem, Rotation, RowCounts},
    Query,
};

use self::{
    falseness::{FalsenessEntry, FalsenessTable},
    optimise::Pass,
};

/// The number of rows required to get from a point in the graph to a start/end.
type Distance = usize;

/// A 'prototype' chunk graph that is (relatively) inefficient to traverse but easy to modify.  This
/// is usually used to build and optimise the chunk graph before being converted into an efficient
/// graph representation for use in tree search.
#[derive(Debug, Clone)]
pub struct Graph {
    // NOTE: References between chunks don't have to be valid (i.e. they can point to a [`Chunk`]
    // that isn't actually in the graph - in this case they will be ignored or discarded during the
    // optimisation process).
    chunks: HashMap<ChunkId, Chunk>,
    /// **Invariant**: If `start_chunks` points to a chunk, it **must** be a start chunk (i.e. not
    /// have any predecessors, and have `start_label` set)
    start_chunks: Vec<(ChunkId, StartIdx, Rotation)>,
    /// **Invariant**: If `start_chunks` points to a chunk, it **must** be a end chunk (i.e. not have
    /// any successors, and have `end_chunks` set)
    end_chunks: Vec<(ChunkId, End)>,
    /// The number of different parts
    num_parts: Rotation,
}

/// A `Chunk` in a chunk [`Graph`].  This is an indivisible chunk of ringing which cannot be split up
/// by calls or splices.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// If this `Chunk` is a 'start' (i.e. it can be the first chunk in a composition), then this is
    /// `Some(label)` where `label` should be appended to the front of the human-friendly
    /// composition string.
    is_start: bool,
    /// If this `Chunk` is an 'end' (i.e. adding it will complete a composition), then this is
    /// `Some(label)` where `label` should be appended to the human-friendly composition string.
    end: Option<End>,
    /// The string that should be added when this chunk is generated
    label: String,

    successors: Vec<Link>,
    predecessors: Vec<Link>,

    /// The chunks which share rows with `self`, including `self` (because all chunks are false
    /// against themselves).  Optimisation passes probably shouldn't mess with falseness.
    false_chunks: Vec<StandardChunkId>,

    /// The number of rows in the range covered by this chunk (i.e. its length in one part of the
    /// composition)
    per_part_length: PerPartLength,
    /// The number of rows that this this chunk adds to the composition (its total length across all
    /// parts).  Optimisation passes can't change this
    total_length: TotalLength,
    /// The number of rows of each method generated by this chunk
    method_counts: RowCounts,
    /// The music generated by this chunk in the composition.  Optimisation passes can't change this
    music: Breakdown,

    /// `true` if this chunk doesn't contain music that's interesting to the composition
    duffer: bool,
    /// A lower bound on the number of rows required to go from any non-duffer chunk to the first
    /// row of `self`
    pub lb_distance_from_non_duffer: usize,
    /// A lower bound on the number of rows required to go from the first row **after** `self` to
    /// any non-duffer chunk.
    pub lb_distance_to_non_duffer: usize,

    /* MUTABLE STATE FOR OPTIMISATION PASSES */
    /// Does this chunk need to be included in every composition in this search?
    pub required: bool,
    /// A lower bound on the number of rows required to go from any rounds to the first row of
    /// `self`
    pub lb_distance_from_rounds: Distance,
    /// A lower bound on the number of rows required to go from the first row **after** `self` to
    /// rounds.
    pub lb_distance_to_rounds: Distance,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Link {
    pub id: ChunkId,
    /// Indexes into `Layout::links`
    pub source_idx: LinkIdx,
    pub rotation: Rotation,
}

impl Link {
    pub fn new(id: ChunkId, source_idx: LinkIdx, rotation: Rotation) -> Self {
        Self {
            id,
            source_idx,
            rotation,
        }
    }
}

// ------------------------------------------------------------------------------------------

impl Graph {
    //! Optimisation

    /// Repeatedly apply a sequence of [`Pass`]es until the graph stops getting smaller, or 20
    /// iterations are made.  Use [`Graph::optimise_with_iter_limit`] to set a custom iteration limit.
    pub fn optimise(&mut self, passes: &mut [Pass], query: &Query) {
        self.optimise_with_iter_limit(passes, query, 20);
    }

    /// Repeatedly apply a sequence of [`Pass`]es until the graph either becomes static, or `limit`
    /// many iterations are performed.
    pub fn optimise_with_iter_limit(&mut self, passes: &mut [Pass], query: &Query, limit: usize) {
        let mut last_size = Size::from(&*self);

        for _ in 0..limit {
            self.run_passes(passes, query);

            let new_size = Size::from(&*self);
            // Stop optimising if the optimisations don't make the graph strictly smaller.  If
            // they make some parts smaller but others larger, then keep optimising.
            match new_size.partial_cmp(&last_size) {
                Some(Ordering::Equal | Ordering::Greater) => return,
                Some(Ordering::Less) | None => {}
            }
            last_size = new_size;
        }
    }

    /// Run a sequence of [`Pass`]es on `self`
    pub fn run_passes(&mut self, passes: &mut [Pass], query: &Query) {
        for p in &mut *passes {
            p.run(self, query);
        }
    }

    /// For each start chunk in `self`, creates a copy of `self` with _only_ that start chunk.  This
    /// partitions the set of generated compositions across these `Graph`s, but allows for better
    /// optimisations because more is known about each `Graph`.
    pub fn split_by_start_chunk(&self) -> Vec<Graph> {
        self.start_chunks
            .iter()
            .cloned()
            .map(|start_id| {
                let mut new_self = self.clone();
                new_self.start_chunks = vec![start_id];
                new_self
            })
            .collect_vec()
    }

    pub fn num_parts(&self) -> Rotation {
        self.num_parts
    }
}

// ------------------------------------------------------------------------------------------

impl Graph {
    //! Helpers for optimisation passes

    /// Removes all chunks for whom `pred` returns `false`
    pub fn retain_chunks(&mut self, pred: impl FnMut(&ChunkId, &mut Chunk) -> bool) {
        self.chunks.retain(pred);
    }

    /// Remove elements from [`Self::start_chunks`] for which a predicate returns `false`.
    pub fn retain_start_chunks(
        &mut self,
        pred: impl FnMut(&(ChunkId, StartIdx, Rotation)) -> bool,
    ) {
        self.start_chunks.retain(pred);
    }

    /// Remove elements from [`Self::end_chunks`] for which a predicate returns `false`.
    pub fn retain_end_chunks(&mut self, pred: impl FnMut(&(ChunkId, End)) -> bool) {
        self.end_chunks.retain(pred);
    }
}

impl Chunk {
    //! Helpers for optimisation passes

    /// A lower bound on the length of a composition which passes through this chunk.
    pub fn min_comp_length(&self) -> usize {
        self.lb_distance_from_rounds + self.length() + self.lb_distance_to_rounds
    }

    /// A lower bound on the length of the run of duffers which passes through this chunk.
    pub fn min_duffer_length(&self) -> usize {
        if self.duffer {
            0 // Make sure that non-duffers are never pruned
        } else {
            self.lb_distance_from_non_duffer + self.length() + self.lb_distance_to_non_duffer
        }
    }
}

/// A measure of the `Size` of a [`Graph`].  Used to detect when further optimisations aren't
/// useful.
#[derive(Debug, PartialEq, Clone, Copy)]
struct Size {
    num_chunks: usize,
    num_links: usize,
    num_starts: usize,
    num_ends: usize,
}

impl From<&Graph> for Size {
    fn from(g: &Graph) -> Self {
        Self {
            num_chunks: g.chunks.len(),
            // This assumes that every successor link also corresponds to a predecessor link
            num_links: g.chunks().map(|(_id, chunk)| chunk.successors.len()).sum(),
            num_starts: g.start_chunks.len(),
            num_ends: g.end_chunks.len(),
        }
    }
}

impl PartialOrd for Size {
    // TODO: Make this into a macro?
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let cmp_chunks = self.num_chunks.cmp(&other.num_chunks);
        let cmp_links = self.num_links.cmp(&other.num_links);
        let cmp_starts = self.num_starts.cmp(&other.num_starts);
        let cmp_ends = self.num_ends.cmp(&other.num_ends);

        let all_comparisons = [cmp_chunks, cmp_links, cmp_starts, cmp_ends];

        let are_any_less = all_comparisons
            .iter()
            .any(|cmp| matches!(cmp, Ordering::Less));
        let are_any_greater = all_comparisons
            .iter()
            .any(|cmp| matches!(cmp, Ordering::Greater));

        match (are_any_less, are_any_greater) {
            (true, false) => Some(Ordering::Less), // If nothing got larger, then the size is smaller
            (false, true) => Some(Ordering::Greater), // If nothing got smaller, then the size is larger
            (false, false) => Some(Ordering::Equal),  // No < or > means all components are equal
            (true, true) => None, // If some are smaller & some are greater then these are incomparable
        }
    }
}

// ------------------------------------------------------------------------------------------

impl Graph {
    //! Getters & Iterators

    // Getters

    pub fn get_chunk<'graph>(&'graph self, id: &ChunkId) -> Option<&'graph Chunk> {
        self.chunks.get(id)
    }

    pub fn get_chunk_mut<'graph>(&'graph mut self, id: &ChunkId) -> Option<&'graph mut Chunk> {
        self.chunks.get_mut(id)
    }

    pub fn start_chunks(&self) -> &[(ChunkId, StartIdx, Rotation)] {
        &self.start_chunks
    }

    pub fn end_chunks(&self) -> &[(ChunkId, End)] {
        &self.end_chunks
    }

    pub fn chunk_map(&self) -> &HashMap<ChunkId, Chunk> {
        &self.chunks
    }

    pub fn get_start(&self, idx: usize) -> Option<(&Chunk, StartIdx, Rotation)> {
        let (start_chunk_id, start_idx, rotation) = self.start_chunks.get(idx)?;
        let start_chunk = self.chunks.get(start_chunk_id)?;
        assert!(start_chunk.is_start);
        Some((start_chunk, *start_idx, *rotation))
    }

    // Iterators

    /// An [`Iterator`] over the [`ChunkId`] of every [`Chunk`] in this `Graph`
    pub fn ids(&self) -> impl Iterator<Item = &ChunkId> {
        self.chunks.keys()
    }

    /// An [`Iterator`] over every [`Chunk`] in this `Graph` (including its [`ChunkId`])
    pub fn chunks(&self) -> impl Iterator<Item = (&ChunkId, &Chunk)> {
        self.chunks.iter()
    }

    /// An [`Iterator`] over every [`Chunk`] in this `Graph`, without its [`ChunkId`].
    pub fn just_chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.values()
    }

    /// A mutable [`Iterator`] over the [`ChunkId`] of every [`Chunk`] in this `Graph`
    pub fn chunks_mut(&mut self) -> impl Iterator<Item = (&ChunkId, &mut Chunk)> {
        self.chunks.iter_mut()
    }
}

impl Chunk {
    //! Getters & Iterators

    pub fn length(&self) -> usize {
        self.total_length.0
    }

    pub fn method_counts(&self) -> &RowCounts {
        &self.method_counts
    }

    pub fn score(&self) -> Score {
        self.music.score
    }

    pub fn label(&self) -> &str {
        self.label.as_str()
    }

    pub fn music(&self) -> &Breakdown {
        &self.music
    }

    pub fn duffer(&self) -> bool {
        self.duffer
    }

    // STARTS/ENDS //

    pub fn is_start(&self) -> bool {
        self.is_start
    }

    pub fn end(&self) -> Option<End> {
        self.end
    }

    pub fn is_end(&self) -> bool {
        self.end.is_some()
    }

    // CROSS-NODE REFERENCES //

    pub fn successors(&self) -> &[Link] {
        self.successors.as_slice()
    }

    pub fn successors_mut(&mut self) -> &mut Vec<Link> {
        &mut self.successors
    }

    pub fn predecessors(&self) -> &[Link] {
        self.predecessors.as_slice()
    }

    pub fn predecessors_mut(&mut self) -> &mut Vec<Link> {
        &mut self.predecessors
    }

    pub fn false_chunks(&self) -> &[StandardChunkId] {
        self.false_chunks.as_slice()
    }

    pub fn false_chunks_mut(&mut self) -> &mut Vec<StandardChunkId> {
        &mut self.false_chunks
    }
}

////////////////////////////////
// LAYOUT -> GRAPH CONVERSION //
////////////////////////////////

impl Graph {
    /// Generate a graph of all chunks which are reachable within a given length constraint.
    pub fn from_layout(
        layout: &Layout,
        music_types: &[MusicType],
        ch_weights: &[(Mask, f32)],
        max_length: usize,
        part_head: &Row,
        start_stroke: Stroke,
        allow_false: bool,
    ) -> Self {
        // Build the shape of the graph using Dijkstra's algorithm
        let (expanded_chunk_ranges, start_chunks, end_chunks, ch_equiv_map, part_heads) =
            build_graph(layout, max_length, part_head);
        let num_parts = part_heads.len() as Rotation;

        // `true` if any of the `music_types` care about stroke
        let dependence_on_stroke = music_types
            .iter()
            .any(|ty| ty.stroke_set() != StrokeSet::Both);

        // Convert each `expanded_chunk_range` into a full `Chunk`, albeit without
        // predecessor/falseness references
        let mut chunks: HashMap<ChunkId, Chunk> = expanded_chunk_ranges
            .iter()
            .map(|(chunk_id, (chunk_range, distance))| {
                // If music types rely on accurate strokes, then we need to make sure that the
                // chunks always start at the same stroke.
                //
                // TODO: If we want to implement this properly, what we should actually check is
                // the start stroke of any chunk is unabiguous.  I doubt this assert will ever be
                // tripped, though.
                if dependence_on_stroke {
                    assert!(
                        chunk_range.per_part_length.0 % 2 == 0 || chunk_range.end().is_some(),
                        "Odd length chunks aren't implemented yet."
                    );
                }
                assert_eq!(chunk_id, &chunk_range.chunk_id);
                let new_chunk = build_chunk(
                    chunk_range,
                    *distance,
                    layout,
                    music_types,
                    ch_weights,
                    &part_heads,
                    // We've asserted that all chunks have even length, so that all chunks must start
                    // at the same stroke.  Therefore, we can simply pass 'start_stroke' straight
                    // to all the comps
                    start_stroke,
                );
                (chunk_id.clone(), new_chunk)
            })
            .collect();

        let plural = |count: usize, singular: &str| -> String {
            let extension = if count == 1 { "" } else { "s" };
            format!("{} {}{}", count, singular, extension)
        };
        log::debug!(
            "Unoptimised graph has {}, with {} and {}.",
            plural(chunks.len(), "chunk"),
            plural(start_chunks.len(), "start"),
            plural(end_chunks.len(), "end"),
        );

        if !allow_false {
            compute_falseness(&mut chunks, layout, &ch_equiv_map);
        }

        // Add predecessor references (every chunk is a predecessor to all of its successors)
        log::debug!("Setting predecessor links");
        for (id, _dist) in expanded_chunk_ranges {
            if let Some(chunk) = chunks.get(&id) {
                for succ_link in chunk.successors.clone() {
                    if let Some(succ_chunk) = chunks.get_mut(&succ_link.id) {
                        assert!(succ_link.rotation < num_parts);
                        succ_chunk.predecessors.push(Link {
                            id: id.clone(),
                            source_idx: succ_link.source_idx,
                            // Passing backwards over a link gives it the opposite rotation to
                            // traversing forward
                            rotation: num_parts - succ_link.rotation,
                        });
                    }
                }
            }
        }

        Self {
            chunks,
            start_chunks,
            end_chunks,
            num_parts,
        }
    }
}

/// Given an initial set of chunks, compute the pairs of false chunks and remove any chunks which are
/// self-false.
fn compute_falseness(
    chunks: &mut HashMap<ChunkId, Chunk>,
    layout: &Layout,
    ch_equiv_map: &HashMap<RowBuf, (RowBuf, Rotation)>,
) {
    // Build a `FalsenessTable` which encodes the falseness for this set of chunks
    log::debug!("Building falseness table");
    let chunk_ids_and_lengths = chunks
        .iter()
        .map(|(id, chunk)| (id.clone(), chunk.per_part_length))
        .collect::<HashSet<_>>();
    let falseness_table = FalsenessTable::from_layout(layout, &chunk_ids_and_lengths);
    log::trace!("Falseness table: {:#?}", falseness_table);

    // Use the table to generate the falseness links and detect which chunks are false against
    // themselves in a different part
    log::debug!("Setting falseness links");
    chunks.retain(|id, chunk| {
        set_falseness_links(
            id,
            chunk,
            &falseness_table,
            ch_equiv_map,
            &chunk_ids_and_lengths,
        )
        .is_true()
    });
}

/// Given a chunk and some lookup tables, set `false_chunks` for that chunk.  This also returns
/// `false` if the chunk is false against itself.
fn set_falseness_links(
    id: &ChunkId,
    chunk: &mut Chunk,
    falseness_table: &FalsenessTable,
    ch_equiv_map: &HashMap<RowBuf, (RowBuf, Rotation)>,
    chunk_ids_and_lengths: &HashSet<(ChunkId, PerPartLength)>,
) -> Truth {
    // Early return for zero-length ends, which can't have any falseness (even against themselves)
    let std_id = match id {
        ChunkId::ZeroLengthEnd => return Truth::True,
        ChunkId::Standard(std_id) => std_id,
    };
    // If the chunk's [`RowRange`] is self-false then return `Truth::False` without setting any
    // falseness links
    let entry = falseness_table.falseness_entry(RowRange {
        start: std_id.row_idx,
        len: chunk.per_part_length,
    });
    let fchs = match entry {
        FalsenessEntry::SelfFalse => return Truth::False,
        FalsenessEntry::FalseCourseHeads(fchs) => fchs,
    };
    // If the chunk has non-zero length and is self-true, then set the falseness pointers according
    // to the FCHs given by the table
    chunk.false_chunks.clear();
    for (false_range, false_ch_transposition) in fchs {
        let false_ch = std_id.course_head.as_ref() * false_ch_transposition;
        if let Some((false_equiv_ch, rotation)) = ch_equiv_map.get(&false_ch) {
            for is_start in [true, false] {
                let false_id =
                    StandardChunkId::new(false_equiv_ch.clone(), false_range.start, is_start);
                let false_id_and_len = (ChunkId::Standard(false_id.clone()), false_range.len);
                if &false_id == std_id && *rotation != 0 {
                    return Truth::False; // Remove chunk if it's false against itself in another part
                }
                if chunk_ids_and_lengths.contains(&false_id_and_len) {
                    // If the chunk at `false_id` is in the graph, then it's false against
                    // `chunk`
                    chunk.false_chunks.push(false_id);
                }
            }
        }
    }
    // If this chunk isn't false against itself in any part (including the first), then it must be
    // self-true
    Truth::True
}

/// Use Dijkstra's algorithm to determine the overall structure of the graph, without computing the
/// [`Chunk`]s themselves.
#[allow(clippy::type_complexity)]
fn build_graph(
    layout: &Layout,
    max_length: usize,
    part_head: &Row,
) -> (
    HashMap<ChunkId, (ChunkRange, Distance)>,
    Vec<(ChunkId, StartIdx, Rotation)>,
    Vec<(ChunkId, End)>,
    HashMap<RowBuf, (RowBuf, Rotation)>,
    Vec<RowBuf>,
) {
    let mut range_factory = RangeFactory::new(layout, part_head);

    let start_chunks = range_factory.start_ids();
    let mut end_chunks = Vec::<(ChunkId, End)>::new();

    // The set of chunks which have already been expanded
    let mut expanded_chunks: HashMap<ChunkId, (ChunkRange, Distance)> = HashMap::new();

    // Initialise the frontier with the start chunks, all with distance 0
    let mut frontier: BinaryHeap<Reverse<FrontierItem<ChunkId>>> = BinaryHeap::new();
    frontier.extend(
        start_chunks
            .iter()
            .cloned()
            .map(|(id, _, _)| FrontierItem::new(id, 0))
            .map(Reverse),
    );

    while let Some(Reverse(FrontierItem {
        item: chunk_id,
        distance,
    })) = frontier.pop()
    {
        // Don't expand chunks multiple times (Dijkstra's algorithm makes sure that the first time
        // it is expanded will be have the shortest distance)
        if expanded_chunks.get(&chunk_id).is_some() {
            continue;
        }
        // If the chunk hasn't been expanded yet, then add its reachable chunks to the frontier
        let chunk_range = range_factory
            .gen_range(&chunk_id)
            .expect("Infinite segment found");
        // If the shortest composition including this chunk is longer the length limit, then don't
        // include it in the chunk graph
        let new_dist = distance + chunk_range.total_length.0;
        if new_dist > max_length {
            continue;
        }
        match &chunk_range.range_end {
            RangeEnd::End(end) => end_chunks.push((chunk_id.clone(), *end)),
            RangeEnd::NotEnd(succ_links) => {
                // Expand the chunk by adding its successors to the frontier
                for (_, id_after_link, _) in succ_links {
                    // Add the new chunk to the frontier
                    frontier.push(Reverse(FrontierItem {
                        item: id_after_link.to_owned(),
                        distance: new_dist,
                    }));
                }
            }
        }
        // Mark this chunk as expanded
        expanded_chunks.insert(chunk_id, (chunk_range, distance));
    }

    // Once Dijkstra's has finished, consume the `RangeFactory` and return the values needed to
    // complete the graph
    let (ch_equiv_map, part_heads) = range_factory.finish();
    (
        expanded_chunks,
        start_chunks,
        end_chunks,
        ch_equiv_map,
        part_heads,
    )
}

/// Construct a [`Chunk`] from the data returned by the `RangeFactory`.  This is mostly
/// unpacking/repacking data, but also involves computing music and 'dufferness'.
fn build_chunk(
    chunk_range: &ChunkRange,
    distance: usize,
    layout: &Layout,
    music_types: &[MusicType],
    ch_weights: &[(Mask, f32)],
    part_heads: &[RowBuf],
    start_stroke: Stroke,
) -> Chunk {
    // Add up music from each part
    let mut music = Breakdown::zero(music_types.len());
    for ph in part_heads {
        if let Some(source_ch) = chunk_range.chunk_id.course_head() {
            let ch = ph * source_ch;
            // Count weight from this part
            music += &Breakdown::from_rows(
                chunk_range.untransposed_rows(layout),
                &ch,
                music_types,
                start_stroke,
            );
            // Count weight from CH masks
            for (mask, weight) in ch_weights {
                if mask.matches(&ch) {
                    music.score += *weight * chunk_range.total_length.0 as f32; // Weight applies to each row
                }
            }
        }
    }

    // Determine if this chunk is (not) a duffer.  A chunk is a duffer it doesn't include any music
    // of types considered 'non-duffer'.
    //
    // TODO: Determine how close to the ends of this chunk the music is generated?
    let non_duffer = music_types
        .iter()
        .zip_eq(&music.counts)
        .any(|(music_type, count)| music_type.non_duffer() && *count > 0);

    Chunk {
        per_part_length: chunk_range.per_part_length,
        total_length: chunk_range.total_length,

        is_start: chunk_range.chunk_id.is_start(),
        end: chunk_range.end(),
        label: chunk_range.label.clone(),

        method_counts: chunk_range.method_counts.clone(),
        music,

        duffer: !non_duffer,
        // Distances will be computed during optimisation passes
        lb_distance_from_non_duffer: 0,
        lb_distance_to_non_duffer: 0,

        required: false,
        lb_distance_from_rounds: distance,
        // Distances to rounds are computed later, but the distance is an lower bound,
        // so we can set it to 0 without breaking any invariants.
        lb_distance_to_rounds: 0,

        successors: chunk_range
            .links()
            .iter()
            .cloned()
            .map(|(idx, id, rotation)| Link::new(id, idx, rotation))
            .collect_vec(),

        // These are populated in separate passes once all the `Chunk`s have been created
        false_chunks: Vec::new(),
        predecessors: Vec::new(),
    }
}
