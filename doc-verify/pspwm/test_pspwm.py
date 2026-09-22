"""Unit tests for the PWM Modulator 2 (Phase-Shift PWM) component-reference entry -- see
README.md.

Run standalone:  python3 doc-verify/pspwm/test_pspwm.py
Run via pytest:  python3 -m pytest doc-verify/pspwm
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_main_and_complement_always_sum_to_one():
    """## Example: 10kHz, zero phase shift, 40% duty -- PSPWM1 + PSPWM1_comp == 1 every step."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-4, dt=1e-6)
    rows = _lib.parse_csv(stdout)
    assert rows
    for r in rows:
        assert r["PSPWM1"] + r["PSPWM1_comp"] == 1.0, f"mismatch at t={r['t']}: {r}"


def test_bad_inputs_count_is_rejected():
    """## Errors: inputs= must have exactly 3 entries (freq,phase,duty)."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_inputs_count.cir", tfinal=1e-4, dt=1e-6, expect_success=False
    )
    assert code != 0
    assert "needs 3 inputs" in stderr


def test_fmin_exceeds_fmax_is_rejected():
    """## Errors: f_min > f_max is rejected at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_fmin_exceeds_fmax.cir", tfinal=1e-4, dt=1e-6, expect_success=False
    )
    assert code != 0
    assert "FMinExceedsFMax" in stderr


def test_ic_is_the_initial_carrier_phase():
    """## Parameters / Example: ic=0.75 shifts the first edge -- 0, 0, 1, 1, 1."""
    _, stdout, _ = _lib.run_transient(HERE / "ic_example.cir", tfinal=0.5, dt=0.1)
    rows = _lib.parse_csv(stdout)
    assert [row["PSPWM1"] for row in rows[:5]] == [0.0, 0.0, 1.0, 1.0, 1.0], rows[:5]
    assert [row["PSPWM1_comp"] for row in rows[:5]] == [1.0, 1.0, 0.0, 0.0, 0.0], rows[:5]


def test_ic_outside_the_unit_interval_is_rejected():
    """## Errors: ic= must satisfy 0 <= ic < 1."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ic_out_of_range.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert (
        "must satisfy 0 <= ic < 1 (got 1)"
        in stderr
    ), stderr


def test_phase_input_is_a_lead_not_a_lag():
    """## Parameters: phase is a lead -- a block commanded phase=0.25 begins its on-interval a
    quarter period BEFORE the phase=0 block, at t = (1 - 0.25) T = 0.75 s, not at 0.25 s
    after it."""
    _, stdout, _ = _lib.run_transient(HERE / "phase_lead.cir", tfinal=1, dt=0.125)
    rows = _lib.parse_csv(stdout)
    ref = [row["REF"] for row in rows[:8]]
    lead = [row["LEAD"] for row in rows[:8]]
    assert ref == [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0], rows[:8]
    assert lead == [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0], rows[:8]

    def rising_edge_time(name):
        prev = None
        for row in rows:
            if prev == 0.0 and row[name] == 1.0:
                return row["t"]
            prev = row[name]
        raise AssertionError(f"{name} never rose")

    t_ref = rising_edge_time("REF")
    t_lead = rising_edge_time("LEAD")
    assert t_ref == 1.0, t_ref
    assert t_lead == 0.75, t_lead
    assert t_lead < t_ref, "phase=0.25 must rise EARLIER than phase=0 (lead, not lag)"
    assert t_ref - t_lead == 0.25


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
