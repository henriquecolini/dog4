//! A hand-written, scannerless recursive-descent parser.
//!
//! dog4 has two lexical modes, so the parser reads characters directly and
//! decides how to interpret them from context:
//!
//! * **command mode** (statements, command arguments, literal elements): bare
//!   words are text, and pieces written without spaces glue together
//!   (`user_$id"!"`).
//! * **expression mode** (inside `( … )` and after `=`): numbers, operators and
//!   calls, see [`expr`](super::expr).

use std::rc::Rc;

use super::ast::*;
use super::{SourceId, Span};
use crate::error::{Error, ErrorKind, Limit, Result};
use crate::value::{Type, UpdateOp, Value};

/// Parses a whole source file into its top-level block.
pub fn parse(src: &str, source: SourceId, max_depth: usize) -> Result<Block> {
    let mut parser = Parser::new(src, source, max_depth);
    let stmts = parser.stmts(false)?;
    Ok(Block {
        stmts,
        span: Span::new(0, src.len()),
    })
}

/// Parses a parameter list written in dog4 syntax, e.g. `"a, b: num, %rest: str"`.
/// Used to declare the signatures of native functions.
pub fn parse_params(src: &str) -> Result<(Vec<Param>, Option<Param>)> {
    let wrapped = format!("({src})");
    let mut parser = Parser::new(&wrapped, SourceId(u32::MAX), 16);
    let params = parser.params()?;
    parser.skip_trivia()?;
    if parser.peek().is_some() {
        return Err(parser.error("unexpected text after the parameter list"));
    }
    Ok(params)
}

/// How a word is being read, which decides where it ends.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WordMode {
    /// A command argument: ends at whitespace or `; { } ( ) [ ]`.
    Command,
    /// An element inside `[ … ]`: also ends at `,` and `=`.
    Bracket,
    /// Inside an expression: only identifier characters, `$` pieces and quotes.
    Expr,
    /// An argument of a command inside an expression: also ends at `,`.
    Embedded,
}

pub(super) struct Parser<'a> {
    pub(super) src: &'a str,
    pub(super) pos: usize,
    source: SourceId,
    depth: usize,
    max_depth: usize,
}

/// Keywords that start a statement.
const STMT_KEYWORDS: &[&str] = &[
    "fn", "mod", "use", "let", "return", "clear", "break", "continue", "throw", "if", "while",
    "for", "try", "else", "catch",
];

pub(super) fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// Letters (any script), ASCII digits and `_`. Other "numeric" characters such
/// as `²` end a name, so `"$unit²"` means `$unit` followed by `²`.
pub(super) fn is_ident_char(c: char) -> bool {
    c.is_alphabetic() || c.is_ascii_digit() || c == '_'
}

