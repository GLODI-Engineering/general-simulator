"""Shared harness for kind=measure end-to-end tests.

Every test_*.py in this directory runs the actual built general-simulator CLI as a subprocess
against a real, committed .cir fixture under fixtures/ -- these are integration tests against the
real binary (mirroring tests/raw-output-python/_lib.py's own convention), not calls into any Rust
code directly.

kind=measure results print to stderr as "name = value" lines (see
crates/general-simulator-cli/src/main.rs's print_measurements, and
book/user-guide/src/measurements.md's "Where results are printed" section, for why stderr and not
stdout -- stdout stays pure CSV/raw, unaffected by whether a netlist uses this feature at all).
parse_measurements() below parses exactly that stream.

Build the CLI first (from the general-simulator repo root):
    cargo build --release -p general-simulator-cli
"""
import pathlib
import re
import subprocess

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
FIXTURES = pathlib.Path(__file__).resolve().parent / "fixtures"
_CLI_RELEASE = REPO_ROOT / "target" / "release" / "general-simulator"
_CLI_DEBUG = REPO_ROOT / "target" / "debug" / "general-simulator"

_LINE_RE = re.compile(r"^(?P<name>\S+) = (?P<value>.+)$")


def cli_path():
    if _CLI_RELEASE.exists():
        return _CLI_RELEASE
    if _CLI_DEBUG.exists():
        return _CLI_DEBUG
    raise FileNotFoundError(
        "general-simulator binary not built -- run `cargo build --release "
        "-p general-simulator-cli` (or plain `cargo build -p general-simulator-cli` for a "
        "debug build) from the general-simulator repo root first."
    )


def run_measure(fixture_name, *extra_args, tfinal=None, dt=None):
    """Runs the CLI in --mode transient against fixtures/<fixture_name> with the given --tfinal/
    --dt (dt=None means adaptive stepping -- no --dt flag at all), plus any extra_args. Returns
    (returncode, stdout, stderr) -- asserts a zero exit code (a measurement fixture is expected
    to run cleanly; a specific measurement's own failure is reported per-line on stderr, not as
    a nonzero exit code, since one bad measurement must not abort the others -- see
    print_measurements's own doc comment)."""
    cir_path = FIXTURES / fixture_name
    args = ["--mode", "transient"]
    if tfinal is not None:
        args += ["--tfinal", str(tfinal)]
    if dt is not None:
        args += ["--dt", str(dt)]
    args += list(extra_args)
    result = subprocess.run(
        [str(cli_path()), str(cir_path), *args],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
    )
    assert result.returncode == 0, (
        f"expected success running {fixture_name}, got exit {result.returncode}\n"
        f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )
    return result.returncode, result.stdout, result.stderr


def parse_measurements(stderr):
    """Parses the CLI's stderr "name = value" lines into a {name: float} dict. A `four`
    measurement's harmonic sub-results (`<name>_h1_mag`, `<name>_h1_phase_deg`, `<name>_dc`,
    `<name>_thd_percent`) come back as ordinary extra keys, exactly as printed."""
    out = {}
    for line in stderr.splitlines():
        m = _LINE_RE.match(line.strip())
        if not m:
            continue
        try:
            out[m.group("name")] = float(m.group("value"))
        except ValueError:
            continue
    return out


def assert_close(actual, expected, tol, label=""):
    assert abs(actual - expected) <= tol, (
        f"{label}: expected {expected} +/- {tol}, got {actual} (diff {abs(actual - expected)})"
    )
