"""Unit tests for the Table component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Table in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/table/test_table.py
Run via pytest:  python3 -m pytest doc-verify/table
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def _row_at(rows, t):
    return next(r for r in rows if abs(r["t"] - t) < 1e-12)


def test_example_interpolates_and_holds_ends():
    """## Example: linear interpolation through [[0,0],[1,10],[2,10],[3,0]] -- 5 at x=0.5,
    10 on the flat [1,2] segment, 5 at x=2.5, held at the last point's 0 past x=3."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=4, dt=0.5)
    rows = _lib.parse_csv(stdout)
    assert _row_at(rows, 0.5)["T"] == 5.0
    for t in (1.0, 1.5, 2.0):
        assert _row_at(rows, t)["T"] == 10.0, f"flat segment: T should be 10 at x={t}"
    assert _row_at(rows, 2.5)["T"] == 5.0
    for t in (3.0, 3.5, 4.0):
        assert _row_at(rows, t)["T"] == 0.0, f"held past last point: T should be 0 at x={t}"


def test_non_pair_row_rejected():
    """## Errors: points=[[0,0],[1]] (a non-pair row) -> "each entry must be a 2-element
    '[x,y]' list (got '[1.0]')", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_row.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "each entry must be a 2-element '[x,y]' list (got '[1.0]')" in stderr


def test_empty_table_panics():
    """## Errors: points=[] is not a clean error -- the evaluation code's own assert fires and
    the process panics with "table needs at least one point"."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_empty.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "table needs at least one point" in stderr


def test_points_sorted_ascending_at_parse_time():
    """## Parameters: points= is sorted ascending by x at parse time, so a newest-point-first
    list interpolates identically to the in-order one."""
    _, stdout, _ = _lib.run_transient(HERE / "out_of_order.cir", tfinal=4, dt=0.5)
    rows = _lib.parse_csv(stdout)
    assert _row_at(rows, 0.5)["A"] == 5.0
    assert _row_at(rows, 1.5)["A"] == 10.0
    assert _row_at(rows, 2.5)["A"] == 5.0
    assert _row_at(rows, 3.5)["A"] == 0.0


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
