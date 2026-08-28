"""Unit tests for the Pwc component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Pwc in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/pwc/test_pwc.py
Run via pytest:  python3 -m pytest doc-verify/pwc
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


def test_example_step_and_repeating_square():
    """## Example: STEP holds the last point's value at or before t (0 until 0.001, then 1);
    SQUARE with repeat=true wraps t into [0, 0.001) -- 1 on [0.0005, 0.001), 0 on
    [0.001, 0.0015), 1 again on [0.0015, 0.002)."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=T_FINAL, dt=DT)
    rows = _lib.parse_csv(stdout)
    for t in (2.5e-4, 5e-4, 7.5e-4):
        assert _row_at(rows, t)["STEP"] == 0.0, f"STEP should hold 0 before t=0.001 (at {t})"
    for t in (1e-3, 1.25e-3, 1.5e-3, 1.75e-3, 2e-3):
        assert _row_at(rows, t)["STEP"] == 1.0, f"STEP should hold 1 from t=0.001 on (at {t})"
    assert _row_at(rows, 5e-4)["SQUARE"] == 1.0
    assert _row_at(rows, 7.5e-4)["SQUARE"] == 1.0
    for t in (1e-3, 1.25e-3):
        assert _row_at(rows, t)["SQUARE"] == 0.0, f"SQUARE should be low on [0.001,0.0015) (at {t})"
    for t in (1.5e-3, 1.75e-3):
        assert _row_at(rows, t)["SQUARE"] == 1.0, f"SQUARE should be high on [0.0015,0.002) (at {t})"
    assert _row_at(rows, 2e-3)["SQUARE"] == 0.0  # wrapped back to t=0


def test_non_pair_row_rejected():
    """## Errors: points=[[0,0],[1]] (a non-pair row) -> "each entry must be a 2-element
    '[x,y]' list (got '[1.0]')", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_row.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "each entry must be a 2-element '[x,y]' list (got '[1.0]')" in stderr


def test_non_list_points_rejected():
    """## Errors: points=abc (not a Python list) -> "field 'points' must be a Python-style
    list, e.g. '[1,2,3]' or '[[1,2],[3,4]]' (got 'abc')", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_not_a_list.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "field 'points' must be a Python-style list" in stderr


def test_points_sorted_at_parse_and_repeat_is_exact_true():
    """## Parameters: points= is sorted ascending by x at parse time (newest-first list behaves
    identically to the in-order one); repeat= only matches the exact string "true" ("True"
    does not wrap)."""
    _, stdout, _ = _lib.run_transient(HERE / "out_of_order_repeat_case.cir", tfinal=T_FINAL, dt=DT)
    rows = _lib.parse_csv(stdout)
    assert _row_at(rows, 5e-4)["A"] == 0.0
    assert _row_at(rows, 1e-3)["A"] == 1.0
    # repeat=True did not wrap: the value holds flat past the last point instead of restarting
    assert _row_at(rows, 1.5e-3)["A"] == 1.0
    assert _row_at(rows, 2e-3)["A"] == 1.0


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
