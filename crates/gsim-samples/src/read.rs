//! Reading a few signals, over a window, at a screen's resolution.

use crate::format::{blocks_at, level_numbers, Index, VectorMeta, MAGIC, TAIL_BYTES};
use crate::{Error, Result};

/// Somewhere bytes can be read from at an offset.
///
/// A trait rather than a `File` because the two callers cannot share one
/// type: natively this is a file, and in the editor's viewer -- the same
/// reader compiled to WebAssembly -- it is a buffer the host handed over, or
/// a range the extension fetched. Both answer the same question, and keeping
/// it to "give me these bytes at this offset" is what lets one reader serve
/// both. Nothing here memory-maps on the reader's behalf; a native caller
/// that wants the page cache to do the work can map the file and hand over
/// the slice.
pub trait Bytes {
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>>;
    fn size(&self) -> u64;
}

impl Bytes for &[u8] {
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let start = offset as usize;
        let end = start
            .checked_add(len)
            .ok_or(Error::Truncated(offset, len))?;
        self.get(start..end)
            .map(<[u8]>::to_vec)
            .ok_or(Error::Truncated(offset, len))
    }

    fn size(&self) -> u64 {
        self.len() as u64
    }
}

impl Bytes for Vec<u8> {
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.as_slice().read_at(offset, len)
    }

    fn size(&self) -> u64 {
        self.len() as u64
    }
}

/// Which rung of the pyramid a read came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// Stored samples. `Level(0)` is the run itself; higher levels are
    /// min/max pairs over `factor^level` samples each.
    Level(u32),
    /// One min/max pair per stripe: the coarsest view there is, and what a
    /// full-extent look at a very long run reads.
    Summary,
}

/// A window of several signals on one shared sweep axis.
///
/// For any `Detail` but `Level(0)` the points come in pairs -- the low and
/// high of each block, at the block's first and last sweep value -- which is
/// how an envelope is drawn: a vertical span per pixel column rather than a
/// line through a sample that happened to be picked. Every signal is on the
/// same axis, because a chart draws them together and a per-signal grid
/// would move the problem rather than solve it.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub sweep: Vec<f64>,
    /// Parallel to the names asked for, each as long as `sweep`.
    pub values: Vec<Vec<f64>>,
    pub detail: Detail,
}

/// An open `.gsim_samples` file.
pub struct Reader<B: Bytes> {
    bytes: B,
    index: Index,
    /// Each stripe's sweep extremes.
    directory: Vec<(f64, f64)>,
    /// Stripe-major, signal-minor.
    summary: Vec<(f32, f32)>,
}

/// Written by hand rather than derived: `B` is whatever the caller reads
/// bytes from -- a file, a host buffer -- and requiring it to be `Debug`
/// would push that onto every caller for the sake of a diagnostic. What is
/// worth printing is the run, not the byte source.
impl<B: Bytes> std::fmt::Debug for Reader<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader")
            .field("rows", &self.index.rows)
            .field("signals", &self.index.signals.len())
            .field("stripes", &self.index.stripes)
            .field("levels", &self.index.levels)
            .finish()
    }
}

