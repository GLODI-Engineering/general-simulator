"""Shared harness for component-reference doc-verify tests.

Every component's test_<kind>.py imports this module. It runs the actual built
general-simulator CLI as a subprocess against a real netlist -- these are integration tests
against the real binary, not calls into any Rust code directly, so they prove exactly what a
user running the CLI would see.

Build the CLI first (from the general-simulator repo root):
    cargo build --release -p general-simulator-cli
    # add --features python for any kind=pyblock/kind=pyfunc test
"""
import csv
import io
import pathlib
import subprocess

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
_CLI_RELEASE = REPO_ROOT / "target" / "release" / "general-simulator"
_CLI_DEBUG = REPO_ROOT / "target" / "debug" / "general-simulator"


def cli_path():
    if _CLI_RELEASE.exists():
        return _CLI_RELEASE
    if _CLI_DEBUG.exists():
        return _CLI_DEBUG
    raise FileNotFoundError(
        "general-simulator binary not built -- run `cargo build --release "
        "-p general-simulator-cli` (add `--features python` for kind=pyblock/kind=pyfunc "
        "tests) from the general-simulator repo root first."
    )


def run(cir_path, *args, expect_success=True):
    """Runs the CLI against cir_path (a path relative to the repo root, or absolute) with the
    given extra CLI args. Returns (returncode, stdout, stderr). Asserts a zero exit code unless
    expect_success=False -- pass that for a test that is specifically verifying an Errors claim."""
    cir_path = pathlib.Path(cir_path)
    if not cir_path.is_absolute():
        cir_path = REPO_ROOT / cir_path
    result = subprocess.run(
        [str(cli_path()), str(cir_path), *args],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,  # so a netlist's own relative lib=/path= fields resolve consistently
        # regardless of the caller's own working directory.
    )
    if expect_success:
        assert result.returncode == 0, (
            f"expected success running {cir_path.name}, got exit {result.returncode}\n"
            f"stderr:\n{result.stderr}"
        )
    return result.returncode, result.stdout, result.stderr


def run_transient(cir_path, tfinal, dt=None, expect_success=True):
    """--mode transient, with or without a fixed --dt (omit dt for adaptive stepping)."""
    args = ["--mode", "transient", "--tfinal", str(tfinal)]
    if dt is not None:
        args += ["--dt", str(dt)]
    return run(cir_path, *args, expect_success=expect_success)


def run_dc(cir_path, expect_success=True):
    return run(cir_path, "--mode", "dc", expect_success=expect_success)


def parse_csv(stdout):
    """Parses the CLI's CSV stdout into a list of dicts, one per row, values coerced to float
    where possible (a block's own extra text output, if any, is left as a string)."""
    reader = csv.DictReader(io.StringIO(stdout))
    rows = []
    for row in reader:
        parsed = {}
        for k, v in row.items():
            try:
                parsed[k] = float(v)
            except (TypeError, ValueError):
                parsed[k] = v
        rows.append(parsed)
    return rows


def run_all(module):
    """Runs every module-level function named test_* in declaration order and reports pass/fail
    -- lets a component's test_<kind>.py double as a standalone script
    (`python3 doc-verify/<kind>/test_<kind>.py`) with no pytest dependency required, while still
    being discoverable by `python3 -m pytest doc-verify` if pytest is available."""
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
