# Test fixture: output() is deliberately read-only (returns the current accumulated sum, no
# mutation at all), and only update() actually advances the accumulator -- the direct Python
# analog of cscript-ffi's own update_counter.c. Confirms calling output() any number of times
# without a matching update() call never changes the result, while update() is the sole place
# state commits forward.


def start():
    return {"sum": 0.0}


def output(state, t, dt, inputs):
    return state["sum"]


def update(state, t, dt, inputs):
    state["sum"] += inputs[0]
