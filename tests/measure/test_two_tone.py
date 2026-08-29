"""FOUR/THD coverage on a signal with known, deliberately-added harmonic content: a two-tone
signal's THD against a hand computation. See fixtures/two_tone.cir's own header comment for the
derivation (fundamental amplitude 1, second harmonic amplitude 0.1 => THD = 10% exactly)."""
import sys
import pathlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from _lib import run_measure, parse_measurements, assert_close  # noqa: E402


def test_two_tone_thd():
    _, _, stderr = run_measure("two_tone.cir", tfinal=0.1, dt=0.000025)
    m = parse_measurements(stderr)
    assert_close(m["MEAS_FOUR_h1_mag"], 1.0, 0.01, "h1 (fundamental) magnitude")
    assert_close(m["MEAS_FOUR_h2_mag"], 0.1, 0.005, "h2 (added harmonic) magnitude")
    assert_close(m["MEAS_FOUR_thd_percent"], 10.0, 0.2, "thd_percent")


if __name__ == "__main__":
    test_two_tone_thd()
    print("ok")
