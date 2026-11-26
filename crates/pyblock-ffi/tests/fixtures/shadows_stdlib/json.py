"""A file named after a stdlib module. The block's directory is *appended* to `sys.path`
rather than prepended, which is what keeps this from shadowing the real `json`."""

NOT_THE_REAL_JSON = True
