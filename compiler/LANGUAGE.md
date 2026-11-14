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

Conditions (`if`, `while`, `until`) compare with `==` `!=` `<` `<=` `>` `>=`
and combine with `and` / `or`. Ordering compares are signed when either side
has a signed type.

An operation works at the width and signedness of its left operand, or of the
right one when the left is a constant (`100 - x`).

Arithmetic on constants is folded at compile time: `const`s, numbers and
expressions built from them (`WIDTH * 2 - 1`, `-5`). Constants may be
negative (`const DOWN = -1`). A constant must fit its destination, either as a
signed or as an unsigned value.

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
