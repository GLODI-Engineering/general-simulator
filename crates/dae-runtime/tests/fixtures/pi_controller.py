# Test fixture for dae-runtime's own kind=pyblock integration test: a hand-written discrete PI
# controller (the exact kind of thing a a reference tool Function block is for), regulating error to zero
# with anti-windup, checked against a plain Rust-side hand computation of the same recursion.
KP = 2.0
KI = 50.0


def start():
    return {"integral": 0.0}


def output(state, t, dt, inputs):
    error = inputs[0]
    state["integral"] += error * dt
    return KP * error + KI * state["integral"]
