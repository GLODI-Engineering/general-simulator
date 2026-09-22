//! Checkpoint/resume: a complete, serializable snapshot of a transient run's loop-carried
//! state, so a run can stop at some $t$ and continue later — from the same process, from a
//! file, on another machine — with a trace **bit-identical** to the uninterrupted run.
//!
//! That bit-identity is the whole contract, and it is what makes the feature testable: every
//! `f64` in the file is the exact value the loop held, every enum is the exact mode the loop
//! was in, and the first resumed step is an ordinary continuation (trapezoidal, with its
//! $x_{k-1}$ history intact), not a restart. Nothing is re-derived on load that the loop would
//! not have re-derived itself on the next step.
//!
//! # What is in a checkpoint
//!
//! Exactly what `simulate_transient_with_blocks_streamed`'s loop carries from one accepted
//! step to the next, and nothing that the netlist itself already determines:
//!
//! - the circuit state $x_k$ and $x_{k-1}$, the previous operating point (its diode segments
//!   and Norton currents), the previous gate states, the diode-segment fallback bookkeeping,
//!   and the ringing cooldown counter;
//! - $t$, the accepted-step count / step index (so `forced` logic continues, not restarts),
//!   and the adaptive controller's next $\Delta t$;
//! - `prev_outputs` — what every `prev:` signal reads on the next step;
//! - every block's own state, as a [`BlockSnapshot`]: state vectors, sample-time accumulators
//!   and held outputs, phases, logic bits, counts — and for the escape-hatch hosts, the opaque
//!   state as bytes (`pickle` for a `pyblock`, the library's own `cscript_state_write` for a
//!   `cscript`, an Octave `save -binary` for an `octblock`).
//!
//! Not in it: the netlist, the block definitions, the diode/switch models, the time-step
//! configuration. A checkpoint is loaded *into* a deck the caller supplies, and refuses a deck
//! that is not the one it came from (see [`Checkpoint::deck_hash`]).
//!
//! # File format
//!
//! `postcard`-encoded [`Checkpoint`], prefixed by the four bytes [`MAGIC`] and a little-endian
//! `u32` [`FORMAT_VERSION`]. postcard because it is compact, has a stable published wire
//! encoding, and round-trips `f64` bit-exactly (a text format would need care to do so).
//! A future incompatible layout bumps `FORMAT_VERSION`; an old file is then rejected up front
//! by [`Checkpoint::from_bytes`] rather than misread. Appending a variant to
//! [`BlockSnapshot`] is *not* such a change — postcard writes a variant as its index, so the
//! existing variants keep their bytes — which is how the `CScript`/`OctBlock` variants landed
//! without a bump.
//!
//! # Escape-hatch blocks
//!
//! `kind=pyblock` state round-trips via `pickle` with no author opt-in (the same reasoning as
//! its `copy.deepcopy`-based clone). `kind=octblock` state is the `__gs_state.<name>` slot in
//! the `octave-cli` child, saved and loaded with Octave's own `save -binary`/`load` through a
//! short-lived temporary file — also no opt-in. `kind=cscript` state is an opaque C heap object
//! only the author's code can lay out, so it is opt-in: a library exporting the
//! `cscript_state_size`/`cscript_state_write`/`cscript_state_read` triple checkpoints; one
//! exporting none of them keeps running unchanged but is refused at checkpoint time with
//! [`DaeError::CheckpointUnsupportedBlock`], naming the block, rather than written partially;
//! one exporting only some of them fails to load at all (`cscript_ffi::CScriptError::MissingSymbol`).

use std::collections::BTreeMap;
use std::hash::Hasher;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::DaeError;

/// The four bytes every checkpoint file starts with.
pub const MAGIC: &[u8; 4] = b"GSCK";
/// Bumped on any incompatible change to [`Checkpoint`]'s layout.
pub const FORMAT_VERSION: u32 = 1;

/// A `SignalValue` without the `general_mna` type — that crate has no serde support, and the
/// distinction is just "was it a vector."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SignalSnapshot {
    Scalar(f64),
    Vector(Vec<f64>),
}

