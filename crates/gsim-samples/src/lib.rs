//! The `.gsim_samples` container: a run written once, read the way a chart
//! reads it.
//!
//! ## The question this format answers
//!
//! A waveform viewer never wants a run. It wants *these five signals, over
//! this time window, at no more resolution than a screen can show*. This
//! solver's other two output formats cannot answer that, and not by
//! oversight -- both are row-major, and CSV's fields are variable-width
//! text, so the byte offset of one signal at one time step is not computable
//! from anything. The only way to find it is to scan from the start counting
//! separators. Reading five of seventy signals therefore costs what reading
//! all seventy costs. Measured on a real 121 MB, 69-signal result,
//! extracting five columns with `awk` takes 2.5 s and touches every byte; at
//! that rate a 10 GB run is about three and a half minutes, per draw,
//! repeated on every pan and zoom.
//!
//! Three properties fix that, and the format is nothing but their
//! consequences.
//!
//! **Signals are addressable.** Within a stripe each signal's samples are
//! contiguous, so reading five of seventy touches five signals' bytes and
//! not the other sixty-five. This is a larger factor than the pyramid
//! provides and it costs nothing but choosing which axis is contiguous.
//!
//! **Windows are addressable.** Every stripe but the last has identical
//! geometry, so every offset is arithmetic over five numbers in the footer,
//! and a directory of one sweep-extreme pair per stripe maps a time window
//! to stripes without reading a sample.
//!
//! **Resolution is precomputed.** Each stripe carries a min/max pyramid over
//! its own samples, so the number of points read is set by the viewer's
//! pixel budget rather than by the length of the run.
//!
//! That last claim is worth stating precisely, because the convenient
//! version of it is not true. The rungs of the pyramid are a fixed ratio
//! apart, so there is a rung *within* [`LEVEL_FACTOR`] of any budget, never
//! one exactly at it: a read returns between the budget and `LEVEL_FACTOR`
//! times the budget in points. At three thousand points and five signals
//! that is between about 84 KB and 5 MB, and which end depends on where the
//! budget happens to fall between two rungs. What does not vary is the run:
//! the same query against a 1 GB file and a 26 GB file reads the same
//! amount, and the bound is a property of the budget alone. Closing the gap
//! would mean a smaller ratio, and the ratio is also what the pyramid costs
//! on disk -- `2/(factor-1)`, about 3% at 64 against 28% at 8. Three percent
//! of a 26 GB run is worth a megabyte of over-read.
//!
//! The one case that does scale with the run is a full-extent view, which
//! lands on the stripe summary: one pair per stripe, so a 26 GB run offers
//! about three thousand points -- a screen's worth, by the time it matters.
//!
//! ## What it is not
//!
//! Not HDF5, which gets all three of those right and still cannot be used
//! here: every binding for it links the libhdf5 C library, and the reader
//! that matters most runs as WebAssembly in an editor's webview, where there
//! is nothing to link. Choosing a Rust HDF5 crate would move that problem
//! between languages rather than solve it.
//!
//! Not a replacement for CSV or the rawfile either. Those are good at being
//! handed to someone, diffed, or loaded by pandas, and they stay exactly as
//! they are. This is what a viewer reads.
//!
//! ## Layout
//!
//! ```text
//! magic "GSIMSAMP", u32 version, u32 flags          16 bytes
//! stripe 0 .. stripe n-1       sweep f64 then each signal f32,
//!                              each followed by its own pyramid levels
//! directory                    n * (f64 first, f64 last)
//! summary                      n * signals * (f32 min, f32 max)
//! footer                       JSON index
//! u64 footer offset, magic again                    16 bytes
//! ```
//!
//! The magic appears at both ends so that a run killed partway is a
//! recognisable failure rather than a file whose footer offset is read out
//! of whatever bytes landed last.

pub mod format;
pub mod read;
pub mod write;

pub use format::{Index, VectorMeta, LEVEL_FACTOR, STRIPE_ROWS, VERSION};
pub use read::{Bytes, Detail, Reader, Window};
pub use write::Writer;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The bytes are not a file this reader can use: wrong magic, a version
    /// it does not know, a footer that does not parse.
    Format(String),
    /// The file is fine and does not hold what was asked for.
    Missing(String),
    /// A read ran past the end, which for this format means truncation
    /// rather than a bad request: every offset is computed from the footer.
    Truncated(u64, usize),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Format(message) => write!(f, "{message}"),
            Error::Missing(name) => write!(f, "this run has no signal named \"{name}\""),
            Error::Truncated(offset, len) => write!(
                f,
                "wanted {len} bytes at {offset}, past the end of the file -- it is truncated"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A file read at offsets, without seeking.
///
/// Positioned reads rather than `Seek` plus `Read` so that `&self` is
/// enough: a reader is shared and asked for windows concurrently, and
/// threading a `&mut File` through that would serialise every draw behind
/// one cursor.
#[cfg(not(target_arch = "wasm32"))]
pub struct FileBytes {
    file: std::fs::File,
    size: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileBytes {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        Ok(Self { file, size })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Bytes for FileBytes {
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut buffer = vec![0u8; len];
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            self.file
                .read_exact_at(&mut buffer, offset)
                .map_err(|_| Error::Truncated(offset, len))?;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            let mut filled = 0usize;
            while filled < len {
                let n = self
                    .file
                    .seek_read(&mut buffer[filled..], offset + filled as u64)
                    .map_err(|_| Error::Truncated(offset, len))?;
                if n == 0 {
                    return Err(Error::Truncated(offset, len));
                }
                filled += n;
            }
        }
        Ok(buffer)
    }

    fn size(&self) -> u64 {
        self.size
    }
}
