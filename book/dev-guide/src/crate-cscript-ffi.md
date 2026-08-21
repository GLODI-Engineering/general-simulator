# `cscript-ffi`: the native escape hatch

*(Skeleton — outline below; not yet written.)*

## What goes here
- Why this exists despite the "Rust throughout" rule elsewhere in the project: genuinely
  stateful block behavior none of `continuous-blocks`'s own types cover, and the deliberate
  scope boundary (this is the *one* place arbitrary native code is allowed, not a crack in the
  rule).
- The full C ABI contract: `cscript_start`/`cscript_output`/optional `cscript_free`/
  `cscript_clone`, ownership/lifetime expectations, what is and isn't checked at load time vs.
  left as the library author's responsibility (this is `unsafe` by construction — say so
  plainly).
- Why `cscript_clone` specifically is required for adaptive stepping (state must be cloneable
  before a trial step so a rejected trial can be discarded) and what happens without it
  (`CScriptRequiresCloneForAdaptiveStep`, a clean error rather than a corrupted run).

## Source material to adapt from
- `crates/cscript-ffi/src/lib.rs` module doc comment — the full contract is already written
  here in detail; this chapter is largely a direct port with added narrative connective tissue.
