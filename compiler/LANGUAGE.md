# Mensch Language Reference (summary)

A quick reference to what the compiler accepts today. See `ABI.md` for how
each construct is represented on the 65816 and the MEGA65.

## Types

| Type | Size | Notes |
|---|---|---|
| `u8` `i8` | 1 | |
| `u16` `i16` | 2 | |
| `u24` | 3 | |
| `u32` `i32` | 4 | MEGA65 only for now |
| `near ^T` | 2 | address in the current bank / CPU space |
| `direct ^T` | 2 | direct-page (base-page) data |
| `far ^T` | 4 | 32-bit flat address (the 65816 uses 24 bits) |
| `[N]T` | N × size | arrays, indexed from 0; `N` is a constant expression (`[ROWS * 2]u8`) |
| `parallel [N]R` | N × size | an array of records stored field by field (below) |
| `record ... end` | sum of fields | fields at fixed offsets, no padding |
| `procedure(...)`, `function(...): T` | 2 | routine pointers (below) |

Integer types convert implicitly between types of the same width. Casts
(`u8(x)`, `i16(x)`, `u32(x)`, ...) truncate or widen. Widening a signed value
copies its sign bit; widening an unsigned one fills with zeros.

## Operators

From loosest to tightest binding:

| Operators | |
|---|---|
| `\|` | bitwise or |
| `xor` | bitwise exclusive or |
| `&` | bitwise and |
| `<<` `>>` | shifts by a constant count; `>>` on a signed type is arithmetic |
| `+` `-` | |
| `*` `/` `%` | division truncates toward zero; `%` takes the dividend's sign |
| unary `-` | |

Conditions (`if`, `while`, `until`) compare with `==` `!=` `<` `<=` `>` `>=`,
combine with `and` / `or`, negate with `not`, and group with parentheses. A
bare value means "is not zero" (`if ready`). Ordering compares are signed
when either side has a signed type.

An operation works at the width and signedness of its left operand, or of the
right one when the left is a constant (`100 - x`).

Arithmetic on constants is folded at compile time: `const`s, numbers and
expressions built from them (`WIDTH * 2 - 1`, `-5`). Constants may be
negative (`const DOWN = -1`), be defined from earlier ones
(`const LAST = COUNT - 1`), and have a declared type (`const MASK: u16 = $ff`).
A constant must fit its destination, either as a signed or as an unsigned
value. `size(T)` is the size of a type in bytes.

## Data

```text
const SINE: [8]u8 = [0, 49, 90, 117, 127, 117, 90, 49]
const GRID: [2][3]u8 = [[1, 2, 3], [4, 5, 6]]
const TITLE: [8]u8 = "MENSCH"                 # NUL-padded
const NAMES: [2]near ^u8 = ["alpha", "beta"]
const HANDLERS: [2]Op = [on_fire, on_move]    # routine pointers

var actors: [2]Actor = [((1, 2), 1, "ant"), ((10, 20), -1, "bee")]
var score: u16 = 500
```

A `const` with an array or record type is a read-only table; a `var` with
`= ...` starts with that value. Arrays are written `[...]` and records `(...)`,
with every element or field given in order. Elements are constants, string
literals and `addr(global)` (for pointers), or routine names (for routine
pointers).

## Elements, fields and pointers

Element and field accesses compose, and can be read or assigned:
`actors[i].pos.x`, `grid[y][x]`, `p^.speed`, `p^.items[i]`. Indexes are 8- or
16-bit integers; a constant index is checked against the array's length.
Arrays indexed by a variable must be global (or reached through a pointer).

A pointer indexes like an array of its target type: for `p: near ^Actor`,
`p[i]` is the element `i * size(Actor)` bytes past `p^`, and `p[i].x` its
field. The pointer carries no length, so only a negative constant index is
rejected. Pointer arithmetic counts bytes: `p = p + size(Actor)`.

