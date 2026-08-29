"""FOUR on a genuinely non-uniform-timestep trace -- the entire reason gs-waveform-measurements
exists (Delta-t-weighted/exact-segment techniques, not naive uniform-grid assumptions). This test
does two things a fixed-dt run cannot: (1) proves the resolved trace is actually non-uniformly
spaced (not just "ran without --dt and hoped"), and (2) proves FOUR still recovers the correct
analytic answer on it. See fixtures/sine_adaptive.cir's own header comment for the full
rationale (the RC branch exists purely to give the adaptive stepper real dynamics to react to;
MEAS_FOUR reads the sine source block's own output directly, so the expected answer is the same
closed form as sine.cir: amplitude 2, 5 Hz => h1_mag=2.0, h1_phase=-90deg, ~0 THD)."""
import csv
import io
import sys
import pathlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from _lib import run_measure, parse_measurements, assert_close  # noqa: E402


def test_trace_is_actually_non_uniform():
    _, stdout, _ = run_measure("sine_adaptive.cir", tfinal=0.2)  # no dt= -> adaptive stepping
    reader = csv.reader(io.StringIO(stdout))
    rows = list(reader)
    ts = [float(row[0]) for row in rows[1:]]
    assert len(ts) > 20, f"expected a meaningfully long adaptive trace, got {len(ts)} points"
    dts = [b - a for a, b in zip(ts, ts[1:])]
    ratio = max(dts) / min(dts)
    assert ratio > 5.0, (
        f"expected a genuinely non-uniform step size (max/min dt ratio > 5), got ratio "
        f"{ratio} (min={min(dts)}, max={max(dts)}) -- if this fails, the RC branch in "
        f"sine_adaptive.cir no longer forces the adaptive stepper to vary its step size, and "
        f"this test no longer proves what it claims to"
    )


def test_four_on_non_uniform_trace_matches_analytic_sine():
    _, _, stderr = run_measure("sine_adaptive.cir", tfinal=0.2)
    m = parse_measurements(stderr)
    assert_close(m["MEAS_FOUR_h1_mag"], 2.0, 0.02, "h1 magnitude (non-uniform trace)")
    assert_close(m["MEAS_FOUR_h1_phase_deg"], -90.0, 0.5, "h1 phase (non-uniform trace)")
    assert m["MEAS_FOUR_thd_percent"] < 0.5, f"thd got {m['MEAS_FOUR_thd_percent']}"


if __name__ == "__main__":
    test_trace_is_actually_non_uniform()
    test_four_on_non_uniform_trace_matches_analytic_sine()
    print("ok")
