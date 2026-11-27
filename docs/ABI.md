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

Parameters can be any scalar: integers up to 32 bits, pointers and routine
pointers (records and arrays are passed by pointer). The 65816 backend does not
handle 32-bit values yet, so there they are limited to 24 bits.

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
reentrant, and callers must know the callee's parameter symbols.

Arguments are evaluated left to right. When an argument contains a call,
the arguments before it (other than constants) are computed into
temporaries first and copied into the parameter symbols afterwards, since
the inner call may use the same direct-page slots.

### Register arguments and carry results

A plain, unexported routine (or an `external` one) can take parameters in
registers and return a 0/1 result in the carry flag:

```text
function pick(index: u8 in x, bias: u8 in a): u8
procedure add(amount: u16 in xy, extra: u8)
function is_even(value: u8 in a): u8 in carry
```

- A byte parameter can be `in a`, `in x` or `in y`; a word `in ax` (low byte
  in A, high in X) or `in xy` (low in X, high in Y). Each register carries
  one argument. Other parameters still use their direct-page symbols.
- The caller loads the registers immediately before `jsr`, X and Y first,
  A last. On the 65816 index registers are 16 bits wide; a byte argument
  arrives zero-extended, and the callee only looks at the low byte.
- The callee stores register arguments to its parameter storage first thing,
  before any frame setup (which uses the registers).
- A carry-result function returns with C set for a non-zero result
  (`sec`, `clc`, or `cmp #1` of a computed byte). `if f()` and `if not f()`
  call it and branch on the carry; used as a value it becomes
  `lda #0 / rol`. The MEGA65 frame release (`__m65_leave`) keeps the carry.
- Exported routines cannot use either: a window bank's call stub uses the
  registers and flags on the way through.

### Direct-page overlay

After storage assignment, each module overlays the direct-page slots that
belong to a single routine (parameters kept across calls, `direct` locals,
private temporaries) in one region, `__dp_overlay`. Two routines share
addresses there only when neither can reach the other through calls, so
they can never be active together. The call graph is the module's own:

- a call the module cannot follow (an external routine, a routine pointer,
  a trap, inline assembly) is assumed to reach every routine that is
  exported, used as a vector, or has its address taken;
- routines reachable from an interrupt handler (and the handler itself)
  keep slots of their own.

The overlaid symbols are emitted as `name = __dp_overlay+offset`. On the
MEGA65 kernel this cut base-page use from 150 to 122 bytes.

`stack` and `reentrant` routines take their arguments on the stack instead.
The caller pushes argument values in reverse parameter order before `jsr`;
the callee reads them in place (stack-relative on the 65816, through the frame
pointer on the MEGA65), and the arguments are removed after the routine
returns. Callers cannot tell the two kinds apart, so routine pointers and trap
vectors accept either. They differ in their locals:

- `stack` routines keep locals and spilled values in static (or direct-page)
  storage, like plain routines. They must not recurse or be re-entered by an
  interrupt.
- `reentrant` routines keep them in a stack frame, so they can recurse and be
  called from interrupt handlers (the only parameterized routines an
  interrupt handler may call).

Locals start undefined, except in `reentrant` routines, where the frame is
zeroed.

Reentrant routines allocate stack storage for default/`stack` locals. On
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

## Routine Pointers

A routine pointer is a 16-bit code address. Only two kinds of routine can be
called through one, because the caller cannot know a particular routine's
parameter storage:

- routines without parameters (any ABI except `far` and `interrupt`), and
- `stack` and `reentrant` routines, whose arguments go on the stack.

An indirect call pushes the arguments exactly as a direct call would and then
calls the address. On the 65816 the address goes in `A` and the caller
`jsr`s to a per-module trampoline, `__mensch_icall` (`dec a / pha / rts`), so
the callee returns straight to the caller. On the MEGA65 the address goes in
`__m65_ptr` and the call is `jsr (__m65_ptr)`.

## Arithmetic Support Routines

`*`, `/` and `%` call support routines (multiplying or dividing by a constant
power of two becomes a shift or mask instead):

