//! Parser tests. Each source is parsed and printed back as a compact
//! S-expression so the expected structure is easy to read.

use proptest::prelude::*;

use super::ast::*;
use super::{SourceId, parse};
use crate::error::{ErrorKind, Limit};
use crate::value::{UpdateOp, Value};

fn sexp_block(block: &Block) -> String {
    let stmts: Vec<String> = block.stmts.iter().map(sexp_stmt).collect();
    format!("{{{}}}", stmts.join(" "))
}

fn sexp_stmt(stmt: &Stmt) -> String {
    match &stmt.kind {
        StmtKind::Output(e) => sexp(e),
        StmtKind::Discard(e) => format!("(discard {})", sexp(e)),
        StmtKind::Assign { target, op, value } => {
            let op = match op {
                UpdateOp::Set => "=",
                UpdateOp::Add => "+=",
                UpdateOp::Sub => "-=",
                UpdateOp::Mul => "*=",
                UpdateOp::Div => "/=",
            };
            format!("({op} {} {})", sexp_place(target), sexp(value))
        }
        StmtKind::Let { name, value } => match value {
            LetValue::Value(v) => format!("(let {name} {})", sexp(v)),
            LetValue::Ref(p) => format!("(let {name} &{})", sexp_place(p)),
        },
        StmtKind::Fn(decl) => sexp_fn(decl),
        StmtKind::Use { source, items } => match items {
            None => format!("(use * {})", sexp(source)),
            Some(items) => {
                let items: Vec<String> = items
                    .iter()
                    .map(|i| {
                        if i.name == i.alias {
                            i.name.to_string()
                        } else {
                            format!("{} as {}", i.name, i.alias)
                        }
                    })
                    .collect();
                format!("(use [{}] {})", items.join(", "), sexp(source))
            }
        },
        StmtKind::Return(v) => format!("(return{})", opt(v)),
        StmtKind::Clear(v) => format!("(clear{})", opt(v)),
        StmtKind::Break(v) => format!("(break{})", opt(v)),
        StmtKind::Continue => "(continue)".into(),
        StmtKind::Throw(v) => format!("(throw {})", sexp(v)),
    }
}

fn opt(v: &Option<Expr>) -> String {
    v.as_ref()
        .map_or(String::new(), |v| format!(" {}", sexp(v)))
}

fn sexp_place(place: &Place) -> String {
    let mut out = place.root.to_string();
    for key in &place.path {
        match key {
            Key::Field(name) => out.push_str(&format!(".{name}")),
            Key::Index(e) => out.push_str(&format!("[{}]", sexp(e))),
        }
    }
    out
}

fn sexp_fn(decl: &FnDecl) -> String {
    let mut params: Vec<String> = decl
        .params
        .iter()
        .map(|p| {
            let amp = if p.by_ref { "&" } else { "" };
            format!("{}:{amp}{}", p.name, p.ty)
        })
        .collect();
    if let Some(rest) = &decl.rest {
        params.push(format!("%{}:{}", rest.name, rest.ty));
    }
    format!(
        "(fn {} ({}) {})",
        decl.name,
        params.join(" "),
        sexp_block(&decl.body)
    )
}

fn sexp_args(args: &[Arg]) -> String {
    args.iter()
        .map(|a| match a {
            Arg::Value(e) => format!(" {}", sexp(e)),
            Arg::Splat(e) => format!(" $%{}", sexp(e)),
            Arg::Ref(p) => format!(" &{}", sexp_place(p)),
        })
        .collect()
}

