# `dog4`

`dog4` is an experimental, fun and decidedly esoteric programming language.
It's a mix of Rust and Bash, it's the successor of [dog3](https://github.com/henriquecolini/dog3),
and it's built to be embedded — its day job is running code that strangers
type into a Telegram bot, so it is fast, small, and sandboxed to the teeth.

**[Try it in your browser](https://henriquecolini.github.io/dog4/)** — the
playground runs dog4 compiled to WebAssembly.

The following program prints "Hello, world!":

```dog
pln Hello, world!
```
```text
Hello, world!
```

This one doesn't:

```dog
{ pln Hello, world! }
```
```text
```

What happened? The output of `pln` got **captured** by the block. In dog4, the
very act of *outputting* a value is how you use it: every statement outputs
into its enclosing block, and a block's value is everything its statements
output. Blocks are values, so you can output them too:

```dog
put { pln Hello, world! }
```
```text
Hello, world!
```

`put` outputs its argument unchanged — it's the most common function you'll
ever use. `pln` does the same, but turns everything into text and adds a newline.

## Running it

```sh
cargo run -- script.dog          # run a file (modules next to it can be `use`d)
cargo run -- -e 'pln hi'         # run a snippet
cargo run                        # a REPL
cargo run -- --sandbox file.dog  # with the limits used for untrusted code
cargo run -- --disasm file.dog   # show the bytecode instead of running
```

## Words

Like in Bash, arguments are separated by spaces, and a bare word is just text.
Quote or escape to keep spaces:

```dog
pln "Double     quotes!";
pln 'Single     quotes!';
pln Escape\ \ \ those\ \ \ spaces!;
pln I'm fine, thanks. a=b 1+1 https://example.com
```
```text
Double     quotes!
Single     quotes!
Escape   those   spaces!
I'm fine, thanks. a=b 1+1 https://example.com
```

One twist: a bare word that is a literal **is** that value. Numbers, `true`,
`false` and `null` are typed; anything else is text, and quotes always make
text:

```dog
pln { type 5 } { type -2.5 } { type 007 } { type true } { type null } { type 5px } { type "5" }
```
```text
num num num bool null str str
```

A backslash escapes punctuation and spaces (`\$`, `\"`, `\ `) or makes a
control character (`\n`, `\t`); anything else, like `\d`, is an error. For text
full of backslashes or quotes, use a **raw string**: three or more `"` on each
side, with no escapes or interpolation inside. Use more quotes than the text
has in a row:

```dog
pln """C:\dir\$file""" """"a "quoted" """word""" """"
```
```text
C:\dir\$file a "quoted" """word""" 
```

Note the semicolons: you need one after every command, except the last one of
a block. Newlines don't end statements.

## Variables

```dog
x = 5;
name = Ana;
pln "Hi $name, you are $x!";
pln '$x stays as is in single quotes';
pln "${name}s" "$(1 + 2) apples"
```
```text
Hi Ana, you are 5!
$x stays as is in single quotes
Anas 3 apples
```

`$name.field` and `$list[0]` read inside values, so write `${name}.txt` when
you mean text.

## Values

dog3 only had strings with a hidden status code. dog4 has everything that fits
in a JSON document — plus functions:

```dog
items = [apple banana "passion fruit"];
user = [name = Ana, age = 30, tags = [admin]];
pln $items $items[0] $items[-1] { len $items };
pln $user $user.name $user.tags[0] $user.missing;
pln { type $items } { type $user } { type (null) } { type $pln }
```
```text
[apple banana "passion fruit"] apple passion fruit 3
[name = Ana, age = 30, tags = [admin]] Ana admin 
arr map null fn
```

`[a b c]` is an array, `[key = value]` is a map; `[]` and `[=]` are empty.
Commas are optional. Ranges slice arrays and text:

```dog
items = [a b c d e];
title = "dog4 rocks";
pln $items[1..3] $items[..2] $items[-2..] $title[5..]
```
```text
[b c] [a b] [d e] rocks
```

## The fold

So what does a block do when its statements output numbers? Arrays? Maps?
It **adds them up**. Every type knows how to combine with itself:

```dog
pln { put 1; put 2; put 3 };
pln { put [a]; put [b] };
pln { put [x = 1]; put [y = 2] };
pln { put true; put false };
pln { put score: ; put 10 }
```
```text
6
[a b]
[x = 1, y = 2]
false
score:10
```

Numbers add, text and arrays concatenate, maps merge, booleans AND, `null` is
nothing at all, and anything added to an array is pushed onto it. This is the
same operation as `+`:

```dog
pln ([1 2] + [3]) ("hi" + 5) ([a = 1] + [b = 2]) (null + 7)
```
```text
[1 2 3] hi5 [a = 1, b = 2] 7
```

Which makes loops into folds:

```dog
prices = [10 25 7];
total = for p in $prices { put $p };
doubled = for p in $prices { put [($p * 2)] };
pln $total $doubled
```
```text
42 [20 50 14]
```

Pieces of a word written without spaces are folded too: `user_$id` is text,
but if `x` is 5, `$x$x` is `10` (inside quotes, `"$x$x"` is always `"55"`).

## Expressions

Math lives inside parentheses and on the right of `=`:

```dog
x = 6 * 7;
pln 1+1 (1 + 1) ($x / 4) (2 * $x - 1) ($x % 5) (-$x);
pln ($x > 40 && $x < 50) (null ?? fallback) (0..4)
```
```text
1+1 2 10.5 83 2 -42
true fallback [0 1 2 3]
```

Inside an expression you call functions as `max(1, 2)`. Commands with
arguments only run inside blocks, so it's `{ len $list }` or `len($list)`;
`(len $list)` and `x = upper hello` are errors.

## Control flow

`if`, `while` and `for` are values too — and like any block, their output is
captured unless you `put` it:

```dog
x = 7;
put if ($x > 5) { pln big } else { pln small };
if ($x > 5) { pln "this is captured and thrown away" };
pln (for word in "the quick fox" { put [{ upper $word }] });
pln (for line in "a,b,c" split "," { put "<$line>" });
pln (for i, v in [x y] { put "$i:$v " });
i = 0;
pln (while ($i < 3) { i += 1; put $i })
```
```text
big
[THE QUICK FOX]
<a><b><c>
0:x 1:y 
6
```

`break` and `continue` work as usual (and keep what was output so far);
`for` over a map gives its keys (or keys and values).

## Functions

```dog
fn greet (name) { pln Hello, $name! }
fn greet () { greet stranger }

fn double (x: num) { put ($x * 2) }

greet; greet dog4;
pln { double 21 } { double "4.5" }
```
```text
Hello, stranger!
Hello, dog4!
42 9
```

Functions can be **overloaded** by number of arguments and by type, and
**typed parameters convert** what they're given when it makes sense: `"21"`
becomes `21` for a `num`, a number becomes text for a `str`, a sentence
becomes an array of words for an `arr`. A `%rest` parameter collects the rest
of the arguments, and `$%list` spreads an array back out:

```dog
fn hug (l, r, %items) { put for x in $items { put $l$x$r } }
words = [a b c];
pln { hug < > $%words d e }
```
```text
<a><b><c><d><e>
```

Functions are values. Lambdas capture variables by reference:

```dog
fn make_counter () {
    n = 0;
    put fn () { n += 1; put $n }
}
count = { make_counter };
pln ($count()) ($count()) ($count());
pln { map [1 2 3] fn (x) { put ($x * $x) } }
```
```text
1 2 3
[1 4 9]
```

## References

Pass `&variable` to let a function change it in place — no copies:

```dog
fn push (list: &arr, item) { list += [$item] }
fn incr (n: &num) { n += 1 }
xs = [];
push &xs a; push &xs b;
score = [points = 0];
incr &score.points;
pln $xs $score
```
```text
[a b] [points = 1]
```

## Errors

```dog
fn risky (x: num) { put (100 / $x) }
pln (try { risky abc } catch e { put "oops: $e.message" });
pln (try { throw [code = 42] } catch e { put $e.value.code })
```
```text
oops: `risky` expects a num for `x`, got str "abc"
42
```

## Modules

A module is a map of the top-level names some code marks `pub`. `mod name`
loads one; the host program finds it, so modules can live on disk, in memory,
or in a database. `mod { … }` is a module literal, and `use` brings a map's
members into scope:

```dog
greet = mod greet;
greet.hi Bob;
pln { keys $greet };
shapes = mod {
    pub fn square (side: num) { put ($side * $side) }
};
pln { shapes.square 4 };
use square from $shapes;
use mod greet;
hi { square 5 }
```
```text
Hello Bob
[hi loaded]
16
Hello 25
```

## Builtins

| group | functions |
|---|---|
| output | `put pln print println` |
| types | `type num str bool arr` |
| sequences | `len first last append range keys values contains count join split chars reverse sum sort index_of last_index_of` |
| arrays & maps | `insert remove_at flatten unique zip chunk remove has entries from_entries` |
| math | `add sub mul div max min floor ceil round trunc abs sign clamp sqrt pow exp ln log sin cos tan asin acos atan atan2 pi` |
| numbers & stats | `fixed parse_int to_base product avg median` |
| random | `random random_float shuffle pick sample uuid` |
| logic | `eq neq lt gt leq geq like and or not true false` |
| text | `upper lower capitalize trim trim_start trim_end pad_left pad_right lines strlen starts_with ends_with ord chr is_alpha is_alphanumeric is_digit is_space` |
| regex | `replace matches search captures captures_all find_all split_re regex_escape` |
| functions | `map filter reduce each sort_by any all find find_index flat_map group_by count_by min_by max_by` |
| encodings | `json.parse json.encode json.pretty gron url.encode url.decode url.query url.parse base64.encode base64.decode hex.encode hex.decode sha256 hmac_sha256` |
| time | `now date.format date.parse date.parts` |
| meta & errors | `src eval fail assert` |
| network | `get post fetch` |

The full reference is in [docs/language.md](docs/language.md).

## Embedding

```rust
use dog4::{Builder, Library, Limits, MemoryProvider, OutputBuffer, Value};

let mut bot = Library::new();
bot.def("reply", "%text", |_, args| {
    let text: Vec<String> = args.iter().map(|v| v.to_string()).collect();
    println!("bot says: {}", text.join(" "));
    Ok(Value::Null)
});

let mut runtime = Builder::sandboxed()
    .limits(Limits::sandbox())
    .output(OutputBuffer::default())
    .provider(MemoryProvider::new().with("util", "pub fn shout (s: str) { put { upper $s } }"))
    .module("bot", bot.into_value())
    .build();

runtime.run("message", r#"bot = mod bot; use mod util; bot.reply { shout hi }"#).unwrap();
```

See [docs/embedding.md](docs/embedding.md) for module providers, native
functions, limits and capabilities, and [docs/internals.md](docs/internals.md)
for how it works inside. Coming from dog3? Read [MIGRATING.md](MIGRATING.md).
Updating older dog4 scripts? Read [CHANGELOG.md](CHANGELOG.md).
