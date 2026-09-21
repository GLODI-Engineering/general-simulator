"""Unit test for the Flip-Flop component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/flipflop/test_flipflop.py
Run via pytest:  python3 -m pytest doc-verify/flipflop
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_only_updates_at_the_clock_rising_edge():
    """## Example: Q1 stays 0 before CLK's rising edge (t=5e-3), then latches D=1."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.01, dt=0.001)
    rows = _lib.parse_csv(stdout)
    before_edge = [r for r in rows if r["t"] < 0.005]
    after_edge = [r for r in rows if r["t"] >= 0.005]
    assert before_edge and after_edge
    assert all(r["Q1"] == 0.0 for r in before_edge)
    assert all(r["Q1"] == 1.0 for r in after_edge)


def test_ic_is_the_initial_output():
    """## Parameters / Example: ic=1 holds 1 while nothing drives a change."""
    _, stdout, _ = _lib.run_transient(HERE / "ic_example.cir", tfinal=0.5, dt=0.1)
    rows = _lib.parse_csv(stdout)
    for row in rows:
        assert row["Q1"] == 1.0, row


def test_ic_that_is_not_a_logic_level_is_rejected():
    """## Errors: ic= must be 0 or 1."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ic_not_a_logic_level.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert (
        "must be 0 or 1 (got '2')"
        in stderr
    ), stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
