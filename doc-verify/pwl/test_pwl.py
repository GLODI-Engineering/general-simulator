"""Unit tests for the Pwl component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Pwl in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/pwl/test_pwl.py
Run via pytest:  python3 -m pytest doc-verify/pwl
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402

T_FINAL = 2e-3
DT = 2.5e-4


def _row_at(rows, t):
    return next(r for r in rows if abs(r["t"] - t) < 1e-12)


def test_example_ramp_and_repeating_triangle():
    """## Example: RAMP interpolates linearly between its two points and holds the last
    value past the end; TRI repeat=true wraps t into [0, 0.001) giving a periodic triangle."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=T_FINAL, dt=DT)
    rows = _lib.parse_csv(stdout)
    assert _row_at(rows, 2.5e-4)["RAMP"] == 0.25
    assert _row_at(rows, 5e-4)["RAMP"] == 0.5
    assert _row_at(rows, 7.5e-4)["RAMP"] == 0.75
    for t in (1e-3, 1.25e-3, 1.5e-3, 2e-3):
        assert _row_at(rows, t)["RAMP"] == 1.0, f"RAMP should hold 1 past its last point (at {t})"
    assert _row_at(rows, 2.5e-4)["TRI"] == 0.5
    assert _row_at(rows, 5e-4)["TRI"] == 1.0
    assert _row_at(rows, 7.5e-4)["TRI"] == 0.5
    assert _row_at(rows, 1e-3)["TRI"] == 0.0
    assert _row_at(rows, 1.25e-3)["TRI"] == 0.5  # second period
    assert _row_at(rows, 1.5e-3)["TRI"] == 1.0
    assert _row_at(rows, 2e-3)["TRI"] == 0.0


def test_non_pair_row_rejected():
    """## Errors: points=[[0,0],[1]] (a non-pair row) -> "each entry must be a 2-element
    '[x,y]' list (got '[1.0]')", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_row.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "each entry must be a 2-element '[x,y]' list (got '[1.0]')" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
