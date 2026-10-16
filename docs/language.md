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
| `\c` | the character `c` (`\n` `\t` `\r` `\0` are control characters) |
| `"…"` | text with escapes and interpolation (always text) |
| `'…'` | text with escapes but no interpolation (only at the start of a word; elsewhere `'` is a plain apostrophe) |
| `$name`, `$name.field`, `$name[expr]`, `$name.0` | a variable and a path into it |
| `${name}` | a variable, with no path |
| `$(expr)` | an expression |
| `$f(args)` | a call (not inside `"…"`) |

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

**Bare literals.** A word of plain characters that reads exactly like a number
or a boolean is that value: the text must be what the value prints as. So
`5`, `-2.5`, `true` are typed, while `007`, `1.50`, `1e3`, `+5` and `null`
stay text.

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
number or bool), and a nested `null` prints as `(null)`, infinities and `NaN` as
`(1 / 0)`, `(-1 / 0)` and `(0 / 0)`. Only functions, printed as `<fn name>`,
have no literal form. Use `json.encode` for JSON.

```dog
pln "a" [1 "two" (null) [k = v]] (null) $pln
```
```text
a [1 two (null) [k = v]]  <fn pln>
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

* numbers `12`, `2.5`, `1e3`, `1_000`; strings `"…"`, `'…'`; `true` `false` `null`
* `$variables` with paths, blocks, literals, nested `( … )`
* a bare identifier is a **string** (`x = hello` → `"hello"`), unless:
  * it is directly followed by `(`: a call, `max(1, 2)`, `json.encode($v)`;
  * it is followed, on the same line, by an argument: a command,
    `(len $list)`, `x = upper hello` (the arguments run to the end of the
    expression; a `-` directly followed by text, as in `abs -3`, is an argument);
* `fn (…) { … }` lambdas, `if`/`while`/`for`/`try` forms, `mod name`, `mod { … }`.

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
| `x.f x[i] x(args)` | written without spaces |

```dog
x = 7;
pln (1 + 2 * 3) (7 / 2) (6 / 3) (-7 % 3) ("ab" * 3) (1..4) ($x > 5 && $x < 10);
pln (0 || "") (null ?? 0) ([a b] == [a b]) (1 == 1.0) ("1" == 1)
```
```text
7 3.5 2 2 ababab [1 2 3] true
 0 true true false
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
(giving text back), like in dog3.

| builtin | does |
|---|---|
| `put %args` | outputs its argument; several are joined as text with spaces |
| `pln %args` | outputs the text of its arguments, space-separated, plus a newline |
| `print %args`, `println %args` | write immediately (capability `print`) |
| `type value` | `"null"`, `"bool"`, `"num"`, `"str"`, `"arr"`, `"map"` or `"fn"` |
| `num v`, `str v`, `bool v`, `arr v` | convert (as typed parameters do) |
| `len items [sep]` | length of an array or map, or number of words in text |
| `first items n [sep]`, `last items n [sep]` | first/last `n` items (`null` if there are fewer) |
| `append a b [sep]` | two arrays or texts joined |
| `range max`, `range min max [step [sep]]` | whole numbers (a text when `sep` is given) |
| `keys map`, `values map` | arrays of a map's keys / values |
| `contains haystack needle` | array element, map key or substring |
| `join items [sep]`, `split text [sep]`, `chars text` | text ⇄ arrays |
| `reverse items`, `sort items`, `sum items`, `index_of items needle` | |
| `add sub mul div max min first %rest` | dog3 arithmetic over all arguments |
| `floor ceil round abs sqrt x`, `pow base exp` | |
| `random max`, `random min max`, `shuffle items`, `pick items` | capability `random` |
| `eq neq lt gt leq geq a b` | numeric comparisons (convert text) |
| `like a b` | same text? |
| `and a b`, `or a b`, `not a`, `true`, `false` | booleans by truthiness |
| `upper lower %args`, `trim text`, `strlen text` | |
| `replace target from to`, `starts_with`, `ends_with` | |
| `search text pattern` | the lines matching a regular expression |
| `matches text pattern` | does the regex match? |
| `is_alpha %args`, `is_alphanumeric %args` | |
| `map items f`, `filter items f`, `each items f`, `reduce items init f`, `sort_by items f` | |
| `json.parse text`, `json.encode v`, `json.pretty v` | |
| `gron json` | flattened, grep-able listing of a JSON document |
| `src function` | source code |
| `eval code` | runs code in the current module (capability `eval`) |
| `fail %message`, `assert cond [%message]` | raise errors |
| `get url [timeout]`, `post url [body [timeout]]` | HTTP (capability `net`) |

```dog
pln { first "a b c" 2 } { first [1 2 3] 2 } { len "a b c" } { range 3 } { join [x y] - };
pln { sort [3 1 2] } { sum [1 2 3] } { upper hi } { replace a-b - + } { json.encode [a = [1]] };
put { gron '{"a": [1, {"b": true}]}' }
```
```text
a b [1 2] 3 [0 1 2] x-y
[1 2 3] 6 HI a+b {"a":[1]}
json = {}
json.a = []
json.a[0] = 1
json.a[1] = {}
json.a[1].b = true
```

## 13. Limits

A host can limit fuel (work), time, call depth, callback depth, string size,
array/map size, printed output and source nesting; and it can withhold the
`print`, `eval`, `random` and `net` capabilities. Hitting a limit is an error of
kind `limit` that `try` can't catch. See [embedding.md](embedding.md).
