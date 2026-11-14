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
| `[N]T` | N × size | arrays, indexed from 0 |
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
Pointer arithmetic counts bytes: `p = p + size(Actor)`.

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

## Routine pointers

```text
type Handler = reentrant procedure(event: u16)
type Compare = reentrant function(a: u16, b: u16): i16
type Thunk = procedure()

var on_event: Handler

on_event = handle_key          # a routine named without arguments
on_event(42)                   # an indirect call
```

A routine type with parameters must be `reentrant`, since the arguments of
an indirect call travel on the stack, and only routines with a matching
signature can be assigned. Routines without parameters can be pointed at
whatever their ABI, except `far` and `interrupt` routines.

## Names

Module-level names (globals and routines) cannot be assembler register
names: `a`, `x`, `y`, `z`, `s`, `q`, `sp`.
