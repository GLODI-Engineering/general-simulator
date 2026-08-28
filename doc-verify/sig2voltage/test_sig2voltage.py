"""Unit test for the Sig2Voltage component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/sig2voltage/test_sig2voltage.py
Run via pytest:  python3 -m pytest doc-verify/sig2voltage
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_drives_the_voltage_source_exactly():
    """## Example: V1's magnitude tracks CMD=5 through the converter -> V(a) == 5."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["V(a)"] - 5.0) < 1e-9, f"got {rows[-1]['V(a)']}"


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
