"""Unit tests for the Waveform component-reference entry -- see README.md for what each proves.
One function per documented behavior (an Example, or an Errors claim), matching the doc
comment on BlockKind::Waveform in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/waveform/test_waveform.py
Run via pytest:  python3 -m pytest doc-verify/waveform
"""
import math
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402

T_FINAL = 2e-3
DT = 2.5e-4


def _row_at(rows, t):
    return next(r for r in rows if abs(r["t"] - t) < 1e-12)


def _example_rows():
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=T_FINAL, dt=DT)
    return _lib.parse_csv(stdout)


def test_example_sinwave_formula():
    """## Example, sinwave: va=10 freq=1000 (v0/td/theta/phase default 0) gives
    10*sin(2*pi*1000*t): 10 at 0.25ms, ~0 at 0.5ms, -10 at 0.75ms, ~0 at 1ms."""
    rows = _example_rows()
    assert _row_at(rows, 2.5e-4)["SINREF"] == 10.0
    assert abs(_row_at(rows, 5e-4)["SINREF"]) < 1e-12
    assert _row_at(rows, 7.5e-4)["SINREF"] == -10.0
    assert abs(_row_at(rows, 1e-3)["SINREF"]) < 1e-12


def test_example_pulsewave_formula():
    """## Example, pulsewave: v1=0 v2=10 td=1ms tr=tf=0.1ms pw=0.2ms per=1ms -- flat at v1
    until td (0 at 0.25ms), then at v2 after the rise (10 at 1.25ms), back at v1 in the
    period's remainder (0 at 1.75ms)."""
    rows = _example_rows()
    assert _row_at(rows, 2.5e-4)["PULSEREF"] == 0.0
    assert _row_at(rows, 7.5e-4)["PULSEREF"] == 0.0
    assert _row_at(rows, 1e-3)["PULSEREF"] == 0.0  # td itself: rise starts, still v1
    assert _row_at(rows, 1.25e-3)["PULSEREF"] == 10.0
    assert _row_at(rows, 1.75e-3)["PULSEREF"] == 0.0


def test_example_expwave_formula():
    """## Example, expwave: v1=0 v2=1 tau1=0.1ms tau2=0.1ms td2=1ms -- rises as
    1 - e^(-t/tau1) until td2 (0.917915 at 0.25ms), then decays back toward v1
    ((1-e^-10)*e^-5 ~ 0.006738 at 1.5ms)."""
    rows = _example_rows()
    assert abs(_row_at(rows, 2.5e-4)["EXPREF"] - (1.0 - math.exp(-2.5))) < 1e-9
    expected_at_1_5ms = (1.0 - math.exp(-10.0)) * math.exp(-5.0)
    assert abs(_row_at(rows, 1.5e-3)["EXPREF"] - expected_at_1_5ms) < 1e-9


def test_example_sffmwave_formula():
    """## Example, sffmwave: v0=0 va=1 fc=1000 mdi=0 fs=100 -- mdi=0 makes it a plain
    sin(2*pi*1000*t): 1 at 0.25ms, -1 at 0.75ms."""
    rows = _example_rows()
    assert _row_at(rows, 2.5e-4)["FMREF"] == 1.0
    assert _row_at(rows, 7.5e-4)["FMREF"] == -1.0


def test_missing_required_field_rejected():
    """## Errors: sinwave without its required va -> "missing field 'va'", at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_missing_va.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "missing field 'va'" in stderr


def test_non_numeric_required_field_rejected():
    """## Errors: va=abc (required field, not a number) -> "field 'va' is not a number",
    at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_bad_va.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "field 'va' is not a number" in stderr


def test_malformed_optional_field_silently_defaults():
    """## Errors: an optional field that fails to parse (td=abc) does NOT error -- it silently
    falls back to its default (td=0), so the run succeeds with the default's waveform."""
    code, stdout, stderr = _lib.run_transient(
        HERE / "error_malformed_optional.cir", tfinal=T_FINAL, dt=DT, expect_success=True
    )
    assert code == 0, f"expected success (silent default), got: {stderr}"
    rows = _lib.parse_csv(stdout)
    assert _row_at(rows, 2.5e-4)["A"] == 10.0  # exactly what td=0 would produce


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