- 65816: per-module copies of `__mensch_mul16`, `__mensch_divu16` and
  `__mensch_divs16` take the left operand in `A` and the right in `X` (16-bit
  registers; bytes are widened first) and return the product or quotient in
  `A` and the remainder in `X`. They keep state in module statics, so they are
  not safe to use from an interrupt handler and the code it interrupts at the
  same time. Signed compares stage their right operand in the static
  `__mensch_compare`, with the same caveat.
- MEGA65: `*`, `/` and `%` use the math unit in line, except a signed `/`
  or `%` outside a loop, which calls a routine (see below).

Division truncates toward zero and the remainder takes the dividend's sign.

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

Mensch code makes a system call with the target-neutral builtin

```text
trap(family, call, arg0, arg1): u16      ; `cop_call` is an older alias
```

Each family has one dispatcher, an ordinary
`reentrant function(call: u16, arg0: u16, arg1: u16): u16`. How a trap
reaches the dispatcher depends on the target:

- 65816: the `COP` instruction, described below.
- MEGA65 (45GS02): the CPU has no software trap outside the hypervisor, so
  `trap` pushes the arguments with the reentrant ABI and calls the dispatcher
  through a fixed vector table: `jsr ($FF00 + 2 * family)`. A module fills
  table slots with `vector trap_XX = dispatcher` (families `$00`-`$0f`); see
  `hellord/traps-mega65.mensch`. The table address is part of the
  MEGA65 kernel ABI, so separately loaded programs can call the kernel
  without linking against it.

On the 65816, Hellord system calls use the `COP` software-interrupt
instruction as the machine equivalent of the TOS `TRAP` families. The
immediate byte selects the call family:

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
registry. The initial registry has two root/list devices:

```text
device 2        ; TOS CON-shaped character device
kind = $0001    ; character I/O
flags = $0003   ; readable and writable
driver = $0001  ; x65-alike USB UART
next = 8

device 8        ; first block device
kind = $0002    ; block I/O
flags = $0003   ; readable and writable
driver = $0002  ; x65-alike block disk registers
next = 0
```

Both initial devices have parent and first child set to `0`.

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

The current BIOS block calls are deliberately small and device-oriented. They
target physical BIOS device ids, not GEMDOS drive letters:

```text
COP #$0d
A = $0200       ; Bblkstat
X = device      ; 8 is the first block device
```

`Bblkstat` returns the low byte of the x65-alike block status register in `A`.
The current status bits are `$01` for media present and `$02` for writable.

```text
COP #$0d
A = $0201       ; Bblkcount
X = device
```

`Bblkcount` returns the attached media sector count in `A`. The current BIOS
wrapper exposes the emulator's 16-bit sector-count register, so this is enough
for small test images but not a final large-disk ABI.

```text
COP #$0d
A = $0202       ; Bsetlba
X = device
Y = low 16 bits of LBA
```

`Bsetlba` selects the next sector for transfer. The current wrapper clears the
high 16 bits of the emulator LBA, so it addresses only the first 65,536 sectors.

```text
COP #$0d
A = $0203       ; Bread
X = device
Y = near pointer to a 512-byte destination buffer
```

`Bread` reads one 512-byte sector from the selected LBA into the destination
buffer and returns `0` on success or a block-device error code in `A`.

```text
COP #$0d
A = $0204       ; Bwrite
X = device
Y = near pointer to a 512-byte source buffer
```

`Bwrite` writes one 512-byte sector from the source buffer to the selected LBA
and returns `0` on success or a block-device error code in `A`.

Block storage follows the same BIOS/DOS layering as TOS-like systems: BIOS owns
physical block devices and media status, while the DOS layer owns drive letters,
paths, and filesystem interpretation. Hellord now initializes a small internal
drive table with backend kind, filesystem driver, BIOS device id, and partition
offset fields. `A:` maps to BIOS block device `8` at partition offset `0`, while
the other drives remain ROMFS-only.

