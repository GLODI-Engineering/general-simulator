# Post-simulation measurements (`kind=measure`)

`kind=measure` is a netlist line that computes one scalar (or, for `four`, a small handful of
related scalars) from an already-completed transient trace — the same job ngspice's `.measure`
and Xyce's `.MEASURE` statements do, expressed in this project's own `kind=` `key=value` grammar
(lowercase field names, exactly like every other `kind=` block). It is evaluated once, after
`--mode transient` finishes, over the whole resolved waveform; it is **not** a circuit element or
a block-graph block (see `general-simulator`'s own `book/dev-guide/src/measurements-architecture.md` for why,
and how it's kept out of `general-mna`/`dae-runtime` entirely).

```text
NAME kind=measure type=<measure-type> out=<signal> [field=value ...]
```

`NAME` is the measurement's own name — it becomes the printed result's label (or a label prefix,
for `four`). `type=` selects which computation to run; the rest of the fields depend on `type=`,
mirroring ngspice/Xyce's own option names (`TRIG`/`TARG`/`VAL`/`AT`/`FROM`/`TO`/`TD`/`RISE`/
`FALL`/`CROSS`/`WHEN`/`FIND`) translated into this grammar's `lower_snake=value` convention.

## `out=`/`ref=`: naming a signal

Every measurement reads one or more already-resolved trace columns by name — exactly the column
names `--format csv`/`--format raw` already print (see
[Reading the output](reading-output.md)): `V(<node>)` for a node voltage, or a declared block's
own name (`SIG`, `PID1`, `PWM1_ON`, `MYBLOCK[0]` for a vector element, ...) for a block output.
There is no other way to name a signal in a `kind=measure` line — in particular, no bare node
name and no `I(branch)` support yet (a current measurement can be added the same way once a
concrete need for it comes up; today the trace has no branch-current columns to read from at
all).

## The measurement window: `from=`/`to=`/`td=`

