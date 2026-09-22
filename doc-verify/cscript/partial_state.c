/* doc-verify fixture: the accumulator with only cscript_state_size and cscript_state_write --
 * a library half-way through adopting the checkpoint state contract. Must be rejected at load
 * time, naming the missing cscript_state_read, even when no checkpoint is asked for. */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    double sum;
} AccState;

void *cscript_start(void) {
    AccState *s = (AccState *)malloc(sizeof(AccState));
    s->sum = 0.0;
    return s;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)dt;
    AccState *s = (AccState *)state;
    if (in_len >= 1) {
        s->sum += in[0];
    }
    if (out_len >= 1) {
        out[0] = s->sum;
    }
}

size_t cscript_state_size(const void *state) {
    return state ? sizeof(AccState) : 0;
}

void cscript_state_write(const void *state, uint8_t *out) {
    memcpy(out, state, sizeof(AccState));
}
