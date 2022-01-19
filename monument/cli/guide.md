# Monument CLI Guide

_**NOTE:** There is no guarantee of the stability of this format; it is, in theory, able to change at
any point without warning.  I'm working on a GUI for Monument which, unlike that of Wheatley in
Ringing Room, will be fully featured and will become the intended way to use Monument (the core Rust
library is still accessible to anyone who wants to do really complex things).  I won't make breaking
changes to the TOML format for the fun of it, but it may change to reflect the GUI layout.  The GUI
also won't obsolete the TOML-based format (I use it for testing purposes), but it's only really
intended for my own use and I won't recommend it for users once the GUI exists._

Monument's CLI reads files in the TOML data format, so if you're wondering then check out TOML's
[helpful website](https://toml.io/en/) for more info.

---

## Example

More examples can be found in [the `examples/` directory](examples/).  This exact example can also
be found there (as [`examples/guide.toml`](example/guide.toml) and
[`examples/guide-music-8.toml`](examples/guide-music-8.toml)).

### File: `guide.toml`
```toml
# General
length = "QP"
num_comps = 10 # just for brevity; you probably want more comps than this

# Methods
methods = [
    "Yorkshire Surprise Major",
    { title = "Lessness Surprise Major", shorthand = "E" },
    { name = "Bastow", place_notation = "x2,1", stage = 8 },
]
splice_style = "calls"
method_count = { min = 100, max = 600 } # relax method balance to allow for Bastow

# Calls
base_calls = "near" # optional, since this is the default
bob_weight = -7
single_weight = -10
calls = [{ symbol = "b", place_notation = "1456", weight = -12 }]

# Music
music_file = "guide-music-8.toml" # Relative to this file, so expects `music-8.toml` in the same folder
music = [
    { patterns = ["5678*", "8765*"], weight = 2 }, # Boost music off the front
    { pattern = "*87", weight = -1, stroke = "back" }, # Slightly penalise 87s at back
]

# Courses
part_head = "124365"
split_tenors = true
# course_heads = ["*78", "*7856"] # uncomment to override `split_tenors`
leadwise = false # optional: Monument would have determined this
ch_weights = [{ pattern = "*78", weight = 0.05 }] # slight boost for tenors-together courses

# Starts/Ends (commented because they currently play badly with multi-part spliced)
# snap_start = true
# end_indices = [0] # only allow non-snap finishes
```

### File: `guide-music-8.toml`

```toml
# This file contains just an array called `music`, and can be imported into other files.  This is
# useful for e.g. default music schemes
[[music]]
run_lengths = [4, 5, 6, 7, 8]
internal = false # optional; this is the default

[[music]]
pattern = "*6578"
weight = 1.2

[[music]]
patterns = ["*5678", "*8765", "5678*", "8765*"]
weight = 0.5
```

### Output

Running `monument_cli guide.toml` outputs:

```text
-- snip --

SEARCH COMPLETE!

len: 1264, ms: [576, 576, 112], score: 137.40, avg: 0.108703, str: BBBB[bB]EEE[H]EEE[B]EEE[sH]YYYY[W]YYYYY[B]BBBBBBB[B]BBB
len: 1296, ms: [576, 512, 208], score: 141.80, avg: 0.109414, str: BBBB[B]YYYY[B]YYYYY[M]BBBBBB[W]E[sH]BBBB[B]E[V]BBBBBBB[F]E[bB]EEE[H]BBBB[B]EE[W]B
len: 1272, ms: [576, 512, 184], score: 139.30, avg: 0.109513, str: BBBB[B]EEE[bH]YYYY[W]YYYYY[B]BBBBBBB[bB]BBBBBBB[B]EEE[H]BBBB[B]EE[W]B
len: 1328, ms: [576, 576, 176], score: 146.30, avg: 0.110166, str: BBBB[B]EEE[bH]EEE[B]BBBB[sH]BBBB[B]EEE[bH]YYYY[W]YYYYY[B]BBBBBBB[bB]BBB
len: 1288, ms: [576, 576, 136], score: 144.10, avg: 0.111879, str: BBBB[B]BBBBB[M]EEEEEE[bH]YYYY[W]YYYYY[bB]EEE[H]BBBB[B]BBB[W]B
len: 1312, ms: [512, 576, 224], score: 148.20, avg: 0.112957, str: BBBB[B]YY[H]YY[B]YY[H]YY[B]BBBBBBB[B]EEEE[sM]EEEE[V]BBBBBBB[F]E[B]BBBBBBB[bB]BBB
len: 1280, ms: [576, 576, 128], score: 146.20, avg: 0.114219, str: BBBB[B]YYYY[B]BBBBBBB[B]EEEE[sM]EEEEE[W]B[W]YYYYY[B]BBBB[sH]
len: 1296, ms: [576, 576, 144], score: 158.00, avg: 0.121914, str: BBBB[B]EEEE[M]BBBB[F]B[I]EEEEE[M]BBBBBB[W]YYYYY[B]YYYY[B]BBB
len: 1296, ms: [576, 576, 144], score: 158.00, avg: 0.121914, str: BBBB[B]YYYY[B]YYYYY[M]BBBBBB[W]EEEEE[F]B[I]BBBB[W]EEEE[B]BBB
len: 1288, ms: [576, 576, 136], score: 159.00, avg: 0.123447, str: EEE[B]E[V]BBBB[M]BBBB[F]E[B]EEEE[M]BBBBBB[W]YYYYY[B]YYYY[B]BBB
Search completed in 3.364357029s
```

3.4s is not bad!  And those compositions are pretty decent too.  You can see how the call weighting
is promoting compositions which make clever use of few calls.

You can use [`to-complib.py`](to-complib.py) to copy the string to paste into CompLib.  For example,
running `./to-complib.py BBBB[B]YYYY[B]YYYYY[M]BBBBBB[W]E[sH]BBBB[B]E[V]BBBBBBB[F]E[bB]EEE[H]BBBB[B]EE[W]B` 
gives this:
```text
M	F	Methods	V	B	H	W
				-
				-
-
						-
					s
				-
			-
	-
				b
					-
				-
						-
		BBBBYYYYYYYYYBBBBBBEBBBBEBBBBBBBEEEEBBBBEEB
```

Which, when copied into CompLib, gives
[this comp](https://complib.org/composition/90918?accessKey=88cedf2b68369eb0987a752ca5b17bc76931eb8c).

---

## Quick List of Parameters

Here's a short list of the parameters, along with their default value.  Clicking each value will
take you to more in-depth docs about it.

**General:**
- [`length`](#length-required)
- [`num_comps = 30`](#num_comps)
- [`allow_false = false`](#allow_false)

**Methods:**
- [`method`](#method)
- [`methods`](#methods-2)
- [`splice_style = "leads"`](#splice_style)
- [`method_count`](#method_count) (default to ±10% balance)

**Calls:**
- [`base_calls = "near"`](#base_calls)
- [`bob_weight = -1.8`](#bob_weight)
- [`single_weight = -2.3`](#single_weight)
- [`calls = []`](#calls-2)

**Music:**
- [`music_file`](#music_file) (optional)
- [`music = []`](#music-2)
- [`start_stroke = "back"`](#start_stroke)

**Courses:**
- [`part_head = ""`](#part_head) (i.e. default to 1-part)
- [`course_heads`](#course_heads) (default set by `split_tenors`)
- [`split_tenors = false`](#split_tenors)
- [`ch_weights = []`](#ch_weights)
- [`leadwise`](#leadwise) (default set by Monument)

**Starts/Ends:**
- [`snap_start = false`](#snap_start)
- [`start_indices`](#start_indices-and-end_indices) (default set by `snap_start`)
- [`end_indices`](#start_indices-and-end_indices) (default to allow any finish)


------

## More Detail on Parameters

### General

#### `length` (required)

Determines the _inclusive_ range of lengths into which the compositions must fit.  This can take the
following forms:

```toml
length = { min = 600, max = 700 } # require length of 600-700 rows (inclusive)
length = 1729        # require exact length

length = "practice"  # equivalent to `{ min =    0, max =  300 }`
length = "QP"        # equivalent to `{ min = 1250, max = 1350 }`
length = "half peal" # equivalent to `{ min = 2500, max = 2600 }`
length = "peal"      # equivalent to `{ min = 5000, max = 5200 }`
```

#### `num_comps`

The number of compositions you want.  Defaults to `30`

#### `allow_false`

If `true`, Monument will ignore falseness and generate potentially false compositions.  Defaults to
`false`.

### Methods

#### `method`

Specifies a single method for the composition:

```toml
method = "Bristol Surprise Major"

# or

[method]
title = "Lincolnshire Surprise Major"
shorthand = "N" # (optional; defaults to the first letter of the title)
lead_locations = { 0: "LE", 16: "HL" } # (optional; defaults to `{0:"LE"}`)
course_heads = ["*78"] # (optional; defaults to global `course_heads`)

# or

[method]
name = "Double Norwich Court" # Note this is *name*, not *title*
place_notation = "x4x36x5x8,8"
stage = 8
shorthand = "N" # (optional; defaults to the first letter of the title)
lead_locations = { 0: "LE", 8: "HL" } # (optional; defaults to `{0:"LE"}`)
course_heads = ["*78"] # (optional; defaults to global `course_heads`)
```

#### `methods`

Same as `method`, but takes a list of methods:

```toml
methods = [
    "Bristol Surprise Major",
    { title = "Lincolnshire Surprise Major", shorthand = "N" },
    { name = "Bastow", place_notation = "x2,1", stage = 8 },
]
```

#### `splice_style`

Determines how methods can be spliced.  Has no effect for single-method compositions.  Options:
```toml
splice_style = "leads"          # (default; change method at every defined lead location)
# or
splice_style = "call locations" # only change method whenever a call _could_ happen
# or
splice_style = "calls"          # only change method when a call does happen
```

Note that currently `splice_style = "call locations"` and `splice_style = "calls"` will both insert
methods splices over part heads.  This is a bug and will be fixed.

#### `method_count`

Min-max limits on how many rows of each method is allowed.  Defaults to +-10% method balance.
```toml
method_count.min = 0 # Allow Monument to ignore methods
# or
method_count = { min = 100, max = 300 } # Force a given method count range
```

### Calls

#### `base_calls`

Lead-end calls which are automatically generated:
```toml
base_calls = "near" # default; 14 bob and 1234 single
# or
base_calls = "far"  # 1(n-2) bob and 1(n-2)(n-1)n single
# or
base_calls = "none" # no base calls, only what you've added
```

#### `bob_weight`, `single_weight`

Sets the score given to the bob/single generated by `base_calls`.  Defaults to `bob_weight = -1.8`,
`single_weight = -2.5`.

#### `calls`

Array of custom calls:
```toml
[[calls]]
place_notation = "16"
symbol = "x"
debug_symbol = "x"     # Optional; symbol to use for debugging.  Defaults to same as `symbol`
lead_location = "LE"   # Optional; where in the method to apply the call.  Defaults to "LE"
calling_positions = "LIBFVXSMWH" # Optional; defaults to some sane values
weight = -4            # Optional; Score given to each instance of this call.  Defaults to -3
```

### Music

#### `music_file`

Relative path to a file containing music definitions (i.e. a single `music` array).

#### `music`

Array of custom music types:
```toml
[[music]]
run_lengths = [4, 5, 6, 7, 8] # or a single length: `run_length = 4`
internal = true               # Optional; defaults to `false`
# or
patterns = ["*6578", "6578*"]       # or a single pattern: `pattern = "*5x6x7x8*"`
count_each = { min = 12, max = 24 } # Count range applied per-pattern.
                                    # Optional; defaults to allowing anything

# common values:
weight = 2    # Score applied per instance of this music type.  Optional; defaults to `1`
count = { min = 12, max = 24 } # Overall required count range
stroke = "back" # On which stroke(s) to count this music.
                # Options: "both" (default), "back", "hand".
```

#### `start_stroke`

The row of the first **lead head** (i.e. rounds).  This is the opposite to what you expect (and will
probably be reversed in future):
```toml
start_stroke = "back" # Default
# or
start_stroke = "hand"
```

### Courses

#### `part_head`

A row which determines which part heads will be generated.  Note that Monument can generate
compositions with a different part head, provided the same set of parts are generated (so
`part_head = "23456781"` and `part_head = "81234567"` are equivalent but `part_head = "56781234"` is
not).  Defaults to rounds (i.e. one part, or `part_head = ""`).

#### `course_heads`

List of masks which define the courses that Monument can use.  Defaults to tenors together, or any
course (if `split_tenors` is set).  For example:
```toml
course_heads = ["*78", "xxxx7856", "12345xxx"]
```

#### `split_tenors`

If `course_heads` isn't specified, this lets Monument use any courses, as opposed to just those with
the tenors together.  Warning: on higher stages, this will cause Monument to use lots of memory.
Defaults to `false`.

#### `ch_weights`

Applies some score to every row in a course which contains a lead head which matches a given mask.
For example, the following will weight Monument to create compositions where handbell pairs are
coursing often:
```toml
[[ch_weights]]
patterns = [
    "*78",
    "*56", "*65",
    "*34", "*43",
] # can also use e.g. `pattern = "*78"`
weight = 0.05 # this is small because the weight is applied per row
```

#### `leadwise`

If set, this will stop Monument using calling positions, and instead label all the calls
positionally.  `course_heads`, `split_tenors` and `ch_weights` are then obviously incompatible, and
this will generate split-tenors compositions.  You should rarely have to set this yourself; by
default, Monument will set this automatically if the tenor is affected by the part head (e.g. in
cyclic) but otherwise will stick to course-wise compositions.  The only times you're likely to need
this is for weird cases like differential methods, which don't have a well-defined concept of a
'course head'.

### Starts/Ends

#### `snap_start`

If no `start_indices` have been set, `snap_start = true` allows both normal and snap starts.
Defaults to `false` (i.e. just lead end starts).

#### `start_indices` and `end_indices`

Sets the indices within the lead where the composition can start/end.  The default value of
`start_indices` is determined by `snap_start`, whereas the `end_indices` defaults to allowing any
value.

---

### That's it folks.  Happy composing!