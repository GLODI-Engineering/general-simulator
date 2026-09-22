/* Test fixture for cscript-ffi: `accumulator.c` with only TWO of the three checkpoint state
 * symbols (cscript_state_size and cscript_state_write, no cscript_state_read) -- a library
 * half-way through adopting the contract. Loading it must fail up front, naming the missing
 * symbol, rather than looking like a library that never opted in at all. */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    double sum;
    long count;
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
}

void cscript_free(void *state) {
    free(state);
}

size_t cscript_state_size(const void *state) {
    return state ? sizeof(AccState) : 0;
}

void cscript_state_write(const void *state, uint8_t *out) {
    memcpy(out, state, sizeof(AccState));
}

/* cscript_state_read deliberately absent. */
