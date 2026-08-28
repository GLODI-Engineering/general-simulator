"""Unit tests for the Sum component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Sum in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/sum/test_sum.py
Run via pytest:  python3 -m pytest doc-verify/sum
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_scalar_error_junction_and_vector_sum():
    """## Example: signs=1,-1 gives A - B = 2 (error junction); all-Vector inputs of one common
    length reduce elementwise, VSUM = OA + OB = [11, 22]."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected at least one output row"
    for r in rows:
        assert r["ERR"] == 2.0
        assert (r["VSUM[0]"], r["VSUM[1]"]) == (11.0, 22.0)


def test_input_sign_count_mismatch_rejected():
    """## Errors: 2 inputs but 1 sign -> "'inputs' has 2 entries but 'signs' has 1 (need one
    sign per input)", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_count.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "'inputs' has 2 entries but 'signs' has 1 (need one sign per input)" in stderr


def test_non_numeric_sign_rejected():
    """## Errors: signs=1,x -> "field 'signs' entry 'x' is not a number", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_sign.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "field 'signs' entry 'x' is not a number" in stderr


def test_mixed_scalar_vector_inputs_rejected():
    """## Errors: one Scalar and one Vector input -> "VectorSignalNotSupported { block: ... }",
    at evaluation time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_mixed.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "VectorSignalNotSupported" in stderr


def test_vector_length_mismatch_rejected():
    """## Errors: Vector inputs of lengths 2 and 3 -> "VectorSignalSizeMismatch { block: ...,
    expected: 2, got: 3 }", at evaluation time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_length.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "VectorSignalSizeMismatch" in stderr and "expected: 2, got: 3" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
