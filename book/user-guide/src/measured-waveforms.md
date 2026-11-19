# Measured waveforms: driving a simulation from a capture, and grading it against one

A simulation agrees with the circuit you built, or it does not, and the only way to find out is
to measure the circuit and compare. This chapter is about doing that with numbers rather than by
eye.

Two things are needed, and the second is the one people do not expect to be hard:

- a source that reads a **capture** — a CSV off an oscilloscope, or a table printed over a
  serial link by the microcontroller driving the board;
- a way to say *how close* the run came, so the answer is a figure you can watch improve rather
  than two traces that look about right.

```text
SCOPE kind=filewave path=capture.csv outputs=VIN_M,VOUT_M after=error
DRIVE kind=sig2phys domain=voltage in=SCOPE
V1 in 0 DRIVE
R1 in out 1000
C1 out 0 1u
M_fit kind=measure type=error out=V(out) ref=VOUT_M norm=l2
```

That is the whole shape. The first line reads the file, the second and third make the measured
input an actual voltage source, and the last line is the comparison.

## `kind=filewave`: the file becomes breakpoints

A capture is a table of times and values. Read once, at load time, it is a breakpoint list —
and between breakpoints it is exactly [`kind=pwl`](sources-and-math.md) (interpolated) or
`kind=pwc` (held). There is no second interpolator: a capture and a hand-written `points=` list
of the same numbers are the same waveform, asserted by a test that compares them row by row.

| field | means | default |
|---|---|---|
| `path` | the file. A relative path resolves against the **process's working directory**, the same rule `kind=cscript`'s `lib=` follows | required |
| `outputs` | one name per value column, in order | the file's own header names, cleaned |
| `time` | which column holds time — a 1-based position, or a header name | the first |
| `cols` | which value columns to read | all but the time column |
| `interp` | `linear` interpolates between samples, `hold` keeps the last one | `linear` |
| `after` | past the last sample: `hold`, `repeat`, `zero`, `error` | `hold` |
| `tshift` | `auto` moves the capture to start at $t = 0$; `none` keeps the file's times; a number shifts by that many seconds | `auto` |
| `scale`, `offset` | $y \mapsto \text{scale} \cdot y + \text{offset}$ — a ×10 probe, or a removed DC offset | `1`, `0` |
| `skip`, `delim` | override the header detection and the separator detection | detected |

**The first channel is the block's own name; the others are names of their own.** In the deck
above, `SCOPE` is the first column and `VOUT_M` the second — not `SCOPE.VOUT_M`. That means a
channel name is a *global* signal name, so `outputs=` is how you keep a capture's `OUT` from
colliding with something else called `OUT`. A schematic drawn in the editor gets this for free:
it emits `outputs=SCOPE,SCOPE_VOUT`, prefixed with the block.

### Three things about real files

A clean two-column CSV is the easy case and not the one you will have.

**The preamble is skipped by shape, not by vendor.** Every leading line whose first field is not
a number is a header. That covers the metadata block every instrument writes, without a sniffer
that has to recognise each of them:

```text
Record Length,2000,,
Sample Interval,1.0E-06,,
Vertical Units,V,V,
TIME,VIN,VOUT
-1.9900000e-04,4.999,0.0066
```

The last non-numeric line with the data's own field count is taken as the column names, so
`TIME,VIN,VOUT` names the outputs and the three lines above it are dropped.

**The time base is moved to zero.** Look at that fifth line: the capture starts at
$-199\,\mu\mathrm{s}$, because a scope window is centred on its trigger. Left alone, a
comparison would be out by the pre-trigger length — the simulation would look wrong for a reason
nowhere on screen. `tshift=auto` subtracts the first sample's time, and `tshift=none` is there
for when the file's own time base is the one you mean.

**A decimal comma is refused, by name.** `0,001;1,23` is four fields if the comma separates them
and two numbers if it does not, and no single reading is right for both files:

```text
'capture.csv' line 2: '0,000' looks like a number written with a decimal comma. This file cannot
be read either way -- with a comma separator those are two fields, without one they are a single
number -- so re-export it with a decimal point.
```

Everything else that can be wrong with a file names the file, the line and the problem: a ragged
row, a non-numeric field, a time column that goes backwards, a one-column file. A column that is
not there lists the ones that are:

```text
error: parsing device/block declarations: line 2: device 'SCOPE': 'capture.csv' has no column
'CH3'; it has 3, named TIME, VIN, VOUT
```

### `after=error`, and why it is worth setting

