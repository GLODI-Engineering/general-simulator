"""Unit tests for the Transfer Function component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/tf/test_tf.py
Run via pytest:  python3 -m pytest doc-verify/tf
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_matches_the_equivalent_statespace_realization():
    """## Example: 1/(s+1) settles toward 1, matching statespace's own equivalent example."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=10, dt=0.1)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["TF1"] - 1.0) < 1e-3, f"got {rows[-1]['TF1']}"


def test_empty_denominator_is_rejected():
    """## Errors: den=[] is rejected at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_empty_denominator.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert "EmptyDenominator" in stderr


def test_improper_transfer_function_is_rejected():
    """## Errors: deg(num) > deg(den) is rejected at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_improper.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert "ImproperTransferFunction" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
