# Test fixture for general-simulator-cli's own kind=pyfunc end-to-end test.


def compute_action_qualifier_180_degree(phase_degree):
    phase_shift = phase_degree % 360

    if phase_shift == 0:
        AQCTLA = 9
        AQCTLB = 6
    elif 0 < phase_shift < 180:
        AQCTLA = 2066
        AQCTLB = 1057
    elif phase_shift == 180:
        AQCTLA = 6
        AQCTLB = 9
    elif 180 < phase_shift < 360:
        AQCTLA = 1057
        AQCTLB = 2066
    else:
        AQCTLA = 9
        AQCTLB = 6

    return AQCTLA, AQCTLB
