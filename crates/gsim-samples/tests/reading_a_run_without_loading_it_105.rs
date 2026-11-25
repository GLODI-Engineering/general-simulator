//! What this format exists for: a few signals, a window, a screen's worth of
//! points, and a read that does not grow with the run.
//!
//! The tests that matter here are the two about cost -- `bytes_read` below --
//! because every other property this format has is in service of them. A
//! round trip that is correct but reads the whole file would be the thing it
//! replaces.

use std::cell::RefCell;

use gsim_samples::{Bytes, Detail, Reader, Result, VectorMeta, Writer, STRIPE_ROWS};

/// Wraps a buffer and records what was asked of it.
///
/// The claim this format is built on is not "the right numbers come back" --
/// CSV manages that -- it is "and only this many bytes were touched". That
/// is invisible to an ordinary assertion, so it is measured.
struct Counting {
    inner: Vec<u8>,
    read: RefCell<u64>,
    calls: RefCell<u64>,
}

impl Counting {
    fn new(inner: Vec<u8>) -> Self {
        Self {
            inner,
            read: RefCell::new(0),
            calls: RefCell::new(0),
        }
    }

    fn bytes_read(&self) -> u64 {
        *self.read.borrow()
    }

    fn reset(&self) {
        *self.read.borrow_mut() = 0;
        *self.calls.borrow_mut() = 0;
    }
}

impl Bytes for &Counting {
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        *self.read.borrow_mut() += len as u64;
        *self.calls.borrow_mut() += 1;
        self.inner.as_slice().read_at(offset, len)
    }

    fn size(&self) -> u64 {
        self.inner.len() as u64
    }
}

/// A run with a known shape: signal `k` is `k + sin`, except signal 1 which
/// carries a single-sample spike a third of the way in -- the event a
/// stride-sampled or mean-averaged reduction loses.
const SPIKE_VALUE: f64 = 1_000.0;

fn write_run(rows: usize, signals: usize) -> (Vec<u8>, usize) {
    let spike_at = rows / 3;
    let names: Vec<VectorMeta> = (0..signals)
        .map(|k| VectorMeta::new(format!("V(n{k})")).with_unit("V"))
        .collect();
    let mut buffer = Vec::new();
    let mut writer = match Writer::new(&mut buffer, VectorMeta::new("t").with_unit("s"), names) {
        Ok(w) => w,
        Err(e) => panic!("a writer: {e}"),
    };
    for row in 0..rows {
        let t = row as f64 * 5e-9;
        let mut values: Vec<f64> = (0..signals)
            .map(|k| k as f64 + (row as f64 / 97.0).sin())
            .collect();
        if row == spike_at {
            values[1] = SPIKE_VALUE;
        }
        writer.push(t, &values).expect("push");
    }
    writer.finish().expect("finish");
    (buffer, spike_at)
}

#[test]
fn a_short_run_round_trips_exactly() {
    let rows = 1_000;
    let (buffer, _) = write_run(rows, 3);
    let reader = Reader::open(buffer.as_slice()).expect("opens");

    assert_eq!(reader.rows(), rows as u64);
    assert_eq!(reader.signals().len(), 3);
    assert_eq!(reader.signals()[2].name, "V(n2)");
    assert_eq!(reader.sweep().unit, "s");

    // Under the budget, so nothing is reduced and the samples come back.
    let window = reader
        .slice(&["V(n2)".to_string()], None, 10_000)
        .expect("slice");
    assert_eq!(window.detail, Detail::Level(0));
    assert_eq!(window.sweep.len(), rows);
    assert_eq!(window.values[0].len(), rows);
    for row in 0..rows {
        let expected = 2.0 + (row as f64 / 97.0).sin();
        // f32 storage, so compare at f32 precision rather than f64.
        assert!(
            (window.values[0][row] - expected).abs() < 1e-6,
            "row {row}: {} vs {expected}",
            window.values[0][row]
        );
        assert!((window.sweep[row] - row as f64 * 5e-9).abs() < 1e-18);
    }
}

#[test]
fn one_signal_of_many_comes_back_right() {
    // The offset arithmetic across signals is the part a round trip of a
    // single-signal run cannot check: every signal would be at offset zero.
    let (buffer, _) = write_run(STRIPE_ROWS as usize + 500, 12);
    let reader = Reader::open(buffer.as_slice()).expect("opens");

    for signal in [0usize, 5, 11] {
        let name = format!("V(n{signal})");
        let window = reader
            .slice(std::slice::from_ref(&name), Some((0.0, 20e-9)), 10_000)
            .expect("slice");
        assert_eq!(window.detail, Detail::Level(0));
        assert!(
            (window.values[0][0] - signal as f64).abs() < 1e-6,
            "{name} starts at {} not {signal}",
            window.values[0][0]
        );
    }
}

