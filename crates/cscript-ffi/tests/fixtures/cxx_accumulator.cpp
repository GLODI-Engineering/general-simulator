// Test fixture for cscript-ffi's C++ header: the same running-accumulator behavior as
// tests/fixtures/accumulator.c (sum and count outputs), but written as a real C++ class with
// member state, exercised through cscript.hpp's own CSCRIPT_EXPORT macro instead of hand-written
// extern "C" glue -- confirms the plain (non-xc) contract, construction/destruction/cloning via
// the compiler's own new/delete/copy-constructor, all work identically to the C fixture's
// hand-rolled malloc/free/field-copy.
#include "../../include/cscript.hpp"

class Accumulator : public cscript::Block {
    double sum_ = 0.0;
    long count_ = 0;

public:
    void output(const double *in, int in_len, double /*dt*/, double *out, int out_len) override {
        if (in_len >= 1) {
            sum_ += in[0];
        }
        count_ += 1;
        if (out_len >= 1) {
            out[0] = sum_;
        }
        if (out_len >= 2) {
            out[1] = static_cast<double>(count_);
        }
    }
};

CSCRIPT_EXPORT(Accumulator)
