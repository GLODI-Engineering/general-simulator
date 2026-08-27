# C++ `cscript` blocks: `cscript.hpp`

*(Implemented this session, as a header-only addition to the existing `cscript-ffi` crate — no
Rust-side changes at all.)*

## The one-sentence version

`kind=cscript` already supports C++ today, with zero changes needed anywhere in this workspace —
`cscript-ffi` loads a `.so` with `dlopen` and resolves `cscript_start`/`cscript_output`/... purely
by C-linkage symbol name, and never inspects what language produced them. A `.cpp` file that
hand-writes its own `extern "C"` functions around a C++ class already works. `cscript.hpp` (new
this session, `crates/cscript-ffi/include/cscript.hpp`) exists purely to make writing that `.cpp`
file *ergonomic*: a small class hierarchy instead of a raw `void *` cast by hand, `cscript_clone`
generated from the compiler's own copy constructor instead of a hand-rolled `malloc` +
field-by-field copy, and a mandatory exception boundary instead of an easy way to trigger
undefined behavior by forgetting one.

## Why this needed no Rust changes at all (and how that was verified, not assumed)

`cscript-ffi`'s own contract (see `python-blocks.md`'s sibling page, `crate-cscript-ffi.md`) is
defined purely in terms of four C-linkage symbol names and their calling conventions — nothing in
`CScriptLibrary::load` or anywhere else in the crate asks what compiler or language produced the
`.so` it's `dlopen`-ing. A C++ compiler that exports `extern "C" void *cscript_start(void)` emits
exactly the same unmangled symbol a C compiler would; the object code underneath may use classes,
`new`/`delete`, exceptions, templates, anything — the ABI boundary this crate actually calls
through only sees four flat function pointers. This was confirmed directly, not just reasoned
about: `nm -D` on a compiled `cxx_accumulator.so` fixture shows the four `cscript_*` symbols
unmangled (`T cscript_start`, etc.) alongside the expected mangled C++ symbols for the class's own
methods, and `cscript-ffi`'s own test suite (`tests/cxx_fixtures.rs`) loads and calls that same
`.so` through the ordinary `CScriptRegistry`/`CScriptInstance` API with no special-casing at all —
identical numeric results to the plain-C `accumulator.c` fixture it mirrors.

## `cscript.hpp`'s own contract

```cpp
#include "cscript.hpp"

class MyBlock : public cscript::Block {
    double sum_ = 0.0;
public:
    void output(const double *in, int in_len, double dt, double *out, int out_len) override {
        if (in_len >= 1) sum_ += in[0];
        if (out_len >= 1) out[0] = sum_;
    }
};
CSCRIPT_EXPORT(MyBlock)
```

`CSCRIPT_EXPORT(ClassName)` generates `cscript_start`/`cscript_output`/`cscript_free`/
`cscript_clone` for you: `cscript_start` default-constructs `ClassName` on the heap;
`cscript_clone` copy-constructs a new one from the existing instance — works automatically as
long as `ClassName`'s own members are copyable (`std::vector`, `double`, plain structs all are;
only a class hand-managing a raw owning resource needs its own copy constructor, the ordinary
C++ rule, not something this header adds). The continuous-state (`xc`) contract has its own
parallel base class and macro:

```cpp
class MyXcBlock : public cscript::XcBlock {
public:
    void derivative(const double *in, int in_len, const double *xc, int xc_len,
                     double *xc_dot, int xc_dot_len) override { ... }
    void output_xc(const double *in, int in_len, double dt, const double *xc, int xc_len,
                    double *out, int out_len) override { ... }
};
CSCRIPT_EXPORT_XC(MyXcBlock)
```

Exactly one of the two macros may appear per translation unit — the underlying C contract
requires the plain or the `xc` contract, never both (see `cscript-ffi`'s own module doc comment);
using both would define `cscript_start`/`cscript_free`/`cscript_clone` twice in the same `.cpp`
file, a compile error, not a runtime ambiguity.

## Exceptions: a mandatory abort, not a design gap

The raw C-side contract this header wraps is entirely `void`-returning — there is no error
channel to report a failure through at all. A C++ exception that unwinds across the `extern "C"`
boundary into `cscript-ffi`'s own Rust caller is undefined behavior, not merely unspecified.
Every function `CSCRIPT_EXPORT`/`CSCRIPT_EXPORT_XC` generates therefore wraps the user's own
override in `try { ... } catch (...)`, printing a diagnostic naming the failing entry point to
`stderr` and calling `std::abort()`. This is deliberate, not a placeholder: a hard abort is
strictly better than either alternative (letting the exception escape into UB, or silently
swallowing it and returning a zero-filled/stale output as if nothing happened) — the same "fail
loudly and diagnosably, never silently" principle `pyblock`'s own catchable `PyBlockError`
follows for Python, enforced here by a process abort instead of a catchable Rust error, since a
`void` C ABI genuinely has nowhere else to put the failure. Verified directly, not just
documented: `tests/cxx_fixtures.rs`'s own
`an_exception_in_output_aborts_the_process_instead_of_unwinding_into_rust` test re-execs the test
binary as a child process (calling into a throwing fixture inline would abort the whole parent
test binary, not just one case) and confirms the child is killed by `SIGABRT`, with the expected
diagnostic on `stderr` — not a clean exit, not a segfault, not silent corruption.

## What's deliberately not built

- No automatic marshaling of anything beyond flat `double` arrays — the underlying C contract's
  own `in`/`out`/`xc` are plain pointer+length pairs, and this header doesn't add a `std::vector`-
  based overload set on top; a `.cpp` author who wants `std::span`/`std::vector` convenience
  inside their own `output()` body is free to wrap the raw pointers themselves.
- No build-system integration — exactly like plain-C `cscript`, the user compiles their own
  `.cpp` file into a shared library themselves (e.g. `c++ -std=c++17 -shared -fPIC -o
  my_block.so my_block.cpp`); this crate never invokes a compiler. `cscript.hpp` only needs to be
  on the include path (`-I<path-to-cscript-ffi>/include`, or simply copied alongside the `.cpp`
  file).
- No RTTI/dynamic_cast requirement — `CSCRIPT_EXPORT`'s generated `cscript_clone` uses a plain
  `static_cast` + the copy constructor, never `dynamic_cast`, so `-fno-rtti` builds work fine.