fn sexp(expr: &Expr) -> String {
    match &expr.kind {
        // Strings always quoted, so the expected trees are unambiguous.
        ExprKind::Lit(Value::Str(s)) => serde_json::Value::from(s.as_str()).to_string(),
        ExprKind::Lit(Value::Null) => "null".into(),
        ExprKind::Lit(v) => v.repr(),
        ExprKind::Var(name) => format!("${name}"),
        ExprKind::Field(base, name) => format!("{}.{name}", sexp(base)),
        ExprKind::Index(base, i) => format!("{}[{}]", sexp(base), sexp(i)),
        ExprKind::Slice(base, lo, hi) => {
            let bound = |b: &Option<Box<Expr>>| b.as_ref().map_or(String::new(), |b| sexp(b));
            format!("{}[{}..{}]", sexp(base), bound(lo), bound(hi))
        }
        ExprKind::Glue(parts) => {
            let parts: Vec<String> = parts.iter().map(sexp).collect();
            format!("(glue {})", parts.join(" "))
        }
        ExprKind::Interp(parts) => {
            let parts: Vec<String> = parts.iter().map(sexp).collect();
            format!("(interp {})", parts.join(" "))
        }
        ExprKind::Block(b) => sexp_block(b),
        ExprKind::Array(items) => format!("(arr{})", sexp_args(items)),
        ExprKind::Map(entries) => {
            let entries: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!(" {}={}", sexp(k), sexp(v)))
                .collect();
            format!("(map{})", entries.concat())
        }
        ExprKind::Unary(op, e) => {
            let op = if *op == UnOp::Neg { "neg" } else { "!" };
            format!("({op} {})", sexp(e))
        }
        ExprKind::Binary(op, l, r) => {
            let op = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Rem => "%",
                BinOp::Eq => "==",
                BinOp::Ne => "!=",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                BinOp::Ge => ">=",
                BinOp::Range => "..",
            };
            format!("({op} {} {})", sexp(l), sexp(r))
        }
        ExprKind::Logic(op, l, r) => {
            let op = match op {
                LogicOp::And => "&&",
                LogicOp::Or => "||",
                LogicOp::Coalesce => "??",
            };
            format!("({op} {} {})", sexp(l), sexp(r))
        }
        ExprKind::Call(callee, args) => format!("(call {}{})", sexp(callee), sexp_args(args)),
        ExprKind::Lambda(decl) => sexp_fn(decl),
        ExprKind::If(c, t, e) => match e {
            Some(e) => format!("(if {} {} {})", sexp(c), sexp(t), sexp(e)),
            None => format!("(if {} {})", sexp(c), sexp(t)),
        },
        ExprKind::While(c, b) => format!("(while {} {})", sexp(c), sexp(b)),
        ExprKind::For(f) => {
            let vars = match &f.second {
                Some(second) => format!("{} {second}", f.first),
                None => f.first.to_string(),
            };
            let split = f
                .split
                .as_ref()
                .map_or(String::new(), |s| format!(" split {}", sexp(s)));
            format!("(for {vars} {}{split} {})", sexp(&f.iter), sexp(&f.body))
        }
        ExprKind::Try(body, catch) => match catch {
            Some((name, handler)) => format!("(try {} {name} {})", sexp(body), sexp(handler)),
            None => format!("(try {})", sexp(body)),
        },
        ExprKind::Mod(name) => format!("(mod {})", sexp(name)),
        ExprKind::ModLit(body) => format!("(mod {})", sexp_block(body)),
    }
}

/// Parses `src` and returns the S-expression of its top-level statements.
fn p(src: &str) -> String {
    match parse(src, SourceId(0), 64) {
        Ok(block) => {
            let s = sexp_block(&block);
            s[1..s.len() - 1].to_string()
        }
        Err(e) => panic!("parse error in {src:?}: {}", e.message()),
    }
}

/// Parses `src`, expecting a syntax error, and returns its message.
fn err(src: &str) -> String {
    match parse(src, SourceId(0), 64) {
        Ok(block) => panic!("expected an error for {src:?}, got {}", sexp_block(&block)),
        Err(e) => e.message().to_string(),
    }
}