The x65-alike boot image should not treat the initial bank-0 ROM window as the
whole operating-system budget. That window is the boot/vector home: it should
perform initial setup, install interrupt and trap entry points, select the
needed ROM/RAM page, and then transfer into banked or paged OS code. The
compiler supports a temporary module-level placement hook for this bring-up:
`metadata code_segment = "BANK1"` emits that module's static data and routines
into the named ca65 segment. `metadata data_segment = "CODE"` can keep static
near-pointer data in bank 0 while routines live elsewhere. The x65-native image
format can carry those segments as 24-bit load blocks with a 24-bit entry point.
Calls declared as `external far procedure` or `external far function` lower to
`JSL`, and far callees return with `RTL`, including the current direct-parameter
passing scheme. The remaining work is to continue splitting Hellord's modules
and to decide where explicit ROMBLOCK page-switching trampolines are needed
instead of plain same-address-space long calls.

There is intentionally no GEMDOS raw-sector syscall in this ABI. TOS had
`Rwabs` in XBIOS, not GEMDOS, and Hellord does not need a public equivalent yet.
The internal `dos_drive_read_blocks` and `dos_drive_write_blocks` helpers accept
a logical drive, sector number, sector count, and memory buffer, apply the
drive's partition offset, and delegate to `dos_volume_read_blocks` or
`dos_volume_write_blocks`. The volume helpers map the request through the drive
table to the BIOS block device and loop over counted 512-byte sector transfers.
A one-sector DOS block cache records the last drive/LBA read or written; reads
copy cache hits into the caller's buffer, and successful writes refresh the
cached sector.

Filesystem implementations sit above those internal raw block helpers. Future
FAT12/FAT16/FAT32 code should call the DOS drive/volume helpers through its
filesystem driver, not talk directly to emulator registers, a future VERA/SPI
transport, or a GEMDOS-style public block call. GEMDOS file calls such as
`Fsfirst`, `Fopen`, `Fread`, and `Fwrite` should route into DOS filesystem
dispatch, and the filesystem driver should perform any raw block access it
needs underneath that boundary.

The current FAT boundary is only a scaffold: `dos_mount_fat_drive` selects the
FAT driver for a logical drive that is already backed by a BIOS block device,
and `fat_read_sector`/`fat_write_sector` move one sector through the DOS drive
helpers. These are filesystem-driver helpers, not public GEMDOS calls. FAT
directory search currently parses enough of a simple one-FAT FAT12/FAT16-style
boot sector to locate the fixed root directory and enumerate simple 8.3 entries
from the first root-directory sector through `Fsfirst`/`Fsnext`.
`Fopen`/`Fread` can open one matching 8.3 file from that root sector and read
bytes from the first data cluster. The read path keeps a small sequential FAT16
cursor and can advance
across the next two cluster boundaries with FAT-table lookups, which proves
non-contiguous media reads without yet being a general unbounded chain walker.
`Fwrite` on a FAT handle currently writes the caller's buffer as a whole sector
to the file's first data cluster and returns the requested byte count on a
successful sector write. That proves the GEMDOS-to-FAT-to-BIOS write path, but
it is not yet a byte-granular FAT update path. Arbitrary-length cluster
traversal, subdirectories, long names, allocation, file-size updates, and FAT32
root-cluster handling are still missing. A real FAT implementation should
continue growing downward from GEMDOS file/path calls into FAT directory,
allocation, and cluster logic, using the sector helpers as its raw media access
layer.

The command-facing file operations now cross a small filesystem-driver dispatch
boundary before reaching the current ROM file table. `DIR`, `TYPE`, and `RUN`
ask the DOS layer for the current drive's filesystem driver, then route to the
ROMFS implementation when the driver is `$0001`. This keeps the existing ROMFS
behavior intact while giving FAT and future filesystems a place to implement
directory enumeration, file reads, and program loading without changing command
parsing.

Open DOS file handles also record the filesystem driver that created them.
`Fread` dispatches through the handle's saved driver instead of the current
drive, so a file opened from ROMFS remains a ROMFS handle even if the current
drive is later remounted as FAT. That is still a tiny handle table, but it is
the boundary needed before FAT can add its own file identifiers and read logic.

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
```

The first drive-management GEMDOS calls use the TOS call numbers and route into
the DOS drive table. These calls initialize the DOS drive table if it has not
already been initialized by `dos_boot`.

```text
COP #$01
A = $0019       ; Dgetdrv, returns current drive index: A=0, B=1, ...

