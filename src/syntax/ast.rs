//! The abstract syntax tree produced by the parser.

use std::rc::Rc;

use super::Span;
use crate::value::{Type, UpdateOp, Value};

/// A name in the source.
pub type Ident = Rc<str>;

/// `{ stmt; stmt; … }` — or a whole file.
#[derive(Debug, Clone)]
pub struct Block {
    /// The statements, in order.
    pub stmts: Vec<Stmt>,
    /// Source span.
    pub span: Span,
}

/// A statement.
#[derive(Debug, Clone)]
pub struct Stmt {
    /// What kind of statement.
    pub kind: StmtKind,
    /// Marked `pub`: the name it declares is exported from its module.
    pub public: bool,
    /// Source span.
    pub span: Span,
}

/// The statement kinds.
#[derive(Debug, Clone)]
pub enum StmtKind {
    /// A command call; its value is folded into the enclosing block.
    Output(Expr),
    /// A bare block or control form; evaluated for its effects, value discarded.
    Discard(Expr),
    /// `place = value`, `place += value`, …
    Assign {
        /// What is assigned to.
        target: Place,
        /// `=` or a compound operator.
        op: UpdateOp,
        /// The new value.
        value: Expr,
    },
    /// `let name = value` (always declares a fresh local).
    Let {
        /// The new variable.
        name: Ident,
        /// Its value, or a reference (`let r = &x`).
        value: LetValue,
    },
    /// `fn name (params) { … }` (hoisted).
    Fn(Rc<FnDecl>),
    /// `use map` (a glob import) or `use a, b as c from map`.
    Use {
        /// The map whose members are imported.
        source: Expr,
        /// The imported names, or `None` for a glob import.
        items: Option<Vec<UseItem>>,
    },
    /// `return [value]`.
    Return(Option<Expr>),
    /// `clear [value]`.
    Clear(Option<Expr>),
    /// `break [value]`.
    Break(Option<Expr>),
    /// `continue`.
    Continue,
    /// `throw value`.
    Throw(Expr),
}

/// The right side of `let`.
#[derive(Debug, Clone)]
pub enum LetValue {
    /// `let x = expr`
    Value(Expr),
    /// `let x = &place`
    Ref(Place),
}

/// An assignable location: a variable followed by fields and indices.
#[derive(Debug, Clone)]
pub struct Place {
    /// The variable.
    pub root: Ident,
    /// The path below it.
    pub path: Vec<Key>,
    /// Source span.
    pub span: Span,
}

/// One step of a path.
#[derive(Debug, Clone)]
pub enum Key {
    /// `.name`
    Field(Ident),
    /// `[expr]`
    Index(Expr),
}

/// An expression.
#[derive(Debug, Clone)]
pub struct Expr {
    /// What kind of expression.
    pub kind: ExprKind,
    /// Source span.
    pub span: Span,
}

/// Binary operators (other than the short-circuiting ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+` (the fold)
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `..`
    Range,
}

/// Short-circuiting operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOp {
    /// `&&`
    And,
    /// `||`
    Or,
    /// `??`
    Coalesce,
}

/// Prefix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `-`
    Neg,
    /// `!`
    Not,
}

/// The expression kinds.
#[derive(Debug, Clone)]
pub enum ExprKind {
    /// A constant.
    Lit(Value),
    /// A variable (or function) by name.
    Var(Ident),
    /// `expr.name`
    Field(Box<Expr>, Ident),
    /// `expr[expr]`
    Index(Box<Expr>, Box<Expr>),
    /// `expr[lo..hi]`, `expr[lo..]`, `expr[..hi]`, `expr[..]`
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    /// Pieces of one word, combined with the fold: `user_$id`.
    Glue(Vec<Expr>),
    /// Pieces of a double-quoted string, concatenated as text.
    Interp(Vec<Expr>),
    /// `{ … }`
    Block(Block),
    /// `[a b c]`
    Array(Vec<Arg>),
    /// `[k = v, …]`
    Map(Vec<(Expr, Expr)>),
    /// `-x`, `!x`
    Unary(UnOp, Box<Expr>),
    /// `a op b`
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `a && b`, `a || b`, `a ?? b`
    Logic(LogicOp, Box<Expr>, Box<Expr>),
    /// A call: `name args…` in command mode, `f(args)` in expressions.
    Call(Box<Expr>, Vec<Arg>),
    /// `fn (params) { … }`
    Lambda(Rc<FnDecl>),
    /// `if cond then [else other]`
    If(Box<Expr>, Box<Expr>, Option<Box<Expr>>),
    /// `while cond body`
    While(Box<Expr>, Box<Expr>),
    /// `for x in list [split sep] body` / `for k, v in …`
    For(Box<ForLoop>),
    /// `try body [catch name handler]`
    Try(Box<Expr>, Option<(Ident, Box<Expr>)>),
    /// `mod name`: loads a module and gives its map.
    Mod(Box<Expr>),
    /// `mod { … }`: a module literal.
    ModLit(Rc<Block>),
}

/// One name in `use a, b as c from map`.
#[derive(Debug, Clone)]
pub struct UseItem {
    /// The key in the map.
    pub name: Ident,
    /// The variable to bind (`name` unless renamed with `as`).
    pub alias: Ident,
}

/// The parts of a `for` loop.
#[derive(Debug, Clone)]
pub struct ForLoop {
    /// `x` in `for x in …`, or `k` in `for k, v in …`.
    pub first: Ident,
    /// `v` in `for k, v in …`.
    pub second: Option<Ident>,
    /// What is iterated.
    pub iter: Expr,
    /// `split sep`.
    pub split: Option<Expr>,
    /// The body.
    pub body: Expr,
}

/// A call argument.
#[derive(Debug, Clone)]
pub enum Arg {
    /// A plain value.
    Value(Expr),
    /// `$%list`: spread an array into separate arguments.
    Splat(Expr),
    /// `&place`: pass a reference.
    Ref(Place),
}

/// A function definition (named or lambda).
#[derive(Debug)]
pub struct FnDecl {
    /// Its name (`<lambda>` for anonymous functions).
    pub name: Ident,
    /// Positional parameters.
    pub params: Vec<Param>,
    /// The `%rest` parameter.
    pub rest: Option<Param>,
    /// The body.
    pub body: Block,
    /// Source span of the whole definition.
    pub span: Span,
    /// Source text of the whole definition (for `src`).
    pub source: Rc<str>,
}

/// A parameter.
#[derive(Debug, Clone)]
pub struct Param {
    /// Its name.
    pub name: Ident,
    /// Accepted types.
    pub ty: Type,
    /// Whether it is a reference parameter (`x: &arr`).
    pub by_ref: bool,
    /// Source span.
    pub span: Span,
}

impl Expr {
    /// Creates an expression.
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        Expr { kind, span }
    }
}