/// One block's own persisted state. Variants mirror `block_graph::BlockState`, minus everything
/// the netlist rebuilds (the compiled state-space, the models, the FFI instances).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BlockSnapshot {
    Stateless,
    /// `Pid`/`StateSpace`/`TransferFunction`.
    Dynamic {
        x: Vec<f64>,
    },
    /// `DiscreteStateSpace`/`DiscreteTransferFunction`.
    DiscreteDynamic {
        x: Vec<f64>,
        time_since_sample: f64,
        last_output: Vec<f64>,
    },
    DiscretePid {
        int_state: f64,
        int_prev_e: f64,
        filt_state: f64,
        filt_prev_v: f64,
        time_since_sample: f64,
        last_output: f64,
    },
    /// `Vco` and `PhaseShiftPwm` — both a single phase in cycles.
    Phase(f64),
    Pmsm {
        x: Vec<f64>,
    },
    /// `Hysteresis` and `SrLatch` — a single persisted bit.
    Bit(bool),
    FlipFlop {
        q: bool,
        prev_clk: f64,
    },
    Counter {
        count: i64,
        prev_clk: f64,
    },
    /// `PyFunction`/`OctFunction` — stateless hosts, only the zero-order-hold bookkeeping.
    SampledFunction {
        time_since_sample: f64,
        last_output: Vec<f64>,
    },
    /// `PyBlock` — the zero-order-hold bookkeeping, the solver-owned `xc`, and the block's own
    /// state object as `pickle` bytes.
    PyBlock {
        time_since_sample: f64,
        next_hit_dt: f64,
        last_output: Vec<f64>,
        xc: Vec<f64>,
        state: Vec<u8>,
    },
    /// `CScript` — the same bookkeeping and `xc` as `PyBlock`, and the opaque C state as the
    /// bytes the library's own `cscript_state_write` produced (see `cscript_ffi`'s module doc
    /// comment, "The optional checkpoint state contract"). Only ever built for a library that
    /// exports that contract; otherwise the run is refused as
    /// [`DaeError::CheckpointUnsupportedBlock`].
    ///
    /// Appended after `PyBlock` on purpose: postcard encodes a variant as its index, so adding
    /// variants at the end leaves every existing file's encoding unchanged and
    /// [`FORMAT_VERSION`] stays at 1.
    CScript {
        time_since_sample: f64,
        next_hit_dt: f64,
        last_output: Vec<f64>,
        xc: Vec<f64>,
        state: Vec<u8>,
    },
    /// `OctBlock` — the same bookkeeping and `xc`, and the instance's `__gs_state.<name>` slot
    /// as the bytes of an Octave `save -binary` of it (see
    /// `octave_ffi::OctaveSession::save_state`). Needs no author opt-in.
    OctBlock {
        time_since_sample: f64,
        next_hit_dt: f64,
        last_output: Vec<f64>,
        xc: Vec<f64>,
        state: Vec<u8>,
    },
}

/// The previous accepted operating point, minus the `unknowns` names (stored once on the
/// [`Checkpoint`] itself).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointSnapshot {
    pub x: Vec<f64>,
    pub diode_names: Vec<String>,
    pub diode_z: Vec<(f64, f64)>,
    pub diode_raw_ioff: Vec<f64>,
}

/// The complete loop-carried state of a transient run at the end of an accepted step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Identifies the deck this was taken from — see [`deck_hash`]. A load into a deck with a
    /// different hash is refused as [`DaeError::CheckpointDeckMismatch`].
    pub deck_hash: u64,
    /// The system's unknown names, in `x` order — reported in the mismatch error so the reader
    /// can see *what* differs, and double-checked on load.
    pub unknowns: Vec<String>,
    pub t: f64,
    /// Accepted steps so far (the adaptive stall guard counts from here).
    pub accepted_steps: usize,
    /// The loop's own step index — nonzero on resume, so the first resumed step is not treated
    /// as a run's first step (which would force backward Euler).
    pub step_index: usize,
    /// The adaptive controller's suggested next step; `None` for a fixed-step run.
    pub dt_next: Option<f64>,
    pub ringing_cooldown: u32,
    pub x_prev: Vec<f64>,
    pub x_prev_prev: Option<Vec<f64>>,
    pub point_prev: PointSnapshot,
    pub prev_diode_raw_ioff: BTreeMap<String, f64>,
    /// Each diode's previous segment, as `classify_segments` numbers them (`0` breakdown, `1`
    /// leakage, `2` forward); `None` before the first accepted step.
    pub prev_segments: Option<Vec<u8>>,
    /// Each ideal switch's previous gate state, `true` for on.
    pub prev_gate_states: Option<BTreeMap<String, bool>>,
    pub prev_outputs: BTreeMap<String, SignalSnapshot>,
    /// One entry per block, in the caller's block order, keyed by name for the mismatch check.
    pub blocks: Vec<(String, BlockSnapshot)>,
}

