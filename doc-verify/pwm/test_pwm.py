"""Unit tests for the PWM Modulator 1 component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/pwm/test_pwm.py
Run via pytest:  python3 -m pytest doc-verify/pwm
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_main_and_complement_always_sum_to_one():
    """## Example: 10kHz, 30% duty, no dead time -- PWM1 + PWM1_comp == 1 at every step."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-4, dt=1e-6)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected output rows"
    for r in rows:
        assert r["PWM1"] + r["PWM1_comp"] == 1.0, f"mismatch at t={r['t']}: {r}"
    # Also confirm the duty ratio is actually applied: high for ~30% of samples, not always on/off.
    high_fraction = sum(1 for r in rows if r["PWM1"] == 1.0) / len(rows)
    assert abs(high_fraction - 0.3) < 0.02, f"got high_fraction={high_fraction}"


def test_bad_outputs_count_is_rejected():
    """## Errors: outputs= must have exactly 2 entries."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_outputs_count.cir", tfinal=1e-4, dt=1e-6, expect_success=False
    )
    assert code != 0
    assert "needs exactly 2 entries" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
