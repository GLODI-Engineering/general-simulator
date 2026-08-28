"""Unit test for the VCO component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/vco/test_vco.py
Run via pytest:  python3 -m pytest doc-verify/vco
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_ramps_by_dt_times_freq_each_step():
    """## Example: freq=10, dt=0.01 -> the [0,1) ramp advances by exactly 0.1 each step."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.2, dt=0.01)
    rows = _lib.parse_csv(stdout)
    assert len(rows) >= 2
    for a, b in zip(rows, rows[1:]):
        step = (b["VCO1"] - a["VCO1"]) % 1.0
        assert abs(step - 0.1) < 1e-9, f"got step {step}"


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
