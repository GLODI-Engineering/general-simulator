# Test fixture: output() deliberately raises, to confirm a Python exception surfaces as a clear
# PyBlockError::Exception rather than a panic/abort.


def start():
    return None


def output(state, t, dt, inputs):
    raise ValueError("deliberate failure for pyblock-ffi's own error-handling test")
