"""Unit tests for the Phys2Sig component-reference entry -- see README.md.

Run standalone:  python3 doc-verify/phys2sig/test_phys2sig.py
Run via pytest:  python3 -m pytest doc-verify/phys2sig
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def test_example_tracks_the_node_voltage_exactly():
    """## Example: a 5V/(1k,1k) divider -> PROBE1 == V(out) == 2.5 exactly, once settled.

    "Once settled" is load-bearing: see the next test. This one deliberately still asserts on
    the LAST row, which is what the entry's own `## Example` claims.
    """
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["PROBE1"] - 2.5) < 1e-9, f"got {rows[-1]['PROBE1']}"
    assert rows[-1]["PROBE1"] == rows[-1]["V(out)"]


def test_the_read_is_one_step_behind_the_quantity_it_reads():
    """## Implementation notes: a phys2sig reports the circuit as of t - dt, and 0.0 in the
    first row, because the block graph is evaluated before the circuit is solved.

    The same divider at a coarse dt, where the delay is visible rather than hidden by the
    settled steady state the test above happens to land in.
    """
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=4e-3, dt=1e-3)
    rows = _lib.parse_csv(stdout)
    assert len(rows) >= 3, f"expected at least three rows, got {len(rows)}"
    assert rows[0]["PROBE1"] == 0.0, f"first row has nothing to sample; got {rows[0]['PROBE1']}"
    assert rows[0]["V(out)"] != rows[0]["PROBE1"], "the first row must visibly differ"
    for i in range(1, len(rows)):
        assert abs(rows[i]["PROBE1"] - rows[i - 1]["V(out)"]) < 1e-12, (
            f"row {i}: PROBE1={rows[i]['PROBE1']} should be the previous row's "
            f"V(out)={rows[i - 1]['V(out)']}"
        )


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


def test_ammeter_idiom_measures_the_series_current_exactly():
    """## Example (ammeter idiom): a 0V source in series with R1 gets IMEAS == I(VAMM) ==
    5 / (1000 + 1000) = 0.0025 A exactly, and doesn't perturb the divider's own node
    voltages."""
    _, stdout, _ = _lib.run_transient(HERE / "ammeter.cir", tfinal=1e-3, dt=1e-4)
    rows = _lib.parse_csv(stdout)
    assert abs(rows[-1]["IMEAS"] - 0.0025) < 1e-9, f"got {rows[-1]['IMEAS']}"
    assert rows[-1]["IMEAS"] == rows[-1]["I(VAMM)"]
    # Same one-step caveat as the divider: the ammeter reads the previous step, so the
    # agreement above holds from the second row on, not the first.
    assert rows[0]["IMEAS"] == 0.0, f"got {rows[0]['IMEAS']}"
    assert abs(rows[-1]["V(out)"] - 2.5) < 1e-9, f"got {rows[-1]['V(out)']}"


def test_branch_naming_a_non_branch_device_is_rejected():
    """## Errors: branch=<name> naming an element with no MNA branch-current unknown (only
    V/L/E/H sources have one) is a hard build-time error, not a silent 0.0."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_branch_not_a_branch_device.cir",
        tfinal=1e-4,
        dt=1e-5,
        expect_success=False,
    )
    assert code != 0
    assert "has no current unknown available" in stderr
    assert "insert a 0V voltage source in series" in stderr


def test_branch_naming_a_nonexistent_element_is_rejected():
    """## Errors: branch=<name> naming an element that doesn't exist at all is a distinct
    build-time error from the wrong-type case above."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_branch_no_such_element.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "names no such element" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