Every measure type accepts an optional window (ngspice/Xyce's `FROM=`/`TO=`/`TD=`): the
effective window is `[max(from, td), to]`, intersected with the trace's own domain. All three
default to unconstrained (the whole trace) when omitted. For `trig_targ`, each side has its own
independent window, prefixed `trig_from=`/`trig_to=`/`trig_td=` and `targ_from=`/`targ_to=`/
`targ_td=`.

## Crossing selection: `rise=`/`fall=`/`cross=`

Every measurement that searches for a threshold crossing (`when`, the `when=` forms of `deriv`/
`find`, and `trig_targ`'s crossing/`frac_max` event forms) picks exactly one of `rise=`, `fall=`,
or `cross=` (rising-only, falling-only, or either direction) with a value that is either a
1-based occurrence count (`rise=1` — the first rising crossing) or the literal `last`
(`rise=last`). Omitting all three defaults to `rise=1` (ngspice's own default when `RISE=`/
`FALL=`/`CROSS=` isn't given at all). `trig_targ` uses the same fields prefixed `trig_`/`targ_`.

## Where results are written

Every `kind=measure` result writes to a `<netlist stem>.log` file next to the netlist (e.g.
`some.cir` → `some.log`), one `name = value` line per result, ngspice's own printed style (its
manual: `tdiff = 1.000000e-003 targ= ... trig= ...`) — never to stdout or stderr. This matches
real SPICE tools' own `.measure`/`.MEASURE` convention (both ngspice and Xyce write measurement
results to a log file, not the console) and is deliberate for the same reason: stdout stays
exactly what `--format csv` always produced (a plain, machine-readable CSV — unaffected byte-for-
byte by whether the netlist has any `kind=measure` lines at all) or, for `--format raw`, the
binary rawfile written to `--out`/its default path, and stderr stays free of measurement text too,
so neither existing output stream is corrupted by mixing in "name = value" lines. A `kind=measure`-
free netlist produces no `.log` file at all, and its stdout is unchanged from before this feature
existed.

A failed measurement (an unknown signal name, a crossing that never occurs in its window, ...)
writes `measurement 'NAME' failed: <reason>` to the same log file and does **not** abort the run
or any other measurement — every other measurement is still evaluated and written.

## Full field reference

Every field below, and every worked example, was run against the real CLI — see
`tests/measure/` for the committed fixtures and their hand-derived expected values (a max/min
ramp, a 50%-duty square wave, a pure sine and a two-tone signal for `four`, an intentionally
non-uniform-timestep adaptive run, and a pair of `err`/`error` fixtures).

### `max` / `min` / `max_at` / `min_at`

```text
M1 kind=measure type=max out=V(vout)
M2 kind=measure type=min out=V(vout) from=0.001 to=0.005
M3 kind=measure type=max_at out=V(vout)
```

The extreme value of `out` over the window (`max`/`min`), or the *time* it occurs at
(`max_at`/`min_at`). Fields: `out=`, and the optional window (`from=`/`to=`/`td=`).

**Errors**: `out=` missing or names an unknown column; the windowed series has fewer than two
samples.

**Worked example** (`tests/measure/fixtures/ramp.cir`, `RAMP(t)=2t` over `[0,10]`):
`type=max out=V(a)` → `MEAS_MAX = 19.999999999998906` (≈20, `RAMP(10)`).

### `pp`

```text
M1 kind=measure type=pp out=V(vout)
```

Peak-to-peak amplitude, `max - min`, over the window. Same fields/errors as `max`/`min`.

### `avg` / `rms` / `integ`

```text
M1 kind=measure type=avg out=V(vout)
M2 kind=measure type=rms out=V(vout)
M3 kind=measure type=integ out=V(vout)
```

The Δt-weighted (trapezoidal, exact-quadrature for `rms`) mean / root-mean-square / definite
integral of `out` over the window — never a plain `sum(values)/count`, which would silently
misweight a variable-timestep trace's own unequal step sizes. Same fields/errors as `max`.

**Worked example** (`ramp.cir`, `RAMP(t)=2t` over `[0,10]`, hand-derived): `avg=10`,
`rms=sqrt(400/3)≈11.547`, `integ=100` — the CLI returns `10.00999999999983`,
`11.55278321444644`, `99.99989999999661` respectively (the ~0.01 offset from the exact values is
the trace's own first-row-is-`t=dt`-not-`t=0` behavior, see [Reading the
output](reading-output.md)).

### `deriv`

```text
* AT form: the derivative of `out` at a fixed time.
M1 kind=measure type=deriv out=V(vout) at=0.002
* WHEN form: the derivative of `out` at the moment `when` crosses a threshold.
M2 kind=measure type=deriv out=V(vout) when=V(vctrl) when_val=2.5 rise=1
```

