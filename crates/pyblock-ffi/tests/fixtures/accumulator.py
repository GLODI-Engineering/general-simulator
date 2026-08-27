# Test fixture: a stateful running accumulator with two outputs (sum, count), the direct Python
# analog of cscript-ffi's own accumulator.c -- exercises the full start/output lifecycle, the
# multi-output return convention, and copy.deepcopy-based cloning (an ordinary dict, no special
# handling needed).


def start():
    return {"sum": 0.0, "count": 0}


def output(state, t, dt, inputs):
    state["sum"] += inputs[0]
    state["count"] += 1
    return (state["sum"], float(state["count"]))
