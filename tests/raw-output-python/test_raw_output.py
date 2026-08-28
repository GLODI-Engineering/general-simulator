"""Cross-validation of `general-simulator --format raw`'s output against two independent
Python SPICE-rawfile readers -- not just this crate's own writer agreeing with itself (see
crates/general-simulator-cli/src/raw_format.rs's own round-trip unit test for that), but real,
independently-implemented parsers from the SPICE tooling ecosystem confirming the file is
actually structured the way the format's own spec says.

Run with:
    python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
    cargo build --release -p general-simulator-cli   # from the repo root
    .venv/bin/python -m pytest tests/raw-output-python -v
    # or, without pytest:
    .venv/bin/python tests/raw-output-python/test_raw_output.py

See README.md for the exact venv setup and what each reader confirms.
"""
import pathlib
import shutil
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from _lib import run_all, run_csv_and_raw  # noqa: E402


def _assert_headers_match_variable_names(csv_headers, variable_names):
    # `t` (this crate's own CSV header for the time/sweep column) is written as SPICE's own
    # conventional `time` variable name in the raw file -- everything else (V(node), I(branch),
    # a block's own name) is unchanged. See raw_format.rs's module doc comment.
    expected = ["time"] + csv_headers[1:]
    assert list(variable_names) == expected, (
        f"raw file variable names {list(variable_names)} != expected {expected}"
    )


def test_pyspice_reads_transient_raw_matches_csv():
    from PySpice.Spice.Xyce.RawFile import RawFile

    headers, rows, raw_path = run_csv_and_raw(
        "rc_diode.cir",
        "rc_diode_devices.txt",
        ["--mode", "transient", "--tfinal", "2.0", "--dt", "0.001"],
    )
    try:
        raw_file = RawFile(raw_path.read_bytes())
        assert raw_file.plot_name == "Transient Analysis"
        assert raw_file.flags == "real"
        assert raw_file.number_of_variables == len(headers)
        assert raw_file.number_of_points == len(rows)
        _assert_headers_match_variable_names(headers, raw_file.variables.keys())

        for col_index, name in enumerate(["time"] + headers[1:]):
            parsed = raw_file.variables[name].data
            expected = [row[col_index] for row in rows]
            assert len(parsed) == len(expected)
            for a, b in zip(parsed, expected):
                assert a == b, f"{name}: PySpice read {a}, CSV had {b}"
    finally:
        shutil.rmtree(raw_path.parent, ignore_errors=True)


def test_spicelib_reads_transient_raw_matches_csv():
    from spicelib import RawRead

    headers, rows, raw_path = run_csv_and_raw(
        "rc_diode.cir",
        "rc_diode_devices.txt",
        ["--mode", "transient", "--tfinal", "2.0", "--dt", "0.001"],
    )
    try:
        # spicelib validates the declared dialect against a fixed whitelist
        # ('ngspice'/'xyce'/'qspice', and one more well-known SPICE-family variant) and can't
        # auto-detect one for a `Command:` value
        # this writer doesn't emit -- 'ngspice' is the closest real match (double-precision
        # values throughout a non-AC plot, the same convention this writer follows).
        raw = RawRead(str(raw_path), dialect="ngspice")
        assert raw.get_trace_names() == ["time"] + headers[1:]

        for col_index, name in enumerate(["time"] + headers[1:]):
            parsed = raw.get_wave(name)
            expected = [row[col_index] for row in rows]
            assert len(parsed) == len(expected)
            for a, b in zip(parsed, expected):
                assert a == b, f"{name}: spicelib read {a}, CSV had {b}"
    finally:
        shutil.rmtree(raw_path.parent, ignore_errors=True)


def test_spicelib_reads_dc_raw_matches_csv():
    from spicelib import RawRead

    headers, rows, raw_path = run_csv_and_raw(
        "two_diode.cir", "two_diode_devices.txt", ["--mode", "dc"]
    )
    try:
        raw = RawRead(str(raw_path), dialect="ngspice")
        assert raw.get_trace_names() == ["time"] + headers[1:]
        assert len(rows) == 1
        for col_index, name in enumerate(["time"] + headers[1:]):
            parsed = raw.get_wave(name)
            assert len(parsed) == 1
            assert parsed[0] == rows[0][col_index], f"{name}: spicelib read {parsed[0]}, CSV had {rows[0][col_index]}"
    finally:
        shutil.rmtree(raw_path.parent, ignore_errors=True)


if __name__ == "__main__":
    run_all(sys.modules[__name__])
