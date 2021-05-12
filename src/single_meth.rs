use std::{
    collections::{HashMap, HashSet},
    fmt::{Display, Formatter},
    ops::Range,
};

use itertools::Itertools;
use proj_core::{place_not::PnBlockParseError, Bell, Method, PlaceNot, PnBlock, Row, Stage};

use crate::engine::{self, Node};

/// A tuple of values which represent the transition between two segments
type Transition = (Call, Row, usize);

/// A compact representation of a call
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct Call {
    call: Option<char>,
    position: char,
}

impl Call {
    fn new(call: Option<char>, position: char) -> Self {
        Call { call, position }
    }
}

impl Display for Call {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.call.unwrap_or('p'), self.position)
    }
}

/// A section of a course of a single method
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct Section {
    ind: usize,
}

impl Section {
    const fn new(ind: usize) -> Self {
        Section { ind }
    }
}

impl Display for Section {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.ind)
    }
}

impl engine::Section for Section {
    type Table = Table;
    type Call = Call;

    #[inline(always)]
    fn stage(table: &Self::Table) -> Stage {
        table.stage
    }

    #[inline(always)]
    fn start() -> Self {
        Self::new(0)
    }

    #[inline(always)]
    fn is_end(node: &Node<Self>) -> bool {
        node.row.is_rounds() && node.section.ind == 0
    }

    #[inline(always)]
    fn length(self, table: &Self::Table) -> usize {
        table.lengths[self.ind]
    }

    #[inline(always)]
    fn falseness(self, table: &Self::Table) -> &[(Row, Self)] {
        table.falseness[self.ind].as_slice()
    }

    #[inline(always)]
    fn expand(self, table: &Self::Table) -> &[(Self::Call, Row, Self)] {
        table.next_nodes[self.ind].as_slice()
    }

    fn comp_string(calls: &[Self::Call]) -> String {
        calls
            .iter()
            .map(|Call { call, position }| match call {
                // Plain leads don't get displayed
                None => String::new(),
                // Bobs are implicit
                Some('-') => format!("{}", position),
                // Any other call is written out in full
                Some(name) => format!("{}{}", name, position),
            })
            .join("")
    }
}

/// The persistent state table for a single method
#[derive(Debug, Clone)]
pub struct Table {
    falseness: Vec<Vec<(Row, Section)>>,
    next_nodes: Vec<Vec<(Call, Row, Section)>>,
    lengths: Vec<usize>,
    stage: Stage,
}

impl Table {
    pub fn from_place_not(
        stage: Stage,
        method_pn: &str,
        fixed_bell_chars: &[char],
        call_pns: &[(&str, char, &str)],
        plain_lead_calling_positions: &str,
    ) -> Result<Table, PnBlockParseError> {
        let (plain_course, fixed_bells, ranges, transitions) = Self::ranges_from_place_not(
            stage,
            method_pn,
            fixed_bell_chars,
            call_pns,
            plain_lead_calling_positions,
        )?;

        Ok(Self::new(
            stage,
            plain_course,
            &fixed_bells,
            &ranges,
            transitions,
        ))
    }

