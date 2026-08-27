// cscript.hpp -- an optional, header-only convenience layer for writing a `kind=cscript` block
// in C++ instead of plain C.
//
// `cscript-ffi` (the Rust crate this header has no compile-time relationship to at all) loads a
// shared library via `dlopen` and calls into it purely by C-linkage symbol name
// (`cscript_start`/`cscript_output`/...) -- it never inspects what language produced those
// symbols. A `.cpp` file that hand-writes its own `extern "C"` functions already works today,
// with zero changes needed on the Rust side; this header exists purely to make that easier to
// write correctly: a small class hierarchy instead of a raw `void *` you cast yourself, a
// generated `cscript_clone` from the compiler's own copy constructor instead of a hand-rolled
// `malloc`+field-copy, and a mandatory exception boundary (see "Exceptions" below) instead of an
// easy way to trigger undefined behavior by accident.
//
// ## Usage -- the plain (non-xc) contract
//
//   #include "cscript.hpp"
//
//   class MyBlock : public cscript::Block {
//       double sum_ = 0.0;
//   public:
//       void output(const double *in, int in_len, double dt, double *out, int out_len) override {
//           (void)dt;
//           if (in_len >= 1) sum_ += in[0];
//           if (out_len >= 1) out[0] = sum_;
//       }
//   };
//   CSCRIPT_EXPORT(MyBlock)
//
// `CSCRIPT_EXPORT(ClassName)` generates `cscript_start`/`cscript_output`/`cscript_free`/
// `cscript_clone` for you: `cscript_start` default-constructs a `ClassName` on the heap;
// `cscript_clone` copy-constructs one from the existing instance (works automatically as long as
// `ClassName`'s members are themselves copyable -- `std::vector`, `double`, plain structs all
// are; only a class managing a raw owning resource by hand needs its own copy constructor,
// exactly the ordinary C++ rule).
//
// ## Usage -- the continuous-state (`xc`) contract
//
//   class MyXcBlock : public cscript::XcBlock {
//   public:
//       void derivative(const double *in, int in_len, const double *xc, int xc_len,
//                        double *xc_dot, int xc_dot_len) override { ... }
//       void output_xc(const double *in, int in_len, double dt, const double *xc, int xc_len,
//                       double *out, int out_len) override { ... }
//   };
//   CSCRIPT_EXPORT_XC(MyXcBlock)
//
// Exactly one of `CSCRIPT_EXPORT`/`CSCRIPT_EXPORT_XC` may appear in one translation unit -- the
// underlying C contract requires one or the other, never both (see `cscript-ffi`'s own module
// doc comment); using both would define `cscript_start`/`cscript_free`/`cscript_clone` twice.
//
// ## Exceptions -- read this before throwing anything
//
// The raw C-side contract this header generates glue for is entirely `void`-returning -- there
// is no error channel to report a failure through at all. A C++ exception that unwinds across an
// `extern "C"` boundary (into `cscript-ffi`'s own Rust caller) is undefined behavior, full stop,
// not merely "unspecified" -- it must never be allowed to happen. Every function
// `CSCRIPT_EXPORT`/`CSCRIPT_EXPORT_XC` generates therefore wraps your override in a
// `try { ... } catch (...)` that prints a diagnostic to `stderr` and calls `std::abort()`. This
// is a deliberate, load-bearing design choice, not a placeholder: aborting loudly is strictly
// better than the alternative (letting the exception escape into undefined behavior, or
// silently swallowing it and returning a zero-filled/stale output as if nothing happened) --
// the same "fail loudly and diagnosably, never silently" principle `pyblock`'s own catchable
// `PyBlockError` follows for Python, just enforced by a hard process abort here instead of a
// catchable error, since a `void` C ABI genuinely has nowhere else to put the failure. Design
// your own `output()`/`derivative()`/`output_xc()` overrides to validate inputs and avoid
// throwing in ordinary operation; reserve exceptions for conditions that really do mean "this
// run cannot continue."

#include <cstdlib>
#include <cstdio>
#include <exception>
#include <new>

