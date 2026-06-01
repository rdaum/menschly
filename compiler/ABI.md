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
