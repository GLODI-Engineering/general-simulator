# Workspace layout

*(Skeleton — outline below; not yet written.)*

## What goes here
- The dependency diagram: `lcp-solver` <- `pwl-devices` <- `dae-runtime` (also depends on
  `continuous-blocks`, `cscript-ffi`) <- `elspice-pwl-cli`, plus the two read-only sibling repos
  (`spice-lsp`/`spice-core`, `elspice-mna`) this workspace treats as external dependencies.
- The "build up trust incrementally" verification order this crate was actually built in
  (`lcp-solver` trusted standalone first, then `pwl-devices` against it, etc.) — this ordering
  is itself part of the rationale for trusting the whole system, worth stating explicitly.
- One paragraph per crate, each linking to its own detail chapter below.

## Source material to adapt from
- `README.md`'s "Relationship to the sibling repos" and "Status" sections.
- `docs/architecture.md`'s milestone list.
