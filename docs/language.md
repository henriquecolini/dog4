# The dog4 language reference

This is the precise description of dog4. The [README](../README.md) is the
gentle introduction; every example here is executed by the test suite
(`cargo test --test docs`).

## 1. Programs and statements

A program is a sequence of statements. Statements are separated by `;`, which
is required after every statement except blocks, control statements, `fn` and
`mod` declarations, and the last statement of a block. **Newlines are not
separators.** Comments are `// to the end of the line` and `/* nestable */`;
they start only between tokens, so `http://x` is a single word.

Every statement may *output* a value. A block `{ … }` evaluates its statements
and its value is the **fold** (§4) of everything they output; an empty block, or
one that outputs nothing, is `null`. The statement kinds:

| statement | outputs |
|---|---|
| `name arg arg …` (a command) | the command's result |
| `x = expr`, `x += expr`, `let x = expr`, `fn …`, `use …` | nothing |
| a bare block `{ … }`, `if`, `while`, `for`, `try`, `mod` | nothing — their value is **discarded** |
| `return [expr]` | ends the function; its value replaces everything output so far |
| `clear [expr]` | replaces what the current block has output so far |
| `break [expr]` / `continue` | see §7 |
| `throw expr` | raises an error |

The discarding rule is dog3's and is the reason for `put`:

```dog
put Hello1;
{ put Hello2 }
put { put Hello3 };
put { if true { put Hello4 } };
put { put if true { put Hello5 } };
x = for i in [1 2] { put $i };
pln "" $x
```
```text
Hello1Hello3Hello5 3
```

The program's own output (what the CLI prints at the end) is the fold of its
top-level statements. `print` and `println` write immediately instead.

## 2. Command mode

Statements are commands: a name (or `$variable`) followed by arguments
separated by whitespace. Arguments are **words** or **values**:

* `{ … }` blocks, `[ … ]` literals (§5), `( … )` expressions (§6), and
  `if`/`while`/`for`/`try`/`fn` forms are values.
* Anything else is a word, which ends at whitespace or one of `; { } ( ) [ ]`.

A word is made of pieces:

| piece | meaning |
|---|---|
| plain characters | text; `,` `=` `'` `:` `+` … are ordinary characters |
| `\c` | an escape (below) |
| `"…"` | text with escapes and interpolation (always text) |
| `'…'` | text with escapes but no interpolation (only at the start of a word; elsewhere `'` is a plain apostrophe) |
| `"""…"""` | a raw string (below): text with no escapes and no interpolation |
| `$name`, `$name.field`, `$name[expr]`, `$name.0` | a variable and a path into it |
| `${name}` | a variable, with no path |
| `$(expr)` | an expression |
| `$f(args)` | a call (not inside `"…"`) |

**Escapes** work the same in words, `"…"` and `'…'`: `\n` `\t` `\r` `\0` are
control characters, and a backslash before ASCII punctuation or whitespace
(`\$` `\"` `\\` `\;` `\ ` …) gives that character. Any other escape, such as
`\d`, `\s` or `\5`, is a syntax error.

**Raw strings** start with three or more `"` and end at the next run of exactly
as many; a longer run inside is an error. Everything in between is literal
text, so a raw string can hold any run of quotes shorter than its delimiters.
If a raw string spans several lines, its opening quotes must end their line and
its closing quotes must be alone on theirs; the text is the lines in between,
minus the closing quotes' indentation (like C#):

```dog
pln """\d+\s$x""" """"say """hi"""!"""";
x = """
    first
      second "quoted"
    """;
pln $x
```
```text
\d+\s$x say """hi"""!
first
  second "quoted"
```

Variable names are letters (any script), ASCII digits and `_`, not starting
with a digit. So in `"$x!"` the variable is `x`; in `"$file.txt"` it is a path
(`txt` field of `file`) — write `"${file}.txt"`.

A word that is one piece is that piece's value. A word of several pieces is
their **fold** (§4) from left to right, text pieces counting as strings:

```dog
x = 5; id = 42; l = [a];
pln user_$id $x$x "$x$x" $x"!" got:$l
```
```text
user_42 10 55 5! [got: a]
```

