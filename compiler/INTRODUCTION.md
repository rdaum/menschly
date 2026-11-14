# An Introduction to Mensch

Mensch is a small systems language for the 65xx family: the 65816 and the
MEGA65's 45GS02. It exists because C and Pascal were designed for machines
the 6502 is not like. Both assume:

- a cheap stack you can index;
- registers at least as wide as an `int`;
- one kind of pointer;
- that every function may recurse.

On a 65xx none of that holds. The hardware stack is 256 bytes on the 6502
and has no stack-relative addressing before the 65816. The accumulator is
8 bits wide, and memory comes in very different kinds: the base page,
ordinary memory, banks beyond 64 KB. A C compiler for the 6502, such as
cc65, keeps a software stack in a pointer pair and does most of its work in
runtime helpers that push, pop and index it.

Mensch keeps the parts of a high-level language that cost nothing:
- types and records;
- structured control flow;
- named storage;
- expressions and calls.

It drops or makes explicit the parts that cost a lot on this CPU.

Each section below shows one difference, with Mensch next to the C you
would write instead.
- `LANGUAGE.md` is the reference.
- `ABI.md` describes the generated code.

## At a glance

Five small routines (string length, summing a byte table, moving actors,
string compare, checksum) were written in C and in Mensch, and their code
sizes measured. The C was compiled with `cc65 -O --cpu 65c02`; the Mensch
was compiled for the MEGA65.

| Compiler | Bytes |
|---|---|
| cc65 -O (65C02) | 290 |
| Mensch, plain routines | 208 |
| Mensch, actors as a `parallel` array | 169 |
| Mensch, every routine `stack` | 364 |
| Mensch, every routine `reentrant` | 376 |

The plain Mensch routines are 28% smaller than C, and 42% smaller with the
data laid out for the CPU (`parallel`). They are also much faster: none of
them calls a runtime helper, while the cc65 code calls `pushax`,
`ldaxysp`, `addeqysp` and `incsp` in nearly every routine.

The last two rows show the cost of the C model. Make every routine
reentrant, as C does, and Mensch is no smaller than cc65. The savings come
from not paying for reentrancy where it isn't needed.

## 1. Storage is static unless you ask

In C, locals and arguments live on the stack, because any function might
recurse. On a 6502 that means a software stack: every local is
`lda (sp),y` with `y` loaded first, and every call pushes its arguments
through a helper.

A plain Mensch routine gives each local and parameter a fixed address
instead. The storage pass shares addresses between routines that can
never be active at the same time, so the memory cost stays close to what a
stack would use.

```c
unsigned int sum_counts(void) {
    unsigned int total = 0;
    unsigned char i;
    for (i = 0; i < 64; ++i) total += counts[i];
    return total;
}
```

```text
export function sum_counts(): u16
    var total: u16
    var i: u8
    total = 0
    for i = 0 to 63
        total = total + u16(counts[i])
    end
    return total
end
```

cc65 compiles the C version to calls to `push0`, `decsp1`, `addeqysp`,
`ldaxysp` and `incsp3`. The Mensch version uses no helpers at all:

```text
__for_top_2:
	ldx z:__m65_tmp+2
	lda a:counts,x
	clc
	adc z:__m65_tmp
	sta z:__m65_tmp
	bcc @m65_carry_0
	inc z:__m65_tmp+1
@m65_carry_0:
	txa
	cmp #$3f
	beq __for_done_3
	inc z:__m65_tmp+2
	bra __for_top_2
```

When you do need recursion, you ask for it (section 8).

## 2. You can see where things live

C hides storage behind the compiler and a linker script. cc65 adds
`register`, `#pragma zpsym` and `--static-locals`, which are hints and
switches rather than parts of the language.

In Mensch, storage is part of the declaration:

```text
fixed($d020) volatile var border: u8      # a hardware register
var buffer: [128]u8                       # ordinary static memory

procedure poke_screen(value: u8)
    direct var cursor: near ^u8           # this local in the base page
    static var calls: u16                 # keeps its value between calls
    ...
end
```

- `volatile` means every access happens exactly as written. The optimizer
  treats only compiler-owned storage as plain RAM, so a `fixed` I/O
  register is never cached, merged or dropped.
- A `fixed` address is a declaration, not a cast of an integer to a
  pointer.

## 3. The base page is a register file

The 65xx has three real registers. Its base page (zero page) works as a
256-byte register file: shorter and faster to reach than other memory, and
the only place an indirect pointer can live.

C compilers for the 6502 use a few base-page bytes for their own runtime
and leave the rest to you. The Mensch compiler manages the base page:

- Expression temporaries go in base-page scratch registers (`__m65_tmp`
  on the MEGA65), as do locals that are not live across a call.
- Parameters and `direct` locals get base-page slots.
- After storage is assigned, the compiler overlays the slots of routines
  that can never be running at the same time. It works this out from the
  call graph, treating interrupt handlers and calls it cannot follow
  conservatively.

