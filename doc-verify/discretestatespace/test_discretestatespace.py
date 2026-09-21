"""Unit test for the Discrete State-Space component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/discretestatespace/test_discretestatespace.py
Run via pytest:  python3 -m pytest doc-verify/discretestatespace
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_reaches_exactly_half_after_five_sample_hits():
    """## Example: x[i+1]=x[i]+0.1*u[i], u=1, ts=0.1 -> DS1=0.5 after t=0.5 (5 hits)."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.5, dt=0.05)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["DS1"] - 0.5) < 1e-9, f"got {rows[-1]['DS1']}"


def test_ic_is_the_state_vector():
    """## Parameters / Example: ic=8 on x[k+1] = 0.5 x[k] reads 4, 2, 1."""
    _, stdout, _ = _lib.run_transient(HERE / "ic_example.cir", tfinal=0.3, dt=0.1)
    rows = _lib.parse_csv(stdout)
    assert [row["DSS1"] for row in rows[:3]] == [4.0, 2.0, 1.0], rows[:3]


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


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
