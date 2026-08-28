"""Unit tests for the Time component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Time in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/time/test_time.py
Run via pytest:  python3 -m pytest doc-verify/time
"""
import math
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402

T_FINAL = 2e-3
DT = 2.5e-4
F = 1000.0


def _row_at(rows, t):
    return next(r for r in rows if abs(r["t"] - t) < 1e-12)


def test_example_sine_of_time():
    """## Example: time -> gain(k=2*pi*1000) -> sin produces S = sin(2*pi*1000*t), and the
    Time block's own output column equals the simulated t at every row."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=T_FINAL, dt=DT)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected at least one output row"
    for r in rows:
        assert r["T"] == r["t"], "Time block must output the step's own simulated time"
    assert _row_at(rows, 2.5e-4)["S"] == 1.0  # sin(pi/2)
    assert abs(_row_at(rows, 5e-4)["S"]) < 1e-12  # sin(pi)
    assert _row_at(rows, 7.5e-4)["S"] == -1.0  # sin(3*pi/2)
    # Spot-check one interior point against the closed form, not just the special angles:
    t = 1.25e-3
    assert abs(_row_at(rows, t)["S"] - math.sin(2 * math.pi * F * t)) < 1e-12


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
