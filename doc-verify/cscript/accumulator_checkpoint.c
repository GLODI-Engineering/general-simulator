/* doc-verify fixture for the CScript escape-hatch chapter's checkpoint section: a running
 * accumulator (out = sum of every input seen so far) that opts in to checkpoint/resume by
 * exporting the cscript_state_size / cscript_state_write / cscript_state_read triple. The
 * state is plain old data, so each half of the contract is a memcpy of the struct. */
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

void cscript_free(void *state) {
    free(state);
}

/* The checkpoint state contract -- all three, or none. */
size_t cscript_state_size(const void *state) {
    return state ? sizeof(AccState) : 0;
}

void cscript_state_write(const void *state, uint8_t *out) {
    memcpy(out, state, sizeof(AccState));
}

void *cscript_state_read(const uint8_t *in, size_t len) {
    if (len != sizeof(AccState)) {
        return NULL;
    }
    AccState *s = (AccState *)malloc(sizeof(AccState));
    memcpy(s, in, sizeof(AccState));
    return s;
}