**Bare literals.** A word of plain characters that is a literal is that value:
`true`, `false`, `null`, and numbers written as in expressions (§6), with an
optional leading `-`: `5`, `-2.5`, `007` (7), `1.50`, `1e3`, `1_000`. Anything
else is text: `+5`, `5px`, `inf`, `1.`. Quoting (`"5"`, `'null'`) or escaping
always gives text.

Two special argument forms exist only in calls:

* `$%name` spreads an array (or the words of a string) as separate arguments.
* `&name`, `&name.field[0]` passes a reference (§9).

## 3. Values

| type | values | notes |
|---|---|---|
| `null` | `null` | nothing; prints as empty text |
| `bool` | `true`, `false` | |
| `num` | `1`, `-2.5`, `inf`, `NaN` | 64-bit integers when whole, floats otherwise; integer overflow becomes a float |
| `str` | `"text"` | UTF-8 |
| `arr` | `[a b c]` | ordered |
| `map` | `[k = v]` | string keys, insertion order |
| `fn` | functions | |

Values have **value semantics**: assigning or passing a value never lets two
variables see each other's changes (use references for that). Arrays and maps
are copy-on-write, so copying is cheap and updating a uniquely owned value
happens in place.

**Truthiness**: `false`, `null`, `0`, `""`, `[]` and `[=]` are falsy;
everything else is truthy.

**Display.** At the top level (printing, `pln`, interpolation) strings print as
is and `null` prints nothing. Arrays and maps print as **dog4 literals**, so
what a program prints can be pasted back into a program: strings are quoted
only when they must be (spaces, special characters, text that would read as a
number, a bool or `null`), and infinities and `NaN` print as `(1 / 0)`,
`(-1 / 0)` and `(0 / 0)`. Only functions, printed as `<fn name>`, have no
literal form. Use `json.encode` for JSON.

```dog
pln "a" [1 "two" null "null" [k = v]] (null) $pln
```
```text
a [1 two null "null" [k = v]]  <fn pln>
```

## 4. The fold (`+`)

One operation combines values, both for `a + b` and for how blocks combine the
values their statements output:

| left \ right | result |
|---|---|
| `null + x`, `x + null` | `x` |
| num + num | sum |
| str + str | concatenation |
| arr + arr | concatenation |
| map + map | merge; the right side wins on equal keys |
| bool + bool | `&&` |
| arr + x, x + arr | push `x` / prepend `x` |
| str + num/bool, num/bool + str | text concatenation |
| anything else | type error (`can't add map to num`) |

```dog
pln { put 1; put 2 } { put [a]; put [b] } { put [x = 1]; put [x = 2] } { put true; put false };
pln ("a" + 1) (1 + "a") ([1] + 2) (0 + [1]) ({ put hi; put 5 })
```
```text
3 [a b] [x = 2] false
a1 1a [1 2] [0 1] hi5
```

## 5. Literals

* `[a b c]`, `[a, b, c]`: arrays; elements are command-mode values, so
  `[x (1 + 2) { put y }]` works and bare words follow the bare-literal rule.
  `$%list` spreads inside an array.
* `[key = value, …]`: maps; keys are words (or `$k`, `(expr)`), turned into text.
* `[]` and `[=]`: the empty array and the empty map.

Inside brackets, words also end at `,` and `=`.

## 6. Expression mode

Inside `( … )`, after `=` (and `+=` …), after `return`/`clear`/`break`/`throw`,
and in call arguments `f(…)`, dog4 reads **expressions**:

* numbers `12`, `2.5`, `1e3`, `1_000`; strings `"…"`, `'…'`, `"""…"""`;
  `true` `false` `null`
* `$variables` with paths, blocks, literals, nested `( … )`
* a bare identifier is a **string** (`x = hello` → `"hello"`), unless it is
  directly followed by `(`: then it's a call, `max(1, 2)`, `json.encode($v)`
* `fn (…) { … }` lambdas, `if`/`while`/`for`/`try` forms, `mod name`, `mod { … }`.

**Commands are not expressions.** A command with arguments only runs inside a
block: write `{ len $list }` or `len($list)`. A name followed by arguments in
an expression, as in `(len $list)` or `x = upper hello`, is a syntax error.

