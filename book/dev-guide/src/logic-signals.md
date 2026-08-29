# Logic signals: gates, latches, flip-flops, counters — design survey

*(Design draft, not yet implemented — the actual code change follows once the open questions
below are settled. Scoped to the subset actually being built this pass: combinational gates, an
SR/fault latch, edge-triggered D/JK/T flip-flops, and a clocked counter. The full brainstormed
list this pass was chosen from — encoders/decoders, shift registers, an ALU, a generic FSM
block, and more — stays a candidate list for a later pass, not designed here.)*

## The type

**No new `SignalValue` variant.** This codebase already answered the "does a logic signal need
its own type" question, twice, before this survey existed: `Hysteresis`'s own comparator output
is already `SignalValue::Scalar(if on {1.0} else {0.0})`, and `GateBinding::resolve` thresholds
*any* block's output at `>= 0.5` regardless of what produced it. A logic gate's output is just
another `Scalar(1.0/0.0)` — directly wireable into a `Gain`, a `Sig2Voltage`, an ideal switch's own
`ctrl=`, with no cast block and no type error, unlike some typed block-diagram tools' own
`bool`/`double` split. That split exists there for code-generation memory-layout correctness (a
real `bool` is 1 byte, a real `double` is 8, and generated C can't silently mix them) — this
project has no code-generation target at all, everything is `f64` at simulation time, so the
justification doesn't transfer. See `docs/journal/2026-08.md` for the fuller discussion this
conclusion came out of.

**Reading a logic input**: every gate/latch/flip-flop/counter input is read via the same `>=
0.5` threshold `GateBinding` already uses — `require_all_scalar` first (a `Vector` input is
rejected, same default rule `vector-signals.md` established), then thresholded. No new
"logic-typed" input validation exists beyond that.

**Multi-bit values stay `Vector`, never a packed number.** A future decoder's N-bit output, or a
counter's own value fanned out to N separate bit-signals, should be N `Scalar` elements in one
`Vector` (a real bus — one wire per bit), not a single float encoding "the number 5." Not needed
by anything in this pass's own scope (the counter below outputs its running count as one
ordinary numeric `Scalar`, the ordinary meaning a control-loop reference or a modulo counter
already has elsewhere in this codebase) — noted here so a later encoder/decoder pass doesn't
have to re-litigate it.

---

## Category 1 — Combinational gates: one kind, an op enum, N-input reduction

**`kind=and` / `or` / `xor` / `nand` / `nor` / `xnor`** (N-input, `N >= 2`), **`kind=not`**
(exactly 1 input)

One `BlockKind::LogicGate(LogicOp)` — matching this codebase's own established "one kind, an op
enum" convention (`MathFn1`/`MathFn2`/`MathFn3`'s own function-name enums), not six-plus separate
`BlockKind` variants for what is structurally identical dispatch. Each variant thresholds every
input at `>= 0.5` (`require_all_scalar`, then threshold, same helper `Hysteresis`'s own arm
already calls), reduces with the obvious boolean rule, and outputs `Scalar(1.0)`/`Scalar(0.0)`:

- `And`/`Nand`: `1.0` iff every input is `true` (negated for `Nand`).
- `Or`/`Nor`: `1.0` iff at least one input is `true` (negated for `Nor`).
- `Xor`/`Xnor`: `1.0` iff an *odd* number of inputs are `true` (negated for `Xnor`) — the
  standard N-input generalization of 2-input XOR, not "exactly one," which doesn't generalize
  past 2 inputs in a way anyone actually expects.
- `Not`: exactly one input, `1.0` iff it's `false`. Structurally a 1-input special case of the
  same enum rather than its own `BlockKind` — `require_all_scalar` on a 1-element slice is no
  different from any other arity check already scattered through `evaluate_blocks`.

**No `Buffer` kind.** A non-inverting digital passthrough is already `kind=gain k=1` (or simply
wiring the same signal to two places) — redundant with an existing block, not worth a seventh
variant just for symmetry with a real chip family's own datasheet naming.

