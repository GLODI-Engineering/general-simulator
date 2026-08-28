/* Test fixture for cscript-ffi's optional cscript_update function: output() is deliberately
 * read-only (returns the current accumulated sum, no mutation at all), and only cscript_update
 * actually advances the accumulator -- confirms calling output() any number of times without a
 * matching update() call never changes the result, while update() is the sole place state
 * commits forward. */
#include <stdlib.h>

typedef struct {
    double sum;
} CounterState;

void *cscript_start(void) {
    CounterState *s = (CounterState *)malloc(sizeof(CounterState));
    s->sum = 0.0;
    return s;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)in;
    (void)in_len;
    (void)dt;
    CounterState *s = (CounterState *)state;
    if (out_len >= 1) {
        out[0] = s->sum;
    }
}

void cscript_update(void *state, const double *in, int in_len, double dt, const double *xc, int xc_len) {
    (void)dt;
    (void)xc;
    (void)xc_len;
    CounterState *s = (CounterState *)state;
    if (in_len >= 1) {
        s->sum += in[0];
    }
}

void cscript_free(void *state) {
    free(state);
}
