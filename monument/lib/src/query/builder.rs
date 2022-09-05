//! Code for the [`QueryBuilder`] API for creating [`Query`]s.

use std::{collections::HashMap, ops::RangeInclusive};

use bellframe::{
    method::LABEL_LEAD_END,
    method_lib::QueryError,
    music::{Elem, Pattern},
    Mask, MethodLib, PlaceNot, RowBuf, Stage, Stroke,
};
use hmap::hmap;
use itertools::Itertools;
use ordered_float::OrderedFloat;

use crate::{
    group::PartHeadGroup,
    search::{Config, Search, Update},
    utils::{Score, TotalLength},
    Composition,
};

use super::{
    CallDisplayStyle, CallVec, MethodId, MethodVec, MusicType, MusicTypeVec,
    OptionalRangeInclusive, Query, SpliceStyle, StrokeSet,
};

#[allow(unused_imports)] // Only used for doc comments
use bellframe::Row;

/// Builder API for creating [`Query`]s.
pub struct QueryBuilder {
    /* GENERAL */
    length_range: RangeInclusive<TotalLength>,
    pub num_comps: usize,
    pub require_truth: bool,
    stage: Stage,

    /* METHODS */
    methods: MethodVec<(bellframe::Method, MethodBuilder)>,
    pub default_method_count: OptionalRangeInclusive,
    pub default_start_indices: Vec<isize>,
    pub default_end_indices: Option<Vec<isize>>, // `None` means 'any index'
    pub splice_style: SpliceStyle,
    pub splice_weight: f32,

    /* CALLS */
    pub base_calls: Option<BaseCalls>,
    pub custom_calls: Vec<CallBuilder>,
    pub call_display_style: CallDisplayStyle,

    /* MUSIC */
    music_types: MusicTypeVec<super::MusicType>,
    pub start_stroke: Stroke,

    /* COURSES */
    pub start_row: RowBuf,
    pub end_row: RowBuf,
    pub part_head: RowBuf,
    pub courses: Option<Vec<Mask>>,
    pub course_weights: Vec<(Mask, f32)>,
}

impl QueryBuilder {
    /* START */

    /// Start building a [`Query`] with the given [`MethodBuilder`]s and [`Length`] range.  The
    /// [`MethodBuilder`]s will be assigned unique [`MethodId`]s, but you won't know which ones
    /// apply to which unless you build a [`MethodSet`] and use [`QueryBuilder::with_method_set`].
    pub fn with_methods(
        methods: impl IntoIterator<Item = MethodBuilder>,
        length: Length,
    ) -> crate::Result<Self> {
        let method_set = MethodSet {
            vec: methods.into_iter().collect(),
        };
        Self::with_method_set(method_set, length)
    }

    /// Create a new `QueryBuilder` with the given [`MethodBuilder`]s and [`Length`] range.
    pub fn with_method_set(method_set: MethodSet, length: Length) -> crate::Result<Self> {
        let method_builders = method_set.vec;

        // Process methods
        if method_builders.is_empty() {
            return Err(crate::Error::NoMethods);
        }
        let cc_lib = bellframe::MethodLib::cc_lib().expect("Can't load Central Council library.");
        let mut methods = MethodVec::new();
        let mut stage = Stage::ONE;
        for method_builder in method_builders {
            let bellframe_method = method_builder.bellframe_method(&cc_lib)?;
            stage = stage.max(bellframe_method.stage());
            methods.push((bellframe_method, method_builder));
        }
        // Process length
        let (min_length, max_length) = match length {
            Length::Practice => (0, 300),
            Length::QuarterPeal => (1250, 1350),
            Length::HalfPeal => (2500, 2600),
            Length::Peal => (5000, 5200),
            Length::Range(range) => (*range.start(), *range.end()),
        };
        let length_range = TotalLength::new(min_length)..=TotalLength::new(max_length);

        Ok(QueryBuilder {
            stage,
            length_range,
            num_comps: 100,
            require_truth: true,

            methods,
            default_method_count: OptionalRangeInclusive::OPEN,
            default_start_indices: vec![0], // Just 'normal' starts by default
            default_end_indices: None,
            splice_style: SpliceStyle::LeadLabels,
            splice_weight: 0.0,

            base_calls: Some(BaseCalls::default()),
            custom_calls: vec![],
            call_display_style: CallDisplayStyle::CallingPositions(stage.tenor()),

            music_types: MusicTypeVec::new(),
            start_stroke: Stroke::Hand,

            start_row: RowBuf::rounds(stage),
            end_row: RowBuf::rounds(stage),
            part_head: RowBuf::rounds(stage),
            courses: None, // Defer setting the defaults until we know the part head
            course_weights: vec![],
        })
    }

