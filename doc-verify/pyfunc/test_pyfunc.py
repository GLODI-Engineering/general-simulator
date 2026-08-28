"""Unit tests for the PyFunction component-reference entry -- see README.md for what each
proves, and for the one Errors claim (PythonSupportNotCompiledIn) intentionally not covered
here.

Run standalone:  python3 doc-verify/pyfunc/test_pyfunc.py
Run via pytest:  python3 -m pytest doc-verify/pyfunc

Requires a --features python build -- see README.md.
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_add_one_function_is_applied():
    """## Example: SRC=3 through pyfunc calling add_one gives G1 = 4."""
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
    bundled list (order-sensitive: swapping A/B would flip the sign if it were bundled wrong)."""
    _, stdout, _ = _lib.run_transient(HERE / "example_two_inputs.cir", tfinal=1e-4, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows[0]["G1"] == 7.0


def test_ts_variable_is_rejected():
    """## Errors: ts=variable is not available for kind=pyfunc, rejected at parse time."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_ts_variable.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "ts=variable is not available for kind=pyfunc" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
