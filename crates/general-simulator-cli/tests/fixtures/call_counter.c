/* Test fixture for the sample-time-offset CLI test: a plain call counter, ignoring inputs --
 * used purely to observe exactly which resolved steps this block's output() actually ran on. */
#include <stdlib.h>

void *cscript_start(void) {
    long *count = (long *)malloc(sizeof(long));
    *count = 0;
    return count;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)in;
    (void)in_len;
    (void)dt;
    long *count = (long *)state;
    *count += 1;
    if (out_len >= 1) {
        out[0] = (double)*count;
    }
}

void cscript_free(void *state) {
    free(state);
}