    /// Gets the [`Stage`] of the [`Query`] being built.  All extra components ([`Mask`]s,
    /// [`Row`]s, etc.) must all have this [`Stage`], or the builder will panic.
    pub fn get_stage(&self) -> Stage {
        self.stage
    }

    /* SETTERS */

    /// Add [`MusicTypeBuilder`]s from the given [`Iterator`].
    pub fn music_types(mut self, music_types: impl IntoIterator<Item = MusicTypeBuilder>) -> Self {
        self.music_types
            .extend(music_types.into_iter().map(|ty| ty.music_type));
        self
    }

    /// Adds [`course_weights`](Self::course_weights) representing every handbell pair coursing,
    /// each with the given `weight`.
    pub fn handbell_coursing_weight(mut self, weight: f32) -> Self {
        if weight == 0.0 {
            return self; // Ignore weights of zero
        }
        // For every handbell pair ...
        for right_bell in self.stage.bells().step_by(2) {
            let left_bell = right_bell + 1;
            // ... add patterns for `*<left><right>` and `*<right><left>`
            for (b1, b2) in [(left_bell, right_bell), (right_bell, left_bell)] {
                let pattern =
                    Pattern::from_elems([Elem::Star, Elem::Bell(b1), Elem::Bell(b2)], self.stage)
                        .expect("Handbell patterns should always be valid regexes");
                let mask = Mask::from_pattern(&pattern)
                    .expect("Handbell patterns should only have one `*`");
                self.course_weights.push((mask, weight));
            }
        }
        self
    }

    /* FINISH */

    /// Finish building and run the search with the default [`Config`], blocking until the required
    /// compositions have been generated.
    pub fn run(self) -> crate::Result<Vec<Composition>> {
        self.run_with_config(Config::default())
    }

    /// Finish building and run the search with a custom [`Config`], blocking until the required
    /// number of [`Composition`]s have been generated.
    pub fn run_with_config(self, config: Config) -> crate::Result<Vec<Composition>> {
        let mut comps = Vec::<Composition>::new();
        let update_fn = |update| {
            if let Update::Comp(comp) = update {
                comps.push(comp);
            }
        };
        Search::new(self.build()?, config)?.run(update_fn);
        Ok(comps)
    }

