// Test fixture for cscript-ffi's C++ header: the continuous-state (xc) contract, mirroring
// tests/fixtures/decay_xc.c's own dx/dt = -k*x (k=2.0) first-order decay, but through
// cscript.hpp's CSCRIPT_EXPORT_XC macro and a real C++ class instead of hand-written extern "C"
// glue over a malloc'd long.
#include "../../include/cscript.hpp"

namespace {
constexpr double kK = 2.0;
}

class DecayXc : public cscript::XcBlock {
    long calls_ = 0;

public:
    void derivative(const double *in, int in_len, const double *xc, int xc_len, double *xc_dot,
                     int xc_dot_len) override {
        if (xc_len >= 1 && xc_dot_len >= 1) {
            double u = (in_len >= 1) ? in[0] : 0.0;
            xc_dot[0] = -kK * xc[0] + u;
        }
    }

    void output_xc(const double * /*in*/, int /*in_len*/, double /*dt*/, const double *xc,
                    int xc_len, double *out, int out_len) override {
        calls_ += 1;
        if (out_len >= 1 && xc_len >= 1) {
            out[0] = xc[0];
        }
        if (out_len >= 2) {
            out[1] = static_cast<double>(calls_);
        }
    }
};

CSCRIPT_EXPORT_XC(DecayXc)
