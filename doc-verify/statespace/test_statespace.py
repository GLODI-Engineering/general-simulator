"""Unit tests for the StateSpace component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/statespace/test_statespace.py
Run via pytest:  python3 -m pytest doc-verify/statespace
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_settles_toward_the_analytic_steady_state():
    """## Example: dx/dt = -x + u, u=1 -> x settles toward 1."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=10, dt=0.1)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["SS1"] - 1.0) < 1e-3, f"got {rows[-1]['SS1']}"


def test_scalar_d_on_a_mimo_system_is_rejected():
    """## Errors: a bare scalar d= on a system with more than one input/output is rejected."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_scalar_d_mimo.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert "bare scalar" in stderr and "MIMO" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
