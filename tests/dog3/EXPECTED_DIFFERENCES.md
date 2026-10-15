# dog3 fixtures: intended differences

Every `.dog` file here is a dog3 example; its `.out` file is what the dog3
interpreter printed. dog4 must produce exactly the same, except where a
`.dog4.out` / `.dog4.err` file records an intended difference:

| fixture | dog3 | dog4 | why |
|---|---|---|---|
| `features_math` | `add word 10` outputs nothing and is falsy | type error: `add` expects a num | status codes are gone; typed parameters reject what they can't convert (use `try` to recover) |
| `html` | refuses to start: can't overload the builtin `div` | gets further, then fails: `split` needs a string | script functions now shadow builtins, but `%content` is an array in dog4; forward it with `$%content` (see `examples/html.dog`) |

Other dog3 behaviours that changed (not exercised by these fixtures) are
listed in `MIGRATING.md`.