**This revises a documented prior decision, not an oversight.** `continuous-blocks`'s own
`waveform_arithmetic.rs` module doc comment explicitly excludes "Boolean/comparison operators
(bitwise and/or/xor, greater/less-than, negation)" from that module, on the grounds that they
"belong to a plot-expression language for post-processing saved waveforms, not simulation
blocks," and that `if`/`limit` already cover "the actual control-logic use case." That reasoning
holds for *stateless* combinational logic used as expression sugar — an `if`/threshold chain
genuinely can express an AND/OR of two signals. It does not hold for this pass's real target:
**sequential** logic with genuine memory (a fault latch that stays latched, a flip-flop, a
counter) — no combination of `MathFn3::If`/`MathFn2` calls can hold state across steps the way a
`BlockState` variant can, so those aren't achievable as expression sugar at all, stateless or
not. The combinational gates in this category are included alongside the sequential ones mainly
because interlock/protection logic (Category 2/the fault-latch use case) is naturally built from
*both* — an AND gating a fault latch's own reset, an OR aggregating several trip conditions into
one latch's `set` — not because the stateless-expression argument against them was wrong.

---

## Category 2 — SR / fault latch: level-triggered, no clock, the one genuinely new state shape

**`kind=srlatch`**

The one block in this pass that's level-sensitive rather than edge-triggered — `set`/`reset`
inputs act every step, immediately, no clock at all. `BlockState` needs exactly one `bool` (the
latched `q`), the same shape `Hysteresis`'s own `on: bool` already has:

```text
set, reset -> q_next
0,   0     -> q (hold)
1,   0     -> 1
0,   1     -> 0
1,   1     -> ??? (both asserted at once)
```

**Open question, not presumed: what happens when `set` and `reset` are both asserted in the same
step.** A real NOR-based SR latch treats this as invalid/undefined; this project's own
motivating use case is a **fault latch** (`set` = a fault trip condition, `reset` = an operator's
manual clear) — for that use case, a fault occurring in the exact same step as a reset command
should almost certainly still latch (silently clearing a live fault because the reset button was
already held down at the wrong instant is the wrong failure mode for a protection circuit).
**Recommendation: `set` dominant** (both asserted -> `q_next = 1`) as the default, with the
priority itself exposed as a parameter (`priority=set` default, `priority=reset` available) so a
netlist author modeling a real reset-dominant latch chip isn't stuck with the fault-latch-biased
default.

---

## Category 3 — Edge-triggered flip-flops: one state shape, a next-state-rule enum

**`kind=dff`** (D), **`kind=jkff`** (J-K), **`kind=tff`** (T, toggle)

Unlike the latch above, these are edge-triggered: the output only changes on a **rising edge**
of `clk` (`clk` crosses from `< 0.5` to `>= 0.5` between this step and the last). All three share
one `BlockState` shape — `{ q: bool, prev_clk: f64 }` — and differ only in the combinational
next-state rule evaluated *at the instant a rising edge is detected*:

- **D**: `q_next = d` (`d` thresholded the same `>= 0.5` way as every other logic input).
- **JK**: the classic 4-row table — `(j,k) = (0,0)` hold, `(1,0)` set, `(0,1)` reset, `(1,1)`
  toggle (`q_next = !q`).
- **T**: `q_next = q XOR t` — toggles when `t` is asserted, holds otherwise (a `T` flip-flop is
  just a `JK` with `j` and `k` tied together, but common enough in counter design — see Category
  4 — to deserve its own name rather than requiring a netlist author to wire `j=T k=T` by hand).

One `BlockKind::FlipFlop(FlipFlopKind)`, mirroring `LogicGate`'s own "one kind, an op enum"
shape — `FlipFlopKind::D | Jk | T`, each declaring its own fixed input arity (`D`: `d`+`clk`;
`Jk`: `j`+`k`+`clk`; `T`: `t`+`clk`) the same way `CoordinateTransform::input_count()` already
does for a fixed-but-per-variant arity.

**Own edge memory, not the generic `prev:` mechanism.** `Signal::BlockPrev`/`prev:` already
exists for "read another block's own previous-step output," and could in principle supply
`prev_clk` from outside — but a flip-flop's own `q` *must* persist regardless of whether `clk`
is even wired to a `prev:`-using block elsewhere, so `prev_clk` living inside this block's own
`BlockState` (the same place `q` already has to live) is simpler than requiring every flip-flop
declaration to also separately wire a `prev:` clock signal by hand. Not a new mechanism — the
same "own state, own bookkeeping" shape `Hysteresis`/`Vco`/every other stateful `BlockKind`
already has, just with two fields instead of one.

