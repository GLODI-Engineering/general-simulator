# Test fixture: a block whose own next execution interval doubles every time it runs (1, 2, 4,
# 8, ... seconds), counting how many times output() has actually been called -- the direct
# Python analog of cscript-ffi's own variable_sample_time.c.


def start():
    return {"calls": 0, "next_interval": 1.0}


def output(state, t, dt, inputs):
    state["calls"] += 1
    return float(state["calls"])


def next_sample_hit(state, t, dt, inputs):
    interval = state["next_interval"]
    state["next_interval"] *= 2.0
    return interval