Operators, from loosest to tightest:

| operators | notes |
|---|---|
| `??` | the left side unless it is `null` |
| `\|\|` | the left side if truthy, else the right (short-circuit) |
| `&&` | the left side if falsy, else the right (short-circuit) |
| `== != < <= > >=` | `==` is deep equality (different types are never equal); `<` compares two numbers or two strings; comparisons don't chain |
| `..` | `a..b` is the array of whole numbers from `a` up to, not including, `b` |
| `+ -` | `+` is the fold |
| `* / %` | `*` also repeats strings and arrays; `/` keeps whole results whole; `%` takes the sign of the divisor |
| `-x !x` | negation, logical not |
| `x.f x[i] x[a..b] x(args)` | written without spaces; see slices below |

```dog
x = 7;
pln (1 + 2 * 3) (7 / 2) (6 / 3) (-7 % 3) ("ab" * 3) (1..4) ($x > 5 && $x < 10);
pln (0 || "") (null ?? 0) ([a b] == [a b]) (1 == 1.0) ("1" == 1)
```
```text
7 3.5 2 2 ababab [1 2 3] true
 0 true true false
```

**Slices.** `x[a..b]` is a copy of the items of an array, or the characters of
a string, from index `a` up to, not including, `b`. Either bound can be left
out (`x[a..]`, `x[..b]`, `x[..]`); negative bounds count from the end, and
bounds past either end are clamped, so a slice never fails for being out of
range. Slicing `null` gives `null`. A slice can't be assigned to or passed as
a reference.

```dog
a = [0 1 2 3 4 5];
s = "héllo wörld";
pln $a[1..3] $a[..2] $a[4..] $a[-2..] $a[3..1] "<$s[1..4]>" $s[-5..]
```
```text
[1 2] [0 1] [4 5] [4 5] [] <éll> wörld
```

## 7. Control flow

All of these are values (and, as statements, their value is discarded):

* `if cond then [else other]` — `cond`, `then` and `other` are command-mode
  values, so `if ($x > 1) { … } else if … `. Without `else`, a false `if` is `null`.
* `while cond body` — the fold of every iteration's body.
* `for x in iterable body`, `for k, v in iterable body` — arrays give elements
  (or index and element), maps give keys (or key and value), strings give their
  whitespace-separated words, `null` gives nothing. `for x in text split sep body`
  splits by `sep` instead (an empty `sep` splits characters). `for i in (a..b)`
  counts without building the array.
* `break [value]` ends the innermost loop; `continue` starts its next iteration.
  Both keep everything output so far, including the current iteration's.
* `try body [catch name handler]` — runs `body`; if it raises an error, the
  result is `handler` with `name` bound to `[message = …, kind = …, value = …]`
  (or `null` without `catch`). Errors from sandbox limits can't be caught.

```dog
pln (for x in (0..10) { if ($x == 2) { continue }; if ($x == 5) { break }; put [$x] });
pln (for k, v in [a = 1, b = 2] { put "$k=$v " });
pln (try { fail nope } catch e { put "$e.kind: $e.message" })
```
```text
[0 1 3 4]
a=1 b=2 
throw: nope
```

## 8. Variables and scope

* `x = value` assigns to the nearest **visible** `x` — in an enclosing block,
  an enclosing function (through a closure), or the module — and declares a
  new `x` in the current block only if there is none. This is dog3's rule.
* `let x = value` always declares a new `x` in the current block, hiding any
  outer one until the block ends.
* `x.a[0] = v`, `x += v`, `-=`, `*=`, `/=` update in place. Assigning a field of
  `null` makes a map; assigning index `len` of an array appends.
* Reading an undeclared name is an error found before the program runs. Using
  a module-level variable before it has a value is an error when it happens.
* A new variable comes into scope after its right side is evaluated
  (`len = { len $x }` still calls the builtin), except that `f = fn …` can call
  itself through `f`.

```dog
i = 10;
fn sum_to (n: num) { let i = 0; let t = 0; while ($i < $n) { t += $i; i += 1 }; put $t }
fn bump () { i += 1 }
bump;
pln { sum_to 4 } $i
```
```text
6 11
```