    /// Finish building and return the [`Query`].
    pub fn build(self) -> crate::Result<Query> {
        let Self {
            stage,
            length_range,
            num_comps,
            require_truth,

            methods,
            default_method_count,
            default_start_indices,
            default_end_indices,
            splice_style,
            splice_weight,

            base_calls,
            custom_calls,
            call_display_style,

            music_types,
            start_stroke,

            start_row,
            end_row,
            part_head,
            courses,
            course_weights,
        } = self;

        // If no courses are set, fix any bell >=7 which aren't affected by the part head.  Usually
        // this will be either all (e.g. 1-part or a part head of `1342` or `124365`) or all (e.g.
        // cyclic), but any other combinations are possible.  E.g. a composition of Maximus with
        // part head of `1765432` will still preserve 8 through 12
        let courses = courses.unwrap_or_else(|| {
            let tenors_unaffected_by_part_head =
                stage.bells().skip(6).filter(|&b| part_head.is_fixed(b));
            vec![Mask::with_fixed_bells(
                stage,
                tenors_unaffected_by_part_head,
            )]
        });

        // Convert each `MethodBuilder` into a `query::Method`, falling back on any defaults when
        // unspecified.
        let mut built_methods = MethodVec::new();
        for (mut bellframe_method, method_builder) in methods {
            // Set lead locations for the `method`
            for (label, indices) in method_builder.lead_labels {
                for index in indices {
                    let lead_len_i = bellframe_method.lead_len() as isize;
                    let index = ((index % lead_len_i) + lead_len_i) % lead_len_i;
                    bellframe_method.add_label(index as usize, label.clone());
                }
            }
            // Parse the courses
            let courses = match method_builder.override_courses {
                Some(unparsed_courses) => unparsed_courses
                    .into_iter()
                    .map(|mask_str| {
                        Mask::parse_with_stage(&mask_str, stage).map_err(|error| {
                            crate::Error::CustomCourseMaskParse {
                                method_title: bellframe_method.title().to_owned(),
                                mask_str,
                                error,
                            }
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                None => courses.clone(),
            };
            // Add the method, falling back on any defaults if necessary
            built_methods.push(super::Method {
                shorthand: method_builder
                    .custom_shorthand
                    .unwrap_or_else(|| default_shorthand(bellframe_method.title())),
                count_range: method_builder.override_count_range.or(default_method_count),
                start_indices: method_builder
                    .override_start_indices
                    .unwrap_or_else(|| default_start_indices.clone()),
                end_indices: method_builder
                    .override_end_indices
                    .or_else(|| default_end_indices.clone()),
                courses,

                inner: bellframe_method,
            });
        }

        // Calls
        let mut calls = CallVec::new();
        if let Some(base_calls) = base_calls {
            calls.extend(base_calls.into_calls(stage));
        }
        calls.extend(custom_calls.into_iter().map(CallBuilder::build));

        Ok(Query {
            length_range,
            stage,
            num_comps,
            require_truth,

            methods: built_methods,
            splice_style,
            splice_weight: OrderedFloat(splice_weight),
            calls,
            call_display_style,

            start_row,
            end_row,
            part_head_group: PartHeadGroup::new(&part_head),
            course_weights: course_weights
                .into_iter()
                .map(|(mask, weight)| (mask, OrderedFloat(weight)))
                .collect(),
            music_types: music_types.into_iter().collect(),
            start_stroke,
        })
    }
}

/////////////
// METHODS //
/////////////

const NUM_METHOD_SUGGESTIONS: usize = 10;

/// Builder API for a method.
pub struct MethodBuilder {
    source: MethodSource,
    // TODO: Add builder API for these fields
    pub lead_labels: HashMap<String, Vec<isize>>,

    pub custom_shorthand: Option<String>,
    pub override_count_range: OptionalRangeInclusive,
    pub override_start_indices: Option<Vec<isize>>,
    pub override_end_indices: Option<Vec<isize>>,
    pub override_courses: Option<Vec<String>>, // TODO: Allow these to be pre-parsed
}

impl MethodBuilder {
    /// Create a new [`MethodBuilder`] by loading a method from the Central Council library by its
    /// [`title`](bellframe::Method::title).
    pub fn with_title(title: String) -> Self {
        Self::new(MethodSource::Title(title))
    }

    /// Create a new [`MethodBuilder`] with custom place notation.
    pub fn with_custom_pn(name: String, pn_str: String, stage: Stage) -> Self {
        Self::new(MethodSource::CustomPn {
            name,
            pn_str,
            stage,
        })
    }

    fn new(source: MethodSource) -> Self {
        Self {
            source,
            lead_labels: hmap! { LABEL_LEAD_END.to_owned() => vec![0] },

            custom_shorthand: None,
            override_count_range: OptionalRangeInclusive::OPEN,
            override_start_indices: None,
            override_end_indices: None,
            override_courses: None,
        }
    }

    /// Force use of a specific shorthand for this [`MethodBuilder`].  By default, the
    /// first character of the method's title will be used as a shorthand.
    pub fn shorthand(mut self, shorthand: String) -> Self {
        self.custom_shorthand = Some(shorthand);
        self
    }

    /// Override the globally set [`method_count`](QueryBuilder::default_method_count) for just this
    /// method.
    pub fn count_range(mut self, range: OptionalRangeInclusive) -> Self {
        self.override_count_range = range;
        self
    }

    /// Override the globally set [`start_indices`](QueryBuilder::default_start_indices) for just
    /// this method.  As with [`QueryBuilder::default_start_indices`], these indices are taken
    /// modulo the method's lead length.
    pub fn start_indices(mut self, indices: Vec<isize>) -> Self {
        self.override_start_indices = Some(indices);
        self
    }

    /// Override the globally set [`end_indices`](QueryBuilder::default_end_indices) for just this
    /// method.  As with [`QueryBuilder::default_end_indices`], these indices are taken modulo the
    /// method's lead length.
    pub fn end_indices(mut self, indices: Vec<isize>) -> Self {
        self.override_end_indices = Some(indices);
        self
    }

    /// Override [`QueryBuilder::courses`] to specify which courses are allowed for this method.
    pub fn courses(mut self, courses: Vec<String>) -> Self {
        self.override_courses = Some(courses);
        self
    }

    /// Applies labels to [`Row`] indices within every lead of this method.  By default,
    /// [`bellframe::method::LABEL_LEAD_END`] is at index `0` (i.e. the lead end).
    pub fn lead_labels(mut self, labels: HashMap<String, Vec<isize>>) -> Self {
        self.lead_labels = labels;
        self
    }

    /// Finishes building and adds `self` to the supplied [`MethodSet`], returning the [`MethodId`]
    /// now used to refer to this [`MethodBuilder`].
    ///
    /// This is equivalent to [`MethodSet::add`], but makes for cleaner code.
    // TODO: Code example
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, set: &mut MethodSet) -> MethodId {
        set.add(self)
    }

    /// Create a [`bellframe::Method`] representing this method.
    fn bellframe_method(&self, cc_lib: &MethodLib) -> crate::Result<bellframe::Method> {
        match &self.source {
            MethodSource::Title(title) => cc_lib
                .get_by_title_with_suggestions(title, NUM_METHOD_SUGGESTIONS)
                .map_err(|err| match err {
                    QueryError::PnParseErr { pn, error } => panic!(
                        "Invalid pn `{}` in CC library entry for {}: {}",
                        pn, title, error
                    ),
                    QueryError::NotFound(suggestions) => crate::Error::MethodNotFound {
                        title: title.to_owned(),
                        suggestions,
                    },
                }),
            MethodSource::CustomPn {
                name,
                pn_str: place_notation_string,
                stage,
            } => bellframe::Method::from_place_not_string(
                name.to_owned(),
                *stage,
                place_notation_string,
            )
            .map_err(|error| crate::Error::MethodPnParse {
                name: name.to_owned(),
                place_notation_string: place_notation_string.to_owned(),
                error,
            }),
        }
    }
}

enum MethodSource {
    /// A method with this title should be found in the Central Council's method library.
    Title(String),
    /// The method should be loaded from some custom place notation
    CustomPn {
        name: String,
        pn_str: String,
        stage: Stage, // TODO: Make stage optional
    },
}

/// Get a default shorthand given a method's title.
fn default_shorthand(title: &str) -> String {
    title
        .chars()
        .next()
        .expect("Can't have empty method title")
        .to_string()
}

/// A set of methods used by a [`Query`].
pub struct MethodSet {
    vec: MethodVec<MethodBuilder>,
}

impl MethodSet {
    /// Creates a `MethodSet` containing no methods.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a [`MethodBuilder`] to this set, returning its unique [`MethodId`].
    ///
    /// You can also use [`MethodBuilder::add`], which usually makes for cleaner code.
    pub fn add(&mut self, method: MethodBuilder) -> MethodId {
        let index = self.vec.push(method);
        MethodId { index }
    }
}

impl Default for MethodSet {
    fn default() -> Self {
        Self {
            vec: MethodVec::new(),
        }
    }
}

impl<I> From<I> for MethodSet
where
    I: IntoIterator<Item = MethodBuilder>,
{
    fn from(iter: I) -> Self {
        Self {
            vec: iter.into_iter().collect(),
        }
    }
}

///////////
// CALLS //
///////////

// TODO: More negative weights to calls
pub const DEFAULT_BOB_WEIGHT: f32 = -1.8;
pub const DEFAULT_SINGLE_WEIGHT: f32 = -2.3;
pub const DEFAULT_MISC_CALL_WEIGHT: f32 = -3.0;

pub struct CallBuilder {
    symbol: String,
    debug_symbol: Option<String>,
    calling_positions: Option<Vec<String>>,

    label_from: String,
    label_to: String,
    place_notation: PlaceNot,

    weight: f32,
}

impl CallBuilder {
    /// Starts building a call which replaces the [lead end](LABEL_LEAD_END) with a given
    /// [`PlaceNot`]ation and is displayed with the given `symbol`.
    pub fn new(symbol: impl Into<String>, place_notation: PlaceNot) -> Self {
        Self {
            symbol: symbol.into(),
            debug_symbol: None,      // Use `symbol`
            calling_positions: None, // Compute default calling positions
            label_from: LABEL_LEAD_END.to_owned(),
            label_to: LABEL_LEAD_END.to_owned(),
            place_notation,
            weight: DEFAULT_MISC_CALL_WEIGHT,
        }
    }

    /// If set, this call will use a different symbol for debugging as displaying.  This is
    /// currently only used for bobs (which display as '' but debug as '-').
    ///
    /// If `debug_symbol` isn't called, this will be the same as the 'display' symbol.
    pub fn debug_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.debug_symbol = Some(symbol.into());
        self
    }

    /// If passed `Some(_)`, the behaviour is the same as [`CallBuilder::debug_symbol`]; if `None`
    /// is passed, the behaviour is reverted to the default (using the same symbol for debug and
    /// display).
    pub fn maybe_debug_symbol<T: Into<String>>(mut self, symbol: Option<T>) -> Self {
        self.debug_symbol = symbol.map(T::into);
        self
    }

    /// Forces the use of a given set of calling positions.  The supplied [`Vec`] contains one
    /// [`String`] for every place, in order, *including leading*.  For example, default calling
    /// positions for near bobs in Major would be `vec!["L", "I", "B", "F", "V", "M", "W", "H"]`.
    pub fn calling_positions(mut self, positions: Vec<String>) -> Self {
        self.calling_positions = Some(positions);
        self
    }

    /// If `Some(_)` is passed, behaves the same as [`CallBuilder::calling_positions`].  If `None`
    /// is passed, the `calling_positions` are reverted to the default.
    pub fn maybe_calling_positions(mut self, positions: Option<Vec<String>>) -> Self {
        self.calling_positions = positions;
        self
    }

    /// Sets the label marking where this call can be placed.  If unset, all calls are applied to
    /// the [lead end](LABEL_LEAD_END).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.label_from = label.clone();
        self.label_to = label;
        self
    }

    /// Makes this call go `to` a different label than it goes `from`.
    pub fn label_from_to(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.label_from = from.into();
        self.label_to = to.into();
        self
    }

    /// Sets the weight which is added every time this call is used.
    pub fn weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Builds a [`CallBuilder`] into a [`crate::query::Call`].
    fn build(self) -> super::Call {
        super::Call {
            debug_symbol: self.debug_symbol.unwrap_or_else(|| self.symbol.clone()),
            display_symbol: self.symbol,
            calling_positions: self
                .calling_positions
                .unwrap_or_else(|| default_calling_positions(&self.place_notation)),

            label_from: self.label_from,
            label_to: self.label_to,
            place_notation: self.place_notation,

            weight: Score::from(self.weight),
        }
    }
}

////////////////
// BASE CALLS //
////////////////

#[derive(Debug, Clone)]
pub struct BaseCalls {
    /// The type of bobs and/or singles generated
    pub ty: BaseCallType,
    /// `None` for no bobs
    pub bob_weight: Option<f32>,
    /// `None` for no singles
    pub single_weight: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
pub enum BaseCallType {
    /// `14` bobs and `1234` singles
    Near,
    /// `1<n-2>` bobs and `1<n-2><n-1><n>` singles
    Far,
}

impl BaseCalls {
    fn into_calls(self, stage: Stage) -> Vec<super::Call> {
        let n = stage.num_bells_u8();

        let mut calls = Vec::new();
        // Add bob
        if let Some(bob_weight) = self.bob_weight {
            let bob_pn = match self.ty {
                BaseCallType::Near => PlaceNot::parse("14", stage).unwrap(),
                BaseCallType::Far => PlaceNot::from_slice(&mut [0, n - 3], stage).unwrap(),
            };
            calls.push(lead_end_call(bob_pn, "", "-", bob_weight)); // Hide in display; '-' in debug
        }
        // Add single
        if let Some(single_weight) = self.single_weight {
            let single_pn = match self.ty {
                BaseCallType::Near => PlaceNot::parse("1234", stage).unwrap(),
                BaseCallType::Far => {
                    PlaceNot::from_slice(&mut [0, n - 3, n - 2, n - 1], stage).unwrap()
                }
            };
            calls.push(lead_end_call(single_pn, "s", "s", single_weight));
        }

        calls
    }
}

impl Default for BaseCalls {
    // Default to fourths-place bobs and singles, with a fairly small negative weight
    fn default() -> Self {
        Self {
            ty: BaseCallType::Near,
            bob_weight: Some(DEFAULT_BOB_WEIGHT),
            single_weight: Some(DEFAULT_SINGLE_WEIGHT),
        }
    }
}

/// Create a [`super::Call`] which replaces the lead end with a given [`PlaceNot`]
fn lead_end_call(
    place_not: PlaceNot,
    display_symbol: &str,
    debug_symbol: &str,
    weight: f32,
) -> super::Call {
    super::Call {
        display_symbol: display_symbol.to_owned(),
        debug_symbol: debug_symbol.to_owned(),
        calling_positions: default_calling_positions(&place_not),
        label_from: LABEL_LEAD_END.to_owned(),
        label_to: LABEL_LEAD_END.to_owned(),
        place_notation: place_not,
        weight: Score::from(weight),
    }
}

#[allow(clippy::branches_sharing_code)]
fn default_calling_positions(place_not: &PlaceNot) -> Vec<String> {
    let named_positions = "LIBFVXSEN"; // TODO: Does anyone know any more than this?

    // TODO: Replace 'B' with 'O' for calls which don't affect the tenor

    // Generate calling positions that aren't M, W or H
    let mut positions =
        // Start off with the single-char position names
        named_positions
        .chars()
        .map(|c| c.to_string())
        // Extending forever with numbers (extended with `ths` to avoid collisions with positional
        // calling positions)
        .chain((named_positions.len()..).map(|i| format!("{}ths", i + 1)))
        // But we consume one value per place in the Stage
        .take(place_not.stage().num_bells())
        .collect_vec();

    /// A cheeky macro which generates the code to perform an in-place replacement of a calling
    /// position at a given (0-indexed) place
    macro_rules! replace_pos {
        ($idx: expr, $new_val: expr) => {
            if let Some(v) = positions.get_mut($idx) {
                v.clear();
                v.push($new_val);
            }
        };
    }

    // Edge case: if 2nds are made in `place_not`, then I/B are replaced with B/T.  Note that
    // places are 0-indexed
    if place_not.contains(1) {
        replace_pos!(1, 'B');
        replace_pos!(2, 'T');
    }

    /// A cheeky macro which generates the code to perform an in-place replacement of a calling
    /// position at a place indexed from the end of the stage (so 0 is the highest place)
    macro_rules! replace_mwh {
        ($ind: expr, $new_val: expr) => {
            if let Some(place) = place_not.stage().num_bells().checked_sub(1 + $ind) {
                if place >= 4 {
                    if let Some(v) = positions.get_mut(place) {
                        v.clear();
                        v.push($new_val);
                    }
                }
            }
        };
    }

    // Add MWH (M and W are swapped round for odd stages)
    if place_not.stage().is_even() {
        replace_mwh!(2, 'M');
        replace_mwh!(1, 'W');
        replace_mwh!(0, 'H');
    } else {
        replace_mwh!(2, 'W');
        replace_mwh!(1, 'M');
        replace_mwh!(0, 'H');
    }

    positions
}

#[cfg(test)]
mod tests {
    use bellframe::{PlaceNot, Stage};
    use itertools::Itertools;