    /// A helper function to generate the ranges & transitions for a given method and calls.  This
    /// is made into a helper function so it can be easily tested in isolation.
    fn ranges_from_place_not(
        stage: Stage,
        method_pn: &str,
        fixed_bell_chars: &[char],
        call_pns: &[(&str, char, &str)],
        plain_lead_calling_positions: &str,
    ) -> Result<
        (
            // The method's plain course
            Vec<Row>,
            // The parsed fixed bells
            Vec<Bell>,
            // The ranges of the course
            Vec<Range<usize>>,
            // The transitions between the ranges
            Vec<Vec<Transition>>,
        ),
        PnBlockParseError,
    > {
        /* Parse everything and cache commonly-used values */

        let method = Method::with_lead_end(String::new(), &PnBlock::parse(method_pn, stage)?);
        let fixed_bells = fixed_bell_chars
            .iter()
            .map(|c| Bell::from_name(*c).unwrap())
            .collect_vec();
        let calls = call_pns
            .iter()
            .map(|(pn, sym, calling_positions)| {
                (
                    PlaceNot::parse(pn, stage).unwrap(),
                    *sym,
                    *calling_positions,
                )
            })
            .collect_vec();

        let tenor = Bell::tenor(stage).unwrap();
        let plain_course = method.plain_course();
        let lead_len = method.lead_len();
        let course_len = plain_course.len();
        let lead_end_indices = plain_course
            .annots()
            .enumerate()
            .filter_map(|(i, (_, pos))| {
                pos.filter(|s| s == &proj_core::method::LABEL_LEAD_END)
                    .map(|_| i)
            })
            .collect_vec();

        /* Generate a mapping from fixed bell indices to the lead head and its index, which we'll
         * then use when deciding which calling positions are and aren't allowed. */

        let pc_lead_heads: HashMap<Vec<usize>, (usize, &Row)> = plain_course.annot_rows()
            [..course_len - 1]
            .iter()
            .enumerate()
            .step_by(lead_len)
            .map(|(i, r)| (get_bell_inds(&fixed_bells, r.row()), (i, r.row())))
            .collect();

        /* Generate a map of which calls preserve fixed bells, and what course jump occurs as a
         * result of those calls. */

        let mut call_jumps = Vec::<(usize, usize, Row, Call)>::new();
        for (pn, call_name, calling_positions) in &calls {
            // Test this call at every lead end and check that it keeps the fixed bells in plain
            // coursing order
            for &lead_end_ind in &lead_end_indices {
                // This unsafety is OK because all the rows & pns are parsed within this function,
                // which is provided a single Stage
                let new_lh = unsafe {
                    pn.permute_new_unchecked(plain_course.get_row(lead_end_ind).unwrap())
                };
                // Check that this call hasn't permuted the fixed bells
                if let Some(&(lh_ind, pc_lh)) =
                    pc_lead_heads.get(&get_bell_inds(&fixed_bells, &new_lh))
                {
                    let tenor_place = new_lh.place_of(tenor).unwrap();
                    let call_pos = calling_positions.chars().nth(tenor_place).unwrap();
                    // This unsafety is OK because all the rows & pns are parsed within this
                    // function, which is provided a single Stage
                    let new_course_head = unsafe { pc_lh.tranposition_to_unchecked(&new_lh) };
                    call_jumps.push((
                        lead_end_ind,
                        lh_ind,
                        new_course_head,
                        Call::new(Some(*call_name), call_pos),
                    ));
                }
            }
        }

        /* Use this call mapping to split every course into a set of (not necessarily mutually
         * exclusive) ranges */

        // We have to start a range after every call, and end a range just before that call
        let mut range_starts = call_jumps.iter().map(|(_, to, ..)| *to).collect_vec();
        let mut range_ends = call_jumps.iter().map(|(from, ..)| *from).collect_vec();
        // If a call is omitted, then we can perform an un-jump from one lead end to the
        // consecutive lead head.  These have to be added manually if we have calls which affect
        // but don't permute fix bells (e.g. 4ths place calls in n-ths place methods).
        range_starts.extend(range_ends.iter().map(|&x| (x + 1) % course_len));

        // Sort and deduplicate the starts and ends so that the later algorithms work properly
        range_starts.sort_unstable();
        range_starts.dedup();
        range_ends.sort_unstable();
        range_ends.dedup();

        // Calculate the ranges.  Each of `range_starts` corresponds to a unique range, which ends
        // at the first range_end which is encountered (wrapping round the end of the course if
        // needed).
        let ranges = range_starts
            .iter()
            .map(|&start| {
                let range_end_ind = range_ends.binary_search(&start).unwrap_err();
                let end = *range_ends.get(range_end_ind).unwrap_or(&range_ends[0]);
                start..end
            })
            .collect_vec();

        /* Use the parsed call data to generate which ranges can be joined together. */

        // Conversion table from range starts to their index within `ranges`
        let range_index_by_start = ranges
            .iter()
            .enumerate()
            .map(|(i, r)| (r.start, i))
            .collect::<HashMap<_, _>>();

        let transitions = ranges
            .iter()
            .map(|range| {
                let lead_head_index = (range.end + 1) % course_len;
                let plain_lead_tenor_place = plain_course
                    .get_row(lead_head_index)
                    .unwrap()
                    .place_of(tenor)
                    .unwrap();
                let plain_calling_pos = plain_lead_calling_positions
                    .chars()
                    .nth(plain_lead_tenor_place)
                    .unwrap();

                // Each call which starts at the last row of this range could cause a call
                let mut ts = vec![(
                    Call::new(None, plain_calling_pos),
                    Row::rounds(stage),
                    range_index_by_start[&lead_head_index],
                )];
                ts.extend(
                    call_jumps
                        .iter()
                        .filter(|(from, ..)| *from == range.end)
                        .map(|(_from, to, course_head, name)| {
                            (name.clone(), course_head.clone(), range_index_by_start[to])
                        }),
                );
                ts
            })
            .collect_vec();

        Ok((
            plain_course.rows().cloned().collect_vec(),
            fixed_bells,
            ranges,
            transitions,
        ))
    }