## 9. Functions

```dog ignore
fn name (a, b: num, c: str|arr, d: num?, e: &map, f: &, %rest: str) { body }
fn name (params) value
lambda = fn (x) { body }
lambda = fn (x) value
```

* A function returns the fold of its body, or the value of `return`.
* The body can also be a **single value**, like the branches of `if`:
  `fn f (x) value` is `fn f (x) { put value }`. It is one value, so a command
  still needs braces (`fn f (x) { upper $x }`); `fn f (x) upper $x` would make
  `upper` the body.

```dog
fn greet (name) "Hello $name"
fn square (x: num) ($x * $x)
fn sign (x: num) if ($x < 0) -1 else if ($x > 0) 1 else 0
pln { greet Ana } { square 4 } { sign -3 } { map [1 2] fn (x) [$x $x] }
```
```text
Hello Ana 16 -1 [[1 1] [2 2]]
```

* `fn` declarations are **hoisted** within their block: they can be used
  before they are defined and can call each other.
* Several `fn` with the same name in the same block form one **overload set**.
  A call picks, among the overloads accepting that many arguments, the first
  whose parameter types match exactly, else the first that matches after
  conversion. Two overloads with identical parameters are an error.
* Types: `any` (the default), `null bool num str arr map fn`, unions `a|b`,
  optional `t?` (= `t|null`). A typed parameter **converts** an argument of
  another type when it can: text → `num` if it parses as a number; num/bool →
  `str`; text → `arr` by splitting into words; anything → `bool` by
  truthiness. Otherwise the call fails.
* `%rest` (last) collects the remaining arguments into an array; each is
  converted to the rest type.
* Call: `name args` / `$f args` in command mode, `f(args)` / `$f(args)` in
  expressions. Functions are values (`$name` is the function).
* Lambdas capture variables **by reference**; every loop iteration gets fresh
  variables.
* `src name` / `src $f` gives a function's source.

```dog
fn show (x: num) { put number }
fn show (x: str) { put text }
fn show (x, y) { put pair }
fn add (a: num, b: num) { put ($a + $b) }
pln { show 5 } { show "5" } { show a b } { add "2" 3 };
adders = for n in [1 2] { put [fn (x) { put ($x + $n) }] };
pln ($adders[0](10)) ($adders[1](10))
```
```text
number text pair 5
11 12
```

### References

A parameter typed `&` (or `&type`) receives a **reference**: reads and writes
go to the caller's variable. The caller must pass `&variable` (or a path,
`&user.tags[0]`); passing a plain value is an error. A reference passed to a
normal parameter is just its current value. `let r = &x` (inside a block or
function) makes `r` an alias. References can't be stored in arrays or maps.

```dog
fn grow (list: &arr, n: num) { for i in (0..$n) { list += [$i] } }
data = [items = []];
grow &data.items 3;
pln $data
```
```text
[items = [0 1 2]]
```

## 10. Modules

A module is a **map**: the `pub` top-level names of some code, after it has run.

* `mod name` loads a module and gives its map. The name is a command-mode value
  (`mod net/http`, `mod "my lib"`, `mod $name`), resolved by the host's module
  provider; `./x` and `../x` are relative to the importing module.
* `mod { … }` is a module literal. It compiles exactly like a file: it has its
  own variables and can't see the enclosing code's.
* Both are expressions and bind nothing: `x = mod foo`, `put mod foo`,
  `(mod foo).hi`. On its own, a `mod` statement runs the module and discards the
  map, like a bare block.
* A module's top-level output is discarded. Each module runs once per runtime;
  later `mod`s of the same name get the same map. Module functions see their
  module's variables live; the map itself is a snapshot.
* `pub` goes before a top-level `fn`, `use`, `let` or `name = value`. A name is
  exported if any of its top-level declarations is `pub` (so one `pub fn`
  exports the whole overload set). The module's own code sees every name, `pub`
  or not. `pub` anywhere but a module's top level is an error.
* Circular imports are an error.

```dog
counter = mod {
    pub n = 0;
    step = 1;
    pub fn next () { n += $step; put $n }
};
area = mod shapes/area;
pln { counter.next } { counter.next } { keys $counter } { keys $area }
```
```text
1 2 [n next] [square]
```

