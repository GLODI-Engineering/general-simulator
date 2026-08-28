"""Unit tests for the Counter component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/counter/test_counter.py
Run via pytest:  python3 -m pytest doc-verify/counter
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_increments_by_one_at_each_rising_edge():
    """## Example: CNT1 increments by exactly 1 at each detected CLK rising edge."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=6e-3, dt=0.5e-3)
    rows = _lib.parse_csv(stdout)
    assert rows[-1]["CNT1"] == 3.0, f"got {rows[-1]['CNT1']}"
    # Monotonically non-decreasing, and only ever changes by 0 or 1 between adjacent samples.
    for a, b in zip(rows, rows[1:]):
        assert b["CNT1"] - a["CNT1"] in (0.0, 1.0)


def test_bad_modulus_is_rejected():
    """## Errors: modulus= must be a non-negative integer."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_modulus.cir", tfinal=1e-3, dt=0.5e-3, expect_success=False
    )
    assert code != 0
    assert "not a non-negative integer" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
