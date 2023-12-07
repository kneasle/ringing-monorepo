//! A representation of a [`Block`] of ringing; i.e. a sort of 'multi-permutation' which takes a
//! starting [`Row`] and yields a sequence of permuted [`Row`]s.

use std::{
    collections::HashSet,
    fmt::{Debug, Display, Formatter},
    iter::repeat_with,
    ops::RangeBounds,
    slice,
};

use itertools::Itertools;

use crate::{
    row::{same_stage_vec, DbgRow},
    utils, Bell, Row, RowBuf, SameStageVec, Stage, Truth,
};

/// A block of [`Row`], each of which can be given an annotation of any type.  Blocks can start
/// from any [`Row`], and can be empty.
///
/// All blocks must finish with a 'left-over' [`Row`].  This [`Row`] denotes the first [`Row`] of
/// any block rung **after** this one.  This is not considered part of the `Block`, and
/// therefore cannot be annotated.  However, it is necessary - for example, if we create a `Block`
/// for the first lead of Cambridge and Primrose Surprise Minor then they would be identical except
/// for their 'left-over' row.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct Block<A> {
    /// The [`Row`]s making up this `Block`.
    ///
    /// **Invariant**: `row.len() >= 1`
    rows: SameStageVec,
    /// The annotations on each [`Row`] in this `Block`.
    ///
    /// **Invariant**: `rows.len() = annots.len() + 1`, because the 'left-over' row cannot be annotated.
    annots: Vec<A>,
}

impl<A> Block<A> {
    //////////////////
    // CONSTRUCTORS //
    //////////////////

    /// Parse a multi-line [`str`]ing into an `Block`, where each row is given the annotation
    /// created by `A::default()`.  Each line in the string is interpreted as a [`Row`], with the
    /// last row being 'left-over'.  The [`Stage`] is inferred from the first line.
    pub fn parse(s: &str) -> Result<Self, ParseError>
    where
        A: Default,
    {
        let rows = SameStageVec::parse(s).map_err(ParseError::Other)?;
        if rows.is_empty() {
            // I'm not sure if this branch is even possible, since a zero-line string is
            // impossible and `SameStageVec` attempts to parse every line as a [`Row`].  But for
            // safety, it's here anyway
            Err(ParseError::ZeroLengthBlock)
        } else {
            Ok(Self::with_default_annots(rows))
        }
    }

    /// Creates a new `Block` from a [`SameStageVec`], where every annotation is
    /// `A::default()`.
    ///
    /// # Panics
    ///
    /// This panics if the [`SameStageVec`] provided is empty.
    pub fn with_default_annots(rows: SameStageVec) -> Self
    where
        A: Default,
    {
        assert!(!rows.is_empty());
        Self {
            annots: repeat_with(A::default).take(rows.len() - 1).collect_vec(),
            rows,
        }
    }

    /// Creates a new [`Block`] with no annotated [`Row`], and a leftover [`Row`] of
    /// [`RowBuf::rounds`].
    pub fn empty(stage: Stage) -> Self {
        Self::with_leftover_row(RowBuf::rounds(stage))
    }

    /// Creates a new [`Block`] with no annotated [`Row`], and a given leftover [`Row`].
    pub fn with_leftover_row(leftover_row: RowBuf) -> Self {
        Self {
            rows: SameStageVec::from_row_buf(leftover_row),
            annots: vec![], // No annotations
        }
    }

    /// Create an [`Block`] from a [`SameStageVec`] of [`Row`]s, where each annotation is
    /// generated by calling a closure on its index within this block.  Returns `None` if the
    /// [`SameStageVec`] contained no [`Row`]s.
    pub fn with_annots_from_indices(rows: SameStageVec, f: impl FnMut(usize) -> A) -> Option<Self> {
        let length_of_self = rows.len().checked_sub(1)?;
        Some(Self {
            annots: (0..length_of_self).map(f).collect_vec(),
            rows,
        })
    }

