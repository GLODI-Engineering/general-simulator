"""Unit tests for the OctFunc component-reference entry -- see README.md for what each proves,
and for OctaveError::NotFound/ProcessExited, intentionally not covered here (see README.md's
"Not covered by the automated test").

Run standalone:  python3 doc-verify/octfunc/test_octfunc.py
Run via pytest:  python3 -m pytest doc-verify/octfunc

Requires octave-cli on PATH at run time, and a build of the CLI (no --features python needed --
see README.md).
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_add_one_function_is_applied():
    """## Example: SRC=3 through octfunc calling add_one gives G1 = 4."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-4, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows[0]["G1"] == 4.0


def test_path_through_a_space_containing_directory_requires_quoting():
    """## Parameters: path= must be "double-quoted" when the path contains whitespace."""
    _, stdout, _ = _lib.run_transient(HERE / "example_quoted_path.cir", tfinal=1e-4, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows[0]["G1"] == 4.0


def test_each_declared_input_arrives_as_its_own_positional_argument():
    """## Description: inputs=A,B calls function(A, B), never function([A, B]) -- subtract(10, 3)
    proves the two signals are genuinely distinct positional arguments, not indices into one
    bundled array (order-sensitive: swapping A/B would flip the sign if it were bundled wrong)."""
    _, stdout, _ = _lib.run_transient(HERE / "example_two_inputs.cir", tfinal=1e-4, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows[0]["G1"] == 7.0


def test_ts_variable_is_rejected():
    """## Errors: ts=variable is not available for kind=octfunc, rejected at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ts_variable.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "ts=variable is not available for kind=octfunc" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
