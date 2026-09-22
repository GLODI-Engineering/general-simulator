"""Unit tests for the CScript component-reference entry -- see README.md for what each proves.
One function per documented behavior, matching the doc comment on BlockKind::CScript in
general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/cscript/test_cscript.py
Run via pytest:  python3 -m pytest doc-verify/cscript
"""
import pathlib
import subprocess
import sys
import tempfile

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
    _compile(HERE / "accumulator_checkpoint.c", HERE / "accumulator_checkpoint.so")
    _compile(HERE / "partial_state.c", HERE / "partial_state.so")


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


def _split_run_matches_whole(cir):
    """Runs cir uninterrupted to 2e-3, then split at 1e-3 through a checkpoint file; returns
    (whole_csv, first_csv + resumed rows) for a byte-for-byte comparison."""
    with tempfile.TemporaryDirectory() as tmp:
        ckpt = str(pathlib.Path(tmp) / "half.ckpt")
        _, whole, _ = _lib.run_transient(cir, tfinal=2e-3, dt=1e-4)
        _, first, _ = _lib.run(cir, "--mode", "transient", "--tfinal", "1e-3", "--dt", "1e-4",
                               "--checkpoint-out", ckpt)
        assert pathlib.Path(ckpt).is_file()
        _, second, _ = _lib.run(cir, "--mode", "transient", "--tfinal", "2e-3", "--dt", "1e-4",
                                "--resume", ckpt)
    first_lines = first.splitlines()
    second_lines = second.splitlines()
    assert first_lines[0] == second_lines[0], "headers differ"
    return whole, "\n".join(first_lines + second_lines[1:]) + "\n"


def test_checkpoint_state_contract_round_trips_bit_for_bit():
    """The CScript chapter, "Checkpoint and resume": a library exporting cscript_state_size/
    _write/_read checkpoints and resumes like any built-in block -- the split run's CSV is
    byte for byte the uninterrupted run's, and the accumulator genuinely carried state."""
    whole, joined = _split_run_matches_whole(HERE / "checkpoint_example.cir")
    assert whole == joined
    rows = _lib.parse_csv(whole)
    assert len(rows) == 20
    assert rows[9]["G1"] != 0.0 and rows[19]["G1"] > rows[9]["G1"]


def test_partial_state_contract_is_rejected_at_load_naming_the_missing_symbol():
    """The CScript chapter, "Checkpoint and resume": exporting only some of the three symbols
    is a load-time error naming the missing one, even with no checkpoint requested."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_partial_state_contract.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "MissingSymbol" in stderr
    assert "cscript_state_read" in stderr


def test_no_state_contract_runs_normally_but_is_refused_when_a_checkpoint_is_requested():
    """The CScript chapter, "Checkpoint and resume": a library exporting none of the three
    symbols runs unchanged without --checkpoint-out, and is refused with
    CheckpointUnsupportedBlock naming the block when one is asked for; no file is written."""
    cir = HERE / "error_checkpoint_unsupported.cir"
    _, stdout, _ = _lib.run_transient(cir, tfinal=1e-4, dt=1e-5)
    assert _lib.parse_csv(stdout)[0]["G1"] == 6.0
    with tempfile.TemporaryDirectory() as tmp:
        ckpt = pathlib.Path(tmp) / "refused.ckpt"
        code, _, stderr = _lib.run(cir, "--mode", "transient", "--tfinal", "1e-4", "--dt", "1e-5",
                                   "--checkpoint-out", str(ckpt), expect_success=False)
        assert code != 0
        assert "CheckpointUnsupportedBlock" in stderr
        assert "G1" in stderr
        assert not ckpt.exists()


if __name__ == "__main__":
    setup_module()
    _lib.run_all(sys.modules[__name__])
