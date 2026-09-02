//! Writes a SPICE **rawfile** (`.raw`) — the binary/ASCII waveform format `ngspice` (and the
//! broader SPICE-tooling ecosystem, including Python readers such as PySpice and `spicelib`)
//! reads and writes — as an alternative to this crate's own CSV output, for interoperability
//! with existing SPICE-adjacent tooling. This module writes the **binary** variant (the format's
//! own default and the one most third-party readers expect first): an ASCII header block
//! (`Title:`/`Date:`/`Plotname:`/`Flags:`/`No. Variables:`/`No. Points:`/`Variables:`, one
//! `\t<index>\t<name>\t<type>` line per variable) followed by a `Binary:` marker and then, for
//! each point in order, one little/native-endian `f64` per variable, contiguous (point-major,
//! variable-minor) — the layout every mainstream SPICE-family writer and reader (`ngspice`,
//! Xyce, and the third-party Python rawfile libraries this feature was cross-validated against;
//! see `docs/journal/` for the exact validation run) agrees on for a "normal access" (not
//! "FastAccess") real (non-complex) plot.
//!
//! ## Header field format, verified against the real spec
//!
//! Confirmed against the ngspice manual's own worked example (§12.13, "console like a raw
//! file") and by writing a file with this module and reading it back with two independent
//! Python rawfile readers (PySpice's `Spice.Xyce.RawFile.RawFile`, which reads a standalone
//! rawfile exactly like this one with no `Circuit:`/`Doing analysis` preamble, and
//! `spicelib.RawRead`, which additionally validates against a whitelist of known SPICE
//! dialects). Both require the file to start with `Title:` as its very first bytes (no leading
//! preamble line), and require `Variables:` to be followed by exactly `No. Variables` lines each
//! `\t<index>\t<name>\t<type>` (tab-separated, `<type>` conventionally `time`/`voltage`/
//! `current` — see "Variable-type compatibility caveat" below).
//!
//! ## Variable-type compatibility caveat
//!
//! Every variable in a real SPICE rawfile is either a circuit voltage, a circuit current, the
//! `time`/`frequency` axis, or (in some dialects) another physically-typed quantity — there is
//! no "generic signal" type in the format. This crate's own CSV output has no such restriction
//! (a `kind=pid`/`kind=phys2sig`/... block's named output is just another column), so a
//! signal-domain block output written here that isn't a `V(...)`/`I(...)` circuit unknown is
//! given the type `voltage` as the broadest-compatibility fallback: PySpice's own rawfile reader
//! hard-codes a 4-entry `time`/`voltage`/`current`/`frequency` lookup table and raises `KeyError`
//! on anything else, so `voltage` (rather than inventing a `signal`/`unknown` type a stricter
//! reader would reject) is the only choice that reads cleanly in every reader this was tested
//! against. This only affects the header's declared *type* string, purely descriptive metadata —
//! the actual numeric value written is identical to the same column's CSV value either way (see
//! `book/user-guide/src/reading-output.md`).

use std::io::{self, Write};

/// One column of the waveform: `name` is the exact CSV header text (`t`, `V(node)`, `I(branch)`,
/// or a block/vector-signal name), `values[i]` is that column's value at point `i`.
pub struct Waveform<'a> {
    pub names: &'a [String],
    /// `rows[point][column]` — same shape as the CSV body, one row per resolved timestep (or a
    /// single row for `--mode dc`).
    pub rows: &'a [Vec<f64>],
    /// `Transient Analysis` / `DC transfer characteristic` — SPICE's own `Plotname:` values,
    /// mirrored here so a reader that branches on plot type (as `spicelib`/PySpice both do)
    /// resolves the same analysis kind this run actually performed.
    pub plot_name: &'static str,
    pub title: &'a str,
}

/// The SPICE rawfile `Variables:` type string for a given CSV column name — see this module's
/// own "Variable-type compatibility caveat" doc section above for why every non-`V`/`I` column
/// (including the leading time axis, handled separately by the caller) falls back to `voltage`.
fn variable_type(name: &str) -> &'static str {
    if name.starts_with("V(") {
        "voltage"
    } else if name.starts_with("I(") {
        "current"
    } else {
        "voltage"
    }
}

