"""Shared harness for tests/raw-output-python's cross-validation tests.

Runs the actual built general-simulator CLI as a subprocess (same discipline as
doc-verify/_lib.py) once with --format csv and once with --format raw, against the exact same
fixture netlists already hand-verified by crates/general-simulator-cli/tests/cli.rs, and hands
the two Python cross-validation tests everything they need to compare them.

Build the CLI first (from the general-simulator repo root):
    cargo build --release -p general-simulator-cli
"""
import csv
import io
import pathlib
import subprocess
import tempfile

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
FIXTURES = REPO_ROOT / "crates" / "general-simulator-cli" / "tests" / "fixtures"
_CLI_RELEASE = REPO_ROOT / "target" / "release" / "general-simulator"
_CLI_DEBUG = REPO_ROOT / "target" / "debug" / "general-simulator"


def cli_path():
    if _CLI_RELEASE.exists():
        return _CLI_RELEASE
    if _CLI_DEBUG.exists():
        return _CLI_DEBUG
    raise FileNotFoundError(
        "general-simulator binary not built -- run `cargo build --release "
        "-p general-simulator-cli` from the general-simulator repo root first."
    )


def run(*args):
    result = subprocess.run(
        [str(cli_path()), *args],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
    )
    assert result.returncode == 0, (
        f"expected success running general-simulator {' '.join(args)}, "
        f"got exit {result.returncode}\nstderr:\n{result.stderr}"
    )
    return result.stdout


def parse_csv(stdout):
    """Parses CSV stdout into (headers, rows) -- rows as lists of float, in column order,
    matching the raw file's own column order."""
    reader = csv.reader(io.StringIO(stdout))
    rows = list(reader)
    headers = rows[0]
    data = [[float(v) for v in row] for row in rows[1:]]
    return headers, data


def run_csv_and_raw(netlist_name, devices_name, mode_args):
    """Runs the same netlist twice -- once for CSV (stdout), once for a binary rawfile written to
    a scratch temp dir -- and returns (csv_headers, csv_rows, raw_path). Caller is responsible for
    cleaning up raw_path's parent directory."""
    netlist = FIXTURES / netlist_name
    devices = FIXTURES / devices_name

    csv_stdout = run(str(netlist), "--devices", str(devices), *mode_args)
    headers, rows = parse_csv(csv_stdout)

    out_dir = pathlib.Path(tempfile.mkdtemp(prefix="general-simulator-raw-output-python-"))
    raw_path = out_dir / (netlist.stem + ".raw")
    stdout = run(
        str(netlist),
        "--devices",
        str(devices),
        *mode_args,
        "--format",
        "raw",
        "--out",
        str(raw_path),
    )
    assert stdout == "", "--format raw must not also print CSV to stdout"
    assert raw_path.exists(), f"expected {raw_path} to be written"

    return headers, rows, raw_path


def run_all(module):
    """Runs every module-level test_* function and reports pass/fail, so this file works both
    standalone (`python3 tests/raw-output-python/test_raw_output.py`) and via pytest."""
    import inspect

    fns = [
        (name, fn)
        for name, fn in vars(module).items()
        if name.startswith("test_") and inspect.isfunction(fn)
    ]
    failures = []
    for name, fn in fns:
        try:
            fn()
            print(f"ok      {name}")
        except AssertionError as e:
            failures.append(name)
            print(f"FAILED  {name}: {e}")
    print(f"\n{len(fns) - len(failures)}/{len(fns)} passed")
    if failures:
        raise SystemExit(1)
