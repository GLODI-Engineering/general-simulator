# doc-verify fixture exercising PyBlock's xc_count>0 continuous-state contract: a first-order
# charge toward 1 (dxc/dt = 1 - xc), starting at rest (xc(0) = 0, the solver's own convention),
# so the analytic solution is xc(t) = 1 - exp(-t) -- a real, nonzero, independently-checkable
# trajectory, not a trivial always-zero one.
def start():
    return None


def derivative(state, t, inputs, xc):
    return [1.0 - xc[0]]


def output_xc(state, t, dt, inputs, xc):
    return xc[0]