**Optional synchronous `reset=`**: if wired, checked at the same rising-edge instant *instead of*
the flip-flop's own next-state rule (`q_next = 0` unconditionally that edge) — synchronous only,
not asynchronous/immediate. An immediate, sub-step reset would need genuine event resolution
this project's fixed-time-step-grid evaluation doesn't have (the same "resolution is bounded by
`dt`, not exact" limitation every other discrete/edge-driven block here already lives with —
`Pwm`'s own carrier comparison, `Hysteresis`'s own threshold crossing — not a new one introduced
by this pass).

---

## Category 4 — Counter: the same edge-triggered shape, generalized from one bit to an integer

**`kind=counter`**

Same `{ ..., prev_clk: f64 }` edge-detection skeleton as Category 3, generalized: `count: i64`
(signed, so a down-counter naturally reads negative rather than needing awkward modulo
gymnastics) instead of `q: bool`, incrementing/decrementing by 1 on each rising `clk` edge.
Output is `Scalar(count as f64)` — an ordinary number, the same way this project already
represents any other integer-valued quantity (a `Pwm` cycle count, a sample index) as a plain
`f64`, not a fixed-width integer type (see "The type" above).

Parameters, all optional:
- `up_down=<signal>` — if wired, thresholded `>= 0.5` each edge to pick increment (`true`) vs.
  decrement (`false`); omitted, always increments (a plain up-counter, the common case).
- `modulus=<N>` — if given, `count` wraps into `[0, N)` (`rem_euclid`, so a decrement past `0`
  correctly wraps to `N-1`, not a negative remainder); omitted, `count` is a free-running signed
  `i64` (wraps at `i64`'s own bounds only, which no realistic run will ever reach — effectively
  unbounded for this project's purposes).
- `reset=<signal>` — same synchronous-only convention as Category 3's flip-flops: thresholded at
  the same rising-`clk` edge, forcing `count = 0` that edge instead of the normal
  increment/decrement.

**Not built as several separate kinds** (`kind=upcounter`/`downcounter`/`modcounter` from the
original brainstormed list) — `up_down=`/`modulus=` being ordinary optional parameters on one
`kind=counter` covers every combination (plain up, plain down via `up_down` tied to a constant
`0`, modulo-N, BCD as `modulus=10`, a ring/Johnson counter as a downstream decode of an ordinary
modulo counter's own value) without a combinatorial explosion of near-identical block kinds —
the same reasoning `Gain`'s scalar-vs-matrix modes and `LogicGate`'s op enum already follow.

---

## Foundational changes this touches, regardless of category

- Two new `BlockKind` variants family-wise (`LogicGate(LogicOp)`, `FlipFlop(FlipFlopKind)`) plus
  two standalone ones (`SrLatch`, `Counter`) in `general-mna`'s `block_graph.rs`, each with its
  own `system_builder.rs` netlist-parsing arm.
- New `BlockState` variants in `dae-runtime`'s `block_graph.rs`: `SrLatch { q: bool }`,
  `FlipFlop { q: bool, prev_clk: f64 }`, `Counter { count: i64, prev_clk: f64 }` — `LogicGate`
  needs no persistent state at all (purely combinational, recomputed fresh every step, the same
  as `Sum`/`Gain` already are).
- A single shared "rising edge detected" helper (`clk >= 0.5 && prev_clk < 0.5`), used by both
  Category 3 and Category 4 rather than duplicated three-plus times.
- **Not affected**: `SignalValue`, `topological_order`/`block_index_by_name` (agnostic to what a
  block computes), the physical/signal-domain converter boundary (`Sig2Voltage`/`Sig2Current`/
  `Probe` are unaffected — a logic block's `Scalar` output crosses that boundary exactly like
  any other block's already does).

## Deferred (explicitly out of scope for this pass)

- Everything else on the original brainstormed list: encoders/decoders, multiplexers, adders/
  ALU, shift registers, ring/Johnson counters as their own dedicated kinds (achievable today by
  decoding an ordinary `kind=counter`'s own value externally — not blocked on anything here),
  a generic Moore/Mealy FSM block, LFSR, debouncer/monostable, digital PLL.
- Asynchronous (immediate, sub-step) reset — see Category 3's own note.
- Multi-bit bus representation for a future encoder/decoder (`Vector`-of-bits, per "The type"
  above) — the type decision is recorded now so a later pass doesn't have to re-derive it, but
  nothing in *this* pass needs it.
