/* Test fixture for cscript-ffi: a genuinely continuous-state block exercising the xc contract
 * (cscript_derivative/cscript_output_xc instead of plain cscript_output). Models a first-order
 * decay dx/dt = -k*x + in[0] (k=2.0), so its own closed-form solution under a zero input and
 * initial condition x0 is x(t) = x0*exp(-k*t) -- a clean, hand-derivable check on the RK4
 * stepper's own correctness, independent of anything cscript-ffi does for a discrete-state
 * block. Also mutates a plain counter in `state` (the xd half) on every cscript_output_xc call,
 * to confirm the two halves (solver-integrated xc, user-managed state) coexist correctly. */
#include <stdlib.h>

#define K (2.0)

void *cscript_start(void) {
    long *calls = (long *)malloc(sizeof(long));
    *calls = 0;
    return calls;
}

void cscript_derivative(void *state, const double *in, int in_len,
                         const double *xc, int xc_len, double *xc_dot, int xc_dot_len) {
    (void)state;
    if (xc_len >= 1 && xc_dot_len >= 1) {
        double u = (in_len >= 1) ? in[0] : 0.0;
        xc_dot[0] = -K * xc[0] + u;
    }
}

void cscript_output_xc(void *state, const double *in, int in_len, double dt,
                        const double *xc, int xc_len, double *out, int out_len) {
    (void)in;
    (void)in_len;
    (void)dt;
    long *calls = (long *)state;
    *calls += 1;
    if (out_len >= 1 && xc_len >= 1) {
        out[0] = xc[0];
    }
    if (out_len >= 2) {
        out[1] = (double)*calls;
    }
}

void cscript_free(void *state) {
    free(state);
}

void *cscript_clone(void *state) {
    long *calls = (long *)state;
    long *copy = (long *)malloc(sizeof(long));
    *copy = *calls;
    return copy;
}
