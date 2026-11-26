"""A block whose own sibling `helper` collides with another block's sibling `helper`."""

import helper


def start():
    return {}


def output(state, t, dt, inputs):
    return helper.K * inputs[0]