    /////////////////
    // STAGE & LEN //
    /////////////////

    /// Gets the [`Stage`] of this `Block`.
    #[inline]
    pub fn stage(&self) -> Stage {
        self.rows.stage()
    }

    /// Gets the effective [`Stage`] of this `Block` - i.e. the smallest [`Stage`] that this
    /// `Block` can be reduced to without producing invalid [`Row`]s.  See
    /// [`Row::effective_stage`] for more info and examples.
    pub fn effective_stage(&self) -> Stage {
        self.all_rows()
            .map(Row::effective_stage)
            .max()
            // Unwrapping here is safe, because blocks must contain at least one Row
            .unwrap()
    }

    /// Shorthand for `self.len() == 0`
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.annots.is_empty()
    }

    /// Gets the length of this `Block` (excluding the left-over [`Row`]).
    #[inline]
    pub fn len(&self) -> usize {
        self.annots.len()
    }

    /////////////
    // GETTERS //
    /////////////

    /// Gets the [`Row`] at a given index, along with its annotation.
    #[inline]
    pub fn get_row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index)
    }

    /// Gets an immutable reference to the annotation of the [`Row`] at a given index, if it
    /// exists.
    #[inline]
    pub fn get_annot(&self, index: usize) -> Option<&A> {
        self.annots.get(index)
    }

    /// Gets an mutable reference to the annotation of the [`Row`] at a given index, if it
    /// exists.
    #[inline]
    pub fn get_annot_mut(&mut self, index: usize) -> Option<&mut A> {
        self.annots.get_mut(index)
    }

    /// Gets the [`Row`] at a given index, along with its annotation.
    #[inline]
    pub fn get_annot_row(&self, index: usize) -> Option<(&A, &Row)> {
        Some((self.get_annot(index)?, self.get_row(index)?))
    }

    /// Gets the first [`Row`] of this `Block`, which may be leftover.
    #[inline]
    pub fn first_row(&self) -> &Row {
        // This `unwrap` won't panic, because we require an invariant that `self.row_buffer` is
        // non-empty
        self.rows.first().unwrap()
    }

    /// Gets the first [`Row`] of this `Block`, along with its annotation.
    #[inline]
    pub fn first_annot_row(&self) -> Option<(&A, &Row)> {
        self.get_annot_row(0)
    }

    /// Returns an immutable reference to the 'left-over' [`Row`] of this `Block`.  This [`Row`]
    /// represents the overall transposition of the `Block`, and should not be used when generating
    /// rows for truth checking.
    #[inline]
    pub fn leftover_row(&self) -> &Row {
        self.rows.last().unwrap()
    }

    /// Returns a mutable reference to the 'left-over' [`Row`] of this `Block`.  This [`Row`]
    /// represents the overall transposition of the `Block`, and should not be used when generating
    /// rows for truth checking.
    #[inline]
    pub fn leftover_row_mut(&mut self) -> &mut Row {
        self.rows.last_mut().unwrap()
    }

    #[inline]
    pub fn is_true(&self) -> bool {
        self.truth().is_true()
    }

    /// Returns whether a row is repeated within this [`Block`] (not including the
    /// [leftover row](Self::leftover_row))
    pub fn truth(&self) -> Truth {
        let mut rows_so_far = HashSet::<&Row>::with_capacity(self.len());
        for row in self.rows() {
            if !rows_so_far.insert(row) {
                return Truth::False; // Row has repeated
            }
        }
        Truth::True // If no rows repeated, composition is true
    }

    //////////////////////////////
    // ITERATORS / PATH GETTERS //
    //////////////////////////////

    /// Returns an [`Iterator`] which yields the [`Row`]s which are directly part of this
    /// `Block`.  This does not include the 'left-over' row; if you want to include the
    /// left-over [`Row`], use [`Block::all_rows`] instead.
    #[inline]
    pub fn rows(&self) -> same_stage_vec::Iter {
        let mut iter = self.all_rows();
        iter.next_back(); // Remove the leftover row
        iter
    }

    /// Returns an [`Iterator`] which yields all the [`Row`]s, including the leftover [`Row`].
    #[inline]
    pub fn all_rows(&self) -> same_stage_vec::Iter {
        self.rows.iter()
    }

    /// Returns an [`Iterator`] which yields the annotations of this [`Block`], in
    /// sequential order.
    #[inline]
    pub fn annots(&self) -> std::slice::Iter<A> {
        self.annots.iter()
    }

    /// Returns an [`Iterator`] which yields the [`Row`]s which are directly part of this
    /// `Block`.  This does not include the 'left-over' row; if you want to include the
    /// left-over [`Row`], use [`Block::all_annot_rows`] instead.
    #[inline]
    pub fn annot_rows(&self) -> impl Iterator<Item = (&A, &Row)> + Clone {
        self.annots().zip_eq(self.rows())
    }

    /// Returns an [`Iterator`] which yields the [`Row`]s which are directly part of this
    /// `Block`.  This **does** include the 'left-over' row, which will have an annotation of
    /// [`None`].
    #[inline]
    pub fn all_annot_rows(&self) -> impl Iterator<Item = (Option<&A>, &Row)> + Clone {
        self.annots()
            .map(Some)
            .chain(std::iter::once(None))
            .zip_eq(self.all_rows())
    }

    /// Returns the places of a given [`Bell`] in each [`Row`] of this `Block`.  Also returns
    /// the place of `bell` in the leftover row.
    pub fn path_of(&self, bell: Bell) -> (Vec<usize>, usize) {
        let mut full_path = self.full_path_of(bell);
        let place_in_leftover_row = full_path.pop().unwrap();
        (full_path, place_in_leftover_row)
    }

    /// Returns the places of a given [`Bell`] in each [`Row`] of this `Block`, **including**
    /// the leftover row.
    pub fn full_path_of(&self, bell: Bell) -> Vec<usize> {
        self.rows.path_of(bell) // Delegate to `SameStageVec`
    }

    /// An [`Iterator`] which yields the annotated [`Row`]s of `self` repeated forever.  This
    /// [`Iterator`] never terminates.
    ///
    /// [`RepeatIter`] is an [`Iterator`] which yields [`RowBuf`]s, causing an allocation for each
    /// call to `next`.  If those allocations are not desired, then use [`RepeatIter::next_into`]
    /// instead to re-use an existing allocation.
    pub fn repeat_iter(&self, start_row: RowBuf) -> RepeatIter<A> {
        RepeatIter::new(start_row, self)
    }

    /////////////////////////
    // IN-PLACE OPERATIONS //
    /////////////////////////

    /// Pre-multiplies every [`Row`] in this `Block` in-place by another [`Row`], whilst preserving
    /// the annotations.
    pub fn pre_multiply(&mut self, lhs_row: &Row) {
        self.rows.pre_multiply(lhs_row) // Delegate to `SameStageVec`
    }

    /// Extends `self` with the contents of another [`Block`], **pre-multiplying** its [`Row`]s so
    /// that it starts with `self`'s [`leftover_row`](Self::leftover_row).
    pub fn extend(&mut self, other: &Self)
    where
        A: Clone,
    {
        self.extend_range(other, ..)
    }

    /// Extends `self` with a region of another [`Block`], **pre-multiplying** its [`Row`]s so
    /// that it starts with `self`'s [`leftover_row`](Self::leftover_row).
    ///
    /// # Panics
    ///
    /// Panics if `range` explicitly states a bound outside `other.len()`
    pub fn extend_range(&mut self, other: &Self, range: impl RangeBounds<usize>)
    where
        A: Clone,
    {
        let range = utils::clamp_range(range, other.len());

        // `transposition` pre-multiplies `other`'s first row to `self`'s leftover row
        let transposition =
            Row::solve_xa_equals_b(other.get_row(range.start).unwrap(), self.leftover_row());

        // Add the transposed rows to `self`
        self.rows.pop(); // If we don't remove the leftover row, then it will get added twice
        self.rows
            .extend_range_transposed(&transposition, &other.rows, range.start..range.end + 1);

        // Add the annotations to `self`
        self.annots.extend_from_slice(&other.annots[range]);
    }

    /// Extends `self` with a chunk of itself, transposed to start with `self.leftover_row()`.
    pub fn extend_from_within(&mut self, range: impl RangeBounds<usize> + Debug + Clone)
    where
        A: Clone + Debug,
    {
        // Clamp open ranges to `0..self.len()`, so our code only has to handle concrete `Range`s
        let range = utils::clamp_range(range, self.len());

        // Compute the pre-transposition required to make the chunk start at `self.leftover_row`
        let first_row_of_chunk = self.get_row(range.start).unwrap();
        // This unwrap is fine because stages must match because both rows were taken from the same
        // `SameStageVec`
        let transposition = Row::solve_xa_equals_b(first_row_of_chunk, self.leftover_row());

        // Extend the rows
        self.rows
            // The range is offset by 1 to exclude the current `leftover_row` of `self`, but
            // include the new leftover row
            .extend_transposed_from_within(range.start + 1..range.end + 1, &transposition);
        self.annots.extend_from_within(range);
    }

    ///////////////////////////////////
    // OPERATIONS WHICH CONSUME SELF //
    ///////////////////////////////////

    /// Consumes this `Block`, and returns a [`SameStageVec`] containing the same [`Row`]s,
    /// **including** the left-over row.
    pub fn into_rows(self) -> SameStageVec {
        self.rows
    }

    /// Borrows the [`SameStageVec`] which contains all the [`Row`]s in this `Block`
    pub fn row_vec(&self) -> &SameStageVec {
        &self.rows
    }

    /// Convert this `Block` into another `Block` with identical [`Row`]s, but where each
    /// annotation is passed through the given function.
    pub fn map_annots<B>(self, f: impl Fn(A) -> B) -> Block<B> {
        Block {
            rows: self.rows, // Don't modify the rows
            annots: self.annots.into_iter().map(f).collect_vec(),
        }
    }

    /// Convert this `Block` into another `Block` with identical [`Row`]s, but where each
    /// annotation is passed through the given function (along with its index within the
    /// `Block`).
    pub fn map_annots_with_index<B>(self, f: impl Fn(usize, A) -> B) -> Block<B> {
        Block {
            rows: self.rows, // Don't modify the rows
            annots: self
                .annots
                .into_iter()
                .enumerate()
                .map(|(i, annot)| f(i, annot))
                .collect_vec(),
        }
    }

    /// Convert this `Block` into another `Block` with identical [`Row`]s, but where each
    /// annotation is passed through the given function (along with its index within the
    /// `Block`).
    pub fn clone_map_annots_with_index<'s, B>(&'s self, f: impl Fn(usize, &'s A) -> B) -> Block<B> {
        Block {
            rows: self.rows.to_owned(), // Don't modify the rows
            annots: self
                .annots
                .iter()
                .enumerate()
                .map(|(i, annot)| f(i, annot))
                .collect_vec(),
        }
    }

    /// Splits this `Block` into two separate `Block`s at a specified index.  This is
    /// defined such the first `Block` has length `index`.  This returns `None` if the second
    /// `Block` would have negative length.
    pub fn split(self, index: usize) -> Option<(Self, Self)> {
        let (first_annots, second_annots) = utils::split_vec(self.annots, index)?;
        let (mut first_rows, second_rows) = self.rows.split(index)?;
        // Copy the first row of `second_rows` back into `first_rows` so it becomes the leftover
        // row of the first block
        let first_row_of_second = second_rows.first().unwrap(); // Unwrap is safe because
                                                                // `self.row_buffer.len() > index + 1`
        first_rows.push(first_row_of_second); // Won't panic because both rows came from `self.row_buffer`

        // Construct the new pair of blocks
        let first_block = Self {
            rows: first_rows,
            annots: first_annots,
        };
        let second_block = Self {
            rows: second_rows,
            annots: second_annots,
        };
        Some((first_block, second_block))
    }
}

