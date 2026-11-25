//! Writing a run, one row at a time, without ever holding it.

use std::io::Write;

use crate::format::{
    blocks_at, levels_for, Index, VectorMeta, HEADER_BYTES, LEVEL_FACTOR, MAGIC, STRIPE_ROWS,
    VERSION,
};
use crate::{Error, Result};

/// A run being written.
///
/// ## Why this streams
///
/// The obvious shape for this crate would take a finished waveform and
/// serialize it, which is what this solver's CSV and rawfile writers do and
/// is right for them. It cannot work here. The runs this format exists for
/// are gigabytes: a 5 ns step over ten grid periods at seventy signals is
/// about 10.6 GB of samples, and the editor's HDF5 writer -- which built the
/// arrays, then built a pyramid over them -- would need all of it resident
/// twice.
///
/// So rows arrive through [`Writer::push`] and are held only until a stripe
/// is full. Peak memory is one stripe, about 18 MB at seventy signals, flat
/// in the length of the run: a ten-hour run and a one-second run cost the
/// same to write. The pyramid is built from each stripe while it is still in
/// cache, so it costs one pass over memory that was going to be touched
/// anyway and never a re-read of the file.
///
/// ## What `push` must be given
///
/// Rows in sweep order, every signal present on every row. A solver that
/// skips a signal on some steps has no row to write, and the format has no
/// way to express a hole; that is a deliberate limitation rather than an
/// oversight, because a chart cannot draw one either.
pub struct Writer<W: Write> {
    sink: W,
    /// Bytes handed to `sink` so far. `Write` does not report position and
    /// the sink may be a pipe, so this is the only offset there is -- which
    /// is also why every offset in the footer is computed rather than
    /// observed, and why `debug_assert`s below check the two agree.
    pos: u64,
    sweep_meta: VectorMeta,
    signals_meta: Vec<VectorMeta>,
    levels: u32,
    sweep_buf: Vec<f64>,
    signal_buf: Vec<Vec<f32>>,
    rows: u64,
    stripes: u64,
    last_stripe_rows: u32,
    /// First and last sweep value of each stripe, so a reader maps a time
    /// window to stripes without touching a sample.
    directory: Vec<[f64; 2]>,
    /// Each signal's extremes over each stripe, stripe-major.
    summary: Vec<[f32; 2]>,
}

impl<W: Write> Writer<W> {
    /// Begins a file. Writes the header immediately, so a sink that fails
    /// fails here rather than after a run has been computed.
    pub fn new(mut sink: W, sweep: VectorMeta, signals: Vec<VectorMeta>) -> Result<Self> {
        if signals.is_empty() {
            return Err(Error::Format(
                "a run with no signals has nothing to store".into(),
            ));
        }
        sink.write_all(MAGIC)?;
        sink.write_all(&VERSION.to_le_bytes())?;
        sink.write_all(&0u32.to_le_bytes())?;
        let count = signals.len();
        Ok(Self {
            sink,
            pos: HEADER_BYTES,
            sweep_meta: sweep,
            signals_meta: signals,
            levels: levels_for(STRIPE_ROWS, LEVEL_FACTOR),
            sweep_buf: Vec::with_capacity(STRIPE_ROWS as usize),
            signal_buf: (0..count)
                .map(|_| Vec::with_capacity(STRIPE_ROWS as usize))
                .collect(),
            rows: 0,
            stripes: 0,
            last_stripe_rows: 0,
            directory: Vec::new(),
            summary: Vec::new(),
        })
    }

    /// One row: the sweep value and every signal at it.
    pub fn push(&mut self, sweep: f64, values: &[f64]) -> Result<()> {
        if values.len() != self.signal_buf.len() {
            return Err(Error::Format(format!(
                "row {} carries {} values for {} signals",
                self.rows,
                values.len(),
                self.signal_buf.len()
            )));
        }
        self.sweep_buf.push(sweep);
        for (buffer, value) in self.signal_buf.iter_mut().zip(values) {
            buffer.push(*value as f32);
        }
        self.rows += 1;
        if self.sweep_buf.len() == STRIPE_ROWS as usize {
            self.flush_stripe()?;
        }
        Ok(())
    }

