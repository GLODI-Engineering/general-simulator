"""Unit tests for the CScript component-reference entry -- see README.md for what each proves.
One function per documented behavior, matching the doc comment on BlockKind::CScript in
general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/cscript/test_cscript.py
Run via pytest:  python3 -m pytest doc-verify/cscript
"""
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def _compile(src, out):
    out.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        ["cc", "-shared", "-fPIC", "-O0", "-o", str(out), str(src)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, f"cc failed to compile {src}:\n{result.stderr}"


def setup_module():
    _compile(HERE / "gain.c", HERE / "gain.so")
    _compile(HERE / "gain.c", HERE / "a lib dir" / "gain.so")


def test_example_gain_block_doubles_its_input():
    """## Example: SRC=3 through a cscript block computing out = in * 2 gives G1 = 6."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-4, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows[0]["G1"] == 6.0


def test_lib_path_through_a_space_containing_directory_requires_quoting():
    """## Parameters: lib= must be "double-quoted" when the path contains whitespace."""
    _, stdout, _ = _lib.run_transient(HERE / "example_quoted_path.cir", tfinal=1e-4, dt=1e-5)
    rows = _lib.parse_csv(stdout)
    assert rows[0]["G1"] == 6.0


def test_adaptive_stepping_without_cscript_clone_is_rejected():
    """## Errors: adaptive stepping (no --dt) with a lib exporting no cscript_clone fails with
    CScriptRequiresCloneForAdaptiveStep."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_adaptive_needs_clone.cir", tfinal=1e-4, dt=None, expect_success=False
    )
    assert code != 0
    assert "CScriptRequiresCloneForAdaptiveStep" in stderr


if __name__ == "__main__":
    setup_module()
    _lib.run_all(sys.modules[__name__])
