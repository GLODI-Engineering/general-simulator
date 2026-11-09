/* Test fixture for cscript-ffi: exports cscript_arity but declines to answer.
 *
 * Returning non-zero means "no opinion" and must be treated exactly like not
 * exporting the symbol at all -- declining is not an error. */

void *cscript_start(void) {
    return (void *)0;
}

int cscript_arity(int *in_len, int *out_len) {
    (void)in_len;
    (void)out_len;
    return 1;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)state;
    (void)dt;
    if (in_len >= 1 && out_len >= 1) {
        out[0] = in[0];
    }
}