COP #$01
A = $000e       ; Dsetdrv
X = drive index ; A=0, B=1, ...
```

`Dsetdrv` updates the DOS current-drive field and currently returns `$ffff` for
valid drive indexes in the supported `A:` through `Z:` range, or `0` for an
invalid drive index. This is a 16-bit placeholder for TOS's wider drive bitmap.

The first path-management call also uses the TOS call number and delegates to
DOS path state:

```text
COP #$01
A = $003b       ; Dsetpath
X = near pointer to a NUL-terminated path string
```

`Dsetpath` currently recognizes `/`, `.`, `..`, `GAMES`, and `/GAMES`,
case-insensitively. It returns `0` on success and `$ffff` for an unknown path.
This is deliberately a GEMDOS path operation over DOS state, not a raw block
operation; later filesystem-backed directories should extend the DOS
filesystem-driver dispatch underneath this call.

Directory search now has the first GEMDOS-shaped entry points:

```text
COP #$01
A = $004e       ; Fsfirst
X = near pointer to a NUL-terminated pattern string
Y = near pointer to a search result buffer

COP #$01
A = $004f       ; Fsnext
X = near pointer to a search result buffer
```

The current `Fsfirst` implementation ignores the pattern and starts a search in
the current DOS directory. `Fsnext` advances that search. Both calls route
through DOS filesystem-driver dispatch; ROMFS and the FAT root-directory
scaffold are the implemented drivers today. They return `0` when an entry is
written and `$fffe` when there are no more entries.

The temporary search result buffer is not a TOS-compatible DTA yet. Hellord
currently writes:

```text
byte 0          GEMDOS-style attribute bits; `$10` marks a directory
byte 1..2       little-endian file size
byte 3..        NUL-terminated filename
```

The first GEMDOS file-handle calls are also present:

```text
COP #$01
A = $003d       ; Fopen
X = near pointer to a NUL-terminated path string
Y = open mode

COP #$01
A = $003f       ; Fread
X = file handle
Y = near pointer to destination buffer

COP #$01
A = $0040       ; Fwrite
X = file handle
Y = near pointer to source buffer

COP #$01
A = $003e       ; Fclose
X = file handle
```

`Fopen` currently opens ROMFS text files from the current DOS directory or a
simple 8.3 FAT root entry and returns handles starting at `6`. `Fread` advances
the handle offset and returns the number of bytes copied. `Fclose` releases the
handle. `Fwrite` returns `$fffb` for ROMFS because it is read-only, while FAT
handles route to the current sector-write scaffold described above. `Fread` and
`Fwrite` carry their byte count through the current GEMDOS stub scratch word,
since the minimal `COP` convention only has `A`, `X`, and `Y`; this is a
temporary ABI detail to revisit when the syscall frame grows beyond two word
arguments.

Hellord also has two private GEMDOS calls for testing and bootstrapping the
standard stream bindings:

```text
COP #$01
A = $0100       ; get stddev, X = stdin=0/stdout=1/stderr=2

