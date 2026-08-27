# Test fixture confirming positional calling works transparently with *args too, not just a
# fixed list of named parameters -- the whole point of calling f(*inputs) rather than doing any
# parameter-name matching. Sums every element across every positional argument, scalar or
# ndarray alike (np.atleast_1d normalizes both to an array before summing).
import numpy as np


def total(*args):
    return float(sum(np.sum(np.atleast_1d(a)) for a in args))