None of this needs any annotation. On the Hellord kernel the overlay alone
cut base-page use from 150 to 122 bytes.

## 4. No integer promotion

C promotes every `char` operand to `int` before arithmetic, which is at
least 16 bits. A compiler may narrow the arithmetic back down only when it
can prove the result is the same, and 6502 compilers often can't. It also
makes the rules surprising:

```c
unsigned char a = 200, b = 100;
if (a + b > 255) { ... }     /* true: the sum is an int, 300 */
unsigned char c = a + b;     /* 44 */
```

In Mensch an operation works at the width of its operands. `u8 + u8` is an
8-bit add, a single `clc / adc`. A wider result is something you write:

```text
var a: u8
var b: u8
var sum: u16

sum = u16(a) + u16(b)         # 16-bit, because you said so
a = a + b                     # 8-bit, wraps
```

Casts truncate or extend (`u8(x)`, `i16(x)`, `u32(x)`); signed values
extend their sign bit. The same holds for comparisons: `x < 10` with a
`u8` is one `cmp #10`.

## 5. Pointers say what they point into

C has one pointer type. On the 65816 a pointer might mean three things:

- an address in the current bank (16 bits);
- an offset into the direct page;
- a full 24-bit address.

On the MEGA65 there are also 28-bit flat addresses beyond 64 KB. cc65
has no far pointers at all, and 65816 C compilers grow `near`/`far`
keywords.

Mensch has a pointer kind for each:

```text
var screen: near ^u8          # 16-bit, current bank: (bp),z / (dp),y
var cursor: direct ^u8        # direct-page data
var ram: far ^u8              # 32-bit flat: [bp],z on the MEGA65

ram = $00040000              # the second 256 KB of chip RAM
ram^ = $41
```

Each kind compiles to the addressing mode that fits it. Pointer arithmetic
counts bytes (`p = p + size(Actor)`) rather than elements, which matches
what the CPU does. It also means a scaled index never hides a multiply.

## 6. Data laid out for the CPU: `parallel` arrays

An array of records in C stores whole records one after another. To reach
`actors[i].x`, the code must compute `i * sizeof(struct actor)`. On a
CPU without a multiply instruction that is a shift-and-add sequence, or a
helper call, before every access.

65xx programmers have long kept one table per field instead, so the index
register reaches any field directly. Mensch makes that layout a single
word:

```c
struct actor { unsigned char x, y; signed char dx; };
struct actor actors[8];

for (i = 0; i < 8; ++i) actors[i].x += actors[i].dx;
```

```text
type Actor = record(x: u8, y: u8, dx: i8)
var actors: parallel [8]Actor

for i = 0 to 7
    actors[i].x = actors[i].x + u8(actors[i].dx)
end
```

The source is the same as it would be for an ordinary array. The generated
loop indexes each column with `x` and needs no address arithmetic:

```text
	ldx z:__m65_tmp
	lda a:actors,x          ; x column
	sta z:__m65_tmp+1
	lda a:actors+16,x       ; dx column
	clc
	adc z:__m65_tmp+1
	sta a:actors,x
```

That loop is 28 bytes; cc65's is 75 and the record-layout Mensch loop is
67. Changing the layout means changing one word in the declaration.
Nothing in the code that uses the array changes.

## 7. Byte-sized indexes

C converts an array index to `int` and computes a 16-bit address. In
Mensch, an array of at most 256 bytes is indexed with a one-byte offset,
computed in 8-bit arithmetic, and the access is `lda table,x`. You get
this for any array small enough; there is nothing to annotate.

```text
const SINE: [64]u8 = [...]
var wave: [64]u8

for i = 0 to 63
    wave[i] = SINE[(i + phase) & 63]
end
```

Constant indexes are checked against the array's length at compile time.

## 8. You choose what each routine costs

C has one kind of function: reentrant, with arguments on a stack. Mensch
has three, and you pick per routine:

| Kind | Arguments | Locals | Use for |
|---|---|---|---|
| (plain) | the routine's own storage | static | most code: smallest and fastest |
| `stack` | the stack | static | routine pointers, traps, calls between modules |
| `reentrant` | the stack | a stack frame | recursion, code called from interrupts |

```text
procedure draw_cell(x: u8, y: u8)              # plain
stack function read_block(lba: u16): u16        # callable through a pointer
reentrant function sum_to(n: u16): u16         # recursive
    if not n
        return 0
    end
    return n + sum_to(n - 1)
end
```

The compiler enforces the rules that C leaves to discipline. An interrupt
handler may only call `reentrant` routines, or routines without
parameters. A routine pointer with parameters must have a `stack` type,
because an indirect call can only pass arguments on the stack.

## 9. Arguments in registers, results in the carry

Hand-written 65xx code passes small values in A, X and Y and returns
yes/no answers in the carry flag. A test is then one `bcs` after the
`jsr`, with no compare.

