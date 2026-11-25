# Runs too large to open: `--format samples`

A power-electronics run gets large quickly. A 5 ns step over one 20 ms grid
period is four million rows; seventy signals at that length is about a
gigabyte of samples, and ten grid periods is ten. The question is not whether
the solver can produce that — it can, and streams it — but whether anything
can then look at it.

CSV and the SPICE rawfile cannot. Both are **row-major**: every signal at one
time step sits together, so the samples of any one signal are scattered the
whole length of the file. CSV is worse again, because its fields are
variable-width text, so the byte offset of a given signal at a given step is
not computable from anything. Finding it means scanning from the start,
counting separators.

Two consequences, and both are the reason this page exists:

- **Plotting five of seventy signals costs what plotting all seventy costs.**
  There is no way to skip the sixty-five you did not ask for.
- **Showing one millisecond out of two hundred costs a full scan** too.

Measured, on a real 121 MB result with 69 signals: pulling five columns out
with `awk` takes 2.5 s and touches all 121 MB. That rate puts a 1 GB run at
about 21 s and a 10 GB run at three and a half minutes — per redraw, every
time you pan or zoom, unless whatever is plotting holds the parsed samples in
memory, which at these sizes is the problem you were trying to avoid.

## What `--format samples` does instead

```bash
general-simulator buck.cir --mode transient --tfinal 20e-3 --dt 5e-9 \
    --format samples --out buck.gsim_samples
```

The container it writes is described in full by the `gsim-samples` crate's
own documentation. Three properties matter here.

**Each signal is stored separately.** Reading five of seventy touches five
signals' bytes.

**Offsets are arithmetic.** Rows are written in fixed-size stripes of
identical geometry, and a directory of one entry per stripe maps a time
window to stripes without reading a sample. Finding a window is a
calculation, not a search.

**Every stripe carries a min/max pyramid over its own samples.** Each level
folds 64 samples of the one below into their minimum and maximum, so a
reader can take a level whose density matches the screen instead of the
run. The pyramid costs about 3% of the file.

Those extremes are the point, not an implementation detail. A plot 1500
pixels wide can distinguish about 3000 points; everything beyond that is
read, paid for and not seen. But *which* 3000 matters enormously: a
switching overshoot lasting one time step has to survive the reduction, and
it only does if each interval contributes its extremes rather than its mean
or whichever sample a stride happened to land on.

## Why not `--out-every`

`--out-every N` keeps every Nth resolved point and drops the rest. It makes
the file smaller and it is the wrong tool for a waveform you intend to look
at: an overshoot between two kept points is simply not in the file, and
nothing downstream can tell that it ever happened. The run saw it, the file
does not.

`--format samples` keeps every sample at level 0 *and* the extremes of every
interval above it. Zoom in and the real samples are there; zoom out and the
peaks are still there.

The two flags compose, and if you reach for both, consider what you want:
`--out-every` to genuinely discard resolution you will never need, `--format
samples` to keep it and still draw quickly.

## What it costs to write

Nothing, roughly, and less than CSV. On a four-million-row run of a
PWM-driven deck:

| | writes in | peak memory | file | one full-extent draw of one signal |
|---|---|---|---|---|
| `--format csv` | 8.99 s | 4.5 MB | 116 MB | 2.16 s, touching all 116 MB |
| `--format samples` | 7.22 s | 5.5 MB | 82.5 MB | under 10 ms, reading 1.5 MB |

It is smaller than CSV and faster to write, because formatting four million
rows of floating point as decimal text costs more than building a pyramid.
Peak memory was 5.5 MB at four million rows and 5.6 MB at four hundred
thousand: the writer holds one stripe, never the run, so memory does not
track how long the simulation was.

## What this format is not for

Handing to someone, diffing, committing, or loading with `pandas`. Use CSV,
which is good at all of those and unchanged. Use the rawfile where a
SPICE-aware tool is going to read it. `--format samples` is for a run you
intend to plot, and particularly one you cannot open any other way.

It is also not HDF5, which gets the three properties above right and was the
obvious candidate. Every HDF5 binding links the libhdf5 C library, which
rules it out in the two places that matter most: it cannot be compiled to
WebAssembly, so a viewer running in a browser or an editor's webview cannot
read one at all, and on Windows it turns `pip install` into a hunt for
`hdf5.dll`. A format whose reader needs no C library has neither problem.
