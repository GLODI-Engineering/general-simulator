"""FOUR (bounded-harmonic Fourier) coverage, fixed-dt/uniform-sampling baseline: a pure sine's
fundamental magnitude/phase against the known analytic answer. See fixtures/sine.cir's own
header comment for the hand-derivation (amplitude 2, 5 Hz => h1_mag=2.0, h1_phase=-90deg, ~0
THD)."""
import sys
import pathlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from _lib import run_measure, parse_measurements, assert_close  # noqa: E402


def test_sine_fundamental_and_thd():
    _, _, stderr = run_measure("sine.cir", tfinal=0.2, dt=0.0001)
    m = parse_measurements(stderr)
    assert_close(m["MEAS_FOUR_dc"], 0.0, 1e-3, "dc")
    assert_close(m["MEAS_FOUR_h1_mag"], 2.0, 5e-3, "h1 magnitude")
    assert_close(m["MEAS_FOUR_h1_phase_deg"], -90.0, 0.5, "h1 phase")
    for h in (2, 3, 4):
        assert_close(m[f"MEAS_FOUR_h{h}_mag"], 0.0, 1e-3, f"h{h} magnitude")
    assert m["MEAS_FOUR_thd_percent"] < 0.1, f"thd got {m['MEAS_FOUR_thd_percent']}"


if __name__ == "__main__":
    test_sine_fundamental_and_thd()
    print("ok")
