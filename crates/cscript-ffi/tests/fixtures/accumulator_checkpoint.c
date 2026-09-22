/* Test fixture for cscript-ffi: `accumulator.c` plus the optional checkpoint state contract
 * (cscript_state_size / cscript_state_write / cscript_state_read), so a running sum and count
 * can be written to a checkpoint and rebuilt from it -- possibly by another process. The state
 * is plain old data, so the serialization is a memcpy of the struct; a real block holding
 * pointers would have to serialize what they point at. */
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
    if (out_len >= 2) {
        out[1] = (double)s->count;
    }
}

void cscript_free(void *state) {
    free(state);
}

void *cscript_clone(void *state) {
    AccState *copy = (AccState *)malloc(sizeof(AccState));
    *copy = *(const AccState *)state;
    return copy;
}

/* The checkpoint state contract. All three, or none. */
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