COP #$01
A = $0101       ; set stddev, X = stream, Y = BIOS device id
```

## MEGA65 (45GS02) Target

The MEGA65 backend (`menschc --target mega65`) shares the IR and storage
assignment with the 65816 backend but lowers everything byte by byte:

- Z is zero between instructions; `(bp),z` and `[bp],z` sequences restore it.
- The IR's virtual accumulator lives in base page (`__m65_acc`, 4 bytes).
  Scratch pointers `__m65_ptr` and `__m65_farp` (a 32-bit flat address), the
  frame pointer `__m65_fp`, the math operand `__m65_arg` (4 bytes) and the
  temp block `__m65_tmp` (8 bytes) are also runtime base-page registers.
- Routines (other than interrupt handlers) keep expression temporaries that
  do not live across a call, and scalar locals that do not (a loop that uses
  a local keeps it live for the whole loop), in `__m65_tmp` instead of their
  stack frame or static storage. A `reentrant` routine's such locals are
  zeroed on entry, like frame locals. The math routines leave `__m65_tmp`
  alone; interrupt handlers that call out save it with the other runtime
  registers.
- `stack` routines with parameters have a frame for the frame pointer (and
  any spilled values), but no frame locals.
- Indexing an array of at most 256 bytes computes a one-byte offset (from
  the index's low byte) in 8-bit arithmetic and uses `base,x`.
- Function results come back in `A` (byte 0), `X` (byte 1), `Y` (byte 2) and
  `__m65_acc+3` (byte 3), which the frame release leaves alone. Functions
  with 1- or 2-byte results return with N/Z describing `A`, so a caller in
  the same module tests the result without a `cmp #0` (other callers, such
  as those going through a window stub, cannot rely on it).
- Far pointers are 32-bit flat addresses, dereferenced through `[__m65_farp],z`.
- `*`, `/` and `%` are done in line on the MEGA65 math unit (`$D768-$D77F`),
  between `php / sei` and `plp` (an interrupt handler, already holding
  interrupts off, skips them; two uses back to back share one hold): the
  operands are written straight to the unit's inputs, the busy flag at
  `$D70F` waited on, and the result read straight from the product or
  quotient. A product writes only as many input bytes as its width. A
  divide zeroes the inputs' upper bytes first, in line inside a loop and
  otherwise by calling `__m65_clear_inputsN` (N = the operand width in
  bytes; it touches nothing else). A signed divide writes the operands'
  magnitudes and negates the quotient when their signs differ; a remainder
  is the dividend less the quotient times the divisor. Dividing by zero
  gives a quotient of all ones (negated for a negative dividend) and a
  remainder of the dividend.
- Outside a loop, where the code it takes counts for more than its time, a
  signed `/` or `%` instead loads the operands into `__m65_acc` and
  `__m65_arg` and calls `__m65_divs` (`X` = the operand width in bytes):
  quotient in `__m65_acc`, remainder in `__m65_arg`, with the same results;
  it clobbers A/X/Y and `__m65_farp`. The math routines live in
  `mega65::CLEAR_RUNTIME` and `mega65::DIVS_RUNTIME`, each linked only into
  images that call it.
- Internal and exported routines use direct-page parameters, as on the 65816
  (with register arguments and carry results as described above).
  `far` currently means nothing: every call is `jsr`/`rts`.
- The runtime runs with a 16-bit stack pointer. Routines with stack storage
  address their frame through `__m65_fp` with `(fp),y`:

  ```text
  fp+1 .. fp+F        locals and spilled scratch (zeroed on entry)
  fp+F+1, fp+F+2      caller's fp
  fp+F+3, fp+F+4      return address
  fp+F+5 ..           stack arguments, first parameter lowest
  ```

  Reentrant callees pop their own arguments with `rtn #n`. Frames are built
  and released by two runtime routines in the kernel's `CRT0`, called with
  `Z` = the local byte count `F` (omitted when it is zero):
  `ldz #F / jsr __m65_enter` first thing in the routine, and
  `ldz #F / jsr __m65_leave` just before its `rts`/`rtn`/`rti`. They live in
  `mega65::RUNTIME`, linked into the kernel and into every program. Both use
  `__m65_acc` and `__m65_ptr` as scratch, and `__m65_leave` keeps the
  A/X/Y result intact.
- Generated code is cleaned up by `optimize_45gs02` (dead-code elimination,
  redundant load/store removal, branch shortening). It treats only
  compiler-owned storage as plain RAM: accesses to user globals, including
  `fixed` hardware registers, are kept exactly as written. Callees never take
  inputs in the base-page scratch registers, except the math routines
  (`__m65_acc`, `__m65_arg`) and indirect calls (the vector in `__m65_ptr`).
- Indexing an array of at most 256 bytes uses `base,x`. Larger arrays form
  the full 16-bit address `base + index` in `__m65_ptr`.
