# Test fixture for dae-runtime's own kind=pyblock xc integration test: dx/dt = -K*x + u
# (K=2.0), the same system pyblock-ffi's own decay_xc.py fixture and general-simulator's
# vector-signals-statespace-vs-cscript experiment's mimo_cscript.c both use, checked here
# against the identical closed-form step response through the real block-graph loop.
K = 2.0


def start():
    return None


def derivative(state, t, inputs, xc):
    u = inputs[0]
    return [-K * xc[0] + u]


def output_xc(state, t, dt, inputs, xc):
    return xc[0]
