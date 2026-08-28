"""Unit tests for the PID component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Pid in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/pid/test_pid.py
Run via pytest:  python3 -m pytest doc-verify/pid
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_fixed_clamp_example_output_stays_zero_for_zero_error():
    """## Example: a zero error signal into kp=1 ki=0 kd=0 must give a zero output every step."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected at least one output row"
    assert all(r["PID1"] == 0.0 for r in rows), "PID1 should stay exactly 0 for a zero error"


def test_proportional_gain_is_applied_correctly():
    """kp is genuinely multiplied into the output, not silently ignored: kp=0.3, ERR=0.5 -> 0.15,
    with the clamp bounds wide enough that clamping never masks the result."""
    _, stdout, _ = _lib.run_transient(HERE / "nonzero_error.cir", tfinal=1e-4, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows[0]["PID1"] == 0.15


def test_nonpositive_n_is_rejected():
    """## Errors: n <= 0 -> invalid PID (NonPositiveFilterCoefficient), at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_nonpositive_n.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "NonPositiveFilterCoefficient" in stderr


def test_unpaired_dynamic_clamp_fields_are_rejected():
    """## Errors: clamp_lo_in without clamp_hi_in (or vice versa) is rejected at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_unpaired_dynamic_clamp.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "clamp_lo_in" in stderr and "clamp_hi_in" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