    /// Converts a string to a list of strings, one of each [`char`] in the input.
    fn char_vec(string: &str) -> Vec<String> {
        string.chars().map(|c| c.to_string()).collect_vec()
    }

    #[test]
    fn default_calling_positions() {
        #[rustfmt::skip]
        let cases = &[
            ("145", Stage::DOUBLES, char_vec("LIBFH")),
            ("125", Stage::DOUBLES, char_vec("LBTFH")),
            ("1", Stage::DOUBLES, char_vec("LIBFH")),

            ("14", Stage::MINOR, char_vec("LIBFWH")),
            ("1234", Stage::MINOR, char_vec("LBTFWH")),
            ("1456", Stage::MINOR, char_vec("LIBFWH")),

            ("147", Stage::TRIPLES, char_vec("LIBFWMH")),
            ("12347", Stage::TRIPLES, char_vec("LBTFWMH")),

            ("14", Stage::MAJOR, char_vec("LIBFVMWH")),
            ("1234", Stage::MAJOR, char_vec("LBTFVMWH")),
            ("16", Stage::MAJOR, char_vec("LIBFVMWH")),
            ("1678", Stage::MAJOR, char_vec("LIBFVMWH")),
            ("1256", Stage::MAJOR, char_vec("LBTFVMWH")),
            ("123456", Stage::MAJOR, char_vec("LBTFVMWH")),

            ("14", Stage::ROYAL, char_vec("LIBFVXSMWH")),
            ("16", Stage::ROYAL, char_vec("LIBFVXSMWH")),
            ("18", Stage::ROYAL, char_vec("LIBFVXSMWH")),
            ("1890", Stage::ROYAL, char_vec("LIBFVXSMWH")),

            ("14", Stage::MAXIMUS, char_vec("LIBFVXSENMWH")),
            ("1234", Stage::MAXIMUS, char_vec("LBTFVXSENMWH")),
        ];

        for (pn_str, stage, exp_positions) in cases {
            let positions =
                super::default_calling_positions(&PlaceNot::parse(pn_str, *stage).unwrap());
            assert_eq!(positions, *exp_positions);
        }
    }
}

///////////
// MUSIC //
///////////

pub struct MusicTypeBuilder {
    music_type: super::MusicType,
}

impl MusicTypeBuilder {
    pub fn new(patterns: impl IntoIterator<Item = Pattern>) -> Self {
        Self {
            music_type: MusicType {
                patterns: patterns.into_iter().collect_vec(),
                strokes: StrokeSet::Both,
                weight: Score::from(1.0),
                count_range: OptionalRangeInclusive::OPEN,
            },
        }
    }

