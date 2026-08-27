# Test fixture for PyFunctionRegistry -- a plain, stateless function with named parameters and
# a multi-value return, the direct Python port of a real named-input/named-output function block
# example the user gave (an SVPWM 180-degree gate-action-qualifier lookup): no start(), no
# state, no t/dt -- just the computation.


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
