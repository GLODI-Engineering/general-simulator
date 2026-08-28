/* doc-verify fixture for the CScript component reference entry: scales its single input by 2. */
void *cscript_start(void) {
    return (void *)0;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)state;
    (void)dt;
    if (in_len >= 1 && out_len >= 1) {
        out[0] = in[0] * 2.0;
    }
}
