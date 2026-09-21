"""Unit test for the VCO component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/vco/test_vco.py
Run via pytest:  python3 -m pytest doc-verify/vco
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_ramps_by_dt_times_freq_each_step():
    """## Example: freq=10, dt=0.01 -> the [0,1) ramp advances by exactly 0.1 each step."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.2, dt=0.01)
    rows = _lib.parse_csv(stdout)
    assert len(rows) >= 2
    for a, b in zip(rows, rows[1:]):
        step = (b["VCO1"] - a["VCO1"]) % 1.0
        assert abs(step - 0.1) < 1e-9, f"got step {step}"


def test_ic_is_the_initial_phase():
    """## Parameters / Example: ic=0.25 at 1 Hz reads 0.25 + t (mod 1)."""
    _, stdout, _ = _lib.run_transient(HERE / "ic_example.cir", tfinal=1, dt=0.1)
    rows = _lib.parse_csv(stdout)
    for row in rows:
        expected = (0.25 + row["t"]) % 1.0
        assert abs(row["VCO1"] - expected) < 1e-9, f"t={row['t']}: got {row['VCO1']}"


def test_ic_outside_the_unit_interval_is_rejected():
    """## Errors: ic= must satisfy 0 <= ic < 1."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ic_out_of_range.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert (
        "must satisfy 0 <= ic < 1 (got 1.5)"
        in stderr
    ), stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
