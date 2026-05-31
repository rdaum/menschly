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

`reentrant` routines pass parameters on the stack instead of through
direct-page symbols. The caller pushes argument values in reverse parameter
order before `jsr`, the callee reads them with stack-relative addressing, and
the caller removes the argument bytes after the routine returns. This currently
supports `u8`, `i8`, `u16`, and `i16` procedure parameters.

Reentrant routines also allocate stack storage for default/`stack` locals. On
entry, the callee subtracts the local frame size from `S`; before every return,
it adds the same byte count back. Locals are addressed from the adjusted stack
pointer starting at `$01,s`. Parameters remain above the return address and are
shifted upward by the local frame size: the first parameter is at
`frame_size + 3,s`.

The stack frame shape during the routine body is:

```text
$01,s .. $frame_size,s       local frame bytes
$(frame_size + 1),s          return address low byte
$(frame_size + 2),s          return address high byte
$(frame_size + 3),s ...      parameters in declaration order
```

## Interrupts

Generated interrupt handlers preserve `A`, `X`, `Y`, `P`, and `DBR` by default,
force the generated handler body to the compiler's normal `A8`/`I16` entry mode,
set `DBR` to the handler code bank, and return with `rti`. This makes
`interrupt procedure` safe for ordinary generated foreground code: an interrupt
may arrive while foreground code is carrying a function return value, call
argument, loop/index state, or data-bank selection in CPU registers.

Latency-sensitive handlers can override the generated save set:

```text
interrupt(preserve = a, x) procedure scanline()
interrupt(preserve = none) procedure hand_tuned_irq()
interrupt(raw) procedure cycle_counted_irq()
```

The current preserve names are `a`, `x`, `y`, `p`, and `dbr`; `default` expands
to the default save set. `preserve = none` is an expert mode for handlers whose
clobber contract is managed outside the compiler-generated save/restore ABI; the
compiler still establishes its handler body entry mode before generated
statements.

`interrupt(raw)` is for fully hand-tuned handlers. It emits no register
save/restore, no `DBR` setup, and no accumulator/index width setup. The compiler
still emits `rti` for `return`, but inline assembly in a raw handler owns every
other machine-state detail.

Interrupt handlers may call parameterless routines, or parameterized
`reentrant` routines that use stack arguments. The frontend rejects calls from
`interrupt procedure` bodies to non-reentrant routines or external declarations
with parameters, because direct-page parameter slots can be clobbered if the
interrupt fires while foreground code is preparing or using the same call.

## System Calls

Hellord system calls use the 65816 `COP` software-interrupt instruction as the
machine equivalent of the TOS `TRAP` families. The immediate byte selects the
call family:

```text
COP #$01   GEMDOS-like process, file, path, and memory services
COP #$0d   BIOS-like device and block services
COP #$0e   XBIOS-like machine-specific services
```

This keeps the public numbering close to TOS where it is useful: BIOS is family
`$0d`, XBIOS is family `$0e`, and GEMDOS is family `$01`. The 65816 native COP
vector is at `$FFE4`. In ca65 inline assembly the signature byte is spelled
without `#`, for example `cop $0d`.

The initial register convention is deliberately small:

```text
A   function number / return status
X   word argument 0
Y   word argument 1
```

Call stubs must be `reentrant` routines. They receive ordinary Mensch stack
parameters, load the syscall registers, execute `COP`, and return. The kernel
side syscall handlers must also be interrupt handlers that avoid caller-specific
direct-page parameter slots. This keeps the syscall path safe if it is entered
while foreground code or an interrupt handler is using non-reentrant routines.

The first BIOS call is TOS-shaped `Bconout`:

```text
COP #$0d
A = $0003       ; Bconout
X = device      ; 2 is the console device
Y = character
```

The call writes the low byte of `Y` to the console when `X == 2`.

`Bcostat` is also implemented for the same registered console device:

```text
COP #$0d
A = $0008       ; Bcostat
X = device      ; 2 is the console device
```

It returns `$ffff` in `A` when the device can accept output and `0` otherwise.
The BIOS implementation routes these calls through a small typed device
registry. The initial registry has one character device, TOS `CON` device `2`,
backed by the x65-alike USB UART.