    /// Set the weight of each instance of this music type.  If unset, defaults to `1.0`.
    pub fn weight(mut self, weight: f32) -> Self {
        self.music_type.weight = Score::from(weight);
        self
    }

    /// Sets this music type to only be scored at [backstrokes](bellframe::Stroke::Back).
    pub fn at_backstroke(mut self) -> Self {
        self.music_type.strokes = StrokeSet::Back;
        self
    }

    /// Sets this music type to only be scored at [handstrokes](bellframe::Stroke::Hand).
    pub fn at_handstroke(mut self) -> Self {
        self.music_type.strokes = StrokeSet::Hand;
        self
    }

    /// Sets range constraints for this music type.  By default, no constraints are enforced.
    pub fn count_range(mut self, range: OptionalRangeInclusive) -> Self {
        self.music_type.count_range = range;
        self
    }
}

////////////////
// MISC TYPES //
////////////////

/// The length range of a [`Query`].
pub enum Length {
    /// Practice night touch.  Equivalent to `Range(0..=300)`.
    Practice,
    /// Equivalent to `Range(1250..=1350)`.
    QuarterPeal,
    /// Equivalent to `Range(2500..=2600)`.
    HalfPeal,
    /// Equivalent to `Range(5000..=5200)`.
    Peal,
    /// Custom range
    Range(RangeInclusive<usize>),
}
