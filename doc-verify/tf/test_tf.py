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


def test_ic_starts_the_canonical_state_where_declared():
    """## Parameters / Example: ic=[0.5] on 1/(s+1) gives y(t) = 1 - 0.5 exp(-t)."""
    _, stdout, _ = _lib.run_transient(HERE / "ic_example.cir", tfinal=1, dt=0.1)
    rows = _lib.parse_csv(stdout)
    import math
    for row in rows:
        expected = 1.0 - 0.5 * math.exp(-row["t"])
        assert abs(row["TF1"] - expected) < 1e-6, f"t={row['t']}: got {row['TF1']}"


def test_y0_with_the_matching_input_is_an_equilibrium():
    """## Parameters / Example: y0=6 with u=4 (DC gain 1.5) holds 6 for the whole run."""
    _, stdout, _ = _lib.run_transient(HERE / "y0_example.cir", tfinal=1, dt=0.1)
    rows = _lib.parse_csv(stdout)
    for row in rows:
        assert abs(row["TF1"] - 6.0) < 1e-9, f"t={row['t']}: got {row['TF1']}"


def test_ic_of_the_wrong_length_is_rejected():
    """## Errors: an ic= whose length is not the state count."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ic_wrong_length.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert (
        "field 'ic' has 2 value(s), but this block has 1 state(s)"
        in stderr
    ), stderr


def test_ic_and_y0_together_are_rejected():
    """## Errors: ic= and y0= are mutually exclusive."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ic_and_y0.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert (
        "both set an initial condition -- give one or the other"
        in stderr
    ), stderr


def test_y0_on_a_dc_blocking_transfer_function_is_rejected():
    """## Errors: y0= needs a nonzero DC numerator."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_y0_zero_dc_numerator.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert (
        "numerator is zero at DC, so its settled output is always 0"
        in stderr
    ), stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
