"""A block whose own directory holds a `json.py`; `import json` must still find the stdlib."""

import json


def start():
    return {}


def output(state, t, dt, inputs):
    # `dumps` exists only on the real stdlib module.
    assert json.dumps([1]) == "[1]"
    return 3.0 * inputs[0]
