# Mensch 65816 ABI Notes

This is the current compiler ABI, not a long-term platform standard.

## Calls and Returns

- Internal and public routines are called with `jsr` and return with `rts`.
- `far` routines return with `rtl`. They are intended for callers that set up a
  long-return frame.
- Function return values are returned in the accumulator. The callee is
  responsible for leaving the accumulator width appropriate for the result
  size.
- `interrupt` routines return with `rti`.

## Parameters

The current parameter convention supports scalar `u8`, `i8`, `u16`, and `i16`
parameters.

Each parameter is assigned a routine-specific direct-page symbol:

```text
__dp_<routine>_param_<name>
```

The caller stores argument values into those symbols before `jsr`. The callee
reads the parameters from the same symbols.

For public routines, parameter symbols are emitted with `.exportzp` so another
object can pass arguments. For external declarations, the caller emits matching
`.importzp` declarations.

This direct-page parameter convention is simple and efficient, but it is not
reentrant. A future stack-based convention is still needed for reentrant
procedures and for code that can be interrupted by another caller of the same
routine.

## Interrupts

Interrupt handlers must preserve every register state they clobber unless the
whole program ABI has explicitly reserved that state for interrupt use. In
particular, an interrupt may arrive while foreground code is carrying a function
return value or a call argument in the accumulator.
