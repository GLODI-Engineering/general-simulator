//! The layout, and the arithmetic that locates anything in it.
//!
//! Nothing here does I/O. Keeping the geometry in one place and free of a
//! reader or a writer is what lets both agree about where a byte is: every
//! offset below is computed from the same four numbers in the footer, so
//! there is no offset table to get out of step with the data.

use serde::{Deserialize, Serialize};

/// Start of the file, and the last eight bytes of it.
///
/// At both ends on purpose. A truncated file is the failure this format is
/// most likely to meet -- a run killed partway, a copy that stopped -- and
/// it is indistinguishable from a complete one if only the head is checked,
/// because the footer offset would then be read out of whatever bytes
/// happened to land last.
pub const MAGIC: &[u8; 8] = b"GSIMSAMP";

/// Bumped when a reader of version N can no longer be trusted with the
/// bytes. Adding a field to the footer is not that -- the footer is JSON
/// with unknown keys ignored, exactly as `.gsim_sim_result` does it.
pub const VERSION: u32 = 1;

/// `MAGIC`, `VERSION`, flags.
pub const HEADER_BYTES: u64 = 16;

/// `u64` footer offset, then `MAGIC` again.
pub const TAIL_BYTES: u64 = 16;

/// Rows buffered before a stripe is flushed.
///
/// This is the one number that trades memory against addressability, and
/// 65,536 is chosen from both ends. Upward: a stripe is held in memory while
/// it fills, so the cost is `stripe_rows * signals * 4` bytes -- about 18 MB
/// at seventy signals, which is flat in the length of the run and small
/// beside a solver's own working set. Downward: a stripe is the unit a read
/// cannot subdivide, so a window narrower than one stripe still reads a
/// whole stripe of each signal it wants; at a 5 ns step 65,536 rows is
/// 328 us, finer than any window a user scrubs to on a grid-period run.
///
/// It is recorded in the footer rather than assumed, so a file written with
/// a different value still reads.
pub const STRIPE_ROWS: u32 = 65_536;

/// Samples folded into one min/max pair at each level.
///
/// 64 rather than the 100/10,000/1,000,000 the editor's HDF5 writer used.
/// Those were absolute factors against an assumed run length, and on a run
/// of a few hundred thousand rows two of its three levels yielded a handful
/// of points and one yielded a single point -- its own comment conceded that
/// "most of today's simulation sizes never build a level at all" -- while
/// the gap between raw and 100x, which is where scrubbing happens, had
/// nothing in it. A fixed ratio per level has no assumed length in it: the
/// levels fall geometrically, so whatever the run, some level is within 64x
/// of any zoom, and the pyramid costs `2/(64-1)` of the raw samples, about
/// 3%, at every size.
pub const LEVEL_FACTOR: u32 = 64;

/// How many pyramid levels a stripe of `stripe_rows` supports.
///
/// Levels stop when a level would hold fewer than two blocks, because a
/// level with one block per stripe is what the stripe summary already is.
pub fn levels_for(stripe_rows: u32, factor: u32) -> u32 {
    let mut levels = 0;
    let mut blocks = stripe_rows as u64;
    loop {
        blocks /= factor as u64;
        if blocks < 2 {
            return levels;
        }
        levels += 1;
    }
}

/// Blocks at `level` over `rows` samples. Level 0 is the samples themselves.
///
/// Rounded up: a final partial block is a real block holding real samples,
/// and dropping it would quietly shorten every stripe's tail.
pub fn blocks_at(rows: u32, level: u32, factor: u32) -> u32 {
    if level == 0 {
        return rows;
    }
    let span = (factor as u64).pow(level);
    ((rows as u64).div_ceil(span)) as u32
}

/// Bytes one vector of `rows` samples occupies across level 0 and `levels`
/// pyramid levels, at `width` bytes per number.
///
/// A pyramid entry is a pair, so it is two numbers wide.
pub fn vector_bytes(rows: u32, levels: u32, factor: u32, width: u64) -> u64 {
    let mut total = rows as u64 * width;
    for level in 1..=levels {
        total += blocks_at(rows, level, factor) as u64 * 2 * width;
    }
    total
}

/// Byte offset of `level` within a vector that starts at the vector's own
/// base, at `width` bytes per number.
pub fn level_offset(rows: u32, level: u32, levels: u32, factor: u32, width: u64) -> u64 {
    debug_assert!(level <= levels);
    let mut offset = 0u64;
    if level > 0 {
        offset += rows as u64 * width;
        for l in 1..level {
            offset += blocks_at(rows, l, factor) as u64 * 2 * width;
        }
    }
    offset
}

/// Numbers held at `level` of a vector of `rows` samples.
pub fn level_numbers(rows: u32, level: u32, factor: u32) -> u32 {
    if level == 0 {
        rows
    } else {
        blocks_at(rows, level, factor) * 2
    }
}

