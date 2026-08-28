"""Unit tests for the Saturation component-reference entry -- see README.md for what each
proves. One function per documented behavior (an Example, or an Errors claim), matching the
doc comment on BlockKind::Saturation in general-mna/src/block_graph.rs 1:1.

Run standalone:  python3 doc-verify/saturation/test_saturation.py
Run via pytest:  python3 -m pytest doc-verify/saturation
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import _lib  # noqa: E402


def _row_at(rows, t):
    return next(r for r in rows if abs(r["t"] - t) < 1e-12)


def test_example_clamps_input_to_limit():
    """## Example: time (a ramp) through limit=1 is passed through while |x| <= 1 and holds
    exactly 1 once x exceeds it."""
    _, stdout, _ = _lib.run_transient(HERE / "example.cir", tfinal=2, dt=0.25)
    rows = _lib.parse_csv(stdout)
    for t in (0.25, 0.5, 0.75, 1.0):
        assert _row_at(rows, t)["LIM"] == t, f"unclamped region: LIM should equal x={t}"
    for t in (1.25, 1.5, 1.75, 2.0):
        assert _row_at(rows, t)["LIM"] == 1.0, f"clamped region: LIM should hold 1.0 at x={t}"


def test_negative_limit_panics():
    """## Errors: limit=-1 is not a clean error -- the evaluation code's own assert fires and
    the process panics with "limit must be nonnegative"."""
    code, _, stderr = _lib.run_transient(
        HERE / "error_negative_limit.cir", tfinal=1e-4, dt=1e-5, expect_success=False
    )
    assert code != 0
    assert "limit must be nonnegative" in stderr


if __name__ == "__main__":
    _lib.run_all(sys.modules[__name__])