////////////////
// FORMATTING //
////////////////

impl<T> Debug for Block<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        /// Struct which debug prints as `K: V`
        struct Pair<K, V>(K, V);

        impl<K, V> Debug for Pair<K, V>
        where
            K: Debug,
            V: Debug,
        {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                write!(f, "{:?}: {:?}", self.0, self.1)
            }
        }

        let mut builder = f.debug_tuple(stringify!(Block));
        for (annot, row) in self.annot_rows() {
            // Format all annotated rows as `row: annot`
            builder.field(&Pair(DbgRow(row), annot));
        }
        // Format the leftover row as `row` (since it has no annotation)
        builder.field(&DbgRow(self.leftover_row()));
        builder.finish()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

/// Iterator used internally by [`RepeatIter`], which yields the (&Row, &annotation) pairs from an
/// [`Block`].
type InnerIter<'b, A> =
    std::iter::Enumerate<itertools::ZipEq<same_stage_vec::Iter<'b>, slice::Iter<'b, A>>>;

/// An [`Iterator`] which yields an [`Block`] forever.  Created with
/// [`Block::repeat_iter`].
#[derive(Clone)]
pub struct RepeatIter<'b, A> {
    /// Invariant: `self.current_block_head` must have the same [`Stage`] as `self.block`
    current_block_head: RowBuf,
    iter: InnerIter<'b, A>,
    block: &'b Block<A>,
}

