/* Test fixture: a stateful counter (increments by 1 every actual cscript_output call,
 * ignoring its input entirely) for general-simulator-cli's kind=cscript sample-time regression
 * test -- the counter's own value directly reveals how many times it actually ran, so the
 * test can check the zero-order-hold sampling behavior without needing to inspect internal
 * dt/timing state. Exports cscript_clone so this fixture also covers the adaptive-step path. */
#include <stdlib.h>

void *cscript_start(void) {
    double *count = (double *)malloc(sizeof(double));
    *count = 0.0;
    return count;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)in;
    (void)in_len;
    (void)dt;
    double *count = (double *)state;
    *count += 1.0;
    if (out_len >= 1) {
        out[0] = *count;
    }
}

void cscript_free(void *state) {
    free(state);
}

void *cscript_clone(void *state) {
    double *count = (double *)state;
    double *copy = (double *)malloc(sizeof(double));
    *copy = *count;
    return copy;
}
