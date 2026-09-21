"""Unit test for the PMSM component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/pmsm/test_pmsm.py
Run via pytest:  python3 -m pytest doc-verify/pmsm
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_stays_at_rest_with_zero_input():
    """## Example: vd=vq=t_load=0 -> the motor stays exactly at rest."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert rows
    for r in rows:
        assert r["M1"] == 0.0 and r["M1_iq"] == 0.0
        assert r["M1_omega_m"] == 0.0 and r["M1_theta_e"] == 0.0


def test_ic_of_the_wrong_length_is_rejected():
    """## Errors: ic= must have exactly four entries."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ic_wrong_length.cir", tfinal=1, dt=0.1, expect_success=False
    )
    assert code != 0
    assert (
        "field 'ic' has 3 value(s), but this block has 4 state(s)"
        in stderr
    ), stderr


def test_ic_is_the_four_machine_states():
    """## Parameters / Example: a torque-free rotor coasts from its declared speed and angle."""
    _, stdout, _ = _lib.run_transient(HERE / "ic_example.cir", tfinal=0.003, dt=0.001)
    rows = _lib.parse_csv(stdout)
    for row in rows:
        assert row["WM"] == 100.0, row
        assert abs(row["TH"] - (0.5 + 400.0 * row["t"])) < 1e-9, row


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
