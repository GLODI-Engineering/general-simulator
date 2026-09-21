"""Unit test for the Discrete Transfer Function component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/discretetf/test_discretetf.py
Run via pytest:  python3 -m pytest doc-verify/discretetf
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_matches_the_equivalent_discretestatespace_realization():
    """## Example: 0.1/(z-1), matching discretestatespace's own equivalent example (0.5 after 5 hits)."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.5, dt=0.05)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["DTF1"] - 0.5) < 1e-9, f"got {rows[-1]['DTF1']}"


def test_y0_with_the_matching_input_is_a_fixed_point():
    """## Parameters / Example: y0=3 with u=2 (H(1) = 1.5) holds exactly 3."""
    _, stdout, _ = _lib.run_transient(HERE / "y0_example.cir", tfinal=0.5, dt=0.1)
    rows = _lib.parse_csv(stdout)
    for row in rows:
        assert row["DTF1"] == 3.0, f"t={row['t']}: got {row['DTF1']}"


def test_ic_is_the_canonical_state():
    """## Parameters: ic=[8] on 1/(z-0.5) reads 4, 2, 1."""
    _, stdout, _ = _lib.run_transient(HERE / "ic_example.cir", tfinal=0.3, dt=0.1)
    rows = _lib.parse_csv(stdout)
    assert [row["DTF1"] for row in rows[:3]] == [4.0, 2.0, 1.0], rows[:3]


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
