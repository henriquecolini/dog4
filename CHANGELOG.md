# Changelog

Every `dog` block in this file is run by the test suite (`cargo test --test
docs`). A block followed by a `text` block starting with `error:` must fail
with that message, so the "before" examples below show the real errors that old
scripts will hit.

## 2026-10-16 — stricter syntax, typed literals, a bigger standard library

This release changes how some existing scripts parse and behave. The first part
of this entry is a **migration guide written to be followed mechanically**, by
a person or an AI, to update old dog4 scripts. The second part lists what's
new, and the third lists changes for Rust hosts.

### Part 1: updating existing scripts

#### How to work through a script

1. Compile it without running it: `dog4 --disasm script.dog`. Syntax errors
   are reported one at a time, so fix the reported error and repeat until it
   compiles. Rules **R1–R3** below are the syntax errors you will see.
2. Run it (or its tests). Rule **R4** (`range` with a separator) fails when it
   runs.
3. Rules **R5–R10** are **silent**: the script still runs but may behave
   differently. Search for them with the patterns given in each rule and
   decide case by case. Don't skip this step: the silent changes are the
   ones that cause real bugs.

Quick reference:

| rule | change | loud or silent | find it with |
|---|---|---|---|
| R1 | commands are not allowed inside `( … )` or after `=` | syntax error | `dog4 --disasm` |
| R2 | unknown escapes such as `\d` are errors | syntax error | `grep -nE '\\[A-Za-z1-9]'` |
| R3 | `"""` starts a raw string | syntax error (usually) | `grep -n '"""'` |
| R4 | `range min max step sep` was removed | runtime arity error | `grep -n 'range'` |
| R5 | bare `null`, `007`, `1.50`, `1e3`, `1_000` are now typed | **silent** | regexes in R5 |
| R6 | `replace` uses regular expressions | **silent** | `grep -n 'replace'` |
| R7 | nested `null` prints as `null`, not `(null)` | **silent** (output text) | `grep -n '(null)'` in expected outputs |
| R8 | module functions are named `json.parse` in errors | **silent** (error text) | `grep -n 'message'` |
| R9 | `\5`-style escapes that forced a word to stay text | syntax error | covered by R2 |
| R10 | new builtin names | no effect on old code | — |

---

#### R1. Commands only run inside blocks

A **command** is a name followed by arguments: `len $x`, `upper hello`,
`$f a b`, `json.parse $text`. Commands used to be allowed in expression
positions; now they must be inside `{ … }`. Expression positions are:

* inside `( … )`, including `if ( … )` and `while ( … )` conditions;
* the right side of `=`, `+=`, `-=`, `*=`, `/=` and `let x =`;
* after `return`, `clear`, `break` and `throw`;
* the arguments of an expression-style call `f( … )`.

```dog
x = [1 2 3];
pln (len $x)
```
```text
error: `len` followed by arguments is a command, and commands only run inside blocks: write `{ len … }`, or call it as `len(…)`
```

```dog
x = upper hello
```
```text
error: `upper` followed by arguments is a command, and commands only run inside blocks: write `{ upper … }`, or call it as `upper(…)`
```

**Rewrite:** replace the parentheses with braces, or wrap the command in braces.
Either form of the fix is fine:

| before | after | or |
|---|---|---|
| `(len $x)` | `{ len $x }` | `len($x)` |
| `x = upper hello;` | `x = { upper hello };` | `x = upper(hello);` |
| `n = (len $xs) + 1;` | `n = { len $xs } + 1;` | `n = len($xs) + 1;` |
| `if (eq $a $b) { … }` | `if { eq $a $b } { … }` | `if ($a == $b) { … }` |
| `while ($i < (len $l)) { … }` | `while ($i < { len $l }) { … }` | |
| `return upper $s;` | `return { upper $s };` | `return upper($s);` |
| `throw fmt $x;` | `throw { fmt $x };` | |
| `y = $f a b;` | `y = { $f a b };` | `y = $f(a, b);` |
| `max(len $l, 2)` | `max({ len $l }, 2)` | `max(len($l), 2)` |
| `r = json.parse $text;` | `r = { json.parse $text };` | `r = json.parse($text);` |
| `(abs -3)` | `{ abs -3 }` | `abs(-3)` |

```dog
x = [1 2 3];
n = { len $x } + 1;
y = upper(hello);
pln (len($x)) $n $y (if { contains $x 2 } yes else no) (max({ len $x }, 5))
```
```text
3 4 HELLO yes 5
```

Notes for the rewrite:

* Inside a block a command keeps working exactly as before. `{ … }` used as an
  *argument* or *expression* is never discarded; only a block used as a
  whole *statement* is.
* The two forms differ in how arguments are read. Inside `{ … }` they are
  command-mode words (`{ upper hello world }` gets two words), while inside
  `f( … )` they are comma-separated expressions (`upper("hello world")`). When
  in doubt, use braces: they are exactly what the old code meant.
* Parentheses that hold only operators and values are still fine:
  `($x + 1)`, `($a == $b)`, `(null)`, `(1..4)`.
* `fn f (x) value` bodies already took a single value, not a command, so they
  don't change.

#### R2. Unknown escapes are errors

A backslash may only come before ASCII punctuation or whitespace (which gives
that character, such as `\$`, `\"`, `\\`, `\;` or `\ `), or form `\n`, `\t`,
`\r` or `\0`. Anything else used to silently drop the backslash, so `"\d+"`
meant `d+`. Now it is an error, in `"…"`, in `'…'` and in bare words alike.

```dog
pln { matches "abc123" "\d+" }
```
```text
error: unknown escape `\d`; write `\\` for a backslash, or use a raw string: `"""…"""`
```

**Find it:** `grep -nE '\\[A-Za-z1-9]' *.dog` (then ignore `\n`, `\t`, `\r`).

**Rewrite:**

* **A regex** (the common case): put it in a raw string, `"""\d+"""`. Raw
  strings keep every backslash and have no interpolation.
* **A regex that needs interpolation:** double the backslashes in a normal
  string, `"\\d+$suffix"`.
* **Windows-style paths** and other literal backslashes: `"""C:\Users"""` or
  `"C:\\Users"`.
* **What the old code actually got:** if a script worked *because* the backslash was dropped
  (`\s` meant `s`), just delete the backslash. Check which meaning was
  intended. In a regex, the old code was usually buggy and the raw string
  fixes it.

```dog
pln { matches "abc123" """\d+""" } { replace "a1b22" """\d+""" "#" } "C:\\Users" """C:\Users"""
```
```text
true a#b# C:\Users C:\Users
```

#### R3. Three or more quotes start a raw string

`"""…"""` (or with more quotes: `""""…""""`) is a raw string. It ends at the
next run of exactly as many quotes, and a longer run inside is an error. Old
code with three quotes in a row (an empty string `""` glued to a `"…"`) will
now fail as an unclosed raw string or read differently. **Rewrite:** separate
them, or use `''` for the empty string.

```dog
pln """a "quoted" word""" """"three """ inside""""
```
```text
a "quoted" word three """ inside
```