/// What a signal or the sweep axis is, for a reader that has to label it.
///
/// Mirrors the vector fields of `.gsim_sim_result` so a manifest can be
/// written beside one of these files without translating names.
///
/// ## What goes in the fields
///
/// These four fields are the whole of the signal-metadata convention — see
/// `book/user-guide/src/signal-metadata.md` for the convention itself, and
/// `general-simulator-cli`'s `signal_meta` module for the producer side. In
/// short:
///
/// - `name` is an interface and carries no unit. It is what a
///   `kind=measure` statement, a script or a CSV header says, and nothing
///   about a quantity's nature is encoded into it.
/// - `quantity` is what kind of thing this is, from a closed vocabulary:
///   `time`, `voltage`, `current`, `signal`. `signal` is a signal-flow
///   value — a duty cycle, an angle, a controller output — and is a
///   positive statement, not a fallback.
/// - `unit` is the SI symbol (`s`, `V`, `A`) and is **empty when it is not
///   known**. Empty never means dimensionless; a dimensionless value says
///   so through `quantity`. A writer that cannot be sure of a unit leaves
///   this empty rather than offering a plausible one.
/// - `analysis` is which analysis produced the run: `transient`, `dc`.
///
/// A reader may treat an unrecognised `quantity` the same way it treats an
/// empty one — as "not known" — so the vocabulary can grow without breaking
/// anything that already reads these files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VectorMeta {
    pub name: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub quantity: String,
    #[serde(default)]
    pub analysis: String,
}

impl VectorMeta {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    pub fn with_quantity(mut self, quantity: impl Into<String>) -> Self {
        self.quantity = quantity.into();
        self
    }

    pub fn with_analysis(mut self, analysis: impl Into<String>) -> Self {
        self.analysis = analysis.into();
        self
    }
}

/// The footer: everything needed to locate any sample, and nothing that
/// grows with the run.
///
/// Deliberately not an offset table. A table of per-stripe, per-signal,
/// per-level offsets would be about four million numbers on a 26 GB run --
/// megabytes of JSON to parse before the first pixel, and a second source of
/// truth about where the data is. Every stripe but the last has identical
/// geometry, so the offsets are arithmetic over the five numbers below.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    /// Samples per signal across the whole run.
    pub rows: u64,
    pub stripe_rows: u32,
    pub factor: u32,
    pub levels: u32,
    pub stripes: u64,
    /// Rows in the final stripe, which is the only short one. Equal to
    /// `stripe_rows` when the run divided evenly.
    pub last_stripe_rows: u32,
    pub data_start: u64,
    /// `stripes * 16`: each stripe's first and last sweep value, so a time
    /// window maps to stripes without reading any samples.
    pub directory_offset: u64,
    /// `stripes * signals * 8`: each signal's extremes over each stripe.
    /// This is the coarsest level there is, one pair per stripe, and it is
    /// what a full-extent view of a very long run reads.
    pub summary_offset: u64,
    pub sweep: VectorMeta,
    pub signals: Vec<VectorMeta>,
}

impl Index {
    /// Rows in stripe `stripe`.
    pub fn stripe_rows_at(&self, stripe: u64) -> u32 {
        if stripe + 1 == self.stripes {
            self.last_stripe_rows
        } else {
            self.stripe_rows
        }
    }

    /// Bytes stripe `stripe` occupies: the sweep axis in `f64`, then every
    /// signal in `f32`.
    ///
    /// The axis is wider than the samples because it is the one vector where
    /// `f32` is not enough: 5 ns resolution over 500 ms needs eight
    /// significant digits and `f32` carries about seven, so a late time step
    /// would land on the same float as its neighbour. The samples are being
    /// reduced to pixels and do not need it.
    pub fn stripe_bytes(&self, stripe: u64) -> u64 {
        let rows = self.stripe_rows_at(stripe);
        vector_bytes(rows, self.levels, self.factor, 8)
            + self.signals.len() as u64 * vector_bytes(rows, self.levels, self.factor, 4)
    }

    /// Where stripe `stripe` begins.
    pub fn stripe_offset(&self, stripe: u64) -> u64 {
        // Only the last stripe can be short, so every stripe before `stripe`
        // is a full one.
        self.data_start
            + stripe * vector_bytes(self.stripe_rows, self.levels, self.factor, 8)
            + stripe
                * self.signals.len() as u64
                * vector_bytes(self.stripe_rows, self.levels, self.factor, 4)
    }

    /// Where `level` of the sweep axis begins, within stripe `stripe`.
    pub fn sweep_level_offset(&self, stripe: u64, level: u32) -> u64 {
        let rows = self.stripe_rows_at(stripe);
        self.stripe_offset(stripe) + level_offset(rows, level, self.levels, self.factor, 8)
    }

    /// Where `level` of signal `signal` begins, within stripe `stripe`.
    pub fn signal_level_offset(&self, stripe: u64, signal: usize, level: u32) -> u64 {
        let rows = self.stripe_rows_at(stripe);
        self.stripe_offset(stripe)
            + vector_bytes(rows, self.levels, self.factor, 8)
            + signal as u64 * vector_bytes(rows, self.levels, self.factor, 4)
            + level_offset(rows, level, self.levels, self.factor, 4)
    }
}
