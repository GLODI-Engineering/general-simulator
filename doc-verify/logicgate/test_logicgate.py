"""Unit tests for the Logic Gate component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/logicgate/test_logicgate.py
Run via pytest:  python3 -m pytest doc-verify/logicgate
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_and_gate_result():
    """## Example: 1 and 0 = 0."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=0.1, dt=0.05)
    rows = _lib.parse_csv(stdout)
    assert rows[-1]["G1"] == 0.0


def test_too_few_inputs_is_rejected():
    """## Errors: and/or/xor/... need at least 2 inputs= entries."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_too_few_inputs.cir", tfinal=0.1, dt=0.05, expect_success=False
    )
    assert code != 0
    assert "needs at least 2" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
