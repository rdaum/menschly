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

Call stubs receive ordinary Mensch parameters, load the syscall registers,
execute `COP`, and return. Most stubs are `reentrant`; small hot-path veneers
may use direct-page parameters when the call site needs the compact non-stack
ABI. The kernel side enters through one native COP-vector dispatcher. That
dispatcher decodes the COP signature byte, saves the original `A`/`X`/`Y`
register arguments, and routes family `$0d` to BIOS or family `$01` to GEMDOS.

The first BIOS calls are TOS-shaped console calls:

```text
COP #$0d
A = $0001       ; Bconstat
X = device      ; 2 is the console device
```

`Bconstat` returns `$ffff` in `A` when input is ready and `0` otherwise.

```text
COP #$0d
A = $0002       ; Bconin
X = device      ; 2 is the console device
```

`Bconin` returns the input character in the low byte of `A`.

```text
COP #$0d
A = $0003       ; Bconout
X = device      ; 2 is the console device
Y = character
```

The call writes the low byte of `Y` to the console when `X == 2`.

```text
COP #$0d
A = $0008       ; Bcostat
X = device      ; 2 is the console device
```

It returns `$ffff` in `A` when the device can accept output and `0` otherwise.
The BIOS implementation routes these calls through a small typed device
registry. The initial registry has one character device, TOS `CON` device `2`,
backed by the x65-alike USB UART.

Hellord also reserves a small private BIOS range for walking that device
registry:

```text
A = $0100       ; first registered device, returns a device id or 0
A = $0101       ; next sibling/list entry, X = device, returns device id or 0
A = $0102       ; parent device, X = device, returns device id or 0
A = $0103       ; first child device, X = device, returns device id or 0
A = $0104       ; device kind, X = device, returns kind or 0
A = $0105       ; device flags, X = device, returns flags or 0
A = $0106       ; device driver, X = device, returns driver id or 0
```

The initial device tree has a single root/list node: device `2`, kind
`$0001` for character I/O, flags `$0003` for readable and writable, and driver
`$0001` for the x65-alike serial UART. Parent, first child, and next sibling
are all `0` until more devices are registered.

The Hellord GEMDOS layer owns the standard stream bindings instead of hardcoding
a console device in callers: at boot it walks the BIOS device registry, chooses
the first character device with both read and write flags, and binds stdin,
stdout, and stderr to that device. Its public console entry points are `COP
#$01` stubs that dispatch to GEMDOS and then delegate to BIOS:

```text
COP #$01
A = $000b       ; Cconis, stdin status

COP #$01
A = $0001       ; Cconin, returns stdin byte in low A

COP #$01
A = $0010       ; Cconos, stdout output-ready status

COP #$01
A = $0002       ; Cconout
X = character   ; low byte is written to stdout

COP #$01
A = $0100       ; get stddev, X = stdin=0/stdout=1/stderr=2

COP #$01
A = $0101       ; set stddev, X = stream, Y = BIOS device id
```
