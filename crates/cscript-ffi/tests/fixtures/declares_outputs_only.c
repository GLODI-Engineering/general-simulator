/* Test fixture for cscript-ffi: declares one side and leaves the other open.
 *
 * A negative value means "not declaring this one", so a block that is genuinely
 * variadic on its inputs can still pin its output count. */

void *cscript_start(void) {
    return (void *)0;
}

int cscript_arity(int *in_len, int *out_len) {
    *in_len = -1; /* any number of inputs */
    *out_len = 1;
    return 0;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)state;
    (void)dt;
    double sum = 0.0;
    for (int i = 0; i < in_len; i++) {
        sum += in[i];
    }
    if (out_len >= 1) {
        out[0] = sum;
    }
}