`use` brings the members of any map into scope:

* `use a, b as c from map` declares the variables `a` and `c`, like `let`,
  with the values of `map.a` and `map.b`. A missing member is an error.
* `use map` is a **glob import**: every member of the map becomes visible by
  name in the enclosing block (at the top level, in the whole module). Its
  members can't be assigned to; `x = …` declares or assigns a real variable,
  which wins over the glob.
* A name is looked up scope by scope from the innermost: a variable declared
  in the scope wins, then the scope's globs, newest first, then the enclosing
  scopes, module globals, the module's globs, and the builtins. So globs can
  shadow builtins and outer variables. Globs are checked when the code runs: with
  a glob in scope, an unknown name is an error only when it is reached.
* `pub use` re-exports: the named members, or every member of the glob (a
  module's own `pub` names win over globbed ones).

```dog
use hi from mod greet;
hi Ana;
use mod shapes/area;
pln { square 3 };
pln { use [len = fn (x) { put "len of $x" }]; len abc } { len abc };
```
```text
Hello Ana
9
len of abc 1
```

## 11. Errors

Errors have a `kind`: `syntax`, `name`, `type`, `arity`, `value`, `throw`,
`module`, `io` or `limit`. `throw value` raises any value; `fail words…` raises
text; `assert cond [message…]` raises when `cond` is falsy. Uncaught errors
report the location and the chain of calls.

## 12. Builtins

Builtins declare typed parameters like any function, so they convert their
arguments the same way. "Sequences" accept arrays, or text split into words
(giving text back), like in dog3. Builtins never change their arguments: they
return new values, so write `list = { insert $list 0 x }`. Functions in a
module (`json.parse`) are named that way in error messages.

**Core**

| builtin | does |
|---|---|
| `put %args` | outputs its argument; several are joined as text with spaces |
| `pln %args` | outputs the text of its arguments, space-separated, plus a newline |
| `print %args`, `println %args` | write immediately (capability `print`) |
| `type value` | `"null"`, `"bool"`, `"num"`, `"str"`, `"arr"`, `"map"` or `"fn"` |
| `num v`, `str v`, `bool v`, `arr v` | convert (as typed parameters do) |
| `src function` | source code |
| `eval code` | runs code in the current module (capability `eval`) |
| `fail %message`, `assert cond [%message]` | raise errors |

**Sequences, arrays and maps**

| builtin | does |
|---|---|
| `len items [sep]` | length of an array or map, or number of words in text |
| `first items n [sep]`, `last items n [sep]` | first/last `n` items (`null` if there are fewer) |
| `append a b [sep]` | two arrays or texts joined |
| `range max`, `range min max [step]` | an array of whole numbers |
| `contains haystack needle` | array element, map key or substring |
| `count haystack needle` | how many times an element (array) or substring (text) occurs |
| `index_of items needle`, `last_index_of items needle` | position of the first/last occurrence, or `null` |
| `join items [sep]`, `split text [sep]`, `chars text` | text ⇄ arrays |
| `reverse items`, `sort items`, `sum items` | |
| `insert items i v`, `remove_at items i` | the array with `v` inserted at / the item removed from index `i` (negative counts from the end) |
| `flatten items`, `unique items` | one level of nesting removed / repeats removed (the first is kept) |
| `zip a b`, `chunk items n` | `[[a0 b0] [a1 b1] …]` (as long as the shorter) / arrays of `n` items |
| `keys map`, `values map`, `entries map`, `from_entries pairs` | a map's keys, values, or `[key value]` pairs, and back |
| `has map key`, `remove map %keys` | is the key there? / the map without those keys |

**Callbacks**

| builtin | does |
|---|---|
| `map items f`, `filter items f`, `each items f`, `reduce items init f` | |
| `sort_by items f`, `min_by items f`, `max_by items f` | by the key `f` gives each item |
| `any items f`, `all items f` | does `f` hold for some / every item? |
| `find items f`, `find_index items f` | the first item for which `f` holds, or its index (`null` if none) |
| `flat_map items f` | `map`, then the resulting arrays concatenated |
| `group_by items f`, `count_by items f` | a map from each key (as text) to the items with it / to how many |

```dog
nums = [3 1 4 1 5 9 2 6];
pln { insert [a c] 1 b } { remove_at [a b c] -1 } { flatten [[1 2] [3] 4] } { unique $nums };
pln { zip [a b c] [1 2] } { chunk $nums 3 } { count $nums 1 } { count "banana" an };
user = [name = Ana, age = 30, admin = true];
pln { remove $user age admin } { has $user age } { entries [a = 1] } { from_entries [[x 1]] };
pln { any $nums fn (n) ($n > 8) } { all $nums fn (n) ($n > 0) } { find $nums fn (n) ($n > 4) };
pln { group_by $nums fn (n) if ($n % 2 == 0) even else odd } { max_by [ab c def] fn (w) { strlen $w } }
```
```text
[a b c] [a b] [1 2 3 4] [3 1 4 5 9 2 6]
[[a 1] [b 2]] [[3 1 4] [1 5 9] [2 6]] 2 2
[name = Ana] true [[a 1]] [x = 1]
true true 5
[odd = [3 1 1 5 9], even = [4 2 6]] def
```

**Numbers**

| builtin | does |
|---|---|
| `add sub mul div max min first %rest` | dog3 arithmetic over all arguments |
| `floor ceil trunc abs sign sqrt exp ln x`, `round x [digits]`, `pow base exp`, `clamp x min max` | |
| `log x [base]` | logarithm, base 10 by default |
| `sin cos tan asin acos atan x`, `atan2 y x`, `pi` | radians |
| `fixed x digits` | text with exactly `digits` decimals |
| `parse_int text base`, `to_base n base` | whole numbers in bases 2 to 36 |
| `product items`, `avg items`, `median items` | over an array of numbers (`avg` and `median` of `[]` are `null`) |
| `random max`, `random min max`, `random_float [min max]` | capability `random` |
| `shuffle items`, `pick items`, `sample items n`, `uuid` | capability `random` |
| `eq neq lt gt leq geq a b` | numeric comparisons (convert text) |
| `like a b` | same text? |
| `and a b`, `or a b`, `not a`, `true`, `false` | booleans by truthiness |

```dog
pln { round 3.14159 2 } { fixed 2 3 } { clamp 15 0 10 } { sign -4 } { log 1000 } { round { pi } 4 };
pln { parse_int ff 16 } { to_base 255 2 } { avg [1 2 3 4] } { median [5 1 3] } { product [2 3 4] }
```
```text
3.14 2.000 10 -1 3 3.1416
255 11111111 2.5 3 24
```

**Text**

| builtin | does |
|---|---|
| `upper lower %args`, `capitalize text` | |
| `trim text`, `trim_start text`, `trim_end text` | |
| `pad_left text width [fill]`, `pad_right text width [fill]` | widen to `width` characters with `fill` (a space by default) |
| `strlen text`, `lines text` | number of characters / an array of lines |
| `starts_with text prefix`, `ends_with text suffix` | |
| `ord char`, `chr code` | Unicode code points |
| `is_alpha is_alphanumeric is_digit is_space %args` | do all the characters qualify? |

**Regular expressions** use the syntax of Rust's `regex` crate. Raw strings
keep their backslashes: `"""\d+"""`.

| builtin | does |
|---|---|
| `matches text pattern` | does the regex match? |
| `search text pattern` | the lines matching, each with its newline |
| `replace text pattern to` | every match replaced; in `to`, `$1` or `${name}` is a group and `$$` is `$` |
| `find_all text pattern` | an array of every match |
| `captures text pattern` | the first match's groups, `[whole first second …]` (`null` for a group that didn't match), or `null` |
| `captures_all text pattern` | the groups of every match |
| `split_re text pattern` | text split at every match |
| `regex_escape text` | `text` with regex syntax escaped, to match it literally |