/// Writes `waveform` as a binary SPICE rawfile to `w`. `waveform.names[0]` is expected to be the
/// time/sweep column (`"t"` in this crate's own CSV header) and is renamed to SPICE's own
/// conventional `time` variable name in the header — see this module's doc comment for "same
/// data, different serialization" and the exact byte layout.
pub fn write_raw<W: Write>(w: &mut W, waveform: &Waveform) -> io::Result<()> {
    let n_vars = waveform.names.len();
    let n_points = waveform.rows.len();

    writeln!(w, "Title: {}", waveform.title)?;
    writeln!(w, "Date: {}", now_unix_seconds())?;
    writeln!(w, "Plotname: {}", waveform.plot_name)?;
    writeln!(w, "Flags: real")?;
    writeln!(w, "No. Variables: {n_vars}")?;
    writeln!(w, "No. Points: {n_points}")?;
    writeln!(w, "Variables:")?;
    for (i, name) in waveform.names.iter().enumerate() {
        if i == 0 {
            writeln!(w, "\t{i}\ttime\ttime")?;
        } else {
            writeln!(w, "\t{i}\t{name}\t{}", variable_type(name))?;
        }
    }
    writeln!(w, "Binary:")?;

    for row in waveform.rows {
        debug_assert_eq!(
            row.len(),
            n_vars,
            "row width must match the declared variable count"
        );
        for value in row {
            w.write_all(&value.to_le_bytes())?;
        }
    }
    Ok(())
}

/// `Date:` is documentation-only metadata to every reader this was validated against (neither
/// PySpice's nor `spicelib`'s parser does anything with its *value* beyond confirming the field
/// is present) — a plain Unix timestamp avoids pulling in a date-formatting dependency for a
/// field nothing downstream actually parses.
fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip proof that this writer is at least self-consistent: parse the header fields
    /// (and the raw binary tail) back out of a file this module wrote and assert they match the
    /// `Waveform` that produced it. This is not a substitute for the Python cross-validation
    /// against real readers (see `doc-verify/raw-output/`) — just the ordinary Rust-side check
    /// that this crate's own writer does what it says.
    #[test]
    fn round_trip_header_and_binary_body() {
        let names = vec!["t".to_string(), "V(a)".to_string(), "I(V1)".to_string()];
        let rows = vec![
            vec![0.0, 1.0, -0.5],
            vec![0.1, 1.1, -0.4],
            vec![0.2, 1.2, -0.3],
        ];
        let waveform = Waveform {
            names: &names,
            rows: &rows,
            plot_name: "Transient Analysis",
            title: "round_trip_test",
        };

        let mut buf = Vec::new();
        write_raw(&mut buf, &waveform).unwrap();

        // Find the `Binary:\n` marker as raw bytes first (the tail after it is arbitrary binary
        // data, not necessarily valid UTF-8, so it must not go through a lossy string
        // conversion before this split -- that could change the byte length and desync the
        // header/data boundary computed below).
        let binary_marker = b"Binary:\n";
        let binary_pos = buf
            .windows(binary_marker.len())
            .position(|w| w == binary_marker)
            .expect("Binary: marker present");
        let data_start = binary_pos + binary_marker.len();
        let header = std::str::from_utf8(&buf[..binary_pos]).expect("header is valid UTF-8");

        assert!(header.starts_with("Title: round_trip_test\n"));
        assert!(header.contains("Plotname: Transient Analysis\n"));
        assert!(header.contains("Flags: real\n"));
        assert!(header.contains("No. Variables: 3\n"));
        assert!(header.contains("No. Points: 3\n"));
        assert!(header.contains("Variables:\n"));
        assert!(header.contains("\t0\ttime\ttime\n"));
        assert!(header.contains("\t1\tV(a)\tvoltage\n"));
        assert!(header.contains("\t2\tI(V1)\tcurrent\n"));

        let data = &buf[data_start..];
        assert_eq!(data.len(), rows.len() * names.len() * 8);

        let mut offset = 0;
        for row in &rows {
            for expected in row {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&data[offset..offset + 8]);
                let actual = f64::from_le_bytes(bytes);
                assert_eq!(actual, *expected);
                offset += 8;
            }
        }
    }

    #[test]
    fn variable_type_falls_back_to_voltage_for_non_circuit_columns() {
        assert_eq!(variable_type("V(out)"), "voltage");
        assert_eq!(variable_type("I(V1)"), "current");
        assert_eq!(variable_type("PID1"), "voltage");
        assert_eq!(variable_type("CLARKE_beta"), "voltage");
    }
}
