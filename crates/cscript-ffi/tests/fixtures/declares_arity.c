/* Test fixture for cscript-ffi: a block that declares its own arity.
 *
 * Two inputs, one output, and it says so. Without cscript_arity a netlist that
 * wires three inputs to this block would hand `in` a three-element buffer and
 * `in_len` of 3, and the mismatch would only show up as a wrong answer -- or,
 * the other way round, as a read past the end of a shorter buffer. */

void *cscript_start(void) {
    return (void *)0;
}

/* Writes the counts this block is built for and returns 0 to mean "I am declaring".
 * A library that would rather not declare simply does not export this symbol, or
 * returns non-zero. */
int cscript_arity(int *in_len, int *out_len) {
    *in_len = 2;
    *out_len = 1;
    return 0;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)state;
    (void)dt;
    if (in_len >= 2 && out_len >= 1) {
        out[0] = in[0] + in[1];
    }
}
