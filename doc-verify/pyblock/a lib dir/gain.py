# doc-verify fixture for the PyBlock component reference entry: doubles its single input.
def start():
    return None


def output(state, t, dt, inputs):
    return inputs[0] * 2.0
