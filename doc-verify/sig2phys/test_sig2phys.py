"""Unit tests for the Sig2Phys component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/sig2phys/test_sig2phys.py
Run via pytest:  python3 -m pytest doc-verify/sig2phys
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_domain_voltage_example_drives_the_voltage_source_exactly():
    """## Example (domain=voltage): V1's magnitude tracks CMD=5 through the converter ->
    V(a) == 5."""
    _, stdout, _ = _lib.run_transient(HERE / "example_voltage.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["V(a)"] - 5.0) < 1e-9, f"got {rows[-1]['V(a)']}"


def test_domain_current_example_drives_the_current_source_magnitude_correctly():
    """## Example (domain=current): I1's magnitude tracks CMD=0.1 through the converter --
    Ohm's law over R1=1k gives |V(a)| == 100."""
    _, stdout, _ = _lib.run_transient(HERE / "example_current.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert abs(abs(rows[-1]["V(a)"]) - 100.0) < 1e-6, f"got {rows[-1]['V(a)']}"


def test_missing_domain_is_rejected():
    """## Errors: domain= is required, no default."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_missing_domain.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "missing field 'domain'" in stderr


def test_invalid_domain_value_is_rejected():
    """## Errors: domain= must be exactly 'voltage' or 'current'."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_invalid_domain.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "must be 'voltage' or 'current'" in stderr
    assert "got 'power'" in stderr


def test_voltage_source_naming_a_domain_current_converter_is_rejected():
    """## Description: a V source's own literal value must name a domain=voltage converter,
    never a domain=current one -- SourceNotSig2PhysicalConverter."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_voltage_source_names_current_domain.cir",
        tfinal=1e-4,
        dt=1e-5,
        expect_success=False,
    )
    assert code != 0
    assert "SourceNotSig2PhysicalConverter" in stderr
    assert "sig2phys(domain=voltage)" in stderr
    assert "sig2phys(domain=current)" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