impl<B: Bytes> Reader<B> {
    pub fn open(bytes: B) -> Result<Self> {
        let size = bytes.size();
        if size < crate::format::HEADER_BYTES + TAIL_BYTES {
            return Err(Error::Format(format!(
                "{size} bytes is shorter than this format's header and footer"
            )));
        }
        let head = bytes.read_at(0, 16)?;
        if &head[0..8] != MAGIC {
            return Err(Error::Format(
                "this is not a .gsim_samples file: the leading magic does not match".into(),
            ));
        }
        let tail = bytes.read_at(size - TAIL_BYTES, TAIL_BYTES as usize)?;
        if &tail[8..16] != MAGIC {
            // Checked separately from the head so the message can say which
            // of the two failed, because they mean different things: a bad
            // head is the wrong file, a bad tail is this file cut short.
            return Err(Error::Format(
                "this file is truncated: its trailing magic is missing, so the run never \
                 finished writing"
                    .into(),
            ));
        }
        let footer_offset = u64::from_le_bytes(tail[0..8].try_into().expect("eight bytes"));
        if footer_offset >= size - TAIL_BYTES {
            return Err(Error::Format(format!(
                "the footer offset {footer_offset} does not point inside a {size}-byte file"
            )));
        }
        let footer = bytes.read_at(footer_offset, (size - TAIL_BYTES - footer_offset) as usize)?;
        let index: Index =
            serde_json::from_slice(&footer).map_err(|e| Error::Format(e.to_string()))?;
        if index.version != crate::format::VERSION {
            return Err(Error::Format(format!(
                "this file is version {}, and this reader knows version {}",
                index.version,
                crate::format::VERSION
            )));
        }

        let stripes = index.stripes as usize;
        let signals = index.signals.len();
        let directory = read_pairs_f64(&bytes, index.directory_offset, stripes)?;
        let summary = read_pairs_f32(&bytes, index.summary_offset, stripes * signals)?;

        Ok(Self {
            bytes,
            index,
            directory,
            summary,
        })
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn signals(&self) -> &[VectorMeta] {
        &self.index.signals
    }

    pub fn sweep(&self) -> &VectorMeta {
        &self.index.sweep
    }

    pub fn rows(&self) -> u64 {
        self.index.rows
    }

    /// The run's sweep extent, read from the directory rather than from any
    /// sample.
    pub fn extent(&self) -> Option<(f64, f64)> {
        let first = self.directory.first()?;
        let last = self.directory.last()?;
        Some((first.0, last.1))
    }

    /// Stripes whose sweep range intersects `window`.
    fn stripes_for(&self, window: Option<(f64, f64)>) -> std::ops::Range<usize> {
        let Some((from, to)) = window else {
            return 0..self.directory.len();
        };
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        // A linear scan, and deliberately: the directory is one pair per
        // stripe, so even a 26 GB run has about 1,500 of them -- a few
        // microseconds, against a binary search that would have to assume
        // the sweep is sorted. It is, in every analysis this solver runs,
        // but the directory stores extremes precisely so that nothing here
        // depends on that.
        let mut start = self.directory.len();
        let mut end = 0usize;
        for (i, (low, high)) in self.directory.iter().enumerate() {
            if *high >= from && *low <= to {
                start = start.min(i);
                end = end.max(i + 1);
            }
        }
        if start >= end {
            0..0
        } else {
            start..end
        }
    }

    /// Points a given detail offers over `stripes`.
    fn points_at(&self, stripes: &std::ops::Range<usize>, detail: Detail) -> u64 {
        match detail {
            Detail::Summary => stripes.len() as u64 * 2,
            Detail::Level(level) => stripes
                .clone()
                .map(|s| {
                    let rows = self.index.stripe_rows_at(s as u64);
                    level_numbers(rows, level, self.index.factor) as u64
                })
                .sum(),
        }
    }

    /// The coarsest detail that still has at least `max_points` to offer,
    /// falling back to the samples themselves when nothing does.
    ///
    /// Coarsest-that-saturates rather than finest-that-fits: both end up
    /// drawing `max_points`, and this one reads the least to get there. If
    /// even level 0 is under budget the whole window is smaller than the
    /// screen, so it is returned entire and nothing is reduced.
    pub fn detail_for(&self, window: Option<(f64, f64)>, max_points: usize) -> Detail {
        let stripes = self.stripes_for(window);
        let budget = max_points.max(2) as u64;
        if self.points_at(&stripes, Detail::Summary) >= budget {
            return Detail::Summary;
        }
        for level in (1..=self.index.levels).rev() {
            if self.points_at(&stripes, Detail::Level(level)) >= budget {
                return Detail::Level(level);
            }
        }
        Detail::Level(0)
    }

    /// Up to roughly `max_points` of each named signal over `window`.
    ///
    /// "Roughly" is exact about one thing: this returns a whole rung of the
    /// pyramid, between `max_points` and `factor` times it, and does not
    /// reduce further. The last step down to a precise budget belongs to the
    /// caller, because the caller is a chart and the bucket edges it picks
    /// have to match the ones it uses on data from anywhere else. Two
    /// implementations of that step is the bug this format is downstream of.
    pub fn slice(
        &self,
        names: &[String],
        window: Option<(f64, f64)>,
        max_points: usize,
    ) -> Result<Window> {
        self.read_detail(names, window, self.detail_for(window, max_points))
    }

    /// The same read, at a rung the caller chose.
    ///
    /// Separate from [`Reader::slice`] because choosing the rung and reading
    /// it are different decisions, and a caller that already knows what it
    /// wants -- a viewer holding a zoom level, a test pinning one rung --
    /// should not have to express it as a point budget and hope.
    pub fn read_detail(
        &self,
        names: &[String],
        window: Option<(f64, f64)>,
        detail: Detail,
    ) -> Result<Window> {
        let wanted = self.resolve(names)?;
        if let Detail::Level(level) = detail {
            if level > self.index.levels {
                return Err(Error::Missing(format!(
                    "level {level}, when this file carries {}",
                    self.index.levels
                )));
            }
        }
        let stripes = self.stripes_for(window);

        let mut sweep: Vec<f64> = Vec::new();
        let mut values: Vec<Vec<f64>> = wanted.iter().map(|_| Vec::new()).collect();

        for stripe in stripes.clone() {
            let s = stripe as u64;
            match detail {
                Detail::Summary => {
                    let (low, high) = self.directory[stripe];
                    sweep.push(low);
                    sweep.push(high);
                    for (slot, signal) in wanted.iter().enumerate() {
                        let (min, max) = self.summary[stripe * self.index.signals.len() + signal];
                        values[slot].push(min as f64);
                        values[slot].push(max as f64);
                    }
                }
                Detail::Level(0) => {
                    let rows = self.index.stripe_rows_at(s);
                    sweep.extend(self.read_sweep(s, 0, rows as usize)?);
                    for (slot, signal) in wanted.iter().enumerate() {
                        values[slot].extend(self.read_signal(s, *signal, 0, rows as usize)?);
                    }
                }
                Detail::Level(level) => {
                    let rows = self.index.stripe_rows_at(s);
                    let count = level_numbers(rows, level, self.index.factor) as usize;
                    sweep.extend(self.read_sweep(s, level, count)?);
                    for (slot, signal) in wanted.iter().enumerate() {
                        values[slot].extend(self.read_signal(s, *signal, level, count)?);
                    }
                }
            }
        }

        if let Some((from, to)) = window {
            let (from, to) = if from <= to { (from, to) } else { (to, from) };
            trim(&mut sweep, &mut values, from, to, detail);
        }

        Ok(Window {
            sweep,
            values,
            detail,
        })
    }

    fn resolve(&self, names: &[String]) -> Result<Vec<usize>> {
        names
            .iter()
            .map(|name| {
                self.index
                    .signals
                    .iter()
                    .position(|s| &s.name == name)
                    .ok_or_else(|| Error::Missing(name.clone()))
            })
            .collect()
    }

    fn read_sweep(&self, stripe: u64, level: u32, count: usize) -> Result<Vec<f64>> {
        let offset = self.index.sweep_level_offset(stripe, level);
        let raw = self.bytes.read_at(offset, count * 8)?;
        Ok(raw
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c))
            .collect())
    }

    fn read_signal(
        &self,
        stripe: u64,
        signal: usize,
        level: u32,
        count: usize,
    ) -> Result<Vec<f64>> {
        let offset = self.index.signal_level_offset(stripe, signal, level);
        let raw = self.bytes.read_at(offset, count * 4)?;
        Ok(raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c) as f64)
            .collect())
    }

    /// Blocks at `level` in stripe `stripe`, for callers sizing a read.
    pub fn blocks(&self, stripe: u64, level: u32) -> u32 {
        blocks_at(self.index.stripe_rows_at(stripe), level, self.index.factor)
    }
}

