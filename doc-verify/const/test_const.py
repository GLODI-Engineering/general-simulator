"""Unit tests for the Const component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Const in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/const/test_const.py
Run via pytest:  python3 -m pytest doc-verify/const
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_scalar_and_vector_constants():
    """## Example: value=5 gives a constant scalar 5; value=[1,2,3] gives a constant vector
    [1, 2, 3], unchanged at every step."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected at least one output row"
    for r in rows:
        assert r["SETPOINT"] == 5.0
        assert (r["OFFSETS[0]"], r["OFFSETS[1]"], r["OFFSETS[2]"]) == (1.0, 2.0, 3.0)


def test_scalar_value_not_a_number():
    """## Errors: value=abc (a scalar that is not a number) -> "field 'value' is not a number",
    at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_not_number.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "field 'value' is not a number" in stderr


def test_vector_entry_not_a_number():
    """## Errors: value=[1,x,3] (a vector entry that is not a number) ->
    "field 'value' entry 'x' is not a number", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_entry.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "field 'value' entry 'x' is not a number" in stderr


def test_nonfinite_vector_entry_rejected():
    """## Errors: value=[1,nan,3] (a non-finite vector entry) -> "field 'value' entry 'nan'
    must be a finite number (got NaN)", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_nan.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "must be a finite number (got NaN)" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
