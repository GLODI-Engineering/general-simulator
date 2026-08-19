/* Test fixture for cscript-ffi: a stateful block (a running accumulator with two outputs, sum
 * and count) to exercise the full Declarations/StartFcn/OutputFcn/free lifecycle and confirm
 * two instances of the same library keep independent state. */
#include <stdlib.h>

typedef struct {
    double sum;
    long count;
    int freed_flag_target_or_zero; /* nonzero: write a sentinel into *freed_flag on free() */
} AccState;

void *cscript_start(void) {
    AccState *s = (AccState *)malloc(sizeof(AccState));
    s->sum = 0.0;
    s->count = 0;
    return s;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)dt;
    AccState *s = (AccState *)state;
    if (in_len >= 1) {
        s->sum += in[0];
    }
    s->count += 1;
    if (out_len >= 1) {
        out[0] = s->sum;
    }
    if (out_len >= 2) {
        out[1] = (double)s->count;
    }
}

void cscript_free(void *state) {
    free(state);
}

void *cscript_clone(void *state) {
    AccState *s = (AccState *)state;
    AccState *copy = (AccState *)malloc(sizeof(AccState));
    *copy = *s;
    return copy;
}