#[test]
fn a_run_longer_than_one_stripe_keeps_its_last_rows() {
    // The last stripe is the only short one, and every offset after it is
    // computed from `stripe_rows` rather than from it -- so an off-by-one
    // there is invisible until the final rows are asked for.
    let rows = STRIPE_ROWS as usize * 2 + 37;
    let (buffer, _) = write_run(rows, 4);
    let reader = Reader::open(buffer.as_slice()).expect("opens");
    assert_eq!(reader.index().stripes, 3);
    assert_eq!(reader.index().last_stripe_rows, 37);

    let last_t = (rows - 1) as f64 * 5e-9;
    let (_, extent_end) = reader.extent().expect("an extent");
    assert!((extent_end - last_t).abs() < 1e-18);

    let window = reader
        .slice(
            &["V(n3)".to_string()],
            Some((last_t - 5e-8, last_t)),
            10_000,
        )
        .expect("slice");
    assert_eq!(window.detail, Detail::Level(0));
    let expected = 3.0 + ((rows - 1) as f64 / 97.0).sin();
    let got = *window.values[0].last().expect("a last value");
    assert!((got - expected).abs() < 1e-6, "{got} vs {expected}");
}

#[test]
fn a_one_sample_spike_survives_every_level() {
    // The reason the pyramid stores both extremes instead of a mean or a
    // stride sample. At 64x a spike is one sample in 64; at 4,096x it is one
    // in 4,096. Both reductions must still show it, because on a switching
    // waveform it is the overshoot the run was done to find.
    let rows = STRIPE_ROWS as usize * 3;
    let (buffer, _) = write_run(rows, 2);
    let reader = Reader::open(buffer.as_slice()).expect("opens");
    assert!(reader.index().levels >= 2, "expected a pyramid to test");

    for level in 0..=reader.index().levels {
        let window = reader
            .read_detail(&["V(n1)".to_string()], None, Detail::Level(level))
            .expect("a level");
        let peak = window.values[0]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (peak - SPIKE_VALUE).abs() < 1.0,
            "level {level} lost the spike: peak {peak}"
        );
    }

    let summary = reader
        .read_detail(&["V(n1)".to_string()], None, Detail::Summary)
        .expect("a summary");
    let peak = summary.values[0]
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (peak - SPIKE_VALUE).abs() < 1.0,
        "the summary lost the spike"
    );
}

#[test]
fn a_full_extent_draw_reads_a_sliver_of_the_file() {
    // The property the format exists for, stated as what is actually
    // guaranteed. A pyramid with a fixed ratio between rungs offers a rung
    // *within* that ratio of any budget, never one exactly at it, so a draw
    // reads between the budget and `factor` times it. What it never does is
    // read the run.
    let rows = STRIPE_ROWS as usize * 20;
    let (buffer, _) = write_run(rows, 8);
    let size = buffer.len() as u64;
    let counting = Counting::new(buffer);

    let reader = Reader::open(&counting).expect("opens");
    counting.reset();

    let budget = 3_000;
    let window = reader
        .slice(&["V(n1)".to_string(), "V(n5)".to_string()], None, budget)
        .expect("slice");

    let read = counting.bytes_read();
    assert!(
        read * 20 < size,
        "a full-extent draw read {read} of {size} bytes, which is not a reduction"
    );
    // Two signals at f32 plus the f64 axis is 16 bytes a point, so the bound
    // the rung guarantees is `factor * budget * 16`.
    let bound = gsim_samples::LEVEL_FACTOR as u64 * budget as u64 * 16;
    assert!(read <= bound, "{read} bytes against a bound of {bound}");
    assert!(
        window.sweep.len() >= budget,
        "{} points is under the budget asked for",
        window.sweep.len()
    );
    assert_eq!(window.values.len(), 2);
    assert_eq!(window.values[0].len(), window.sweep.len());
}