fn escape(c: char) -> char {
    match c {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        '0' => '\0',
        other => other,
    }
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, source: SourceId, max_depth: usize) -> Parser<'a> {
        Parser {
            src,
            pos: 0,
            source,
            depth: 0,
            max_depth,
        }
    }

    // ----- character-level helpers -------------------------------------------------

    pub(super) fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    pub(super) fn peek_nth(&self, n: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(n)
    }

    pub(super) fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    pub(super) fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    pub(super) fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s)
    }

    pub(super) fn span_from(&self, start: usize) -> Span {
        Span::new(start, self.pos)
    }

    pub(super) fn error(&self, message: impl Into<String>) -> Error {
        let end = self.pos + self.peek().map_or(0, char::len_utf8);
        self.error_at(message, Span::new(self.pos, end))
    }

    pub(super) fn error_at(&self, message: impl Into<String>, span: Span) -> Error {
        Error::new(ErrorKind::Syntax, message).at(self.source, span)
    }

    pub(super) fn expect(&mut self, c: char) -> Result<()> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(match self.peek() {
                Some(found) => self.error(format!("expected `{c}`, found `{found}`")),
                None => self.error(format!("expected `{c}`, found the end of the input")),
            })
        }
    }

    pub(super) fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return Err(Error::limit(Limit::Nesting).at(self.source, Span::new(self.pos, self.pos)));
        }
        Ok(())
    }

    pub(super) fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Skips whitespace and comments (`// …` and nestable `/* … */`).
    pub(super) fn skip_trivia(&mut self) -> Result<()> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.starts_with("//") => {
                    while let Some(c) = self.bump() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                Some('/') if self.starts_with("/*") => {
                    let start = self.pos;
                    self.pos += 2;
                    let mut depth = 1;
                    while depth > 0 {
                        if self.starts_with("/*") {
                            self.pos += 2;
                            depth += 1;
                        } else if self.starts_with("*/") {
                            self.pos += 2;
                            depth -= 1;
                        } else if self.bump().is_none() {
                            return Err(
                                self.error_at("unclosed `/*` comment", Span::new(start, start + 2))
                            );
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// Reads an identifier at the cursor.
    pub(super) fn ident(&mut self) -> Option<Ident> {
        let start = self.pos;
        if !self.peek().is_some_and(is_ident_start) {
            return None;
        }
        while self.peek().is_some_and(is_ident_char) {
            self.bump();
        }
        Some(self.src[start..self.pos].into())
    }

    fn expect_ident(&mut self, what: &str) -> Result<Ident> {
        self.ident()
            .ok_or_else(|| self.error(format!("expected {what}")))
    }

    /// The keyword at the cursor, if the next identifier is `word` exactly.
    pub(super) fn at_keyword(&self, word: &str) -> bool {
        self.starts_with(word) && !self.src[self.pos + word.len()..].starts_with(is_ident_char)
    }

    /// Like [`at_keyword`](Self::at_keyword), for keywords that start a value
    /// (`if`, `for`, `fn`, …): they must be followed by a space or the start of
    /// another value, so words like `if?` or `for:` stay plain text.
    pub(super) fn at_form_keyword(&self, word: &str) -> bool {
        self.at_keyword(word)
            && match self.src[self.pos + word.len()..].chars().next() {
                None => true,
                Some(c) => c.is_whitespace() || matches!(c, '(' | '{' | '[' | '$' | '"' | '\''),
            }
    }

    fn peek_ident(&self) -> Option<&'a str> {
        let rest = &self.src[self.pos..];
        if !rest.starts_with(is_ident_start) {
            return None;
        }
        let end = rest.find(|c| !is_ident_char(c)).unwrap_or(rest.len());
        Some(&rest[..end])
    }

    fn at_stmt_end(&self) -> bool {
        matches!(self.peek(), None | Some(';') | Some('}'))
    }

    // ----- statements --------------------------------------------------------------

    /// Parses statements until the end of input, or until `}` when `closing`.
    fn stmts(&mut self, closing: bool) -> Result<Vec<Stmt>> {
        let mut stmts = Vec::new();
        loop {
            self.skip_trivia()?;
            if self.eat(';') {
                continue;
            }
            match self.peek() {
                None if closing => {
                    return Err(self.error("expected `}`, found the end of the input"));
                }
                None => break,
                Some('}') if closing => break,
                Some('}') => return Err(self.error("unexpected `}`")),
                _ => {}
            }
            let stmt = self.stmt()?;
            let open = needs_separator(&stmt);
            stmts.push(stmt);
            self.skip_trivia()?;
            if !self.eat(';') && open && !self.at_stmt_end() {
                return Err(self.error("expected `;` between statements"));
            }
        }
        Ok(stmts)
    }

    fn stmt(&mut self) -> Result<Stmt> {
        let start = self.pos;
        let kind = match self.peek_ident() {
            Some(word) if STMT_KEYWORDS.contains(&word) => self.keyword_stmt(word)?,
            _ => match self.peek() {
                Some('{') => StmtKind::Discard(self.block_expr()?),
                Some('$') => self.command_stmt_from_value()?,
                Some(c) if is_ident_start(c) => self.name_stmt()?,
                Some('(') => {
                    return Err(self.error(
                        "a statement can't start with `(`; use `put (…)` to output an expression",
                    ));
                }
                Some(c) => return Err(self.error(format!("expected a statement, found `{c}`"))),
                None => return Err(self.error("expected a statement")),
            },
        };
        Ok(Stmt {
            kind,
            span: self.span_from(start),
        })
    }

    fn keyword_stmt(&mut self, word: &str) -> Result<StmtKind> {
        match word {
            "fn" => {
                let save = self.pos;
                self.pos += 2;
                self.skip_trivia()?;
                let named = self.peek().is_some_and(is_ident_start);
                self.pos = save;
                if !named {
                    return Err(
                        self.error("an anonymous `fn` on its own does nothing; give it a name")
                    );
                }
                Ok(StmtKind::Fn(self.fn_decl()?))
            }
            "mod" => {
                self.pos += 3;
                self.skip_trivia()?;
                let name = self.expect_ident("a module name")?;
                self.skip_trivia()?;
                if self.peek() != Some('{') {
                    return Err(self.error("expected `{` to start the module body"));
                }
                let body = self.block()?;
                Ok(StmtKind::Mod {
                    name,
                    body: Rc::new(body),
                })
            }
            "use" => {
                self.pos += 3;
                self.skip_trivia()?;
                let path = self.value(WordMode::Command)?;
                let save = self.pos;
                self.skip_trivia()?;
                let alias = if self.at_keyword("as") {
                    self.pos += 2;
                    self.skip_trivia()?;
                    self.expect_ident("a name after `as`")?
                } else {
                    self.pos = save;
                    default_alias(&path).ok_or_else(|| {
                        self.error_at(
                            "can't pick a name for this module; add `as name`",
                            path.span,
                        )
                    })?
                };
                Ok(StmtKind::Use { path, alias })
            }
            "let" => {
                self.pos += 3;
                self.skip_trivia()?;
                let name = self.expect_ident("a variable name after `let`")?;
                self.skip_trivia()?;
                if !self.eat('=') || self.peek() == Some('=') {
                    return Err(self.error("expected `=` after the variable name"));
                }
                self.skip_trivia()?;
                let value =
                    if self.peek() == Some('&') && self.peek_nth(1).is_some_and(is_ident_start) {
                        self.bump();
                        LetValue::Ref(self.place()?)
                    } else {
                        LetValue::Value(self.expr()?)
                    };
                Ok(StmtKind::Let { name, value })
            }
            "return" | "clear" | "break" | "throw" => {
                let keyword_span = Span::new(self.pos, self.pos + word.len());
                self.pos += word.len();
                self.skip_trivia()?;
                let value = if self.at_stmt_end() {
                    None
                } else {
                    Some(self.expr()?)
                };
                Ok(match word {
                    "return" => StmtKind::Return(value),
                    "clear" => StmtKind::Clear(value),
                    "break" => StmtKind::Break(value),
                    _ => StmtKind::Throw(
                        value
                            .ok_or_else(|| self.error_at("`throw` needs a value", keyword_span))?,
                    ),
                })
            }
            "continue" => {
                self.pos += word.len();
                Ok(StmtKind::Continue)
            }
            "if" | "while" | "for" | "try" => {
                Ok(StmtKind::Discard(self.control(WordMode::Command)?))
            }
            _ => Err(self.error(format!(
                "`{word}` without a matching `{}`",
                if word == "else" { "if" } else { "try" }
            ))),
        }
    }

    /// A statement starting with an identifier: an assignment or a command.
    fn name_stmt(&mut self) -> Result<StmtKind> {
        let start = self.pos;
        let place = self.place()?;
        if self.peek() == Some('(') {
            let callee = place_to_expr(place);
            let args = self.call_args()?;
            return Ok(StmtKind::Output(Expr::new(
                ExprKind::Call(Box::new(callee), args),
                self.span_from(start),
            )));
        }
        let after_name = self.pos;
        self.skip_trivia()?;
        if let Some((op, len)) = self.assign_op() {
            self.pos += len;
            self.skip_trivia()?;
            let value = self.expr()?;
            return Ok(StmtKind::Assign {
                target: place,
                op,
                value,
            });
        }
        self.pos = after_name;
        let callee = place_to_expr(place);
        self.command(callee, start)
    }

    /// `$f args…` or `$(expr) args…`.
    fn command_stmt_from_value(&mut self) -> Result<StmtKind> {
        let start = self.pos;
        let callee = match self.dollar_piece(false)? {
            Some(callee) => callee,
            None => return Err(self.error("expected a variable after `$`")),
        };
        if self.peek() == Some('(') {
            let args = self.call_args()?;
            return Ok(StmtKind::Output(Expr::new(
                ExprKind::Call(Box::new(callee), args),
                self.span_from(start),
            )));
        }
        self.command(callee, start)
    }

    fn command(&mut self, callee: Expr, start: usize) -> Result<StmtKind> {
        let mut args = Vec::new();
        loop {
            self.skip_trivia()?;
            match self.peek() {
                None | Some(';') | Some('}') => break,
                Some(c @ (')' | ']')) => return Err(self.error(format!("unexpected `{c}`"))),
                _ => args.push(self.arg(WordMode::Command)?),
            }
        }
        Ok(StmtKind::Output(Expr::new(
            ExprKind::Call(Box::new(callee), args),
            self.span_from(start),
        )))
    }

    fn assign_op(&self) -> Option<(UpdateOp, usize)> {
        let rest = &self.src[self.pos..];
        for (text, op) in [
            ("+=", UpdateOp::Add),
            ("-=", UpdateOp::Sub),
            ("*=", UpdateOp::Mul),
            ("/=", UpdateOp::Div),
        ] {
            if rest.starts_with(text) {
                return Some((op, 2));
            }
        }
        (rest.starts_with('=') && !rest.starts_with("==")).then_some((UpdateOp::Set, 1))
    }

    /// `name`, `name.field`, `name[expr]`, …
    pub(super) fn place(&mut self) -> Result<Place> {
        let start = self.pos;
        let root = self.expect_ident("a name")?;
        let mut path = Vec::new();
        loop {
            if self.peek() == Some('.') && self.peek_nth(1).is_some_and(is_ident_char) {
                self.bump();
                path.push(Key::Field(self.field_name()));
            } else if self.peek() == Some('[') {
                path.push(Key::Index(self.index()?));
            } else {
                break;
            }
        }
        Ok(Place {
            root,
            path,
            span: self.span_from(start),
        })
    }

    /// A field name after `.`: identifier characters, digits allowed first (`$list.0`).
    pub(super) fn field_name(&mut self) -> Ident {
        let start = self.pos;
        while self.peek().is_some_and(is_ident_char) {
            self.bump();
        }
        self.src[start..self.pos].into()
    }

    /// `[expr]` after a value.
    pub(super) fn index(&mut self) -> Result<Expr> {
        self.expect('[')?;
        self.skip_trivia()?;
        let index = self.expr()?;
        self.skip_trivia()?;
        self.expect(']')?;
        Ok(index)
    }

    // ----- functions -----------------------------------------------------------------

    /// `fn name (params) { … }`
    fn fn_decl(&mut self) -> Result<Rc<FnDecl>> {
        let start = self.pos;
        self.pos += 2;
        self.skip_trivia()?;
        let name = self.expect_ident("a function name")?;
        self.fn_rest(start, name)
    }

    /// `fn (params) { … }`
    pub(super) fn lambda(&mut self) -> Result<Rc<FnDecl>> {
        let start = self.pos;
        self.pos += 2;
        self.fn_rest(start, "<lambda>".into())
    }

    fn fn_rest(&mut self, start: usize, name: Ident) -> Result<Rc<FnDecl>> {
        self.skip_trivia()?;
        if self.peek() != Some('(') {
            return Err(self.error("expected `(` to start the parameter list"));
        }
        let (params, rest) = self.params()?;
        self.skip_trivia()?;
        if self.peek() != Some('{') {
            return Err(self.error("expected `{` to start the function body"));
        }
        let body = self.block()?;
        let span = self.span_from(start);
        Ok(Rc::new(FnDecl {
            name,
            params,
            rest,
            body,
            span,
            source: self.src[start..self.pos].into(),
        }))
    }

    /// `(a, b: num, c: &arr, %rest: str)`
    fn params(&mut self) -> Result<(Vec<Param>, Option<Param>)> {
        self.expect('(')?;
        let mut params: Vec<Param> = Vec::new();
        let mut rest: Option<Param> = None;
        loop {
            self.skip_trivia()?;
            if self.eat(')') {
                break;
            }
            if let Some(rest) = &rest {
                return Err(self.error_at("the `%` parameter must be the last one", rest.span));
            }
            let start = self.pos;
            let is_rest = self.eat('%');
            let name = self.expect_ident("a parameter name")?;
            self.skip_trivia()?;
            let (ty, by_ref) = if self.eat(':') {
                self.skip_trivia()?;
                self.param_type()?
            } else {
                (Type::ANY, false)
            };
            let param = Param {
                name,
                ty,
                by_ref,
                span: self.span_from(start),
            };
            if params
                .iter()
                .chain(rest.iter())
                .any(|p| p.name == param.name)
            {
                return Err(
                    self.error_at(format!("duplicate parameter `{}`", param.name), param.span)
                );
            }
            if is_rest {
                if by_ref {
                    return Err(self.error_at("a `%` parameter can't be a reference", param.span));
                }
                rest = Some(param);
            } else {
                params.push(param);
            }
            self.skip_trivia()?;
            if !self.eat(',') {
                self.skip_trivia()?;
                self.expect(')')?;
                break;
            }
        }
        Ok((params, rest))
    }

    /// `num`, `str|num`, `num?`, `&arr`, `&`.
    fn param_type(&mut self) -> Result<(Type, bool)> {
        let by_ref = self.eat('&');
        if by_ref && !self.peek().is_some_and(is_ident_start) {
            return Ok((Type::ANY, true));
        }
        let mut ty: Option<Type> = None;
        loop {
            let start = self.pos;
            let name = self.expect_ident("a type name")?;
            let one = Type::from_name(&name).ok_or_else(|| {
                self.error_at(
                    format!(
                        "unknown type `{name}` (expected any, str, num, bool, arr, map, fn or null)"
                    ),
                    self.span_from(start),
                )
            })?;
            ty = Some(ty.map_or(one, |t| t.union(one)));
            if !self.eat('|') {
                break;
            }
        }
        let mut ty = ty.unwrap_or(Type::ANY);
        if self.eat('?') {
            ty = ty.union(Type::NULL);
        }
        Ok((ty, by_ref))
    }

    // ----- command-mode values ------------------------------------------------------------

    /// One argument: `&place`, `$%list`, or a value.
    pub(super) fn arg(&mut self, mode: WordMode) -> Result<Arg> {
        if self.peek() == Some('&') && self.peek_nth(1).is_some_and(is_ident_start) {
            self.bump();
            let place = self.place()?;
            self.end_of_word("a reference")?;
            return Ok(Arg::Ref(place));
        }
        if self.starts_with("$%") {
            let start = self.pos;
            self.pos += 2;
            if !self.peek().is_some_and(is_ident_start) {
                return Err(self.error("expected a variable after `$%`"));
            }
            let place = self.place()?;
            let expr = place_to_expr(place);
            self.end_of_word("a `$%` splat")?;
            return Ok(Arg::Splat(Expr::new(expr.kind, self.span_from(start))));
        }
        Ok(Arg::Value(self.value(mode)?))
    }

    fn end_of_word(&self, what: &str) -> Result<()> {
        match self.peek() {
            Some(c) if !c.is_whitespace() && !matches!(c, ';' | '}' | ')' | ']' | ',') => {
                Err(self.error(format!("{what} must be a whole word")))
            }
            _ => Ok(()),
        }
    }

    /// A command-mode value: block, literal, `( expr )`, control form, lambda, or word.
    pub(super) fn value(&mut self, mode: WordMode) -> Result<Expr> {
        self.enter()?;
        let result = self.value_inner(mode);
        self.leave();
        result
    }

    fn value_inner(&mut self, mode: WordMode) -> Result<Expr> {
        let atom = match self.peek() {
            Some('{') => self.block_expr()?,
            Some('[') => self.literal()?,
            Some('(') => self.paren()?,
            _ => {
                for word in ["if", "while", "for", "try"] {
                    if self.at_form_keyword(word) {
                        return self.control(mode);
                    }
                }
                if self.at_form_keyword("fn") {
                    return self.lambda_expr();
                }
                return self.word(mode);
            }
        };
        self.path_postfix(atom)
    }

    /// `.field` and `[index]` directly after a value.
    pub(super) fn path_postfix(&mut self, mut expr: Expr) -> Result<Expr> {
        loop {
            let start = expr.span.start as usize;
            if self.peek() == Some('.') && self.peek_nth(1).is_some_and(is_ident_char) {
                self.bump();
                let name = self.field_name();
                expr = Expr::new(ExprKind::Field(Box::new(expr), name), self.span_from(start));
            } else if self.peek() == Some('[') {
                let index = self.index()?;
                expr = Expr::new(
                    ExprKind::Index(Box::new(expr), Box::new(index)),
                    self.span_from(start),
                );
            } else {
                return Ok(expr);
            }
        }
    }

    pub(super) fn lambda_expr(&mut self) -> Result<Expr> {
        let decl = self.lambda()?;
        let span = decl.span;
        Ok(Expr::new(ExprKind::Lambda(decl), span))
    }

    fn block(&mut self) -> Result<Block> {
        let start = self.pos;
        self.enter()?;
        self.expect('{')?;
        let stmts = self.stmts(true)?;
        self.expect('}')?;
        self.leave();
        Ok(Block {
            stmts,
            span: self.span_from(start),
        })
    }

    pub(super) fn block_expr(&mut self) -> Result<Expr> {
        let block = self.block()?;
        let span = block.span;
        Ok(Expr::new(ExprKind::Block(block), span))
    }

    /// `( expr )`
    pub(super) fn paren(&mut self) -> Result<Expr> {
        let start = self.pos;
        self.expect('(')?;
        self.skip_trivia()?;
        let mut inner = self.expr()?;
        self.skip_trivia()?;
        if self.peek() != Some(')') {
            return Err(match self.peek() {
                Some(c) => self.error(format!("expected `)` or an operator, found `{c}`")),
                None => self.error("expected `)`, found the end of the input"),
            });
        }
        self.bump();
        inner.span = self.span_from(start);
        Ok(inner)
    }

    /// `[a b c]`, `[k = v]`, `[]`, `[=]`.
    fn literal(&mut self) -> Result<Expr> {
        let start = self.pos;
        self.enter()?;
        self.expect('[')?;
        self.skip_trivia()?;
        if self.peek() == Some('=') && self.peek_nth(1) != Some('=') {
            self.bump();
            self.skip_trivia()?;
            self.expect(']')?;
            self.leave();
            return Ok(Expr::new(ExprKind::Map(Vec::new()), self.span_from(start)));
        }
        let mut items = Vec::new();
        let mut entries = Vec::new();
        loop {
            self.skip_trivia()?;
            while self.eat(',') {
                self.skip_trivia()?;
            }
            if self.eat(']') {
                break;
            }
            if self.peek().is_none() {
                return Err(self.error_at("unclosed `[`", Span::new(start, start + 1)));
            }
            let arg = self.arg(WordMode::Bracket)?;
            self.skip_trivia()?;
            if self.peek() == Some('=') && self.peek_nth(1) != Some('=') {
                let Arg::Value(key) = arg else {
                    return Err(self.error("a map key must be a plain value"));
                };
                if !items.is_empty() {
                    return Err(self.error_at("can't mix array items and map entries", key.span));
                }
                self.bump();
                self.skip_trivia()?;
                let value = self.value(WordMode::Bracket)?;
                entries.push((key, value));
            } else {
                if !entries.is_empty() {
                    return Err(self.error("expected `=` after the map key"));
                }
                items.push(arg);
            }
        }
        self.leave();
        let kind = if entries.is_empty() {
            ExprKind::Array(items)
        } else {
            ExprKind::Map(entries)
        };
        Ok(Expr::new(kind, self.span_from(start)))
    }

    /// `if`, `while`, `for` and `try` forms. Their parts are command-mode values.
    pub(super) fn control(&mut self, mode: WordMode) -> Result<Expr> {
        let start = self.pos;
        let word = self.peek_ident().unwrap_or_default();
        self.pos += word.len();
        self.skip_trivia()?;
        let kind = match word {
            "if" => {
                let cond = self.value(mode)?;
                self.skip_trivia()?;
                let then = self.value(mode)?;
                let save = self.pos;
                self.skip_trivia()?;
                let otherwise = if self.at_keyword("else") {
                    self.pos += 4;
                    self.skip_trivia()?;
                    Some(Box::new(self.value(mode)?))
                } else {
                    self.pos = save;
                    None
                };
                ExprKind::If(Box::new(cond), Box::new(then), otherwise)
            }
            "while" => {
                let cond = self.value(mode)?;
                self.skip_trivia()?;
                let body = self.value(mode)?;
                ExprKind::While(Box::new(cond), Box::new(body))
            }
            "for" => {
                let first = self.expect_ident("a loop variable")?;
                self.skip_trivia()?;
                let second = if self.eat(',') {
                    self.skip_trivia()?;
                    let second = self.expect_ident("a second loop variable")?;
                    self.skip_trivia()?;
                    Some(second)
                } else {
                    None
                };
                if !self.at_keyword("in") {
                    return Err(self.error("expected `in`"));
                }
                self.pos += 2;
                self.skip_trivia()?;
                let iter = self.value(mode)?;
                self.skip_trivia()?;
                let split = if self.at_keyword("split") {
                    self.pos += 5;
                    self.skip_trivia()?;
                    let sep = self.value(mode)?;
                    self.skip_trivia()?;
                    Some(sep)
                } else {
                    None
                };
                let body = self.value(mode)?;
                ExprKind::For(Box::new(ForLoop {
                    first,
                    second,
                    iter,
                    split,
                    body,
                }))
            }
            _ => {
                let body = self.value(mode)?;
                let save = self.pos;
                self.skip_trivia()?;
                let catch = if self.at_keyword("catch") {
                    self.pos += 5;
                    self.skip_trivia()?;
                    let name = self.expect_ident("a name for the caught error")?;
                    self.skip_trivia()?;
                    Some((name, Box::new(self.value(mode)?)))
                } else {
                    self.pos = save;
                    None
                };
                ExprKind::Try(Box::new(body), catch)
            }
        };
        Ok(Expr::new(kind, self.span_from(start)))
    }

    // ----- words and strings -------------------------------------------------------------

    /// One word: bare text, quoted strings and `$` pieces glued together.
    pub(super) fn word(&mut self, mode: WordMode) -> Result<Expr> {
        let start = self.pos;
        let mut pieces: Vec<Expr> = Vec::new();
        let mut text = String::new();
        let mut text_start = self.pos;
        // Whether the word is plain text, with no quotes, escapes or `$` pieces.
        let mut plain = true;
        let flush = |pieces: &mut Vec<Expr>, text: &mut String, from: usize, to: usize| {
            if !text.is_empty() {
                pieces.push(Expr::new(
                    ExprKind::Lit(Value::str(std::mem::take(text))),
                    Span::new(from, to),
                ));
            }
        };
        while let Some(c) = self.peek() {
            let ends = match mode {
                WordMode::Expr => !(is_ident_char(c) || matches!(c, '$' | '"' | '\'')),
                _ => {
                    c.is_whitespace()
                        || matches!(c, ';' | '{' | '}' | '(' | ')' | '[' | ']')
                        || (mode == WordMode::Bracket && matches!(c, ',' | '='))
                        || (mode == WordMode::Embedded && c == ',')
                }
            };
            if ends {
                break;
            }
            match c {
                '"' => {
                    plain = false;
                    flush(&mut pieces, &mut text, text_start, self.pos);
                    pieces.push(self.dquote()?);
                }
                '\'' if (pieces.is_empty() && text.is_empty()) || mode == WordMode::Expr => {
                    plain = false;
                    flush(&mut pieces, &mut text, text_start, self.pos);
                    pieces.push(self.squote()?);
                }
                '\\' if mode != WordMode::Expr => {
                    plain = false;
                    if text.is_empty() {
                        text_start = self.pos;
                    }
                    self.bump();
                    match self.bump() {
                        Some(e) => text.push(escape(e)),
                        None => return Err(self.error("nothing to escape after `\\`")),
                    }
                }
                '$' => match self.dollar_piece(true)? {
                    Some(piece) => {
                        plain = false;
                        flush(&mut pieces, &mut text, text_start, self.pos);
                        pieces.push(piece);
                    }
                    None if mode == WordMode::Expr => {
                        return Err(self.error("expected a variable after `$`"));
                    }
                    None => {
                        if text.is_empty() {
                            text_start = self.pos;
                        }
                        text.push('$');
                        self.bump();
                    }
                },
                c => {
                    if text.is_empty() {
                        text_start = self.pos;
                    }
                    text.push(c);
                    self.bump();
                }
            }
        }
        if plain && pieces.is_empty() && mode != WordMode::Expr {
            if let Some(value) = crate::value::bare_literal(&text) {
                return Ok(Expr::new(ExprKind::Lit(value), self.span_from(start)));
            }
        }
        flush(&mut pieces, &mut text, text_start, self.pos);
        match pieces.len() {
            0 => Err(self.error("expected a value")),
            1 => Ok(pieces.pop().unwrap_or_else(|| unreachable!())),
            _ => Ok(Expr::new(ExprKind::Glue(pieces), self.span_from(start))),
        }
    }

    /// A `$` piece at the cursor: `$name.path`, `${name}` or `$(expr)`, and with
    /// `calls`, `$f(args)`. Returns `None` (consuming nothing) when `$` isn't
    /// followed by one of those.
    pub(super) fn dollar_piece(&mut self, calls: bool) -> Result<Option<Expr>> {
        let start = self.pos;
        match self.peek_nth(1) {
            Some(c) if is_ident_start(c) => {
                self.bump();
                let name_start = self.pos;
                let name = self.ident().unwrap_or_else(|| unreachable!());
                let var = Expr::new(ExprKind::Var(name), self.span_from(name_start));
                let mut expr = self.var_postfix(var)?;
                while calls && self.peek() == Some('(') {
                    let args = self.call_args()?;
                    expr = Expr::new(ExprKind::Call(Box::new(expr), args), self.span_from(start));
                    expr = self.var_postfix(expr)?;
                }
                expr.span = self.span_from(start);
                Ok(Some(expr))
            }
            Some('{') => {
                self.bump();
                self.bump();
                let name = self.expect_ident("a variable name inside `${…}`")?;
                self.expect('}')?;
                Ok(Some(Expr::new(ExprKind::Var(name), self.span_from(start))))
            }
            Some('(') => {
                self.bump();
                let mut inner = self.paren()?;
                inner.span = self.span_from(start);
                Ok(Some(inner))
            }
            _ => Ok(None),
        }
    }

    /// Fields and indices after `$name`: `$user.name`, `$list[0]`, `$m.a[$i].b`.
    fn var_postfix(&mut self, mut expr: Expr) -> Result<Expr> {
        let start = expr.span.start as usize;
        loop {
            if self.peek() == Some('.') && self.peek_nth(1).is_some_and(is_ident_char) {
                self.bump();
                let name = self.field_name();
                expr = Expr::new(ExprKind::Field(Box::new(expr), name), self.span_from(start));
            } else if self.peek() == Some('[') {
                let index = self.index()?;
                expr = Expr::new(
                    ExprKind::Index(Box::new(expr), Box::new(index)),
                    self.span_from(start),
                );
            } else {
                return Ok(expr);
            }
        }
    }

    /// `"…"`: escapes and `$` interpolation; always produces text.
    pub(super) fn dquote(&mut self) -> Result<Expr> {
        let start = self.pos;
        self.expect('"')?;
        let mut pieces = Vec::new();
        let mut text = String::new();
        let mut text_start = self.pos;
        loop {
            match self.peek() {
                None => return Err(self.error_at("unclosed string", Span::new(start, start + 1))),
                Some('"') => {
                    self.bump();
                    break;
                }
                Some('\\') => {
                    self.bump();
                    match self.bump() {
                        Some(e) => text.push(escape(e)),
                        None => {
                            return Err(
                                self.error_at("unclosed string", Span::new(start, start + 1))
                            );
                        }
                    }
                }
                Some('$') => match self.dollar_piece(false)? {
                    Some(piece) => {
                        if !text.is_empty() {
                            pieces.push(Expr::new(
                                ExprKind::Lit(Value::str(std::mem::take(&mut text))),
                                Span::new(text_start, piece.span.start as usize),
                            ));
                        }
                        pieces.push(piece);
                        text_start = self.pos;
                    }
                    None => {
                        text.push('$');
                        self.bump();
                    }
                },
                Some(c) => {
                    text.push(c);
                    self.bump();
                }
            }
        }
        let span = self.span_from(start);
        if pieces.is_empty() {
            return Ok(Expr::new(ExprKind::Lit(Value::str(text)), span));
        }
        if !text.is_empty() {
            pieces.push(Expr::new(
                ExprKind::Lit(Value::str(text)),
                Span::new(text_start, self.pos - 1),
            ));
        }
        Ok(Expr::new(ExprKind::Interp(pieces), span))
    }

    /// `'…'`: escapes, no interpolation.
    pub(super) fn squote(&mut self) -> Result<Expr> {
        let start = self.pos;
        self.expect('\'')?;
        let mut text = String::new();
        loop {
            match self.bump() {
                None => return Err(self.error_at("unclosed string", Span::new(start, start + 1))),
                Some('\'') => break,
                Some('\\') => match self.bump() {
                    Some(e) => text.push(escape(e)),
                    None => {
                        return Err(self.error_at("unclosed string", Span::new(start, start + 1)));
                    }
                },
                Some(c) => text.push(c),
            }
        }
        Ok(Expr::new(
            ExprKind::Lit(Value::str(text)),
            self.span_from(start),
        ))
    }

    /// `(args, …)` with expression-mode arguments.
    pub(super) fn call_args(&mut self) -> Result<Vec<Arg>> {
        self.expect('(')?;
        let mut args = Vec::new();
        loop {
            self.skip_trivia()?;
            if self.eat(')') {
                break;
            }
            let arg = if self.peek() == Some('&') && self.peek_nth(1).is_some_and(is_ident_start) {
                self.bump();
                Arg::Ref(self.place()?)
            } else if self.starts_with("$%") {
                let start = self.pos;
                self.pos += 2;
                let place = self.place()?;
                let expr = place_to_expr(place);
                Arg::Splat(Expr::new(expr.kind, self.span_from(start)))
            } else {
                Arg::Value(self.expr()?)
            };
            args.push(arg);
            self.skip_trivia()?;
            if !self.eat(',') {
                self.skip_trivia()?;
                self.expect(')')?;
                break;
            }
        }
        Ok(args)
    }
}

/// Converts a parsed place into the expression that reads it.
pub(super) fn place_to_expr(place: Place) -> Expr {
    let start = place.span.start as usize;
    let mut expr = Expr::new(ExprKind::Var(place.root), Span::new(start, start));
    for key in place.path {
        let kind = match key {
            Key::Field(name) => ExprKind::Field(Box::new(expr), name),
            Key::Index(index) => ExprKind::Index(Box::new(expr), Box::new(index)),
        };
        expr = Expr::new(kind, place.span);
    }
    expr.span = place.span;
    expr
}

/// Statements that must be followed by `;` (or end their block).
fn needs_separator(stmt: &Stmt) -> bool {
    !matches!(
        stmt.kind,
        StmtKind::Discard(_) | StmtKind::Fn(_) | StmtKind::Mod { .. }
    )
}

/// `use "net/http"` binds `http`; `use "my-lib.dog"` binds `my_lib`.
fn default_alias(path: &Expr) -> Option<Ident> {
    let ExprKind::Lit(Value::Str(text)) = &path.kind else {
        return None;
    };
    let last = text.rsplit('/').next()?;
    let last = last.strip_suffix(".dog").unwrap_or(last);
    if last.is_empty() || last == "." || last == ".." {
        return None;
    }
    let mut name: String = last
        .chars()
        .map(|c| if is_ident_char(c) { c } else { '_' })
        .collect();
    if !name.starts_with(is_ident_start) {
        name.insert(0, '_');
    }
    Some(name.into())
}