    pub fn new(
        stage: Stage,
        plain_course_rows: Vec<Row>,
        fixed_bells: &[Bell],
        ranges: &[Range<usize>],
        next_nodes: Vec<Vec<Transition>>,
    ) -> Table {
        /* Group rows in each range by the locations of the fixed bells.  By the definition of
         * fixed bells, we only consider falseness between rows which have the fixed bells in the
         * same places. */
        type FalsenessMap<'a> = HashMap<Vec<usize>, Vec<&'a Row>>;

        let grouped_rows: Vec<FalsenessMap> = ranges
            .iter()
            .map(|lead_range| {
                // Group all the rows by the indices of the fixed bells
                let mut rows_by_fixed_bell_indices: FalsenessMap =
                    HashMap::with_capacity(lead_range.len());
                for r in &plain_course_rows[lead_range.clone()] {
                    let fixed_bell_inds = get_bell_inds(fixed_bells, r);
                    rows_by_fixed_bell_indices
                        .entry(fixed_bell_inds)
                        .or_insert_with(Vec::new)
                        .push(r);
                }
                // Return this grouping so it can be combined to generate the falseness table
                rows_by_fixed_bell_indices
            })
            .collect();

        /* Use these grouped rows to iterate over all pairs of ranges and use this to generate a
         * map of ranges are false against which ranges of the plain course. */

        // If (i, j, r) is in this set, then it means that range `i` of the plain course is false
        // against range `j` of the course starting with `r`.
        //
        // Equivalently, if (i, j, r) is in this set then it means that the range `i` of some course
        // `s` is false against the range `j` of the course starting with `s * r`.
        let mut falseness_map: HashSet<(usize, usize, Row)> = HashSet::new();
        // Iterate over every pair of ranges to compute the relative falseness
        for ((i1, map1), (i2, map2)) in grouped_rows
            .iter()
            .enumerate()
            .cartesian_product(grouped_rows.iter().enumerate())
        {
            // If `map1` and `map2` contain entries with the same locations of the fixed bells,
            // then this will cause some transposition of them to be false
            for (fixed_bell_inds, rows1) in map1.iter() {
                if let Some(rows2) = map2.get(fixed_bell_inds) {
                    for (&r1, &r2) in rows1.iter().cartesian_product(rows2.iter()) {
                        // If
                        //      `r1` from range `i1`
                        //    has the same pattern of fixed bells as
                        //      `r2` from range `i2`,
                        // then
                        //      the range `i1` of the plain course
                        //    is false against
                        //      the range `i2` of `r2.tranposition_to(r1)`
                        //
                        //  (i.e. we find `X` where `X * r2 == r1`)
                        let false_course = unsafe { r2.tranposition_to_unchecked(r1) };
                        falseness_map.insert((i1, i2, false_course));
                    }
                }
            }
        }

        // Convert the hash table into a jagged 2D array, indexed by the first element of the tuple
        // (so that the lookups we want to do are faster).
        let mut final_table: Vec<Vec<(Row, Section)>> = vec![Vec::new(); ranges.len()];
        for (i, j, r) in falseness_map {
            final_table[i].push((r, Section::new(j)));
        }

        // Sort the falseness tables
        final_table
            .iter_mut()
            .for_each(|v| v.sort_by(|a, b| (a.1.ind, &a.0).cmp(&(b.1.ind, &b.0))));

        Table {
            falseness: final_table,
            // The `+ 1` corrects for the fact that we are using inclusive ranges (i.e. the first
            // lead of surprise major will be represented as 0..31 not 0..32).
            lengths: ranges.iter().map(|r| r.len() + 1).collect(),
            stage,
            next_nodes: next_nodes
                .into_iter()
                .map(|vs| {
                    vs.into_iter()
                        .map(|(s, r, i)| (s, r, Section::new(i)))
                        .collect()
                })
                .collect(),
        }
    }

    pub fn print_falseness(&self) {
        for (i, secs) in self.falseness.iter().enumerate() {
            println!("{}", i);
            for (r, sec) in secs {
                println!("   {}: {}", sec.ind, r);
            }
        }
    }
}

/// Returns the indices of a set of [`Bells`] within a given [`Row`]
fn get_bell_inds(bells: &[Bell], r: &Row) -> Vec<usize> {
    bells.iter().map(|b| r.place_of(*b).unwrap()).collect_vec()
}
