// Test fixture for cscript-ffi's C++ header: confirms an exception thrown from output() is
// caught by CSCRIPT_EXPORT's own generated glue and turned into a clean process abort (see
// cscript.hpp's own "Exceptions" section) rather than being allowed to unwind across the
// extern "C" boundary into undefined behavior.
#include <stdexcept>

#include "../../include/cscript.hpp"

class Throws : public cscript::Block {
public:
    void output(const double *, int, double, double *, int) override {
        throw std::runtime_error("deliberate test failure");
    }
};

CSCRIPT_EXPORT(Throws)
