"""Unit tests for the PyBlock component-reference entry -- see README.md for what each proves,
and for the one Errors claim (PythonSupportNotCompiledIn) intentionally not covered here.

Run standalone:  python3 doc-verify/pyblock/test_pyblock.py
Run via pytest:  python3 -m pytest doc-verify/pyblock

Requires a --features python build -- see README.md.
"""
import math
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_gain_block_doubles_its_input():
    """## Example: SRC=3 through a pyblock computing out = in * 2 gives G1 = 6."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-4, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows[0]["G1"] == 6.0


def test_path_through_a_space_containing_directory_requires_quoting():
    """## Parameters: path= must be "double-quoted" when the path contains whitespace."""
    _, stdout, _ = _lib.run_transient(HERE / "example_quoted_path.cir", tfinal=1e-4, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows[0]["G1"] == 6.0


def test_xc_count_continuous_state_block_integrates_correctly():
    """## Description: xc_count>0 uses derivative()/output_xc() instead of output(), and the
    resulting continuous state is genuinely solver-integrated (RK4), not hand-rolled by the
    block itself. xc_decay.py implements dxc/dt = 1 - xc, starting at rest (xc(0) = 0) -- the
    analytic solution is xc(t) = 1 - exp(-t); at t=0.01 that's ~0.0099501663."""
    _, stdout, _ = _lib.run_transient(HERE / "xc_example.cir", tfinal=0.01, dt=0.001)
    rows = _lib.parse_csv(stdout)
    expected = 1.0 - math.exp(-0.01)
    assert abs(rows[-1]["G1"] - expected) < 1e-6, f"got {rows[-1]['G1']}, expected ~{expected}"


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