/// Drops points outside the window.
///
/// Pairs are kept or dropped together: half an envelope block is a vertical
/// span with one end missing, which draws as a line to nowhere.
fn trim(sweep: &mut Vec<f64>, values: &mut [Vec<f64>], from: f64, to: f64, detail: Detail) {
    let paired = detail != Detail::Level(0);
    let step = if paired { 2 } else { 1 };
    let mut keep = Vec::with_capacity(sweep.len());
    let mut i = 0;
    while i < sweep.len() {
        let last = (i + step - 1).min(sweep.len() - 1);
        let low = sweep[i].min(sweep[last]);
        let high = sweep[i].max(sweep[last]);
        if high >= from && low <= to {
            for k in i..=last {
                keep.push(k);
            }
        }
        i += step;
    }
    if keep.len() == sweep.len() {
        return;
    }
    *sweep = keep.iter().map(|k| sweep[*k]).collect();
    for column in values.iter_mut() {
        *column = keep.iter().map(|k| column[*k]).collect();
    }
}

fn read_pairs_f64<B: Bytes>(bytes: &B, offset: u64, count: usize) -> Result<Vec<(f64, f64)>> {
    let raw = bytes.read_at(offset, count * 16)?;
    Ok(raw
        .as_chunks::<16>()
        .0
        .iter()
        .map(|c| {
            let (low, high) = c.split_at(8);
            (
                f64::from_le_bytes(low.try_into().expect("eight bytes")),
                f64::from_le_bytes(high.try_into().expect("eight bytes")),
            )
        })
        .collect())
}

fn read_pairs_f32<B: Bytes>(bytes: &B, offset: u64, count: usize) -> Result<Vec<(f32, f32)>> {
    let raw = bytes.read_at(offset, count * 8)?;
    Ok(raw
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| {
            let (low, high) = c.split_at(4);
            (
                f32::from_le_bytes(low.try_into().expect("four bytes")),
                f32::from_le_bytes(high.try_into().expect("four bytes")),
            )
        })
        .collect())
}
