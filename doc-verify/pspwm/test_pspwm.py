"""Unit tests for the PWM Modulator 2 (Phase-Shift PWM) component-reference entry -- see
README.md.

Run standalone:  python3 doc-verify/pspwm/test_pspwm.py
Run via pytest:  python3 -m pytest doc-verify/pspwm
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_main_and_complement_always_sum_to_one():
    """## Example: 10kHz, zero phase shift, 40% duty -- PSPWM1 + PSPWM1_comp == 1 every step."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-4, dt=1e-6)
    rows = _lib.parse_csv(stdout)
    assert rows
    for r in rows:
        assert r["PSPWM1"] + r["PSPWM1_comp"] == 1.0, f"mismatch at t={r['t']}: {r}"


def test_bad_inputs_count_is_rejected():
    """## Errors: inputs= must have exactly 3 entries (freq,phase,duty)."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_inputs_count.cir", tfinal=1e-4, dt=1e-6, expect_success=False
    )
    assert code != 0
    assert "needs 3 inputs" in stderr


def test_fmin_exceeds_fmax_is_rejected():
    """## Errors: f_min > f_max is rejected at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_fmin_exceeds_fmax.cir", tfinal=1e-4, dt=1e-6, expect_success=False
    )
    assert code != 0
    assert "FMinExceedsFMax" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