namespace cscript {

// Base class for the plain (non-xc) contract. Override `output`; `CSCRIPT_EXPORT` handles the
// rest (construction, destruction, cloning).
class Block {
public:
    virtual ~Block() = default;
    virtual void output(const double *in, int in_len, double dt, double *out, int out_len) = 0;
};

// Base class for the continuous-state (xc) contract. Override `derivative`/`output_xc`;
// `CSCRIPT_EXPORT_XC` handles construction/destruction/cloning, exactly like `Block` above.
class XcBlock {
public:
    virtual ~XcBlock() = default;
    virtual void derivative(const double *in, int in_len, const double *xc, int xc_len,
                             double *xc_dot, int xc_dot_len) = 0;
    virtual void output_xc(const double *in, int in_len, double dt, const double *xc, int xc_len,
                            double *out, int out_len) = 0;
};

namespace detail {

// Prints a diagnostic naming the failing entry point and aborts -- see this header's own
// "Exceptions" section above for why abort, not a swallowed return, is the correct behavior
// here.
[[noreturn]] inline void abort_from_exception(const char *entry_point, const char *what) {
    std::fprintf(stderr, "cscript (C++): %s threw: %s -- aborting (no C-ABI error channel exists "
                          "to report this through; see cscript.hpp's own \"Exceptions\" section)\n",
                 entry_point, what);
    std::abort();
}

} // namespace detail

} // namespace cscript

#define CSCRIPT_EXPORT(ClassName)                                                                \
    extern "C" void *cscript_start(void) {                                                       \
        try {                                                                                     \
            return new ClassName();                                                               \
        } catch (const std::exception &e) {                                                       \
            cscript::detail::abort_from_exception("cscript_start", e.what());                     \
        } catch (...) {                                                                            \
            cscript::detail::abort_from_exception("cscript_start", "non-std::exception");          \
        }                                                                                          \
    }                                                                                              \
    extern "C" void cscript_output(void *state, const double *in, int in_len, double dt,          \
                                    double *out, int out_len) {                                    \
        try {                                                                                       \
            static_cast<ClassName *>(state)->output(in, in_len, dt, out, out_len);                 \
        } catch (const std::exception &e) {                                                        \
            cscript::detail::abort_from_exception("cscript_output", e.what());                     \
        } catch (...) {                                                                             \
            cscript::detail::abort_from_exception("cscript_output", "non-std::exception");          \
        }                                                                                           \
    }                                                                                               \
    extern "C" void cscript_free(void *state) { delete static_cast<ClassName *>(state); }          \
    extern "C" void *cscript_clone(void *state) {                                                  \
        try {                                                                                       \
            return new ClassName(*static_cast<ClassName *>(state));                                \
        } catch (const std::exception &e) {                                                        \
            cscript::detail::abort_from_exception("cscript_clone", e.what());                      \
        } catch (...) {                                                                             \
            cscript::detail::abort_from_exception("cscript_clone", "non-std::exception");           \
        }                                                                                           \
    }

#define CSCRIPT_EXPORT_XC(ClassName)                                                             \
    extern "C" void *cscript_start(void) {                                                       \
        try {                                                                                     \
            return new ClassName();                                                               \
        } catch (const std::exception &e) {                                                       \
            cscript::detail::abort_from_exception("cscript_start", e.what());                     \
        } catch (...) {                                                                            \
            cscript::detail::abort_from_exception("cscript_start", "non-std::exception");          \
        }                                                                                          \
    }                                                                                              \
    extern "C" void cscript_derivative(void *state, const double *in, int in_len,                 \
                                        const double *xc, int xc_len, double *xc_dot,              \
                                        int xc_dot_len) {                                          \
        try {                                                                                       \
            static_cast<ClassName *>(state)->derivative(in, in_len, xc, xc_len, xc_dot,            \
                                                          xc_dot_len);                              \
        } catch (const std::exception &e) {                                                        \
            cscript::detail::abort_from_exception("cscript_derivative", e.what());                 \
        } catch (...) {                                                                             \
            cscript::detail::abort_from_exception("cscript_derivative", "non-std::exception");      \
        }                                                                                           \
    }                                                                                               \
    extern "C" void cscript_output_xc(void *state, const double *in, int in_len, double dt,       \
                                       const double *xc, int xc_len, double *out, int out_len) {    \
        try {                                                                                       \
            static_cast<ClassName *>(state)->output_xc(in, in_len, dt, xc, xc_len, out, out_len);   \
        } catch (const std::exception &e) {                                                        \
            cscript::detail::abort_from_exception("cscript_output_xc", e.what());                  \
        } catch (...) {                                                                             \
            cscript::detail::abort_from_exception("cscript_output_xc", "non-std::exception");       \
        }                                                                                           \
    }                                                                                               \
    extern "C" void cscript_free(void *state) { delete static_cast<ClassName *>(state); }          \
    extern "C" void *cscript_clone(void *state) {                                                  \
        try {                                                                                       \
            return new ClassName(*static_cast<ClassName *>(state));                                \
        } catch (const std::exception &e) {                                                        \
            cscript::detail::abort_from_exception("cscript_clone", e.what());                      \
        } catch (...) {                                                                             \
            cscript::detail::abort_from_exception("cscript_clone", "non-std::exception");           \
        }                                                                                           \
    }
