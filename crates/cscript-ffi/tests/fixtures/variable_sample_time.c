/* Test fixture for cscript-ffi's optional cscript_next_sample_hit function: a block whose own
 * next execution interval doubles every time it runs (1, 2, 4, 8, ... "seconds"), counting how
 * many times output() has actually been called. Confirms the block-controlled schedule is
 * driven purely by this function's own return value, not by anything cscript-ffi computes on
 * its own. */
#include <stdlib.h>

typedef struct {
    long calls;
    double next_interval;
} VarState;

void *cscript_start(void) {
    VarState *s = (VarState *)malloc(sizeof(VarState));
    s->calls = 0;
    s->next_interval = 1.0;
    return s;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)in;
    (void)in_len;
    (void)dt;
    VarState *s = (VarState *)state;
    s->calls += 1;
    if (out_len >= 1) {
        out[0] = (double)s->calls;
    }
}

double cscript_next_sample_hit(void *state, const double *in, int in_len, const double *xc, int xc_len) {
    (void)in;
    (void)in_len;
    (void)xc;
    (void)xc_len;
    VarState *s = (VarState *)state;
    double interval = s->next_interval;
    s->next_interval *= 2.0;
    return interval;
}

void cscript_free(void *state) {
    free(state);
}
