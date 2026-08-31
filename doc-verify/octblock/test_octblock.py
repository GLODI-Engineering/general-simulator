"""Unit tests for the OctBlock component-reference entry -- see README.md for what each proves,
and for what is intentionally not covered here.

Run standalone:  python3 doc-verify/octblock/test_octblock.py
Run via pytest:  python3 -m pytest doc-verify/octblock

Requires octave-cli on PATH at run time, and a build of the CLI (no --features python needed --
see README.md).
"""
import math
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_accumulator_carries_state_forward():
    """## Example: SRC=3 through octblock accumulating SRC every step gives G1 = 3, 6, 9, ..."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=5e-5, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert [r["G1"] for r in rows] == [3.0, 6.0, 9.0, 12.0, 15.0]


def test_xc_count_continuous_state_block_integrates_correctly():
    """## Description: xc_count>0 uses <function>_derivative/<function>_output_xc instead of
    the plain <function>, and the resulting continuous state is genuinely solver-integrated
    (RK4). xc_charge_derivative.m implements dxc/dt = 1 - xc, starting at rest (xc(0) = 0) --
    the analytic solution is xc(t) = 1 - exp(-t); at t=0.01 that's ~0.0099501663."""
    _, stdout, _ = _lib.run_transient(HERE / "xc_example.cir", tfinal=0.01, dt=0.001)
    rows = _lib.parse_csv(stdout)
    expected = 1.0 - math.exp(-0.01)
    assert abs(rows[-1]["G1"] - expected) < 1e-6, f"got {rows[-1]['G1']}, expected ~{expected}"


def test_ts_variable_uses_next_sample_hit():
    """## Parameters: ts=variable requires <function>_next_sample_hit.m; ticker_next_sample_hit.m
    always requests a fixed 0.002s interval, so G1 should have advanced to 5 by t=0.01."""
    _, stdout, _ = _lib.run_transient(HERE / "ts_variable_example.cir", tfinal=0.01, dt=0.0005)
    rows = _lib.parse_csv(stdout)
    assert rows[-1]["G1"] == 5.0, f"got {rows[-1]['G1']}"


def test_missing_required_file_is_rejected_at_construction():
    """## Errors: a required <function>_*.m file missing from path='s own directory is rejected
    at construction time with OctBlockMissingRequiredFile, before octave-cli is ever asked."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_missing_required_file.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "OctBlockMissingRequiredFile" in stderr
    assert "accumulate.m" in stderr


def test_ts_variable_without_next_sample_hit_is_rejected():
    """## Errors: ts=variable without <function>_next_sample_hit.m present is rejected at
    construction time with OctBlockRequiresNextSampleHitForVariableSampleTime."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ts_variable_missing_next_sample_hit.cir",
        tfinal=1e-4,
        dt=1e-5,
        expect_success=False,
    )
    assert code != 0
    assert "OctBlockRequiresNextSampleHitForVariableSampleTime" in stderr


def test_adaptive_stepping_is_rejected():
    """## Errors: kind=octblock does not support TimeStep::Adaptive -- rejected at construction
    time with OctBlockDoesNotSupportAdaptiveStep (this instance's own opaque state lives inside
    the shared octave-cli session, which a rejected adaptive trial step cannot roll back)."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_adaptive_not_supported.cir", tfinal=1e-4, dt=None, expect_success=False
    )
    assert code != 0
    assert "OctBlockDoesNotSupportAdaptiveStep" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
