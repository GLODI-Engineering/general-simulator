# Test fixture: a stateless block scaling its single scalar input by 2, for pyblock-ffi's own
# regression tests -- the direct Python analog of cscript-ffi's own stateless_gain.c.


def start():
    return None


def output(state, t, dt, inputs):
    return inputs[0] * 2.0
