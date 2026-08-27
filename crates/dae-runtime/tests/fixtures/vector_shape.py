# Test fixture confirming pyblock's own inputs= list keeps each declared signal's own shape --
# unlike cscript's flat C array, a vector signal here stays a genuine numpy.ndarray, not
# concatenated together with the scalar signal that follows it.
import numpy as np


def start():
    return None


def output(state, t, dt, inputs):
    vec, scalar = inputs
    assert isinstance(vec, np.ndarray), f"expected ndarray for inputs[0], got {type(vec)}"
    assert isinstance(scalar, float), f"expected float for inputs[1], got {type(scalar)}"
    return float(np.sum(vec)) + scalar
