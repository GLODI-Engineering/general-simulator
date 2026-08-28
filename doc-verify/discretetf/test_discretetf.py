"""Unit test for the Discrete Transfer Function component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/discretetf/test_discretetf.py
Run via pytest:  python3 -m pytest doc-verify/discretetf
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_matches_the_equivalent_discretestatespace_realization():
    """## Example: 0.1/(z-1), matching discretestatespace's own equivalent example (0.5 after 5 hits)."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.5, dt=0.05)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["DTF1"] - 0.5) < 1e-9, f"got {rows[-1]['DTF1']}"


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