#[test]
fn a_draw_never_returns_more_than_the_factor_times_the_budget() {
    // The bound that does not depend on the run. `detail_for` takes the
    // coarsest rung with at least the budget on it; the rung above had less
    // than the budget and holds `factor` times fewer points, so the one
    // chosen carries under `factor * budget`. That holds at any run length,
    // which is the sense in which the cost of a draw is bounded -- not that
    // it is the same number every time, but that it is never set by how long
    // the run was.
    for stripes in [2usize, 8, 40] {
        let (buffer, _) = write_run(STRIPE_ROWS as usize * stripes, 6);
        let reader = Reader::open(buffer.as_slice()).expect("opens");
        for budget in [500usize, 3_000, 20_000] {
            let window = reader
                .slice(&["V(n2)".to_string()], None, budget)
                .expect("slice");
            let points = window.sweep.len();
            let bound = budget * gsim_samples::LEVEL_FACTOR as usize;
            assert!(
                points < bound,
                "{stripes} stripes at a {budget} budget returned {points}, over {bound} \
                 ({:?})",
                window.detail
            );
            // And at least the budget, unless the whole window is smaller
            // than the screen, in which case everything there is comes back.
            assert!(
                points >= budget || window.detail == Detail::Level(0),
                "{points} points at a {budget} budget from {:?}",
                window.detail
            );
        }
    }
}

#[test]
fn a_narrow_window_reads_the_samples_and_a_wide_one_does_not() {
    let rows = STRIPE_ROWS as usize * 20;
    let (buffer, _) = write_run(rows, 4);
    let reader = Reader::open(buffer.as_slice()).expect("opens");

    // One stripe's worth of time: level 0 has 65,536 points for it, which is
    // over a 3,000 budget, so a coarser rung is picked.
    let narrow = reader.detail_for(Some((0.0, 100e-9)), 3_000);
    assert_eq!(
        narrow,
        Detail::Level(0),
        "a 20-sample window is under budget"
    );

    // 20 stripes give 40 summary points and 640 at level 2, both under a
    // 3,000 budget, so level 1 is the coarsest rung that saturates a screen.
    // The summary is reached when a run is long enough to have thousands of
    // stripes, which is where it earns its place.
    let whole = reader.detail_for(None, 3_000);
    assert_eq!(
        whole,
        Detail::Level(1),
        "20 stripes do not reach the summary"
    );

    let coarse = reader.detail_for(None, 30);
    assert_eq!(
        coarse,
        Detail::Summary,
        "a 30-point budget is what 20 stripes serve"
    );
}

#[test]
fn a_truncated_file_says_so_rather_than_reading_rubbish() {
    let (buffer, _) = write_run(2_000, 3);
    let cut = &buffer[..buffer.len() - 4];
    let error = Reader::open(cut).expect_err("a truncated file must not open");
    let message = error.to_string();
    assert!(
        message.contains("truncated"),
        "unhelpful message: {message}"
    );
}

#[test]
fn bytes_that_are_not_this_format_are_refused() {
    let error = Reader::open(vec![0u8; 64]).expect_err("random bytes must not open");
    assert!(error.to_string().contains("not a .gsim_samples file"));
}

#[test]
fn asking_for_a_signal_that_is_not_there_names_it() {
    let (buffer, _) = write_run(100, 2);
    let reader = Reader::open(buffer.as_slice()).expect("opens");
    let error = reader
        .slice(&["V(nowhere)".to_string()], None, 100)
        .expect_err("an unknown signal");
    assert!(error.to_string().contains("V(nowhere)"), "{error}");
}

#[test]
fn a_run_with_no_signals_is_refused_at_the_writer() {
    let mut buffer = Vec::new();
    let error = match Writer::new(&mut buffer, VectorMeta::new("t"), Vec::new()) {
        Ok(_) => panic!("no signals is not a run"),
        Err(e) => e,
    };
    assert!(error.to_string().contains("nothing to store"), "{error}");
}

#[test]
fn a_row_of_the_wrong_width_is_refused() {
    let mut buffer = Vec::new();
    let mut writer = match Writer::new(
        &mut buffer,
        VectorMeta::new("t"),
        vec![VectorMeta::new("a"), VectorMeta::new("b")],
    ) {
        Ok(w) => w,
        Err(e) => panic!("a writer: {e}"),
    };
    writer.push(0.0, &[1.0, 2.0]).expect("a good row");
    let error = writer.push(1.0, &[1.0]).expect_err("a short row");
    assert!(
        error.to_string().contains("1 values for 2 signals"),
        "{error}"
    );
}
