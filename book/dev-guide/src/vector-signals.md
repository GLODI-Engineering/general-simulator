# Vector signals: per-block survey

*(Design draft, not yet implemented — the actual code change follows once the open questions
below are settled. See `open-questions.md`'s own "MIMO `StateSpace`/`TransferFunction` in the
block graph" entry, which this proposal folds in rather than duplicates.)*

## The type

Every signal in the graph today is a bare `f64` (`outputs`/`prev_outputs`:
`BTreeMap<String, f64>`, `Signal::resolve` returns `f64`). This proposal introduces

```rust
enum SignalValue {
    Scalar(f64),
    Vector(Vec<f64>),
}
```

as the new value type flowing through `outputs`, with `Signal::Block`/`Signal::BlockPrev`
resolving to a `SignalValue` instead of a bare `f64`. Arity for a `Vector` is fixed once a block
declares it (known at graph-build time, same as everything else in this codebase) — no
dynamic/runtime-determined length in this pass; that would be a separate, larger feature (see
"Deferred" below).

**No type tagging beyond `Scalar`/`Vector` is needed.** There is no `bool` type anywhere in this
codebase today, even for scalars — a "boolean" signal (a `Hysteresis` output, a PWM
`main`/`complement` output) is just an `f64` interpreted via a `>= 0.5` threshold by whatever
reads it. A vector element is exactly as untyped as a scalar signal already is; nothing about
`Vec<f64>` needs to distinguish "this element is really a voltage" from "this element is really
a gate command" — that was never tracked for scalars either, and doesn't need to become tracked
now.

**Default rule for every block not explicitly listed below: reject a `Vector` input with a
clear, new error** (e.g. `DaeError::VectorSignalNotSupported { block, input }`) rather than
silently truncating, silently using only the first element, or panicking. Fail closed.

**A `GateBinding::Block` target must resolve to a `Scalar`** — rejected the same way if it
doesn't; a gate reading "a vector, thresholded how?" is exactly as ill-defined as the codebase
already refuses to guess elsewhere.

---

## Category 1 — Elementwise unary map (broadcasts automatically, never rejects)

**`MathFn1`** (all 24: `abs acos acosh asin asinh atan atanh buf ceil cos cosh exp floor int inv
ln log10 round sgn sin sinh sqrt tan tanh u uramp`), **`Table`**

`Scalar -> Scalar` unchanged. `Vector(N) -> Vector(N)`, the function/table applied independently
to each element. No rejection case exists — any vector length is valid.

---

## Category 2 — Elementwise combine with scalar broadcast

**`MathFn2`** (`atan2 angle_wrapped hypot max min pow pwr pwrs`), **`MathFn3`** (`if limit`)

Per-operand rule, applied to each of the 2 (or 3) operands:
- All operands `Scalar` -> `Scalar` output, unchanged.
- Every `Vector` operand present must share one common length `N`; a lone `Scalar` operand
  broadcasts against that `N` (same value used for every element). Output is `Vector(N)`.
- Two `Vector` operands of *different* nonzero lengths -> rejected, clear size-mismatch error.

---

## Category 3 — N-input reduction (no implicit broadcast)

**`Sum`**, **`Product`**

- All inputs `Scalar` -> `Scalar`, unchanged (today's exact behavior).
- All inputs `Vector`, same length `N` -> elementwise reduction, `Vector(N)` output.
- **Mixed `Scalar`+`Vector` inputs are rejected**, not broadcast. Unlike category 2 (one scalar,
  one vector — an unambiguous pairing), a reduction over several inputs has no unambiguous
  answer for "which position does a lone scalar broadcast into" once more than one vector is
  already present; rather than guess, this is a hard error.
- Differing-length `Vector` inputs among each other -> rejected.

---

## Category 4 — Single-operand scale, scalar or matrix

**`Gain`**

Exactly the two modes described in the request:
- **Scalar gain `k`** (today's `Gain(f64)`, unchanged): `Scalar -> Scalar` (`k*x`, as today);
  `Vector(N) -> Vector(N)`, every element scaled by `k` (broadcast).
- **Matrix gain `K` (M×N)** — new: requires a `Vector` input of length exactly `N` (rejects a
  `Scalar` input, rejects a `Vector` of the wrong length); output is `Vector(M)`, the matrix-
  vector product `K*x`.

**Open question:** represent both modes as one `Gain(GainValue)` where `GainValue =
Scalar(f64) | Matrix(Vec<Vec<f64>>)`, or add a new sibling kind (`GainMatrix(Vec<Vec<f64>>)`)
and leave `Gain(f64)`'s own type untouched? The sibling-kind route costs one more `BlockKind`
variant but touches zero existing code paths or serialized netlists; the enum-payload route is
more "the same block, richer," but changes `Gain`'s own type everywhere it's matched today.
**Recommendation: sibling kind**, matching this codebase's own established preference (e.g.
`Pwm`/`PhaseShiftPwm` as two kinds rather than one `Pwm` with a mode flag) — flagged here rather
than presumed.

---

## Category 5 — Elementwise saturation

**`Saturation`**

`Scalar -> Scalar` unchanged (clamped to `[-limit, limit]`). `Vector(N) -> Vector(N)`, every
element independently clamped to the *same* scalar bound (broadcast, same spirit as `Gain`'s
scalar mode). A per-element limit vector is not proposed here — noted as a possible future
extension, not needed for the motivating use case.

---

## Category 6 — Vector-capable sources

**`Const`**

Trivial and useful: a literal constant vector (e.g. a per-element offset feeding a `Vector` into
`Sum`). Same representation question as `Gain`: **recommend a new sibling kind
(`ConstVector(Vec<f64>)`)** rather than changing `Const(f64)`'s own type.

**`Time`, `Pwc`, `Pwl`, `Waveform`** — **out of scope for this pass.** All four are inherently
scalar, time-indexed functions; a genuinely vector-*valued* waveform (its own breakpoints one
per element) is a real, separate, larger feature not needed to cover the motivating case
(bundling a `cscript` block's own array output into one wire). Deferred, not rejected outright —
revisit if a concrete need for a vector-valued time-series source shows up.

---

## Category 7 — Dynamic/stateful single-input blocks: reject for now, `StateSpace`/
`TransferFunction` flagged as the *real* home for MIMO

**`Pid`**, **`Vco`**, **`Hysteresis`** — **reject `Vector` inputs.** Each carries genuinely
scalar-only internal logic (anti-windup clamp comparison, oscillator phase, a boolean-like
on/off latch) with no natural per-element generalization; a "vectorized" version of any of these
would mean *N independent instances*, a real state-fan-out feature, not an elementwise op — out
of scope here, noted as a possible future extension.

**`StateSpace`, `TransferFunction`** — **don't bolt on an elementwise rule here.** This
project's own `open-questions.md` already records that `continuous_blocks::StateSpace` supports
a general `(A, B, C, D)` system, but `block_graph.rs`'s own dispatch currently hardcodes
single-input/single-output. Genuine MIMO support for these two block kinds *is* vector-signal
support for them — a `Vector(N)` input feeding `B`'s `N` columns, a `Vector(M)` output reading
`C`'s `M` rows, `x`/RK4 stepping unchanged (already a `Vec<f64>` internally). Recommend doing
this as a coordinated follow-up once the rest of the vector-signal plumbing (the `SignalValue`
type, `evaluate_blocks`' dispatch) exists, rather than a bespoke rule invented for this survey —
it closes an already-recorded open question instead of duplicating it.

---

## Category 8 — PWM modulators: not applicable

**`Pwm`, `PhaseShiftPwm`** — duty/frequency/phase are each a single physical carrier-timing
quantity; there's no coherent meaning for a "vector duty command" on one modulator (N
independent modulators is just N separate declared blocks, already possible today without
vector signals at all). **Reject `Vector` inputs.** Their own two outputs (`main`/`complement`)
stay two separate scalar names, unaffected.

---

## Category 9 — Fixed-arity multi-output transforms: input bundling, output flagged open

**`CoordinateTransform`** (six transforms), **`Pmsm`**

Both already have a *fixed*, known-at-parse-time arity (`kind.input_count()` /
`CoordinateTransform`'s own 3 or 4; always 3 for `Pmsm`: `vd, vq, t_load`) — the cleanest,
lowest-risk place to add real vector-signal value.

- **Input side (recommended, low-risk):** accept *either* the current `inputs=A,B,C,...` form
  (N separately-named scalars) *or* one `Vector` signal of exactly the required length in its
  place. Unambiguous, since the arity is already fixed and known.
- **Output side — open question, not presumed:** today the block's own `.name` aliases
  `output_names[0]` (a single scalar — e.g. `Clarke`'s own `alpha`). Changing `.name` to mean
  "the whole output vector" would silently break every existing netlist reading that name
  expecting a scalar. **Recommendation: additive only** — an optional new alias (e.g.
  `vector_output=<name>`) binding a *new*, separately-named `Vector` value spanning the whole
  output, leaving the existing per-component scalar names and `.name`'s own scalar meaning
  completely untouched by default.

---

## Category 10 — `cscript`: the motivating case

- **Input side:** each entry in `inputs=` can independently be a `Scalar` or a `Vector` signal;
  `evaluate_blocks` flattens them, in declared order, into the same flat `in[]`/`in_len` array
  `cscript_output`/`cscript_output_xc`/`cscript_derivative` already receive. **Zero change to
  the C ABI** — only to how the Rust side assembles the array before the call. This alone fully
  covers the motivating case (pointing one `in=` entry at an upstream block's whole vector
  output) with no `cscript_ffi` changes at all.
- **Output side — open question, symmetric with category 9:** an opt-in `vector_output=<name>`
  binding the *whole* `out[]` array as one `Vector` value, additive alongside the existing
  per-slot `outputs=X,Y,Z` scalar names (which stay the default).
- **`xc` (continuous state):** unaffected either way. `xc` is private per-block state, never
  exposed as a signal, exactly as today — no interaction with vector-signal support.

---

## Category 11 — Physical/signal-domain converters: not applicable

**`Probe`, `Sig2Voltage`, `Sig2Current`** — each is tied to exactly one circuit quantity (one
node voltage, one branch current, one source's own magnitude — `Sig2Voltage` also covers a
MOSFET gate's on/off state, no separate gate-only converter exists). There is no vector
generalization that means anything physically here. `Sig2Voltage`/`Sig2Current` **reject a
`Vector` input** with a clear error; `Probe` has zero inputs and stays scalar-output-only.

---

## Foundational changes this touches, regardless of category

- `outputs`/`prev_outputs`: `BTreeMap<String, f64>` -> `BTreeMap<String, SignalValue>`.
- `Signal::resolve` returns `SignalValue`; every `evaluate_blocks` dispatch arm pattern-matches
  it per its own category's rule above.
- New `DaeError` variants: unsupported-vector-input, and a size-mismatch case (categories 2/3/4/
  9's "vectors must agree in length" rules).
- `resolve_gates` requires `Scalar`, same rejection story.
- **Not affected:** `topological_order`/`block_index_by_name` — both are keyed purely on names,
  agnostic to whether a name resolves to a `Scalar` or `Vector` value. `xc_count`/`cscript`'s
  continuous-state machinery — orthogonal, unaffected.
- **Open, CLI-level:** how a `Vector`-valued top-level output (a new `ConstVector`, a
  `vector_output=` alias) prints as a CSV column — one column per element (`NAME[0]`, `NAME[1]`,
  ...) is the natural choice, flagged here rather than presumed.

## Deferred (explicitly out of scope for this pass)

- Dynamic/runtime-determined vector length.
- Vectorized `Pid`/`Vco`/`Hysteresis` (N independent instances, a state-fan-out feature).
- Vector-valued time-series sources (`Pwc`/`Pwl`/`Waveform`).
- Per-element `Saturation` limits.