#[test]
fn commands_and_words() {
    assert_eq!(p("pln Hello, world!"), r#"(call $pln "Hello," "world!")"#);
    assert_eq!(
        p("pln 1+1 a=b http://x/y?z"),
        r#"(call $pln "1+1" "a=b" "http://x/y?z")"#
    );
    assert_eq!(p("pln I'm here"), r#"(call $pln "I'm" "here")"#);
    assert_eq!(p("put"), "(call $put)");
    assert_eq!(p("a; b;; c"), "(call $a) (call $b) (call $c)");
    assert_eq!(p("json.gron $x"), "(call $json.gron $x)");
    assert_eq!(p("$f a"), r#"(call $f "a")"#);
    assert_eq!(p("$m.f a"), r#"(call $m.f "a")"#);
}

#[test]
fn bare_literals() {
    assert_eq!(p("put 5 -2.5 true false"), "(call $put 5 -2.5 true false)");
    assert_eq!(
        p("put 007 1.50 1e3 1_000 null"),
        "(call $put 7 1.5 1000 1000 null)"
    );
    assert_eq!(
        p("put +5 5px inf 1. .5 1e 1.2.3 -"),
        r#"(call $put "+5" "5px" "inf" "1." ".5" "1e" "1.2.3" "-")"#
    );
    assert_eq!(p(r#"put "5" '5' "#), r#"(call $put "5" "5")"#);
}

#[test]
fn quoting_and_escapes() {
    assert_eq!(p(r#"put "a b" 'c d'"#), r#"(call $put "a b" "c d")"#);
    assert_eq!(p(r#"put "x\ty\"" 'n\n'"#), r#"(call $put "x\ty\"" "n\n")"#);
    assert_eq!(p(r"put a\ b \$x"), r#"(call $put "a b" "$x")"#);
    assert_eq!(p(r#"put 'raw $x'"#), r#"(call $put "raw $x")"#);
    assert_eq!(p(r#"put cost:$ $"#), r#"(call $put "cost:$" "$")"#);
    assert_eq!(p(r#"put "\.\{\ " '\0'"#), r#"(call $put ".{ " "\u0000")"#);
    for src in [r#"put "\s""#, r"put '\d'", r"put a\wb", r#"put "\é""#] {
        assert!(err(src).contains("unknown escape"), "{src}");
    }
}

#[test]
fn raw_strings() {
    assert_eq!(p(r#"put """a\s $x""""#), r#"(call $put "a\\s $x")"#);
    assert_eq!(
        p(r##"put """"""a """"b"""" c"""""""##),
        r#"(call $put "a \"\"\"\"b\"\"\"\" c")"#
    );
    assert_eq!(p(r#"put x"""y"""z"#), r#"(call $put (glue "x" "y" "z"))"#);
    assert_eq!(
        p("put \"\"\"\n    one\n\n      two\n    \"\"\""),
        r#"(call $put "one\n\n  two")"#
    );
    assert_eq!(p("put \"\"\"\r\n  a\r\n  \"\"\""), r#"(call $put "a")"#);
    assert!(err(r#"put """a"#).contains("unclosed raw string"));
    assert!(err(r##"put """a"""""##).contains("contains 4 quotes"));
    assert!(err("put \"\"\"a\n\"\"\"").contains("starts on the line after"));
    assert!(err("put \"\"\"\na\"\"\"").contains("on their own line"));
    assert!(err("put \"\"\"\n a\n  \"\"\"").contains("indented less"));
}

#[test]
fn slices() {
    assert_eq!(p("put $a[1..2]"), "(call $put $a[1..2])");
    assert_eq!(
        p("put $a[..2] $a[1..] $a[..]"),
        "(call $put $a[..2] $a[1..] $a[..])"
    );
    assert_eq!(p("x = $a[$i + 1..-1]"), "(= x $a[(+ $i 1)..-1])");
    assert_eq!(p("x = $a[$i == 1]"), "(= x $a[(== $i 1)])");
    assert_eq!(p("x = f()[1..][0]"), "(= x (call $f)[1..][0])");
    assert!(err("x[0..1] = 2").contains("can't be assigned"));
}

#[test]
fn interpolation() {
    assert_eq!(
        p(r#"put "hi $name!""#),
        r#"(call $put (interp "hi " $name "!"))"#
    );
    assert_eq!(
        p(r#"put "$u.name $l[0]""#),
        r#"(call $put (interp $u.name " " $l[0]))"#
    );
    assert_eq!(p(r#"put "${n}.txt""#), r#"(call $put (interp $n ".txt"))"#);
    assert_eq!(p(r#"put "$(1 + 2)""#), r#"(call $put (interp (+ 1 2)))"#);
    assert_eq!(p(r#"put "$f(1)""#), r#"(call $put (interp $f "(1)"))"#);
}

#[test]
fn gluing() {
    assert_eq!(p("put user_$id"), r#"(call $put (glue "user_" $id))"#);
    assert_eq!(p("put $x$x"), "(call $put (glue $x $x))");
    assert_eq!(p(r#"put $x"!""#), r#"(call $put (glue $x "!"))"#);
    assert_eq!(p("put $a.b.c!"), r#"(call $put (glue $a.b.c "!"))"#);
    assert_eq!(p("put $x."), r#"(call $put (glue $x "."))"#);
    assert_eq!(p("put $f(1)x"), r#"(call $put (glue (call $f 1) "x"))"#);
}

#[test]
fn literals() {
    assert_eq!(p("put [a b c]"), r#"(call $put (arr "a" "b" "c"))"#);
    assert_eq!(p("put [1, 2,3]"), "(call $put (arr 1 2 3))");
    assert_eq!(p("put [] [=] [ = ]"), "(call $put (arr) (map) (map))");
    assert_eq!(
        p("put [a = 1, b = [x]]"),
        r#"(call $put (map "a"=1 "b"=(arr "x")))"#
    );
    assert_eq!(p("put [$k = (1 + 1)]"), "(call $put (map $k=(+ 1 1)))");
    assert_eq!(p("put [$%xs y]"), r#"(call $put (arr $%$xs "y"))"#);
}

#[test]
fn literal_errors() {
    assert!(err("put [a b = c]").contains("can't mix"));
    assert!(err("put [a = 1 b]").contains("expected `=`"));
    assert!(err("put [a b").contains("unclosed `[`"));
    assert!(err("put [&x = 1]").contains("plain value"));
}

#[test]
fn expressions() {
    assert_eq!(p("x = 1 + 2 * 3"), "(= x (+ 1 (* 2 3)))");
    assert_eq!(p("x = (1 + 2) * 3"), "(= x (* (+ 1 2) 3))");
    assert_eq!(p("x = 10 - 4 - 3"), "(= x (- (- 10 4) 3))");
    assert_eq!(p("x = -$a.b"), "(= x (neg $a.b))");
    assert_eq!(p("x = -5"), "(= x -5)");
    assert_eq!(p("x = !$a || $b && $c"), "(= x (|| (! $a) (&& $b $c)))");
    assert_eq!(p("x = $a ?? $b ?? c"), r#"(= x (?? (?? $a $b) "c"))"#);
    assert_eq!(p("x = 0..$n + 1"), "(= x (.. 0 (+ $n 1)))");
    assert_eq!(p("x = $a == $b"), "(= x (== $a $b))");
    assert_eq!(p("x = 1.5e3"), "(= x 1500)");
    assert_eq!(p("x = 1_000"), "(= x 1000)");
}

#[test]
fn expression_atoms() {
    assert_eq!(p("x = hello"), r#"(= x "hello")"#);
    assert_eq!(p("x = hello.world"), r#"(= x "hello.world")"#);
    assert_eq!(p("x = true"), "(= x true)");
    assert_eq!(p("x = null"), "(= x null)");
    assert_eq!(p("x = Hi$name"), r#"(= x (glue "Hi" $name))"#);
    assert_eq!(p("x = 5px"), r#"(= x (glue 5 "px"))"#);
    assert_eq!(p("x = { put a }"), r#"(= x {(call $put "a")})"#);
    assert_eq!(p("x = [a]"), r#"(= x (arr "a"))"#);
    assert_eq!(p("x = (y)"), r#"(= x "y")"#);
}

#[test]
fn calls_in_expressions() {
    assert_eq!(p("x = max(1, $y)"), "(= x (call $max 1 $y))");
    assert_eq!(p("x = json.encode($v)"), "(= x (call $json.encode $v))");
    assert_eq!(p("x = $f(1)(2)"), "(= x (call (call $f 1) 2))");
    assert_eq!(p("x = f()"), "(= x (call $f))");
    assert_eq!(p("x = f(&y, $%z)"), "(= x (call $f &y $%$z))");
    assert_eq!(
        p("x = (fn (a) { put $a })(1)"),
        "(= x (call (fn <lambda> (a:any) {(call $put $a)}) 1))"
    );
}

#[test]
fn commands_are_not_expressions() {
    for src in [
        "x = len $l",
        "x = (type $l) + 1",
        "x = (abs -3)",
        "x = ($f a b)",
        "x = max(len $l, 2)",
        "x = upper hello world",
        "x = $m.f a",
    ] {
        assert!(
            err(src).contains("commands only run inside blocks"),
            "{src}"
        );
    }
    assert_eq!(p("x = (a - 3)"), r#"(= x (- "a" 3))"#);
    assert_eq!(p("x = { len $l }"), "(= x {(call $len $l)})");
    assert_eq!(
        p("x = max({ len $l }, 2)"),
        "(= x (call $max {(call $len $l)} 2))"
    );
}

#[test]
fn expression_errors() {
    assert!(err("x = 1 < 2 < 3").contains("can't be chained"));
    assert!(err("x = (1 + 2").contains("expected `)`"));
    assert!(err("x = 1 +").contains("expected an expression"));
    assert!(err("x = &y").contains("only allowed as a call argument"));
    assert!(err("x = $%y").contains("only allowed as a call argument"));
}

#[test]
fn assignments() {
    assert_eq!(p("x = 1"), "(= x 1)");
    assert_eq!(p("x.a[0] = 1"), "(= x.a[0] 1)");
    assert_eq!(
        p("x += 1; y -= 2; z *= 3; w /= 4"),
        "(+= x 1) (-= y 2) (*= z 3) (/= w 4)"
    );
    assert_eq!(p("let x = 1"), "(let x 1)");
    assert_eq!(p("let r = &x.a"), "(let r &x.a)");
    assert_eq!(p("x == y"), r#"(call $x "==" "y")"#);
}

#[test]
fn control_flow() {
    assert_eq!(
        p("if $c { put a }"),
        r#"(discard (if $c {(call $put "a")}))"#
    );
    assert_eq!(
        p("if $c a else if $d b else c"),
        r#"(discard (if $c "a" (if $d "b" "c")))"#
    );
    assert_eq!(
        p("put if { true } yes"),
        r#"(call $put (if {(call $true)} "yes"))"#
    );
    assert_eq!(
        p("while ($i < 3) { i += 1 }"),
        "(discard (while (< $i 3) {(+= i 1)}))"
    );
    assert_eq!(
        p("for x in $l { put $x }"),
        "(discard (for x $l {(call $put $x)}))"
    );
    assert_eq!(
        p(r#"for k, v in $m split "," $v"#),
        r#"(discard (for k v $m split "," $v))"#
    );
    assert_eq!(
        p("x = try { a } catch e { b }"),
        "(= x (try {(call $a)} e {(call $b)}))"
    );
    assert_eq!(p("x = try $y"), "(= x (try $y))");
    assert_eq!(
        p("for x in $l { if $x { break done }; continue }"),
        r#"(discard (for x $l {(discard (if $x {(break "done")})) (continue)}))"#
    );
    assert_eq!(
        p("return; clear x; throw $e"),
        r#"(return) (clear "x") (throw $e)"#
    );
}

#[test]
fn functions() {
    assert_eq!(p("fn f () { put 1 }"), "(fn f () {(call $put 1)})");
    assert_eq!(p("fn f(a,b) {}"), "(fn f (a:any b:any) {})");
    assert_eq!(
        p("fn f (a: num, b: str|num, c: arr?, d: &map, e: &, %rest: str) {}"),
        "(fn f (a:num b:num|str c:arr? d:&map e:&any %rest:str) {})"
    );
    assert_eq!(
        p("x = fn (a) { put $a }"),
        "(= x (fn <lambda> (a:any) {(call $put $a)}))"
    );
    assert_eq!(
        p("map $l fn (x) { put $x }"),
        "(call $map $l (fn <lambda> (x:any) {(call $put $x)}))"
    );
    // A value body is the block `{ put value }`.
    assert_eq!(
        p(r#"fn f (n) "hi $n""#),
        r#"(fn f (n:any) {(interp "hi " $n)})"#
    );
    assert_eq!(
        p("fs = [fn (x) $x, fn (x) ($x + 1)]"),
        "(= fs (arr (fn <lambda> (x:any) {$x}) (fn <lambda> (x:any) {(+ $x 1)})))"
    );
    assert_eq!(p("f &x &y.z[0]"), "(call $f &x &y.z[0])");
    assert_eq!(p("f $%args"), "(call $f $%$args)");
}

#[test]
fn function_errors() {
    assert!(err("fn f (%a, b) {}").contains("must be the last"));
    assert!(err("fn f (a, a) {}").contains("duplicate parameter"));
    assert!(err("fn f (a: wat) {}").contains("unknown type `wat`"));
    assert!(err("fn f (%a: &) {}").contains("can't be a reference"));
    assert!(err("fn () {}").contains("give it a name"));
    assert!(err("fn f").contains("expected `(`"));
    assert!(err("fn f ()").contains("expected the function body"));
    assert!(err("x = [fn (x)]").contains("expected the function body"));
}

#[test]
fn modules() {
    assert_eq!(p(r#"x = mod "net/http""#), r#"(= x (mod "net/http"))"#);
    assert_eq!(p("x = mod ./util"), r#"(= x (mod "./util"))"#);
    assert_eq!(p("x = mod $name"), "(= x (mod $name))");
    assert_eq!(p("x = mod { y = 1 }"), "(= x (mod {(= y 1)}))");
    assert_eq!(p("put mod x"), r#"(call $put (mod "x"))"#);
    assert_eq!(
        p("mod x; mod {}"),
        r#"(discard (mod "x")) (discard (mod {}))"#
    );
    assert_eq!(p("use $m"), "(use * $m)");
    assert_eq!(p("use mod foo"), r#"(use * (mod "foo"))"#);
    assert_eq!(p("use a from $m"), "(use [a] $m)");
    assert_eq!(
        p("use a, b as c,\n d from mod m"),
        r#"(use [a, b as c, d] (mod "m"))"#
    );
    assert_eq!(p("pub use a from $m"), "(use [a] $m)");
    assert!(err("use foo").contains("use mod foo;"));
    assert!(err(r#"use "net/http""#).contains("use mod net/http;"));
    assert!(err("mod m { x = 1 }").contains("`m = mod { … };`"));
    assert!(err("use a, b $m").contains("expected `,` or `from`"));
    assert!(err("x = (use $m)").contains("write `mod name`"));
    assert!(err("mod x pln y").contains("`;`"));
}

#[test]
fn statement_separators() {
    assert_eq!(
        p("if $a {} pln x"),
        r#"(discard (if $a {})) (call $pln "x")"#
    );
    assert_eq!(
        p("{ a } { b }"),
        "(discard {(call $a)}) (discard {(call $b)})"
    );
    assert_eq!(p("fn f () {} f"), "(fn f () {}) (call $f)");
    assert!(err("x = 1 y = 2").contains("expected `;`"));
    assert!(err("pln a }").contains("unexpected `}`"));
    assert!(err("{ pln a").contains("expected `}`"));
}

#[test]
fn comments() {
    assert_eq!(
        p("pln a; // comment\npln b"),
        r#"(call $pln "a") (call $pln "b")"#
    );
    assert_eq!(p("pln a // comment\nb"), r#"(call $pln "a" "b")"#);
    assert_eq!(
        p("pln a /* x /* nested */ y */ b"),
        r#"(call $pln "a" "b")"#
    );
    assert_eq!(p("pln a//b"), r#"(call $pln "a//b")"#);
    assert_eq!(
        p(r#"pln "//not a comment""#),
        r#"(call $pln "//not a comment")"#
    );
    assert!(err("pln /* open").contains("unclosed `/*`"));
}

#[test]
fn unicode() {
    assert_eq!(
        p("cão = 1; pln $cão é"),
        r#"(= cão 1) (call $pln $cão "é")"#
    );
    assert_eq!(p(r#"pln "$unit²""#), r#"(call $pln (interp $unit "²"))"#);
}

#[test]
fn nesting_limit() {
    let deep = format!("x = {}1{}", "(".repeat(100), ")".repeat(100));
    let err = parse(&deep, SourceId(0), 64).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Limit(Limit::Nesting));
    let ok = format!("x = {}1{}", "(".repeat(20), ")".repeat(20));
    assert!(parse(&ok, SourceId(0), 64).is_ok());
    let blocks = format!("{}{}", "{".repeat(100), "}".repeat(100));
    assert!(parse(&blocks, SourceId(0), 64).is_err());
}

#[test]
fn spans_point_at_the_problem() {
    let e = parse("pln \"abc", SourceId(0), 64).unwrap_err();
    assert_eq!(e.loc().map(|(_, s)| (s.start, s.end)), Some((4, 5)));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// The parser must never panic, whatever it is fed.
    #[test]
    fn never_panics_on_arbitrary_text(src in "\\PC{0,80}") {
        let _ = parse(&src, SourceId(0), 32);
    }

    /// Nor on text made of dog4's own punctuation, which reaches deeper paths.
    #[test]
    fn never_panics_on_syntax_soup(src in "[ a$%&()\\[\\]{}=;,.:'\"\\\\+*/<>!?|0-9-]{0,60}") {
        let _ = parse(&src, SourceId(0), 32);
    }
}

#[test]
fn a_missing_semicolon_is_not_a_command() {
    assert!(err("x = Hello\nfn hi () {}").contains("expected `;`"));
}
