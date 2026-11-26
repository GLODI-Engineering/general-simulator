"""A block that imports the module beside it -- the ordinary way anyone factors two blocks."""

import helper


def start():
    return {}


def output(state, t, dt, inputs):
    return helper.K * inputs[0]
