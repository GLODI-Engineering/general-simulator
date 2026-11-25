//! Prints what a `.gsim_samples` file holds, and a window of it.
//!
//! `cargo run --example peek -- <file> [signal] [max_points]`
//!
//! Exists because a binary container is otherwise unopenable by eye, and the
//! first question about a new format is always "is what I think is in there
//! actually in there". It also reports the bytes a draw cost, which is the
//! property the format is for and the one a size on disk cannot show.

use std::cell::Cell;

use gsim_samples::{Bytes, FileBytes, Reader, Result};

struct Counting {
    inner: FileBytes,
    read: Cell<u64>,
}

impl Bytes for &Counting {
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.read.set(self.read.get() + len as u64);
        self.inner.read_at(offset, len)
    }

    fn size(&self) -> u64 {
        self.inner.size()
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: peek <file> [signal] [max_points]")?;
    let wanted = args.next();
    let budget: usize = match args.next() {
        Some(n) => n.parse()?,
        None => 3_000,
    };

    let counting = Counting {
        inner: FileBytes::open(&path)?,
        read: Cell::new(0),
    };
    let size = counting.inner.size();
    let reader = Reader::open(&counting)?;
    let index = reader.index();

    println!("{path}: {} bytes", size);
    println!(
        "  {} rows x {} signals, {} stripes of {}, {} pyramid levels at {}x",
        index.rows,
        index.signals.len(),
        index.stripes,
        index.stripe_rows,
        index.levels,
        index.factor
    );
    if let Some((from, to)) = reader.extent() {
        println!("  {} from {from:e} to {to:e}", index.sweep.name);
    }
    let raw = index.rows * (8 + 4 * index.signals.len() as u64);
    println!(
        "  samples alone would be {raw} bytes, so the index and pyramid are {:.2}%",
        (size as f64 - raw as f64) / raw as f64 * 100.0
    );
    for signal in index.signals.iter().take(12) {
        println!("    {}", signal.name);
    }
    if index.signals.len() > 12 {
        println!("    ... and {} more", index.signals.len() - 12);
    }

    let name = match wanted {
        Some(n) => n,
        None => return Ok(()),
    };
    counting.read.set(0);
    let window = reader.slice(std::slice::from_ref(&name), None, budget)?;
    let cost = counting.read.get();
    println!();
    println!(
        "{name}: {} points from {:?}, reading {cost} bytes ({:.4}% of the file)",
        window.sweep.len(),
        window.detail,
        cost as f64 / size as f64 * 100.0
    );
    let low = window.values[0]
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let high = window.values[0]
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    println!("  extremes over the whole run: {low} .. {high}");
    Ok(())
}
