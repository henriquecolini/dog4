# From dog3 to dog4

Most dog3 programs run unchanged: all the dog3 examples in `tests/dog3` produce
exactly dog3's output except the two listed in
[`tests/dog3/EXPECTED_DIFFERENCES.md`](tests/dog3/EXPECTED_DIFFERENCES.md).
This page lists everything that behaves differently.

## Status codes are gone

dog3 strings carried a hidden status code; dog4 has real types (`null`,
`bool`, `num`, `str`, `arr`, `map`, `fn`). Conditions use **truthiness**
instead: `false null 0 "" [] [=]` are falsy. Comparisons return booleans, so
`if { eq $a $b } …` works as before, and so does `if { search $text x } …`
(no match is `""`, which is falsy).

* `status` no longer exists.
* Builtins that used to "return falsy" on bad input now either raise an error
  (when an argument can't be converted, e.g. `add word 10`) or return `null`
  (`first`/`last` past the end, `gron` of invalid JSON). Recover with `try`.
* `eval` raises the error instead of returning its message.
* `get`/`post` raise an error for non-2xx responses.

## Bare words can be numbers, and numbers add up

A bare word that is a literal is that value: numbers (`5`, `-2.5`, `1e3`,
`1_000`), `true`, `false` and `null`. A number prints in its own form, so
`pln 007 1.50` prints `7 1.5` where dog3 printed `007 1.50`; quote words that
must stay text (`"007"`). `null` as a bare word is nothing, which prints as
nothing.

Blocks fold their outputs by type, and numbers **add**:

```dog
pln { put 1; put 2 } { put "1"; put "2" } { put 1; put "2" }
```
```text
3 12 12
```

So a `while` that `put`s a counter now sums it. To get dog3's text, output
text: `put "$i"`.

Pieces of a word glue with `+` as well, so if `x` is 5, `$x$x` is `10`; inside
quotes, `"$x$x"` is always text (`"55"`).

## `%rest` parameters are arrays

```dog
fn count (%args) { put { len $args } }
fn forward (%args) { count $%args }
pln { count a b c } { forward a b c }
```
```text
3 3
```

`pln $args` prints an array literal such as `[a b c]`. Forward rest arguments with `$%args`
(dog3 passed them as one space-joined string). Natives receive them as
separate arguments.

## Names

* Variable names are letters, digits and `_`, so `$x!` reads `x` followed by
  `!` (dog3 read a variable called `x!`). `$x.y` and `$x[0]` read inside
  values: write `${x}.txt` for text.
* Functions and variables share one namespace. A variable named like a
  builtin hides it in that scope.
* Script functions may now be named like builtins (`fn div …`); dog3 refused.
* Unknown variables and commands are reported before the program runs.

## Words

* Commas and mid-word apostrophes are plain text: `pln Hello, I'm here`.
* `//` starts a comment only between tokens, so `pln http://x.com` works.
* A backslash only escapes punctuation and spaces (`\$`, `\"`, `\ `) or makes
  a control character (`\n`, `\t`, `\r`, `\0`). dog3 dropped the backslash of
  any other escape, so `"\d+"` was `d+`; dog4 reports an error. Write `\\d+`,
  or a raw string: `"""\d+"""`.
* `range` always returns an array of numbers. dog3's `range min max step sep`,
  which returned text, is gone: write `{ join { range min max step } sep }`.

## New things

Raw strings (`"""…"""`), slices (`$list[1..3]`), expressions in `( … )` and
after `=` (commands go in blocks there: `x = { len $list }`), typed and
overloaded functions, lambdas
and closures, value bodies (`fn f (x) ($x * 2)`), references (`&x`), `let`,
`try`/`catch`/`throw`, `break`/`continue`, map and array literals, modules
(`mod`, `use`, `pub`), and a sandbox for running untrusted code. See the [README](README.md) and the
[language reference](docs/language.md).
