# Test fixture: sums the elements of a numpy-array input -- confirms a PyInput::Vector arrives
# as a genuine numpy.ndarray usable with numpy's own vectorized operations, not a plain list.
import numpy as np


def start():
    return None


def output(state, t, dt, inputs):
    assert isinstance(inputs[0], np.ndarray), f"expected ndarray, got {type(inputs[0])}"
    return float(np.sum(inputs[0]))
