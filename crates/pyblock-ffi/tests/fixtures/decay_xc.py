# Test fixture exercising the xc contract: dx/dt = -K*x + u (K=2.0), the direct Python analog of
# cscript-ffi's own decay_xc.c fixture -- same system, same closed-form check
# (x(t) = (u/K)*(1-exp(-K*t)) for a constant step input), used to confirm the Rust-side RK4
# stepper produces identical results whether it's calling into C or Python.

K = 2.0


def start():
    return {"calls": 0}


def derivative(state, t, inputs, xc):
    u = inputs[0] if len(inputs) >= 1 else 0.0
    return [-K * xc[0] + u]


def output_xc(state, t, dt, inputs, xc):
    state["calls"] += 1
    return (xc[0], float(state["calls"]))