```dog
pln { replace "2024-06-01" """(\d+)-(\d+)-(\d+)""" '$3/$2/$1' } { replace "a.b.c" { regex_escape . } - };
pln { captures "id: 42, id: 7" """id: (\d+)""" } { captures_all "a=1 b=2" """(\w)=(\d)""" };
pln { find_all "1 fish 2 fish" """\d+""" } { split_re "a, b;c" """[,;]\s*""" } { pad_left 7 3 0 } { capitalize élan }
```
```text
01/06/2024 a-b-c
["id: 42" "42"] [["a=1" a "1"] ["b=2" b "2"]]
["1" "2"] [a b c] 007 Élan
```

**Data formats and encodings**

| builtin | does |
|---|---|
| `json.parse text`, `json.encode v`, `json.pretty v` | |
| `gron json` | flattened, grep-able listing of a JSON document |
| `url.encode text`, `url.decode text` | percent-encoding (everything but `A-Z a-z 0-9 - . _ ~` is encoded) |
| `url.query map` | `a=1&b=2`; an array value repeats its key, `null` values are left out |
| `url.parse url` | `[scheme user password host port path query fragment]`, with `query` a map |
| `base64.encode text`, `base64.decode text` | standard base64; decoding also takes the URL-safe alphabet and missing padding |
| `hex.encode text`, `hex.decode text` | |
| `sha256 text`, `hmac_sha256 key text` | hex digests |

