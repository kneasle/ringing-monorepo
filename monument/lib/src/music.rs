use std::ops::{Add, AddAssign, Sub, SubAssign};

use crate::utils::OptRange;
use bellframe::{music::Regex, Row, RowBuf, Stage, Stroke};
use itertools::Itertools;
use ordered_float::OrderedFloat;
use serde::Deserialize;

pub type Score = OrderedFloat<f32>;

/// A class of music that Monument should care about
#[derive(Debug, Clone)]
pub struct MusicType {
    regexes: Vec<Regex>,
    weight: Score,
    count_range: OptRange,
    non_duffer: bool,
    stroke_set: StrokeSet,
}

impl MusicType {
    pub fn new(
        regexes: Vec<Regex>,
        weight: f32,
        count_range: OptRange,
        non_duffer: bool,
        strokes: StrokeSet,
    ) -> Self {
        Self {
            regexes,
            weight: OrderedFloat(weight),
            count_range,
            non_duffer,
            stroke_set: strokes,
        }
    }

    pub fn count_range(&self) -> OptRange {
        self.count_range
    }

    pub fn non_duffer(&self) -> bool {
        self.non_duffer
    }

    pub fn stroke_set(&self) -> StrokeSet {
        self.stroke_set
    }
}

/// A breakdown of the music generated by a composition
#[derive(Debug, Clone)]
pub struct Breakdown {
    pub score: Score,
    /// The number of occurrences of each [`MusicType`] specified in the current
    /// [`Query`](crate::Query)
    pub counts: Vec<usize>,
}

impl Breakdown {
    /// Creates the `Score` of 0 (i.e. the `Score` generated by no rows).
    pub fn zero(num_music_types: usize) -> Self {
        Self {
            score: Score::from(0.0),
            counts: vec![0; num_music_types],
        }
    }

    /// Returns the `Score` generated by a sequence of [`Row`]s, (pre-)transposed by some course head.
    pub fn from_rows<'r>(
        rows: impl IntoIterator<Item = &'r Row>,
        course_head: &Row,
        music_types: &[MusicType],
        start_stroke: Stroke,
    ) -> Self {
        let mut temp_row = RowBuf::rounds(Stage::ONE);
        let mut occurences = vec![0; music_types.len()];
        // For every (transposed) row ...
        for (idx, r) in rows.into_iter().enumerate() {
            course_head.mul_into(r, &mut temp_row).unwrap();
            // ... for every music type ...
            for (num_instances, ty) in occurences.iter_mut().zip_eq(music_types) {
                if ty.stroke_set.contains(start_stroke.offset(idx)) {
                    // ... count the number of instances of that type of music
                    for regex in &ty.regexes {
                        if regex.matches(&temp_row) {
                            *num_instances += 1;
                        }
                    }
                }
            }
        }

        Self {
            score: occurences
                .iter()
                .zip_eq(music_types)
                .map(|(&num_instances, ty)| ty.weight * num_instances as f32)
                .sum(),
            counts: occurences,
        }
    }

    /// # Panics
    ///
    /// Panics if the number of [`MusicType`]s in `rhs` is different to that of `self`.
    pub fn saturating_sub(&self, rhs: &Self) -> Self {
        Breakdown {
            score: self.score - rhs.score,
            counts: self
                .counts
                .iter()
                .zip_eq(rhs.counts.iter())
                .map(|(a, b)| a.saturating_sub(*b))
                .collect_vec(),
        }
    }

    /// # Panics
    ///
    /// Panics if the number of [`MusicType`]s in `rhs` is different to that of `self`.
    pub fn saturating_sub_assign(&mut self, rhs: &Self) {
        self.score -= rhs.score;
        for (a, b) in self.counts.iter_mut().zip_eq(rhs.counts.iter()) {
            *a = a.saturating_sub(*b);
        }
    }
}

impl Add for &Breakdown {
    type Output = Breakdown;

    /// Combines two [`Score`]s to create one [`Score`] representing both `self` and `rhs`.
    ///
    /// # Panics
    ///
    /// Panics if the number of [`MusicType`]s in `rhs` is different to that of `self`.
    fn add(self, rhs: &Breakdown) -> Self::Output {
        Breakdown {
            score: self.score + rhs.score,
            counts: self
                .counts
                .iter()
                .zip_eq(rhs.counts.iter())
                .map(|(a, b)| a + b)
                .collect_vec(),
        }
    }
}

impl AddAssign<&Breakdown> for Breakdown {
    /// Combines the scores from another [`Score`] into `self` (so that `self` now represents the
    /// score generated by `self` and the RHS).
    ///
    /// # Panics
    ///
    /// Panics if the number of [`MusicType`]s in `rhs` is different to that of `self`.
    fn add_assign(&mut self, rhs: &Breakdown) {
        self.score += rhs.score;
        for (a, b) in self.counts.iter_mut().zip_eq(rhs.counts.iter()) {
            *a += *b;
        }
    }
}

impl Sub for &Breakdown {
    type Output = Breakdown;

    /// Combines two [`Score`]s to create one [`Score`] representing both `self` and `rhs`.
    ///
    /// # Panics
    ///
    /// Panics if the number of [`MusicType`]s in `rhs` is different to that of `self`.
    fn sub(self, rhs: &Breakdown) -> Self::Output {
        Breakdown {
            score: self.score - rhs.score,
            counts: self
                .counts
                .iter()
                .zip_eq(rhs.counts.iter())
                .map(|(a, b)| a - b)
                .collect_vec(),
        }
    }
}

impl SubAssign<&Breakdown> for Breakdown {
    /// Combines the scores from another [`Score`] into `self` (so that `self` now represents the
    /// score generated by `self` and the RHS).
    ///
    /// # Panics
    ///
    /// Panics if the number of [`MusicType`]s in `rhs` is different to that of `self`.
    fn sub_assign(&mut self, rhs: &Breakdown) {
        self.score -= rhs.score;
        for (a, b) in self.counts.iter_mut().zip_eq(rhs.counts.iter()) {
            *a -= *b;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StrokeSet {
    Hand,
    Back,
    Both,
}

impl StrokeSet {
    fn contains(self, stroke: Stroke) -> bool {
        match (self, stroke) {
            (Self::Both, _) | (Self::Hand, Stroke::Hand) | (Self::Back, Stroke::Back) => true,
            (Self::Hand, Stroke::Back) | (Self::Back, Stroke::Hand) => false,
        }
    }
}

impl Default for StrokeSet {
    fn default() -> Self {
        StrokeSet::Both
    }
}