- Interrupt routines save A/X/Y/Z, re-establish Z = 0, and save the
  base-page runtime registers they can disturb (all of them if the handler
  calls other routines; the accumulator and pointer if it has a frame). On both targets the storage pass gives interrupt
  routines private direct-page argument and scratch slots, so a handler never
  shares a slot with the code it interrupted. Routines called from a handler
  should be `reentrant`.

### MEGA65 kernel image

The kernel ROM (`mega65.rs`) replaces `MEGA65.ROM`. The hypervisor loads the
file at linear `$20000`; a two-stage loader in the `$E000` ROM window copies
the kernel to the same addresses in bank 0 and maps the ROMs out:

```text
$0002-$00ff   base page
$0200-$1fff   free (the console screen is at $0400)
$2000-$7fff   resident kernel code, data and BSS
$8000-$bfff   window: the mapped window bank's code
$c000-$c7ff   resident core state (mapping, switch stack)
$c800-$cfff   16-bit stack, growing down from $cfff
$d000-$dfff   I/O
$e000-$febf   startup and the resident core's code
$fec0-$fedf   the kernel's trap dispatchers (the modules' trap vectors)
$fee0-$fee5   the kernel's NMI / IRQ handlers
$ff00-$ff1f   trap vector table (the core's trap entries)
$fffa-$ffff   NMI / RESET / IRQ vectors (IRQ: the core's entry)
```

This is the *split* kernel, the layout of any kernel with window banks
(which Hellord's is). While a program runs, `$0000-$bfff` show the program
bank instead (below). A kernel without window banks is one bank-0 image up
to `$cfff`: its programs load after its BSS, up to `$c7ff`, share its base
page, and its vectors point straight at its handlers.

### Window banks

A kernel module can be built into one of four *window banks*
(`mega65::KernelModule::window`, or `mensch-run-mega65 --window N:PATH`).
Hellord puts its DOS in bank 1 and the FAT driver and program loader in bank 2.

- A bank's code is linked to run at `$8000-$bfff` and stays in the ROM image
  at linear `$30000 + (N - 1) * $4000`; `MAP` points the upper half's blocks
  4-5 there. Nothing is copied at boot.
- Only code moves. The module's globals, string literals and metadata stay
  in the resident image, so pointers keep meaning the same thing whichever
  bank is mapped. Buffers passed between banks must not live in the window.
- Each routine that can be reached from outside the module (exported, address
  taken, or a trap vector) becomes a resident 6-byte stub with the routine's
  name, and its body is renamed `__window_<name>`:

  ```text
  foo:  jsr __m65_farcall
        .byte N
        .addr __window_foo
  ```

  Calls within the module go straight to the bodies; everything else, and
  every pointer to such a routine, goes through the stub. Interrupt vectors
  cannot point into a window bank.
- `__m65_farcall` (in `mega65::WINDOW_RUNTIME`) moves the caller's flags,
  return address and current bank onto a resident window stack, maps bank N
  and enters the body so that it sees exactly the stack a direct call would
  give it (arguments, then a return address). When the body returns, it maps
  the caller's bank back, restores the flags and returns with A/X/Y intact
  (`__m65_acc+3` is never touched). The window stack only changes with
  interrupts off, so interrupt handlers can call into banks too. It is 32
  entries deep; overflowing it stops the machine with a red border.
- The current bank is `__m65_window` (0 = nothing mapped).
- All mapping changes, including the program-bank switches below, go
  through one routine of the resident core, `__m65_switch`
  (`mega65::WINDOW_RUNTIME`), with one switch stack. Calling into a
  bank and back costs roughly 250 cycles (about 6 µs at 40 MHz).

### Machine drivers

Hellord's BIOS is machine-independent. Each machine supplies a driver module
with the console entry points (`serial_init`, `serial_write`,
`serial_has_data`, `serial_read`, `serial_irq`), the block device
(`blockdev_status`, `blockdev_sector_count`, `blockdev_set_lba`,
`blockdev_read`, `blockdev_write`) and `machine_reboot`:

- x65-alike: `serial-ring.mensch` plus `x65-machine.mensch`
- MEGA65: `mega65-machine.mensch`. The console is the 40x25 VIC-IV text
  screen and the `$D610` keyboard queue. The block device is the SD card,
  driven through the SD controller at `$D680` with the sector buffer mapped
  at `$DE00`; it presents the card's first FAT partition (or the whole card
  without a partition table), adding the partition offset to 32-bit block
  numbers.

