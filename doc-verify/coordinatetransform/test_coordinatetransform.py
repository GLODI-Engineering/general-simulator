"""Unit test for the Coordinate Transform component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/coordinatetransform/test_coordinatetransform.py
Run via pytest:  python3 -m pytest doc-verify/coordinatetransform
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_clarke_transform_of_a_phase_a_peak():
    """## Example: a=1,b=-0.5,c=-0.5 -> alpha=1, beta=0 exactly."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.1, dt=0.05)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["CT1"] - 1.0) < 1e-9, f"got alpha={rows[-1]['CT1']}"
    assert abs(rows[-1]["CT1_beta"] - 0.0) < 1e-9, f"got beta={rows[-1]['CT1_beta']}"


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