Past its last sample a capture holds its final value. So a run that outlasts its data is driven
by a **flat line** for the remainder, and the waveform that comes out looks entirely plausible —
the exact failure mode [rules](rules-and-ratings.md) exist to catch. `after=error` refuses that
run, before the first step rather than after you have waited for it:

```text
error: WaveCaptureTooShort { block: "SCOPE", source: "capture.csv", covered: 0.001999,
t_final: 0.003 }: 'SCOPE' reads 'capture.csv', which ends at 1.999e-3 s, but the run goes to
3e-3 s. Past its last sample a capture holds its final value, so the rest of the run would be
driven by a flat line -- which is what `after=error` exists to refuse. Shorten the run to
1.999e-3 s, capture more data, or choose `after=hold`/`repeat`/`zero` if a held, repeated or
zero tail is what you meant.
```

## Grading the run: `type=error`

[`kind=measure`](measurements.md) has had `err1`, `err2` and `error` all along, and until a
source could read a file they were nearly useless: `ref=` could only name a waveform the
simulation itself had computed, so the comparison was a run against itself. Pointed at a
capture, it is the number this whole chapter is for.

`type=error` is the normalised difference between two waveforms over the run,

$$
E_{L_2} = \sqrt{\frac{1}{N}\sum_{i=1}^{N} \left(\frac{y_i - r_i}{\max(|r_i|,\,\text{minval})}\right)^2}
$$

with `norm=l1`, `norm=l2` (the default) or `norm=infnorm` choosing the sum, the root-mean-square
or the worst single point. `minval` (default $10^{-12}$) keeps a reference that passes through
zero from dividing the whole figure by nothing, and `ymin`/`ymax` restrict the comparison to a
band of the reference's own range.

## A worked comparison

An RC low-pass, $R = 1\,\mathrm{k}\Omega$, nominally $C = 1\,\mu\mathrm{F}$, driven with a
$0\text{–}5\,\mathrm{V}$ step. The board was measured: the capture is 2000 samples at
$1\,\mu\mathrm{s}$, with the trigger $200\,\mu\mathrm{s}$ in, both the input and the output
recorded, and the quantisation and probe noise a real measurement comes with.

The deck at the top of this chapter is the model: it takes **the measured input** as its source,
so the comparison is not also a comparison of two different stimuli, and grades its own output
against the measured output. Running it:

```text
M_fit = 0.11038089970026241
```

Eleven percent. The traces look similar on screen; the number says they are not the same. Sweep
the model's capacitor and the number says where the circuit actually is:

| `C1` in the model | `M_fit` |
|---|---|
| $1.00\,\mu\mathrm{F}$ | 0.1104 |
| $1.05\,\mu\mathrm{F}$ | 0.0528 |
| $1.10\,\mu\mathrm{F}$ | **0.0047** |
| $1.15\,\mu\mathrm{F}$ | 0.0538 |

The fitted capacitor is $1.1\,\mu\mathrm{F}$ — ten percent above its nominal value, which is an
ordinary tolerance for a part nobody measured. A factor of **24** between the nominal model and
the fitted one, from a difference that eyeballing two curves would have called agreement.

**The error does not reach zero, and it should not.** The 0.0047 that remains at the right value
is the measurement's own noise: the capture carries quantisation and probe noise, and no model
of the circuit can agree with those. That floor is worth knowing, because it is the resolution
of the comparison — a change that moves `M_fit` from 0.0050 to 0.0048 has told you nothing.

### What this does not do

It does not fit anything for you. The sweep above is four runs and a table; the engine computes
the number and you decide what to change. It also compares what you asked it to compare: a model
and a capture agreeing to 0.5% over a step says nothing about how they agree during a switching
transient that the capture's sample rate cannot resolve.

And it is worth being explicit about the trap in using a capture as a source: **the solver does
not see more than the file holds**. A capture at $1\,\mu\mathrm{s}$ driving a run stepping at
$10\,\mathrm{ns}$ is interpolated — smooth, and smoother than the real signal was. The capture's
sample interval is the real resolution of anything it drives, whatever `--dt` says.

## If you drew the circuit

A file source is in the palette as **File Source**, and the editor fills in the same fields. One
thing differs, and it is in your favour: the document carries the column names, so the symbol
draws its pins whether the data file is next to it or not — a sheet is reviewable without its
capture, and a file swapped for one with fewer columns is a diagnostic instead of a silently
redrawn symbol. The reasoning is in
[`gsim-core`'s decision 0005](https://github.com/GLODI-Engineering/gsim-core/blob/master/docs/decisions/0005-a-document-that-names-a-file.md).

Because a relative path resolves against the working directory, a netlist compiled from a
document is meant to be run **from that document's own directory**.