`at=` (a fixed time) and `when=`/`when_val=`/`when_ref=` (a crossing search, see above) are
mutually exclusive — give exactly one. `deriv` treats the trace as piecewise-linear, so the
result is the *exact* slope of the segment containing the resolved time (the average of the two
adjacent segments' slopes if the time lands exactly on a sample boundary), not a finite-difference
approximation. `when_val=<f64>` is a fixed threshold; `when_ref=<signal>` compares against
another signal's own value instead (the crossing is where their difference changes sign) —
mutually exclusive with `when_val=`.

**Worked example** (`ramp.cir`): `type=deriv out=V(a) at=5` → `2.000000000008882` (the ramp's own
constant slope, exactly).

### `find`

```text
* AT form: the value of `out` at a fixed time.
M1 kind=measure type=find out=V(vout) at=0.002
* WHEN form: the value of `out` at the moment `when` crosses a threshold.
M2 kind=measure type=find out=V(iload) when=V(vout) when_val=10 rise=1
```

Same `at=`/`when=` shape as `deriv`, but returns `out`'s own value (by linear interpolation) at
the resolved time instead of its slope — ngspice's `FIND <var> AT=`/`FIND <var> WHEN <var2>=...`.

**Worked example** (`ramp.cir`): `type=find out=V(b) when=V(a) when_val=10 rise=1` → `150.0...`
(`V(a)` crosses 10 at `t=5`; `V(b)=OTHER(5)=100+10*5=150`).

### `when`

```text
M1 kind=measure type=when out=V(vout) when_val=10 rise=1
M2 kind=measure type=when out=V(vout) when_ref=V(vref) cross=last
```

The *time* `out` crosses a threshold (a fixed `when_val=`, or another signal `when_ref=`),
selecting the requested edge/occurrence. ngspice's `WHEN <var>=<value>|<var2>`.

**Worked example** (`ramp.cir`): `type=when out=V(a) when_val=10 rise=1` → `5.000000000000022`.

### `trig_targ`

```text
M1 kind=measure type=trig_targ trig_at=0.0 targ_var=V(vout) targ_val=10 targ_rise=1
M2 kind=measure type=trig_targ \
     trig_var=V(vout) trig_frac_max=0.1 trig_rise=1 \
     targ_var=V(vout) targ_frac_max=0.9 targ_rise=1
```

The time difference `t_targ - t_trig` between two independently-specified events — ngspice/
Xyce's `TRIG ... TARG ...` (the classic rise-time/propagation-delay measurement). Each side
(`trig_`/`targ_` prefix) is exactly one of:

- `{prefix}at=<time>` — a fixed, already-known time (no search).
- `{prefix}var=<signal>` + (`{prefix}val=<f64>` or `{prefix}ref=<signal>`) +
  (`{prefix}rise=`/`{prefix}fall=`/`{prefix}cross=`) — an ordinary threshold crossing, plus its
  own optional window (`{prefix}from=`/`{prefix}to=`/`{prefix}td=`).
- `{prefix}var=<signal>` + `{prefix}frac_max=<f64>` (a fraction of that signal's own peak value
  over its window, computed first) + (`{prefix}rise=`/`{prefix}fall=`/`{prefix}cross=`) — a
  10%-90%-style rise-time crossing, useful when the signal's peak isn't known in advance.

**Worked example** (`ramp.cir`): `trig_at=1 targ_var=V(a) targ_val=15 targ_rise=1` → `6.5`
(`V(a)` crosses 15 at `t=7.5`; `7.5 - 1 = 6.5`).

### `freq` / `on_time` / `off_time`

```text
M1 kind=measure type=freq out=V(gate) on=2.5 off=2.5
M2 kind=measure type=on_time out=V(gate) on=2.5 off=2.5
M3 kind=measure type=off_time out=V(gate) on=2.5 off=2.5
```

A two-level (Schmitt-trigger) cycle detector: `out` enters the "on" state rising through `on`,
"off" falling through `off` (`on >= off`; equal is fine for a clean digital signal, no
hysteresis). `freq` is `(complete cycles)/(time spanned by those cycles)`; `on_time`/`off_time`
are the total time spent above `on`/below `off`, each normalized by the cycle count — Xyce's own
definitions.

**Worked example** (`tests/measure/fixtures/square.cir`, a 1 Hz 50%-duty square wave): `freq ≈
1.0`, `on_time ≈ off_time ≈ 0.5`.

### `four`

```text
M1 kind=measure type=four out=V(vout) fundamental=100000 harmonics=5
```

Bounded-harmonic Fourier analysis: `fundamental=` (Hz) and `harmonics=` (an integer ≥ 1, DC
included — `harmonics=5` yields orders 0..=4) — exact analytic integration of each linear
segment against the harmonic kernel, not a resampled FFT (see `general-simulator`'s own
`book/dev-guide/src/measurements-architecture.md` and `gs-waveform-measurements`'s own docs for
the technique and why it matters on a non-uniform trace). The window's own duration is
the integration period — choose `from=`/`to=`/`td=` to cover a meaningful number of fundamental
periods, exactly as in ngspice/Xyce.

Unlike every other type, `four` prints **several** result lines from one `kind=measure` line,
named from `NAME`:

- `NAME_dc` — the DC (average) term.
- `NAME_h<n>_mag` / `NAME_h<n>_phase_deg` for each harmonic `n = 1..harmonics-1` — peak amplitude
  and phase in degrees (`v(t) = dc + sum_n mag_n * cos(n*omega0*t + phase_n)`).
- `NAME_thd_percent` — total harmonic distortion as a percentage, only when `harmonics >= 3`
  (DC + fundamental + at least one overtone).

**Worked examples**:
- Pure sine (`tests/measure/fixtures/sine.cir`, amplitude 2, 5 Hz, fixed `--dt`): `h1_mag ≈ 2.0`,
  `h1_phase_deg ≈ -90` (`v(t)=2 sin(wt) = 2 cos(wt-90°)`), `thd_percent ≈ 0`.
- Two-tone signal (`tests/measure/fixtures/two_tone.cir`, `sin(wt) + 0.1*sin(2wt)`):
  `h1_mag ≈ 1.0`, `h2_mag ≈ 0.1`, `thd_percent ≈ 10.0` (hand: `100% * 0.1/1`).
- **Non-uniform-timestep trace** (`tests/measure/fixtures/sine_adaptive.cir`, same sine as
  above, run *without* `--dt` — adaptive stepping): the resolved trace's own step size ranges
  from `1e-6` s to `2e-4` s (a 200× spread — see `tests/measure/test_sine_adaptive.py`), and
  `four` still recovers `h1_mag ≈ 2.0`, `h1_phase_deg ≈ -90`, `thd_percent < 0.5%` — proof this
  is genuinely Δt-aware, not merely correct on a convenient fixed grid.

### `err1` / `err2`

```text
M1 kind=measure type=err1 out=V(vout) ref=V(vout_ref)
M2 kind=measure type=err2 out=V(vout) ref=V(vout_ref) minval=1e-9 ymin=0 ymax=1e6
```

Per-sample relative-difference statistics between `out` (measured) and `ref` (comparison,
interpolated onto `out`'s own sample times) — Xyce's `ERR1` (RMS of the per-sample relative
difference) and `ERR2` (its mean). `minval=` floors the denominator (`max(minval, |out_i|)`) to
avoid dividing by a near-zero sample; `ymin=`/`ymax=` restrict which samples (by `|out_i|`)
participate. Defaults: `minval=1e-12`, `ymin=0`, `ymax=` unbounded.

**Worked example** (`tests/measure/fixtures/err.cir`, `RAMP(t)=2t` always exactly double
`REF(t)=t`): `err1 = err2 = 0.5` exactly (every term is `(2t-t)/2t = 0.5`).

### `error`

```text
M1 kind=measure type=error out=V(vout) ref=V(vout_ref) norm=l2
```

The norm between `out` (measured) and `ref` (a reference waveform, interpolated onto `out`'s own
sample times restricted to the overlap of both domains) — Xyce's `ERROR` measure. `norm=` is one
of `l1` (mean absolute difference), `l2` (RMS difference, the default), or `infnorm` (maximum
absolute difference). Xyce's own `FILE=`/reference-from-a-file form isn't supported — `ref=` must
name a signal already in this run's own trace (a second circuit branch computing the reference,
or a second netlist run separately and its own waveform compared out-of-band, are both outside
this measurement's scope today).

**Worked example** (`err.cir`, a constant `+3` offset between `out` and `ref` at every instant):
`l1 = l2 = infnorm = 3` exactly (a constant-offset difference makes all three norms coincide).

## What's not supported

- `EQN` (an expression over *other measurements'* own results) and the `FILE=` half of `ERROR`
  (reading a reference waveform out of a file) are not implemented — `gs-waveform-measurements`
  itself deliberately doesn't implement either (see its own `src/lib.rs` doc comment); see the
  `general-simulator`'s own `book/dev-guide/src/measurements-architecture.md` for why.
- `--mode dc` has no meaningful trace for most measure types to run over (a single operating
  point has no window/crossing/Fourier structure) — `kind=measure` lines are meant for
  `--mode transient`.