A raw string that spans lines must start its text on the line after the
opening quotes and put the closing quotes on their own line; that line's
indentation is removed from every line (like C# raw strings).

#### R4. `range` always returns an array

The four-argument `range min max step sep`, which returned text, is gone.

```dog
pln { range 0 10 3 , }
```
```text
error: no overload of `range` takes 4 arguments; it has fn range(max: num), fn range(min: num, max: num), fn range(min: num, max: num, step: num)
```

**Rewrite:** `{ range min max step sep }` → `{ join { range min max step } sep }`.

```dog
pln { join { range 0 10 3 } , }
```
```text
0,3,6,9
```

#### R5. Bare literals are typed (silent)

A bare word (unquoted, unescaped, with no `$`) that is a **literal** is now
that value:

* `true`, `false` and **`null`**. `null` used to be the text `"null"`.
* **Any number written as in expressions**: an optional `-`, digits with
  optional `_` separators, an optional `.fraction` and an optional exponent.
  Previously only text that printed back identically was a number, so
  `007`, `1.50`, `1e3`, `1_000`, `-0` and `00` used to be text and are now
  the numbers 7, 1.5, 1000, 1000, 0 and 0.

Still text: `+5`, `5px`, `inf`, `NaN`, `1.`, `.5`, `1.2.3`, `0x1F`,
`2024-01-01`, and anything quoted or escaped.

```dog
pln 007 1.50 1e3 1_000 "007" { type null } { type "null" } { type 2024-01-01 }
```
```text
7 1.5 1000 1000 007 null str str
```

What changes, and how to spot it:

| old code | old meaning | new meaning |
|---|---|---|
| `pln 007` | prints `007` | prints `7` |
| `put 1.50` | the text `1.50` | the number `1.5` |
| `send 0042 hi` | text `0042` | number `42` |
| `put null` | the text `null` | nothing |
| `if null …` (as a word) | truthy (non-empty text) | falsy |
| `for x in null { … }` | one iteration, `x` = `"null"` | no iterations |
| `[null 1]` | `["null" 1]` | `[null 1]` |
| `[007 = a]`, `[null = b]` | keys `"007"`, `"null"` | keys `"7"`, `""` |
| `{ like 1 1.0 }` | `false` | `true` (both are the number 1) |
| `{ put 1.0; put 2 }` | text `1.02` | number `3` |

```dog
pln { put if null yes else no } [null = b, 007 = a] { like 1 1.0 } { put 1.0; put 2 }
```
```text
no ["" = b, 7 = a] true 3
```

Only **command-mode** words change: command arguments, `[ … ]` elements and
map keys, and the parts of `if`/`while`/`for`. Expressions (inside `( … )`,
after `=`, in `f( … )`) already read `0042` as 42 and `null` as null, so
`id = 0042;` is unchanged.

**Find it:**

* `grep -nwE 'null'`, then check each one that is a bare command-mode word.
* Leading zeros and non-canonical numbers:
  `grep -nE '(^|[[:space:][{(;,=])-?0[0-9]'`,
  `grep -nE '[0-9]\.[0-9]*0([[:space:]]|[];}),]|$)'`,
  `grep -nE '[0-9][eE][+-]?[0-9]|[0-9]_[0-9]'`.

**Rewrite:** quote the word when it must stay text: `"007"`, `"null"`,
`"1.50"`. Typical cases are IDs, phone numbers, zip codes, version-like
strings and fixed decimal amounts (`put "$price"` won't help: the value is
already a number; write the literal as `"1.50"`, or format with
`{ fixed $price 2 }`).

#### R6. `replace` uses regular expressions (silent)

`replace text pattern to` is dog3's regex replace again: every match of
`pattern` is replaced, and in `to`, `$1` / `${name}` stand for groups and `$$`
for a literal `$`. It used to replace a literal substring.

```dog
pln { replace "a.b.c" . - } { replace "a.b.c" { regex_escape . } - } { replace "1+1" """\+""" " plus " }
```
```text
----- a-b-c 1 plus 1
```

**Find it:** `grep -nw 'replace'`. For every call:

* If the **pattern** contains any of `. * + ? ( ) [ ] { } | ^ $ \`, it now
  means something else. To keep matching it literally, write
  `{ replace $t { regex_escape $from } $to }`, or escape the character in a raw
  string (`"""\."""`).
* If the **replacement** contains `$`, it now refers to a group. Write `$$`
  for a literal dollar (`'$$'` in single quotes, or `"\$\$"` in double quotes),
  or `$1`/`${name}` on purpose.
* An **empty pattern** used to return the text unchanged; now it matches
  between every character (`{ replace abc "" - }` is `-a-b-c-`).
* An invalid regex is a runtime error (`invalid regular expression: …`).

The same regex syntax is used by `matches`, `search` and the new regex
functions (Rust's `regex` crate).

#### R7. Nested `null` prints as `null` (silent)

Inside arrays and maps, `null` used to print as `(null)` and now prints as
`null`, because `null` reads back as null now. The string `"null"` prints
quoted. This only matters when a script compares printed output, such as
golden files or string comparisons.

```dog
pln [1 null "null"]
```
```text
[1 null "null"]
```

#### R8. Module functions are named by their path in errors (silent)

Error messages and traces name functions of the builtin modules, and of
modules registered by the host, by their full path: `json.parse`, not `parse`.
Only scripts that inspect `$e.message` text are affected.

```dog
pln (try { json.parse (null) } catch e { put $e.message })
```
```text
`json.parse` expects a str for `text`, got null
```

#### R9. `\5` no longer forces a word to be text

A digit can't be escaped any more (see R2). **Rewrite:** `\5` → `"5"` (or
`'5'`).

#### R10. New builtins don't break old code

Old scripts that define their own `count`, `find`, `remove`, `has`, `insert`,
`all`, `any`, `lines`, `zip`, `pi`, `now`, `fetch`, … keep working: a script's
own functions and variables hide builtins of the same name. Nothing to do.

---

### Part 2: what's new

#### Syntax

* **Raw strings**: `"""…"""` with three or more quotes, no escapes, no
  interpolation, multi-line with indentation stripping. See R3.
* **Slices**: `$x[a..b]`, `$x[a..]`, `$x[..b]`, `$x[..]` on arrays and strings
  (strings by character). Negative bounds count from the end; out-of-range
  bounds are clamped; slicing `null` gives `null`; a slice can't be assigned
  to. Not the same as `first`/`last`: `{ first $l 5 }` is `null` when there
  are fewer than 5 items, while `$l[..5]` gives what there is.

```dog
a = [0 1 2 3 4 5];
s = "héllo";
pln $a[1..3] $a[..2] $a[-2..] $a[4..100] $s[1..3] $s[-2..]
```
```text
[1 2] [0 1] [4 5] [4 5] él lo
```

#### Builtins

Text:

| builtin | does |
|---|---|
| `trim_start text`, `trim_end text` | |
| `pad_left text width [fill]`, `pad_right text width [fill]` | widen to `width` characters (fill: one character, default space) |
| `lines text` | array of lines (`\n` or `\r\n`) |
| `capitalize text` | first character uppercased |
| `ord char`, `chr code` | Unicode code points |
| `count haystack needle` | occurrences of a substring (text) or element (array) |
| `last_index_of items needle` | like `index_of`, from the end |
| `is_digit %args`, `is_space %args` | like `is_alpha` |

Regular expressions:

| builtin | does |
|---|---|
| `replace text pattern to` | regex replace (see R6) |
| `captures text pattern` | first match's groups `[whole g1 g2 …]` (`null` for unmatched groups), or `null` |
| `captures_all text pattern` | groups of every match |
| `find_all text pattern` | array of every matched text |
| `split_re text pattern` | split at every match |
| `regex_escape text` | escape regex syntax |

Arrays and maps (all return new values; assign the result back):

| builtin | does |
|---|---|
| `insert items i v`, `remove_at items i` | negative `i` counts from the end; out of range is an error |
| `flatten items`, `unique items` | one level / without repeats |
| `zip a b`, `chunk items n` | pairs / groups of `n` |
| `remove map %keys`, `has map key` | delete keys / key present? |
| `entries map`, `from_entries pairs` | map ⇄ `[[key value] …]` |

Callbacks: `any all find find_index flat_map group_by count_by min_by max_by`
(each `items f`).

Numbers: `round x digits`, `trunc`, `sign`, `clamp x min max`, `exp`, `ln`,
`log x [base]` (base 10 by default), `sin cos tan asin acos atan`, `atan2 y x`, `pi`,
`fixed x digits` (text), `parse_int text base`, `to_base n base`, `product`,
`avg`, `median`. Random (capability `random`): `random_float [min max]`,
`sample items n`, `uuid`.

Encodings: `url.encode`, `url.decode`, `url.query map`, `url.parse url`,
`base64.encode`, `base64.decode` (also URL-safe, padding optional),
`hex.encode`, `hex.decode`, `sha256 text`, `hmac_sha256 key text`.

Dates (UTC, Unix-second timestamps): `date.format ts [fmt]`,
`date.parse text [fmt]` (ISO 8601 by default), `date.parts ts`; `now` needs
the new `time` capability.

```dog
pln { captures "v1.2" """v(\d+)\.(\d+)""" } { pad_left 7 3 0 } { unique [a b a] } { fixed 3.14159 2 };
pln { group_by [1 2 3 4] fn (n) if ($n % 2 == 0) even else odd } { url.query [q = "a b"] } { date.format 0 "%F" }
```
```text
[v1.2 "1" "2"] 007 [a b] 3.14
[odd = [1 3], even = [2 4]] q=a%20b 1970-01-01
```

HTTP (capability `net`):

* `fetch url [options]` returns `[status ok url headers body]` and never
  fails because of the status code.
* `get url [options]` and `post url [body [options]]` take the same options
  map. A number in place of the options is still a timeout, as before.
* Options: `method`, `headers` (map), `query` (map), `body`, `form` (map,
  URL-encoded), `timeout` (seconds), `redirects` (`false` or a count).
* A body that isn't text is sent as JSON with `Content-Type: application/json`.
* Response header names are lowercase.

The full reference for all of the above is in
[docs/language.md §12](docs/language.md).

---

### Part 3: changes for Rust hosts

* `Capability::Time` (new) gates `now`. It is allowed by default, including
  in `Builder::sandboxed()`. Withhold it on targets without a clock (the
  wasm playground does).
* `Capability::Random` now also covers `random_float`, `sample` and `uuid`;
  `Capability::Net` also covers `fetch`.
* `Builder::module(name, exports)`, and native modules returned by a
  `ModuleProvider`, now rename their functions from `f` to `name.f` for
  error messages. New: `Library::into_module(name)` and
  `Library::module(name, inner)` for nested native modules.
* `Value`'s nested display prints `null` instead of `(null)`.
* `dog4::value::bare_literal` follows the new rule (R5); new
  `dog4::value::number_literal` parses expression-syntax numbers.
* New dependency: `sha2`.
