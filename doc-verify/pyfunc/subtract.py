# Proves each declared `inputs=` entry arrives as its own positional argument (f(*inputs)),
# never bundled into one list -- a and b are genuinely distinct arguments here, not indices
# into a single collection.
def subtract(a, b):
    return a - b