`addr(x)` is the 16-bit address of a global, or of an element or field of
one or of memory reached through a near pointer: `addr(nodes[i])`,
`addr(actors[i].name)`, `addr(p^.items[2])`. Locals and parameters have no
address.

A record can point at its own type, for linked structures:

```text
type Node = record(value: u16, next: near ^Node)
total = total + head.next^.value
```

It cannot contain itself other than through a pointer.

A `parallel` array of records stores each field as its own column (all the
`x` fields, then all the `y` fields, ...), the layout 65xx code favours:
`actors[i].x` is then indexed by `i` times the field's size, not the
record's, which for byte fields is just `lda actors_x,x`. A parallel
array's elements are reachable only through their fields, and it must be
global.

```text
var actors: parallel [16]Actor
actors[i].pos.x = actors[i].pos.x + u8(actors[i].dx)
```

## Statements

```text
if cond ... elsif cond ... else ... end
while cond ... end
repeat ... until cond
for i = 0 to 9 ... end          # inclusive; `downto` counts down
```

A `for` loop runs zero times when the start is already past the end, and
stops after the pass where the variable equals the end, so it works up to a
type's largest value. The end is evaluated on every pass.

## Routines

```text
procedure draw(x: u8, y: u8)              # parameters in fixed storage
stack function read(handle: u16): u16      # parameters on the stack
reentrant function depth(node: u16): u16   # recursive / interrupt-safe
interrupt procedure tick()
far procedure entry()                      # long call (65816)
```

| Kind | Arguments | Locals | Use for |
|---|---|---|---|
| (plain) | the routine's own storage | static | most code: smallest and fastest |
| `stack` | the stack | static | routine pointers, traps, calls from any module |
| `reentrant` | the stack | a stack frame | recursion, routines called from interrupts |

Locals start undefined, except in `reentrant` routines, where they start at
zero. An interrupt handler may only call `reentrant` routines (or ones
without parameters).

Plain routines that are not exported, and `external` declarations, can take
small arguments in registers and return a flag in the carry:

```text
function pick(index: u8 in x, bias: u8 in a): u8   # a byte in a, x or y
procedure add(amount: u16 in xy, extra: u8)         # a word in ax or xy
function is_even(value: u8 in a): u8 in carry       # returns 0 or 1

if is_even(n) ...                                   # branches on the carry
flag = is_even(n)                                   # 0 or 1
```

A carry result is a one-byte function returning 0 or 1 (any non-zero value
counts as 1). Arguments are evaluated left to right.

## Routine pointers

```text
type Handler = stack procedure(event: u16)
type Compare = stack function(a: u16, b: u16): i16
type Thunk = procedure()

var on_event: Handler

on_event = handle_key          # a routine named without arguments
on_event(42)                   # an indirect call
HANDLERS[i](42)                # through an element or field
COMMANDS[i].run(argument)
```

A routine type with parameters must be `stack` (or, equivalently,
`reentrant`), since the arguments of an indirect call travel on the stack;
`stack` and `reentrant` routines with a matching signature can be assigned.
Routines without parameters can be pointed at whatever their ABI, except
`far` and `interrupt` routines.

## Modules

```text
module Vic4
export const JOY_FIRE = $10
export type Point = record(x: u8, y: u8)
export procedure vic4_init() ... end

module Game
import Vic4                     # vic4_init, JOY_FIRE and Point are now known
```

`import M` takes in what module `M` exports: its exported routines (called
as if declared `external`), and its `export const` and `export type`
declarations. Only scalar constants can be exported. Names a module imports
are its own: they do not pass on to the modules that import it, and an
imported name may not clash with one already declared. Import cycles are
rejected.

The compiler finds an imported module among the sources it is given
together (every module of a MEGA65 kernel or game, for instance) and, for
`menschc`, in the directories named with `-I`. The imported module must
still be compiled and linked into the program.

## Names

Module-level names (globals and routines) cannot be assembler register
names: `a`, `x`, `y`, `z`, `s`, `q`, `sp`.
