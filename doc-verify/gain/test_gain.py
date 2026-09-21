"""Unit tests for the Gain component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Gain in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/gain/test_gain.py
Run via pytest:  python3 -m pytest doc-verify/gain
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_scalar_and_matrix_gain():
    """## Example: scalar k=3 gives G = 3*2 = 6; matrix k=[[1,0,0],[0,1,0]] (2x3) times
    V=[1,2,3] is the matrix-vector product [1,2]."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected at least one output row"
    for r in rows:
        assert r["G"] == 6.0
        assert (r["M[0]"], r["M[1]"]) == (1.0, 2.0)


def test_flat_list_k_rejected():
    """## Errors: k=[1,2,3] (a flat list, never a valid Gain shape) -> "field 'k' must be a
    scalar (e.g. '2.0') or a matrix (e.g. '[[1,0],[0,1]]') -- got a flat list '[1,2,3]',
    which is not a valid Gain shape", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_flat_list.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "got a flat list '[1,2,3]', which is not a valid Gain shape" in stderr


def test_non_numeric_k_rejected():
    """## Errors: k=abc -> "field 'k' is not a number", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_not_number.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "field 'k' is not a number" in stderr


def test_matrix_gain_with_scalar_input_rejected():
    """## Errors: matrix k with a Scalar input -> "VectorSignalNotSupported { block: ... }",
    at evaluation time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_scalar_input.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "VectorSignalNotSupported" in stderr


def test_matrix_gain_with_wrong_length_vector_rejected():
    """## Errors: 2-column matrix k with a length-3 Vector input ->
    "VectorSignalSizeMismatch { block: ..., expected: 2, got: 3 }", at evaluation time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_size_mismatch.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "VectorSignalSizeMismatch" in stderr and "expected: 2, got: 3" in stderr


def test_ic_on_a_stateless_block_is_rejected():
    """BlockInstance's generic errors: ic= on a kind with no state."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ic_on_stateless_block.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert (
        "this kind has no state to initialize"
        in stderr
    ), stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
