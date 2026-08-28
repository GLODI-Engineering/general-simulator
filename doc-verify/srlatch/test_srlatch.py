"""Unit tests for the SR Latch component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/srlatch/test_srlatch.py
Run via pytest:  python3 -m pytest doc-verify/srlatch
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_stays_latched_after_the_fault_clears():
    """## Example: FAULT pulses to 1 then drops to 0 at t=0.1; LATCH1 stays 1 afterward."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.3, dt=0.05)
    rows = _lib.parse_csv(stdout)
    after_fault_clears = [r for r in rows if r["t"] > 0.1]
    assert after_fault_clears, "expected rows after the fault clears"
    assert all(r["FAULT"] == 0.0 for r in after_fault_clears), "fault should have cleared"
    assert all(r["LATCH1"] == 1.0 for r in after_fault_clears), "latch should stay set"


def test_bad_priority_value_is_rejected():
    """## Errors: priority= must be 'set' or 'reset'."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_priority.cir", tfinal=0.1, dt=0.05, expect_success=False
    )
    assert code != 0
    assert "must be 'set' or 'reset'" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
