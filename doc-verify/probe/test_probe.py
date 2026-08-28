"""Unit tests for the Probe component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/probe/test_probe.py
Run via pytest:  python3 -m pytest doc-verify/probe
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_tracks_the_node_voltage_exactly():
    """## Example: a 5V/(1k,1k) divider -> PROBE1 == V(out) == 2.5 exactly."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["PROBE1"] - 2.5) < 1e-9, f"got {rows[-1]['PROBE1']}"
    assert rows[-1]["PROBE1"] == rows[-1]["V(out)"]


def test_both_node_and_branch_is_rejected():
    """## Errors: node= and branch= are mutually exclusive."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_both_node_and_branch.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "mutually exclusive" in stderr


def test_neither_node_nor_branch_is_rejected():
    """## Errors: at least one of node=/branch= is required."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_neither.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "needs 'node=" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
