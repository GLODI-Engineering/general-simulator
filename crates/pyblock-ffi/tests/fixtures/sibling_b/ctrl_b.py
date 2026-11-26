"""A block in a second directory, importing its own sibling."""

import helper_b


def start():
    return {}


def output(state, t, dt, inputs):
    return helper_b.K * inputs[0]