Decoding to bytes that aren't UTF-8 text is an error.

```dog
pln { url.query [q = "dog 4", tag = [a b]] } { url.decode "caf%C3%A9" } { base64.encode hi } { hex.encode hi };
pln { url.parse "https://example.com:8080/a?x=1#top" };
pln { sha256 abc }
```
```text
q=dog%204&tag=a&tag=b café aGk= 6869
[scheme = https, user = null, password = null, host = example.com, port = 8080, path = /a, query = [x = "1"], fragment = top]
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
```

**Dates.** Timestamps are Unix seconds, and dates are in UTC. Formats use
`%Y %y %m %d %e %H %I %p %M %S %f` (milliseconds) `%j %a %A %b %B %u %w %F %T
%s %z %Z %%`; parsing reads the same directives.

| builtin | does |
|---|---|
| `now` | the current time (capability `time`) |
| `date.format ts [format]` | text; ISO 8601 (`2024-06-01T12:00:00Z`) by default |
| `date.parse text [format]` | a timestamp; ISO 8601 / RFC 3339 by default, with an optional offset |
| `date.parts ts` | `[year month day hour minute second millis weekday yday]` (`weekday` 1 is Monday) |

```dog
t = { date.parse "2024-02-29T13:45:00+02:00" };
pln $t { date.format $t } { date.format $t "%a %d %b %Y, %I:%M %p" } { date.parts $t }.weekday
```
```text
1709207100 2024-02-29T11:45:00Z Thu 29 Feb 2024, 11:45 AM 4
```

**HTTP** (capability `net`)

| builtin | does |
|---|---|
| `get url [options]`, `post url [body [options]]` | the response body; a status outside 200–299 is an error |
| `fetch url [options]` | the whole response, `[status ok url headers body]`, whatever its status |

The options are a map: `method` (`GET`, or `POST` when there is a body),
`headers` (a map), `query` (a map added to the URL, as by `url.query`), `body`,
`form` (a map sent as `application/x-www-form-urlencoded`), `timeout` (seconds,
1–60, default 10; also cut short by the run's time limit) and `redirects`
(`false`, or how many to follow; default 10). A body that isn't text is sent as
JSON with `Content-Type: application/json`, unless the headers set another.
Response header names are lowercase. `get` and `post` also accept a timeout
number in place of the options, as in dog3.

```dog ignore
r = { fetch https://api.example.com/items [
    method = PUT,
    headers = [Authorization = "Bearer $token"],
    query = [page = 2],
    body = [name = Ana, tags = [a b]],
] };
if (!$r.ok) { fail "got $r.status" };
items = json.parse($r.body);
pln $r.headers["content-type"]
```

## 13. Limits

A host can limit fuel (work), time, call depth, callback depth, string size,
array/map size, printed output and source nesting; and it can withhold the
`print`, `eval`, `random`, `time` and `net` capabilities. Hitting a limit is an error of
kind `limit` that `try` can't catch. See [embedding.md](embedding.md).
