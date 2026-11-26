/* gsim_cscript.h -- the ABI a `kind=cscript` block implements.
 *
 * Include this instead of retyping the prototypes. Every block in every project this header
 * was written for had retyped them by hand, because there was no header to include, and a
 * hand-typed prototype is one of the few mistakes in this contract that a compiler cannot
 * catch and a Linux test run cannot see:
 *
 *   On x86-64 System V, integer/pointer arguments and floating-point arguments are assigned
 *   from *separate* register files. A `cscript_output` declared as
 *   `(state, t, dt, in, in_len, out, out_len)` -- a plausible, wrong guess -- therefore puts
 *   `state`, `in`, `in_len` and `out` in the same registers as the real
 *   `(state, in, in_len, dt, out, out_len)`, and `dt` in the same float register. It produces
 *   numbers correct to the last digit, with no diagnostic from `-Wall -Wextra`.
 *
 *   Windows x64 assigns the first four arguments to RCX, RDX, R8, R9 *positionally regardless
 *   of class*. The same declaration there reads the caller's `in` pointer as `dt` and builds
 *   its own `in` pointer out of the integer `in_len`, then dereferences it in a loop. A crash,
 *   or garbage.
 *
 * So the prototypes have to be right, and the only way to be sure of that is to take them from
 * the engine rather than from memory. This header is that copy, and
 * `crates/cscript-ffi/tests/published_header.rs` compiles a block against it and loads it
 * through the real loader, so a drift between the two is a failing test rather than a wrong
 * waveform.
 *
 * Usage:
 *
 *     #include "gsim_cscript.h"
 *
 *     int cscript_arity(int *in_len, int *out_len) { *in_len = 1; *out_len = 1; return 0; }
 *     void *cscript_start(void) { return NULL; }
 *     void cscript_output(void *state, const double *in, int in_len, double dt,
 *                         double *out, int out_len) {
 *         (void)state; (void)in_len; (void)dt; (void)out_len;
 *         out[0] = 2.0 * in[0];
 *     }
 *
 *     cc -shared -fPIC -O2 -I <this directory> -o libctrl.so ctrl.c shared/law.c -lm
 *
 * Declaring a function here and not defining it is fine: every symbol below except
 * `cscript_start` is optional, and what each one's absence means is in `cscript-ffi`'s own
 * module documentation. The header declares the whole contract so that whichever subset a
 * block implements, it implements with the right signature.
 *
 * There is no `t` anywhere in this ABI. A `cscript` block never receives absolute time; a
 * block that needs it takes it as an input signal.
 *
 * In C++ this header's declarations are already wrapped in `extern "C"`, which is what keeps
 * the exported names unmangled. Nothing else is needed.
 *
 * SPDX-License-Identifier: the license of the general-simulator repository (see its LICENSE).
 */

#ifndef GSIM_CSCRIPT_H
#define GSIM_CSCRIPT_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------------------------------------
 * Required.
 * ------------------------------------------------------------------------------------------ */

/* Called once per block instance, when the instance is created. Return an opaque pointer to
 * this instance's own state (typically malloc'd), or NULL if the block carries no state.
 *
 * Two instances naming the same library share one loaded image but each get their own call to
 * this and their own returned pointer, so per-instance state is never accidentally shared. */
void *cscript_start(void);

/* ---------------------------------------------------------------------------------------------
 * One of these two contracts, depending on whether the block owns continuous state.
 * ------------------------------------------------------------------------------------------ */

/* The plain contract: called once per resolved transient step.
 *
 * `state` is whatever this instance's `cscript_start` returned. `in`/`in_len` is this step's
 * input vector, in the order the netlist wires it. `dt` is the step just taken. Write exactly
 * `out_len` values into `out`.
 *
 * Read-only in spirit when `cscript_update` is also exported: see that function. */
void cscript_output(void *state, const double *in, int in_len, double dt, double *out,
                    int out_len);

/* The `xc` contract, for a block declaring `xc_count > 0`: the engine owns and integrates the
 * continuous state and this block says only what its derivative is.
 *
 * Write exactly `xc_dot_len` values into `xc_dot`. Never mutate `xc`. */
void cscript_derivative(void *state, const double *in, int in_len, const double *xc, int xc_len,
                        double *xc_dot, int xc_dot_len);

/* The `xc` contract's output function, which additionally sees the engine-owned continuous
 * state. Write exactly `out_len` values into `out`. */
void cscript_output_xc(void *state, const double *in, int in_len, double dt, const double *xc,
                       int xc_len, double *out, int out_len);

/* ---------------------------------------------------------------------------------------------
 * Optional.
 * ------------------------------------------------------------------------------------------ */

/* Declare how many inputs and outputs this block was built for, and return 0; return non-zero
 * to decline (which is treated exactly as if the symbol were absent). Write a negative value
 * for a side this block is genuinely variadic on.
 *
 * Worth exporting from every block. It is the one thing the engine checks against the netlist
 * before the first step, and without it a disagreement is an out-of-bounds C array access --
 * a silent wrong answer rather than an error. */
int cscript_arity(int *in_len, int *out_len);

/* Advance this block's own discrete state, once per step, after `cscript_output`/
 * `cscript_output_xc` has been called. Exporting this is what lets the output function be a
 * pure read of the state: a solver may call the output function more than once per step. */
void cscript_update(void *state, const double *in, int in_len, double dt, const double *xc,
                    int xc_len);

/* When this block next wants to be sampled, in seconds from now. Required when the block's
 * netlist line declares `ts=variable`, and never called otherwise. Must return > 0. */
double cscript_next_sample_hit(void *state, const double *in, int in_len, const double *xc,
                               int xc_len);

/* Release whatever `cscript_start` returned. Omit it for a block whose state is NULL or
 * otherwise needs no freeing. */
void cscript_free(void *state);

/* Return an independent copy of `state`. Required only for an adaptive-step run, which has to
 * clone every block's state before each trial step; a fixed-step run never calls it. */
void *cscript_clone(void *state);

/* ---------------------------------------------------------------------------------------------
 * The checkpoint state contract: all three, or none. Exporting one or two is refused at load.
 * ------------------------------------------------------------------------------------------ */

/* How many bytes `cscript_state_write` will write for this state. */
size_t cscript_state_size(const void *state);

/* Serialize `state` into `out`, which the caller has sized with `cscript_state_size`. */
void cscript_state_write(const void *state, unsigned char *out);

/* Rebuild a state from `len` bytes written by `cscript_state_write`, returning it exactly as
 * `cscript_start` would. */
void *cscript_state_read(const unsigned char *in, size_t len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* GSIM_CSCRIPT_H */
