"""Unit test for the Sig2Current component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/sig2current/test_sig2current.py
Run via pytest:  python3 -m pytest doc-verify/sig2current
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_drives_the_current_source_magnitude_correctly():
    """## Example: I1's magnitude tracks CMD=0.1 through the converter -- Ohm's law over R1=1k
    gives |V(a)| == 100."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert abs(abs(rows[-1]["V(a)"]) - 100.0) < 1e-6, f"got {rows[-1]['V(a)']}"


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
