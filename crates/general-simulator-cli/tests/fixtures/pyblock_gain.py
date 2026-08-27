# Test fixture: a stateless block scaling its single input by 2, for general-simulator-cli's
# own kind=pyblock regression test -- the Python-side direct analog of cscript_gain.c.


def start():
    return None


def output(state, t, dt, inputs):
    return inputs[0] * 2.0