    /// Flushes what is buffered, then the directory, summary and footer.
    /// Returns the file's total length.
    pub fn finish(mut self) -> Result<u64> {
        if !self.sweep_buf.is_empty() {
            self.flush_stripe()?;
        }

        let directory_offset = self.pos;
        let directory = std::mem::take(&mut self.directory);
        for [first, last] in &directory {
            self.write_f64(*first)?;
            self.write_f64(*last)?;
        }
        self.directory = directory;

        let summary_offset = self.pos;
        let summary = std::mem::take(&mut self.summary);
        for [low, high] in &summary {
            self.write_f32(*low)?;
            self.write_f32(*high)?;
        }
        self.summary = summary;

        let index = Index {
            version: VERSION,
            rows: self.rows,
            stripe_rows: STRIPE_ROWS,
            factor: LEVEL_FACTOR,
            levels: self.levels,
            stripes: self.stripes,
            last_stripe_rows: self.last_stripe_rows,
            data_start: HEADER_BYTES,
            directory_offset,
            summary_offset,
            sweep: self.sweep_meta.clone(),
            signals: self.signals_meta.clone(),
        };
        // Checks the arithmetic the reader will rely on against the bytes
        // actually emitted. If these ever disagree the file is unreadable in
        // a way no checksum would catch, so it is worth asserting at the one
        // moment both numbers exist.
        debug_assert_eq!(index.directory_offset, directory_offset);
        if self.stripes > 0 {
            debug_assert_eq!(
                index.stripe_offset(self.stripes - 1) + index.stripe_bytes(self.stripes - 1),
                directory_offset
            );
        }

        let footer = serde_json::to_vec(&index).map_err(|e| Error::Format(e.to_string()))?;
        let footer_offset = self.pos;
        self.write_all(&footer)?;
        self.write_all(&footer_offset.to_le_bytes())?;
        self.write_all(MAGIC)?;
        self.sink.flush()?;
        Ok(self.pos)
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.sink.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    fn write_f64(&mut self, value: f64) -> Result<()> {
        self.write_all(&value.to_le_bytes())
    }

    fn write_f32(&mut self, value: f32) -> Result<()> {
        self.write_all(&value.to_le_bytes())
    }

    fn flush_stripe(&mut self) -> Result<()> {
        let rows = self.sweep_buf.len() as u32;

        let sweep = std::mem::take(&mut self.sweep_buf);
        for value in &sweep {
            self.write_f64(*value)?;
        }
        for level in 1..=self.levels {
            for (low, high) in envelope_f64(&sweep, level, self.factor()) {
                self.write_f64(low)?;
                self.write_f64(high)?;
            }
        }
        // The axis is monotonic in every analysis this solver runs, so these
        // are its first and last value. Taken as extremes rather than ends
        // so a non-monotonic sweep still produces a directory a window
        // search can trust.
        let span = [
            sweep.iter().copied().fold(f64::INFINITY, f64::min),
            sweep.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        ];
        self.directory.push(span);
        self.sweep_buf = sweep;
        self.sweep_buf.clear();

        let mut buffers = std::mem::take(&mut self.signal_buf);
        for samples in &buffers {
            for value in samples {
                self.write_f32(*value)?;
            }
            for level in 1..=self.levels {
                for (low, high) in envelope_f32(samples, level, self.factor()) {
                    self.write_f32(low)?;
                    self.write_f32(high)?;
                }
            }
            self.summary.push([
                samples.iter().copied().fold(f32::INFINITY, f32::min),
                samples.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            ]);
        }
        for samples in &mut buffers {
            samples.clear();
        }
        self.signal_buf = buffers;

        self.stripes += 1;
        self.last_stripe_rows = rows;
        Ok(())
    }

    fn factor(&self) -> u32 {
        LEVEL_FACTOR
    }
}

/// The min/max pair of every block at `level`.
///
/// Extremes, never a mean and never a stride sample. This is the whole
/// reason the pyramid is worth storing: a switching overshoot lasting one
/// time step is the thing a power-electronics waveform is read for, and
/// averaging or sampling a block throws it away silently -- the reduced
/// trace looks clean and the event is simply absent. Taking both extremes
/// keeps it at every reduction factor, which is what `--out-every`'s stride
/// cannot do.
macro_rules! envelope {
    ($name:ident, $t:ty) => {
        fn $name(samples: &[$t], level: u32, factor: u32) -> Vec<($t, $t)> {
            let rows = samples.len() as u32;
            let blocks = blocks_at(rows, level, factor);
            let span = (factor as u64).pow(level) as usize;
            let mut out = Vec::with_capacity(blocks as usize);
            for block in 0..blocks as usize {
                let lo = block * span;
                let hi = ((block + 1) * span).min(samples.len());
                let window = &samples[lo..hi];
                let mut low = <$t>::INFINITY;
                let mut high = <$t>::NEG_INFINITY;
                for value in window {
                    if *value < low {
                        low = *value;
                    }
                    if *value > high {
                        high = *value;
                    }
                }
                out.push((low, high));
            }
            out
        }
    };
}

envelope!(envelope_f64, f64);
envelope!(envelope_f32, f32);
