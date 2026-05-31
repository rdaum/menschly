# Register and Direct-Page Planning

This compiler targets the 65816 as a small-register machine, not as a generic
load/store target with a uniform register file. The current plan is therefore
to keep allocation target-specific until the compiler has a machine IR rich
enough to describe 65816 constraints directly.

## Current Pipeline

The compiler currently uses three layers of register and direct-page planning:

1. `storage.rs` assigns source-level storage and compiler temporaries before
   lowering. Direct-page parameters, direct locals, scratch temps, and
   `scratch var` locals are assigned here. Non-overlapping scratch lifetimes can
   reuse direct-page slots; values that cross calls get private slots; reentrant
   routines use stack storage.
2. `lower_65816.rs` chooses 65816 addressing forms while emitting assembly IR.
   For example, direct-page near-pointer dereferences use `(ptr)` rather than
   zeroing `Y` and using `(ptr),y`.
3. `optimize_65816.rs` performs conservative machine-level cleanup after
   lowering. It prefers direct operands for direct symbols, shortens local
   branches, removes redundant accumulator reloads/stores, avoids redundant
   index transfers in straight-line code, and cleans up repeated mode changes.

These passes are deliberately local and conservative. They are easy to reason
about against 65816 flags, accumulator/index width changes, interrupt entry
state, direct-page ABI slots, and fixed/volatile I/O symbols.

## Why Not regalloc2 Yet

`regalloc2` is a serious allocator, but it is shaped around a machine
environment with allocatable registers, register classes, and per-class scratch
registers over post-instruction-selection machine code:

https://docs.rs/regalloc2/latest/regalloc2/

That is not a natural fit for the compiler's current IR. The 65816 has three
visible registers with very different roles:

- `A` is the main arithmetic, load/store, call-result, and condition-setting
  register. Its width changes independently of index width.
- `X` and `Y` are mostly address/index and syscall argument registers. They are
  not interchangeable with `A`, and often not interchangeable with each other.
- `P`, `DBR`, direct page, stack-relative locations, and addressing-mode choice
  are part of the cost model. Many good "allocation" decisions are really
  decisions to use direct-page memory, direct/indirect addressing, or a mode
  change, not to keep a value in a general register.

Using regalloc2 now would require inventing a pseudo machine with a tiny,
irregular register set, then lowering its allocation results back into
addressing-mode decisions that the allocator cannot directly see. That risks
making the compiler more complex without giving it the facts it needs.

## Better Next Step

The useful next abstraction is a 65816 machine IR between frontend IR and final
assembly. It should make these facts explicit:

- accumulator width and index width requirements
- instruction operands that require `A`, `X`, or `Y`
- values that are valid only if condition flags still match `A`
- call, interrupt, raw-assembly, and label boundaries
- direct-page candidates, direct-page aliases, and call-live direct-page values
- fixed/volatile memory accesses that must not be cached or coalesced
- costs for direct, absolute, stack-relative, indirect, and indexed forms

Once that exists, the allocator can remain target-specific at first:

1. Build basic blocks and liveness for values that can live in `A`, `X`, `Y`,
   stack slots, or direct-page slots.
2. Allocate `A` locally around expression trees and compare/branch sequences,
   because flags make `A` more than a value register.
3. Allocate `X` and `Y` around indexed addressing and syscall/COP calls.
4. Allocate direct-page slots with interval/lifetime reuse and an explicit
   budget, treating direct page as the preferred spill/addressing area.
5. Add a verifier that checks the assigned machine IR for register, flag, width,
   and direct-page aliasing mistakes before assembly emission.

At that point regalloc2 can be reconsidered for a narrow subset, such as
allocating a small class of pseudo index registers within straight-line machine
IR. Until then, a custom allocator is more appropriate because the hard part is
not choosing among many equivalent registers; it is choosing among 65816-specific
storage and addressing forms.
