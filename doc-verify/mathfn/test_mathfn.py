"""Unit tests for the MathFn1/MathFn2/MathFn3 component-reference entries -- see README.md for
what each proves. One function per documented behavior (an Example, or an Errors claim),
matching the doc comments on BlockKind::MathFn1/MathFn2/MathFn3 in
general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/mathfn/test_mathfn.py
Run via pytest:  python3 -m pytest doc-verify/mathfn
"""
import math
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def _row_at(rows, t):
    return next(r for r in rows if abs(r["t"] - t) < 1e-12)


def test_example_fn1_scalar_functions_and_elementwise_vector():
    """## Example (MathFn1): S = sin(2*pi*1000*t) (1 at 0.25ms, -1 at 0.75ms), C = cos of the
    same (0 at 0.25ms), E = exp(t); a Vector input maps elementwise: sqrt([1,4,9]) = [1,2,3]."""
    _, stdout, _ = _lib.run_transient(HERE / "example_fn1.cir", tfinal=2e-3, dt=2.5e-4)
    rows = _lib.parse_csv(stdout)
    assert _row_at(rows, 2.5e-4)["S"] == 1.0
    assert _row_at(rows, 7.5e-4)["S"] == -1.0
    assert abs(_row_at(rows, 2.5e-4)["C"]) < 1e-12  # cos(pi/2)
    assert abs(_row_at(rows, 2.5e-4)["E"] - math.exp(2.5e-4)) < 1e-12
    for r in rows:
        assert (r["SQ[0]"], r["SQ[1]"], r["SQ[2]"]) == (1.0, 2.0, 3.0)


def test_example_fn2_hypot_and_anglewrap():
    """## Example (MathFn2): hypot(3,4) = 5 (the 3-4-5 triangle); anglewrap(3,4) =
    atan2(4,3) wrapped into [0, 2*pi) ~ 0.9272952 rad."""
    _, stdout, _ = _lib.run_transient(HERE / "example_fn2.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected at least one output row"
    for r in rows:
        assert r["H"] == 5.0
        assert abs(r["AW"] - math.atan2(4.0, 3.0)) < 1e-12


def test_example_fn3_if_select_and_limit_clamp():
    """## Example (MathFn3): if(1, 10, 20) selects the then-branch (10); limit(5, 0, 2) clamps
    to the span of its two bounds (2)."""
    _, stdout, _ = _lib.run_transient(HERE / "example_fn3.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected at least one output row"
    for r in rows:
        assert r["SEL"] == 10.0
        assert r["L"] == 2.0


def test_fn2_scalar_broadcasts_against_vector():
    """## Parameters (MathFn2): a lone Scalar operand broadcasts against a Vector operand --
    hypot(4, [3,4,5]) = [5, sqrt(32), sqrt(41)] elementwise."""
    _, stdout, _ = _lib.run_transient(HERE / "broadcast.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected at least one output row"
    for r in rows:
        assert abs(r["HB[0]"] - 5.0) < 1e-12
        assert abs(r["HB[1]"] - math.sqrt(32.0)) < 1e-12
        assert abs(r["HB[2]"] - math.sqrt(41.0)) < 1e-12


def test_unknown_kind_rejected():
    """## Errors: kind=cosx (no such block kind or function name) -> "unknown device kind
    'cosx'", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_unknown_kind.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "unknown device kind 'cosx'" in stderr


def test_fn1_missing_in_rejected():
    """## Errors (MathFn1): kind=cos without in= -> "missing field 'in'", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_missing_in.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "missing field 'in'" in stderr


def test_fn2_missing_in2_rejected():
    """## Errors (MathFn2): kind=hypot with in1= but no in2= -> "missing field 'in2'",
    at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_missing_in2.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "missing field 'in2'" in stderr


def test_fn3_missing_in3_rejected():
    """## Errors (MathFn3): kind=limit with in1=/in2= but no in3= -> "missing field 'in3'",
    at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_missing_in3.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "missing field 'in3'" in stderr


def test_fn2_vector_length_mismatch_rejected():
    """## Errors (MathFn2): Vector operands of lengths 2 and 3 -> "VectorSignalSizeMismatch
    { block: ..., expected: 2, got: 3 }", at evaluation time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_size_mismatch.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "VectorSignalSizeMismatch" in stderr and "expected: 2, got: 3" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
