/* Test fixture: a stateless C block scaling its single input by 2, for
 * elspice-pwl-cli's kind=cscript regression test. No cscript_free/cscript_clone needed --
 * confirms both stay genuinely optional at the CLI level too, not just inside cscript-ffi's
 * own unit tests. */

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
