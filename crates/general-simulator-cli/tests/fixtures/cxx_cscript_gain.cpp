// Test fixture for general-simulator-cli's own kind=cscript end-to-end test, exercising a
// C++-authored library via cscript.hpp: scales its single input by 2, exactly like
// cscript_gain.c's own plain-C version.
#include "../../../cscript-ffi/include/cscript.hpp"

class Gain2 : public cscript::Block {
public:
    void output(const double *in, int in_len, double /*dt*/, double *out, int out_len) override {
        if (in_len >= 1 && out_len >= 1) {
            out[0] = in[0] * 2.0;
        }
    }
};

CSCRIPT_EXPORT(Gain2)
