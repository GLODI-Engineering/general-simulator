"""Unit test for the Gate Binding component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/gatebinding/test_gatebinding.py
Run via pytest:  python3 -m pytest doc-verify/gatebinding
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_mosfet_held_on_matches_the_hand_derived_divider():
    """## Example: a permanently-on 0.1 ohm MOSFET in series with a 1k load -> V(out) ==
    5 * 1000 / 1000.1."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    expected = 5.0 * 1000.0 / 1000.1
    assert abs(rows[-1]["V(out)"] - expected) < 1e-6, f"got {rows[-1]['V(out)']}"


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