impl Checkpoint {
    /// The file encoding: [`MAGIC`], [`FORMAT_VERSION`], then the postcard payload.
    pub fn to_bytes(&self) -> Result<Vec<u8>, DaeError> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        let payload = postcard::to_stdvec(self)
            .map_err(|e| DaeError::CheckpointFormat(format!("encoding failed: {e}")))?;
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// The inverse of [`Self::to_bytes`]; a wrong magic or an unknown version is reported as
    /// such rather than decoded into garbage.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DaeError> {
        if bytes.len() < 8 || &bytes[..4] != MAGIC {
            return Err(DaeError::CheckpointFormat(
                "not a general-simulator checkpoint (missing GSCK header)".to_string(),
            ));
        }
        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if version != FORMAT_VERSION {
            return Err(DaeError::CheckpointFormat(format!(
                "checkpoint format version {version} is not the {FORMAT_VERSION} this build \
                 reads"
            )));
        }
        postcard::from_bytes(&bytes[8..])
            .map_err(|e| DaeError::CheckpointFormat(format!("decoding failed: {e}")))
    }

    pub fn write_to(&self, path: &Path) -> Result<(), DaeError> {
        let bytes = self.to_bytes()?;
        // Write-then-rename, so a run killed mid-write (the very case `--checkpoint-every`
        // exists for) never leaves a truncated file where the last good checkpoint was.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes).map_err(|e| DaeError::CheckpointIo {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        std::fs::rename(&tmp, path).map_err(|e| DaeError::CheckpointIo {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
    }

    pub fn read_from(path: &Path) -> Result<Self, DaeError> {
        let bytes = std::fs::read(path).map_err(|e| DaeError::CheckpointIo {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        Self::from_bytes(&bytes)
    }
}

/// A deterministic 64-bit FNV-1a over everything that defines the deck a run is stepping:
/// the flattened statements, the dialect, the blocks (name, kind, inputs, `ic`), the diode and
/// switch models, and the shared on-resistance. Deliberately *not* `std`'s `DefaultHasher`,
/// whose output is not stable across Rust versions — this value lives in files.
///
/// Uses each value's `Debug` rendering as the hashed bytes: every one of these types already
/// derives `Debug` for its own reasons, and a `Debug` string changes exactly when the value
/// does, which is the property needed. It would also change if a type's `Debug` layout
/// changed between builds — acceptable, since that reads as "different deck" (a refusal), never
/// as a silent misload.
pub fn deck_hash(parts: &[&dyn std::fmt::Debug]) -> u64 {
    struct Fnv(u64);
    impl Hasher for Fnv {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            for b in bytes {
                self.0 ^= u64::from(*b);
                self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    for part in parts {
        h.write(format!("{part:?}").as_bytes());
        h.write(&[0xff]);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_round_trip_bit_exactly() {
        let ckpt = Checkpoint {
            deck_hash: 42,
            unknowns: vec!["V(a)".into()],
            t: 0.1 + 0.2,
            accepted_steps: 3,
            step_index: 3,
            dt_next: Some(f64::MIN_POSITIVE),
            ringing_cooldown: 1,
            x_prev: vec![1.0 / 3.0, -0.0, f64::MAX],
            x_prev_prev: None,
            point_prev: PointSnapshot {
                x: vec![1e-300],
                diode_names: vec![],
                diode_z: vec![],
                diode_raw_ioff: vec![],
            },
            prev_diode_raw_ioff: BTreeMap::new(),
            prev_segments: Some(vec![0, 1, 2]),
            prev_gate_states: Some([("S1".to_string(), true)].into_iter().collect()),
            prev_outputs: [("G".to_string(), SignalSnapshot::Scalar(0.7))]
                .into_iter()
                .collect(),
            blocks: vec![("G".into(), BlockSnapshot::Dynamic { x: vec![2.5] })],
        };
        let bytes = ckpt.to_bytes().unwrap();
        assert_eq!(&bytes[..4], MAGIC);
        let back = Checkpoint::from_bytes(&bytes).unwrap();
        assert_eq!(back, ckpt);
        // `-0.0 == 0.0` under PartialEq, so check the sign bit separately.
        assert!(back.x_prev[1].is_sign_negative());
    }

    #[test]
    fn a_foreign_or_future_file_is_refused_up_front() {
        assert!(matches!(
            Checkpoint::from_bytes(b"hello world"),
            Err(DaeError::CheckpointFormat(_))
        ));
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        assert!(matches!(
            Checkpoint::from_bytes(&bytes),
            Err(DaeError::CheckpointFormat(m)) if m.contains("version")
        ));
    }

    #[test]
    fn deck_hash_is_order_sensitive_and_stable() {
        let a = deck_hash(&[&"R1 a b 1", &"C1 b 0 1u"]);
        let b = deck_hash(&[&"C1 b 0 1u", &"R1 a b 1"]);
        assert_ne!(a, b);
        assert_eq!(a, deck_hash(&[&"R1 a b 1", &"C1 b 0 1u"]));
        // FNV-1a of the empty input is the offset basis -- pins the algorithm, since these
        // values live in files.
        assert_eq!(deck_hash(&[]), 0xcbf2_9ce4_8422_2325);
    }
}
