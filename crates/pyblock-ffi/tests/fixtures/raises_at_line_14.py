"""A block that raises inside `output`, on a line this file's name pins.

Line 1 is the opening of this docstring. The `undefined_name` reference below is on line 14,
and `tests/fixtures.rs` asserts that the reported line is 14 — so inserting a line above it
breaks the test, which is the point.
"""


def start():
    return {}


def output(state, t, dt, inputs):
    return 2.0 * inputs[0] + undefined_name  # noqa: F821  <- this is line 14
