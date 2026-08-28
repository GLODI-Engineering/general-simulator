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


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
