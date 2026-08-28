"""Unit tests for the Product component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Product in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/product/test_product.py
Run via pytest:  python3 -m pytest doc-verify/product
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_product_of_three_inputs():
    """## Example: P = 2 * 3 * 4 = 24, every step."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert rows, "expected at least one output row"
    for r in rows:
        assert r["P"] == 24.0


def test_mixed_scalar_vector_inputs_rejected():
    """## Errors: one Scalar and one Vector input -> "VectorSignalNotSupported { block: ... }",
    at evaluation time (the same all-or-nothing rule as Sum)."""
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