BIOS block numbers are 32-bit: `bios_block_set_lba(dev, lba)` selects a
block and clears bits 16-31, and `bios_block_set_lba_high(dev, high)`
(BIOS call `$0205`) sets them. Drivers implement `blockdev_set_lba_high`.

### Hellord filesystems

Drive `A:` is the built-in ROM filesystem. Drive `S:` is the BIOS block
device mounted as a FAT volume by `hellord/fat.mensch`, which supports FAT16
and FAT32 (FAT12 is rejected): root-directory listing across multiple
sectors and clusters, opening 8.3 names case-insensitively, reading through
cluster chains, and in-place writes within a file's existing size. File
offsets are 16-bit for now, so reads stop at 64 KiB, and subdirectories are
not yet supported.

### Hellord programs (MEGA65)

On a FAT drive, `RUN NAME` or just `NAME` loads and runs `NAME.PRG` (the
extension is added when the name has none) with `hellord/exec.mensch`.
Programs are relocatable o65 files ("small" mode, byte-wise relocation)
built by `mensch-mega65-program` (`mega65::build_program`):

- On a split kernel a program runs in its own bank, linear `$40000`: `MAP`
  shows `$40000-$4bfff` at CPU `$0000-$bfff` while it runs. The stack
  (`$c800`), I/O, and the resident core stay shared. The program area is
  `$0200-$bfff` (47.5 KB), and the program has its own base page.
- The whole file is read into the program area and relocated in place: text
  and data stay where they were read, BSS follows the data (over the
  relocation tables), and the base-page segment goes at `$02` of the
  program's base page (after the kernel's, on a kernel without window
  banks). BSS and base page are cleared. The loader writes the program
  bank through flat pointers (`machine_peek`, `machine_poke`,
  `machine_copy_out`, `machine_fill` in `mega65-machine.mensch`).
- The loader calls the exported `main` (else the start of the text) with
  `JSR`, Z = 0 and the kernel's stack; `A` on return is the exit status,
  which `RUN` reports when it is non-zero.
- Programs link their own copy of the runtime (`mega65::RUNTIME`: the
  base-page registers and `__m65_enter`/`__m65_leave`), so they never share
  scratch state with the kernel. They reach the kernel only through the trap
  table, so a file with undefined references is rejected.
- Traps and interrupts switch back to the kernel's bank 0 (through the core's
  `$FF00` and IRQ entries) and back to the program afterwards. A trap's
  pointer arguments are program addresses, which the kernel cannot see
  directly: GEMDOS asks `machine_caller_bank()` who called and copies path
  strings (`Fopen`, `Dsetpath`), DTAs (`Fsfirst`, `Fsnext`) and data
  (`Fread`, `Fwrite`, through a 128-byte buffer) between the program and
  kernel buffers. Trap `$0102` (`Fcount`) sets the byte count for the next
  `Fread`/`Fwrite`, since the trap ABI has two arguments.
- Program code gets a base-page budget of `PROGRAM_BASE_PAGE_BUDGET` bytes
  (plus the 16 runtime bytes); the loader rejects a program whose base-page
  segment does not fit.

The kernel side needs these machine services, provided by the MEGA65
runtime (`mega65::WINDOW_RUNTIME` for split kernels, `BANK0_SERVICES`
otherwise): `machine_program_start`, `machine_program_end`,
`machine_base_page_start`, `machine_program_bank`, `machine_caller_bank`,
and `machine_call(entry)` to enter the program. The x65-alike's machine
driver stubs `exec_program` out and has trivial `machine_caller_bank` and
copy routines.

Mensch's `addr(global)` builtin gives a near byte pointer to a global
variable, so buffers such as the DOS sector buffer live in BSS instead of at
machine-specific fixed addresses.