C has no way to say any of this. A cc65 function returning a `char` sets
A and X, and the caller compares the result against zero.

Mensch can say it for plain routines and `external` declarations:

```text
function pick(index: u8 in x, bias: u8 in a): u8
    return TABLE[index] + bias
end

function is_even(value: u8 in a): u8 in carry
    if value & 1
        return 0
    end
    return 1
end

if is_even(count)          # jsr is_even / bcc ...
    total = pick(2, 5)     # ldx #2 / lda #5 / jsr pick
end
```

This is also how Mensch calls existing assembly or ROM routines without
glue code:

```text
external function kernal_getin(): u8                        # result in A
external function chkin(device: u8 in x): u8 in carry       # C = error
```

## 10. Interrupt handlers are a routine kind

In C an interrupt handler needs compiler-specific attributes, assembly
stubs, or both. You then have to work out by hand which runtime state the
handler can disturb: the software stack pointer, the compiler's base-page
temporaries, any helper's scratch bytes.

In Mensch:

```text
vector irq = tick
fixed($dc0d) volatile var cia1_icr: u8
var ticks: u16

interrupt procedure tick()
    var ack: u8
    ack = cia1_icr              # acknowledge the timer
    ticks = ticks + 1
    return
end
```

The compiler saves the registers, plus whichever runtime base-page
registers the handler can disturb, and ends the routine with `rti`. It
gives the handler storage that never overlaps the code it interrupts. It
rejects calls from the handler to routines that are not safe to re-enter.

Handlers that need control can ask for it, for example the 65816 `COP`
trap handler:
- `interrupt(cop, preserve = x, y, p, dbr)` chooses what is saved;
- `interrupt(raw)` saves nothing.

## 11. System calls are an expression

An operating-system call on the 65816 is a `COP` instruction with a
signature byte. Writing one in C needs inline assembly to load the
registers and read the result. In Mensch it is an expression:

```text
const BIOS_FAMILY = $0d
const BIOS_BCONOUT = 3

function bconout(dev: u16, ch: u16): u16
    return trap(BIOS_FAMILY, BIOS_BCONOUT, dev, ch)
end
```

The handler side reads the signature and registers the trap was raised
with, using `cop_signature()`, `cop_a()`, `cop_x()` and `cop_y()`.

## 12. Code beyond 64 KB without far calls in the source

The MEGA65 sees 64 KB of address space at a time out of a much larger
memory. The Hellord kernel builds modules into *window banks*: 16 KB banks
mapped in at `$8000` while they run.

A call into a window-bank module is written like any other call. The build
puts a small stub under the routine's name. The stub maps the bank, calls the
routine, and restores the previous mapping on return. Interrupts taken
while a window bank is mapped are handled too.

C compilers for the 6502 reach this with overlays and linker magic, if at
all. In Mensch it is a choice made when building a module, and the source
doesn't change.

## 13. Tables are data, not code

C initializes arrays of records and pointers well. The issue on small
machines is where the data ends up. A `const` table of pointers may still
be copied to RAM at startup, and a table of function pointers needs
casts.

In Mensch:
- a `const` with an array or record type is a read-only table, emitted
  once next to the code;
- a `var` with an initializer starts with that value;
- elements can be strings, addresses of globals, or routine names.

```text
type Handler = stack procedure(arg: u16)
type Command = record(name: near ^u8, run: Handler)

const COMMANDS: parallel [3]Command = [
    ("DIR", run_dir),
    ("TYPE", run_type),
    ("COPY", run_copy),
]

for i = 0 to 2
    if name_is(COMMANDS[i].name)
        handler = COMMANDS[i].run
        handler(argument)
        return
    end
end
```

## 14. Loops that reach the top of a type

In C, a loop over every value of an `unsigned char` is a classic trap:

```c
unsigned char i;
for (i = 0; i <= 255; ++i) { ... }      /* never ends */
```

A Mensch `for` loop is inclusive and stops after the pass where the
variable equals the end, so this works and compiles to a byte counter:

```text
for i = 0 to 255
    page[i] = 0
end
```

`downto` counts down the same way.

## What C still does better

Mensch is deliberately smaller than C, and some of the missing pieces are
real costs:

- **Portability.** Mensch code is 65xx code. That is the point, but you
  can't take it to another CPU.
- **Expressions.** Comparisons are conditions, not values (there is no
  `x = a < b`). Records and arrays are passed by pointer, never by value.
- **Recursion is opt-in.** A plain routine that ends up re-entered (from
  an interrupt, or through a pointer back into itself) is a bug. The
  compiler catches the interrupt case but cannot check everything.
- **32-bit arithmetic** works on the MEGA65 only; the 65816 backend stops
  at 24 bits for now.
- **Ecosystem.** There is no standard library, preprocessor or
  separate-compilation tooling beyond modules and `external` declarations.

Mensch is meant for the code where the machine matters most: an operating
system, drivers, and games.
