"""Unit tests for the Discrete PID Controller component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/discretepid/test_discretepid.py
Run via pytest:  python3 -m pytest doc-verify/discretepid
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_steps_by_a_fixed_increment_at_each_sample_hit():
    """## Example: ERR=0.5, ki=0.5, ts=0.1 -> DPID1 steps 0.5, 0.525, 0.55, 0.575, 0.6, held
    constant between sample hits (the CSV grid at dt=0.05 samples each hit's value twice)."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.5, dt=0.05)
    rows = _lib.parse_csv(stdout)
    expected = [0.5, 0.5, 0.525, 0.525, 0.55, 0.55, 0.575, 0.575, 0.6, 0.6]
    actual = [r["DPID1"] for r in rows]
    for a, e in zip(actual, expected):
        assert abs(a - e) < 1e-9, f"sequence mismatch: got {actual}, expected {expected}"


def test_unknown_integration_method_is_rejected():
    """## Errors: integration_method must be forward/backward/trapezoidal."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_unknown_method.cir", tfinal=0.1, dt=0.05, expect_success=False
    )
    assert code != 0
    assert "unknown integration_method" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
