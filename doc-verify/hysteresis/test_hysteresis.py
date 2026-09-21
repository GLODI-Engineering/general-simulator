"""Unit tests for the Hysteresis component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/hysteresis/test_hysteresis.py
Run via pytest:  python3 -m pytest doc-verify/hysteresis
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_switches_high_only_after_crossing_the_high_threshold():
    """## Example: LOW while RAMP < 1, HIGH once RAMP >= 1."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=2, dt=0.1)
    rows = _lib.parse_csv(stdout)
    for r in rows:
        expected = 1.0 if r["RAMP"] >= 1.0 else 0.0
        assert r["HYST1"] == expected, f"at t={r['t']}, RAMP={r['RAMP']}: got {r['HYST1']}"
    assert any(r["HYST1"] == 1.0 for r in rows), "expected the trace to actually go HIGH"
    assert any(r["HYST1"] == 0.0 for r in rows), "expected the trace to actually start LOW"


def test_low_exceeds_high_is_rejected():
    """## Errors: low > high is rejected at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_low_exceeds_high.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert "LowExceedsHigh" in stderr


def test_ic_is_the_initial_output():
    """## Parameters / Example: ic=1 holds 1 while nothing drives a change."""
    _, stdout, _ = _lib.run_transient(HERE / "ic_example.cir", tfinal=0.5, dt=0.1)
    rows = _lib.parse_csv(stdout)
    for row in rows:
        assert row["H1"] == 1.0, row


def test_ic_that_is_not_a_logic_level_is_rejected():
    """## Errors: ic= must be 0 or 1."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ic_not_a_logic_level.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert (
        "must be 0 or 1 (got '2')"
        in stderr
    ), stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