impl<'b, A> RepeatIter<'b, A> {
    fn new(start_row: RowBuf, block: &'b Block<A>) -> Self {
        Self {
            current_block_head: start_row,
            iter: Self::get_iter(block),
            block,
        }
    }

    /// Same as `self.next` but re-uses an existing allocation.
    #[must_use]
    pub fn next_into(&mut self, out: &mut RowBuf) -> Option<(usize, &'b A)> {
        let (source_idx, (untransposed_row, annot)) = self.next_untransposed_row()?;
        self.current_block_head.mul_into(untransposed_row, out);
        Some((source_idx, annot))
    }

    fn get_iter(block: &'b Block<A>) -> InnerIter<'b, A> {
        block.rows().zip_eq(block.annots()).enumerate()
    }

    fn next_untransposed_row(&mut self) -> Option<(usize, (&'b Row, &'b A))> {
        self.iter.next().or_else(|| {
            // Apply the block we've just finished to the block head
            self.current_block_head = self.current_block_head.as_row() * self.block.leftover_row();
            // Start a new block
            self.iter = Self::get_iter(self.block);
            // Get the first row/annot of the next block.  If `self.iter.next()` is None, then the
            // block must be empty, and `self` should finish immediately
            self.iter.next()
        })
    }
}

impl<'b, A> Iterator for RepeatIter<'b, A> {
    type Item = (RowBuf, usize, &'b A);

    fn next(&mut self) -> Option<Self::Item> {
        let (source_idx, (untransposed_row, annot)) = self.next_untransposed_row()?;
        let next_row = self.current_block_head.as_row() * untransposed_row;
        Some((next_row, source_idx, annot))
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

/// The possible ways that [`Block::parse`] could fail
#[derive(Debug, Clone)]
pub enum ParseError {
    ZeroLengthBlock,
    Other(same_stage_vec::ParseError),
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::ZeroLengthBlock => write!(f, "Blocks must contain at least one row"),
            ParseError::Other(inner) => write!(f, "{}", inner),
        }
    }
}

impl std::error::Error for ParseError {}
