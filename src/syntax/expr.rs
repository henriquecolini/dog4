//! Expression mode: what appears inside `( … )` and after `=`.
//!
//! Operators, loosest first: `??`, `||`, `&&`, comparisons (which don't chain),
//! `..`, `+ -`, `* / %`, then prefix `- !`, then postfix `.f [i] (args)`.
//! Bare identifiers are strings unless directly followed by `(`.

use super::ast::*;
use super::parser::{Parser, WordMode, is_ident_char, is_ident_start};
use crate::error::Result;
use crate::value::Value;

enum Op {
    Bin(BinOp),
    Logic(LogicOp),
}

/// Whether an expression is a plain variable path (`$f`, `$m.f`), which may be
/// followed by command arguments.
fn is_path(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Var(_) => true,
        ExprKind::Field(base, _) | ExprKind::Index(base, _) => is_path(base),
        _ => false,
    }
}

/// Binding power of comparisons, which may not chain.
const COMPARISON: u8 = 4;

impl Parser<'_> {
    /// Parses one expression.
    pub(super) fn expr(&mut self) -> Result<Expr> {
        self.enter()?;
        let result = self.binary(0);
        self.leave();
        result
    }

    fn binary(&mut self, min_power: u8) -> Result<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let save = self.pos;
            self.skip_trivia()?;
            let Some((op, power, len)) = self.peek_operator() else {
                self.pos = save;
                return Ok(lhs);
            };
            if power < min_power {
                self.pos = save;
                return Ok(lhs);
            }
            self.pos += len;
            self.skip_trivia()?;
            let rhs = self.binary(power + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = match op {
                Op::Bin(op) => Expr::new(ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span),
                Op::Logic(op) => Expr::new(ExprKind::Logic(op, Box::new(lhs), Box::new(rhs)), span),
            };
            if power == COMPARISON {
                let save = self.pos;
                self.skip_trivia()?;
                if let Some((_, COMPARISON, _)) = self.peek_operator() {
                    return Err(self.error("comparisons can't be chained; combine them with `&&`"));
                }
                self.pos = save;
            }
        }
    }

    fn peek_operator(&self) -> Option<(Op, u8, usize)> {
        let rest = &self.src[self.pos..];
        let two = [
            ("??", Op::Logic(LogicOp::Coalesce), 1),
            ("||", Op::Logic(LogicOp::Or), 2),
            ("&&", Op::Logic(LogicOp::And), 3),
            ("==", Op::Bin(BinOp::Eq), COMPARISON),
            ("!=", Op::Bin(BinOp::Ne), COMPARISON),
            ("<=", Op::Bin(BinOp::Le), COMPARISON),
            (">=", Op::Bin(BinOp::Ge), COMPARISON),
            ("..", Op::Bin(BinOp::Range), 5),
        ];
        for (text, op, power) in two {
            if rest.starts_with(text) {
                return Some((op, power, 2));
            }
        }
        let op = match rest.chars().next()? {
            '<' => (Op::Bin(BinOp::Lt), COMPARISON),
            '>' => (Op::Bin(BinOp::Gt), COMPARISON),
            '+' if !rest.starts_with("+=") => (Op::Bin(BinOp::Add), 6),
            '-' if !rest.starts_with("-=") => (Op::Bin(BinOp::Sub), 6),
            '*' if !rest.starts_with("*=") => (Op::Bin(BinOp::Mul), 7),
            '/' if !rest.starts_with("/=") => (Op::Bin(BinOp::Div), 7),
            '%' => (Op::Bin(BinOp::Rem), 7),
            _ => return None,
        };
        Some((op.0, op.1, 1))
    }

    fn unary(&mut self) -> Result<Expr> {
        let start = self.pos;
        let op = match self.peek() {
            Some('-') => UnOp::Neg,
            Some('!') if self.peek_nth(1) != Some('=') => UnOp::Not,
            _ => return self.postfix(),
        };
        self.bump();
        self.skip_trivia()?;
        self.enter()?;
        let operand = self.unary();
        self.leave();
        let operand = operand?;
        let span = self.span_from(start);
        // Fold `-5` into a literal so constants stay constants.
        if op == UnOp::Neg {
            if let ExprKind::Lit(Value::Int(i)) = operand.kind {
                if let Some(n) = i.checked_neg() {
                    return Ok(Expr::new(ExprKind::Lit(Value::Int(n)), span));
                }
            }
            if let ExprKind::Lit(Value::Float(f)) = operand.kind {
                return Ok(Expr::new(ExprKind::Lit(Value::Float(-f)), span));
            }
        }
        Ok(Expr::new(ExprKind::Unary(op, Box::new(operand)), span))
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut expr = self.atom()?;
        loop {
            let start = expr.span.start as usize;
            match self.peek() {
                Some('.') if self.peek_nth(1).is_some_and(is_ident_char) => {
                    self.bump();
                    let name = self.field_name();
                    expr = Expr::new(ExprKind::Field(Box::new(expr), name), self.span_from(start));
                }
                Some('[') => {
                    let index = self.index()?;
                    expr = Expr::new(
                        ExprKind::Index(Box::new(expr), Box::new(index)),
                        self.span_from(start),
                    );
                }
                Some('(') => {
                    let args = self.call_args()?;
                    expr = Expr::new(ExprKind::Call(Box::new(expr), args), self.span_from(start));
                }
                _ => return Ok(expr),
            }
        }
    }

    fn atom(&mut self) -> Result<Expr> {
        let start = self.pos;
        match self.peek() {
            Some('(') => self.paren(),
            Some('{') => self.block_expr(),
            Some('[') => self.value(WordMode::Command),
            Some('"' | '\'' | '$') => {
                if self.starts_with("$%") {
                    return Err(self.error("a `$%` splat is only allowed as a call argument"));
                }
                let word = self.word(WordMode::Expr)?;
                if is_path(&word) {
                    return self.embedded_command(word, start);
                }
                Ok(word)
            }
            Some(c) if c.is_ascii_digit() => {
                let number = self.number()?;
                self.glue_tail(number, start)
            }
            Some(c) if is_ident_start(c) => self.ident_atom(),
            Some('&') => {
                Err(self
                    .error("a `&` reference is only allowed as a call argument or after `let x =`"))
            }
            Some(c) => Err(self.error(format!("expected an expression, found `{c}`"))),
            None => Err(self.error("expected an expression, found the end of the input")),
        }
    }

    /// A keyword, a call like `max(1, 2)` or `json.encode($x)`, or a bare string.
    fn ident_atom(&mut self) -> Result<Expr> {
        let start = self.pos;
        for word in ["if", "while", "for", "try"] {
            if self.at_form_keyword(word) {
                return self.control(WordMode::Command);
            }
        }
        if self.at_form_keyword("fn") {
            return self.lambda_expr(WordMode::Command);
        }
        if self.at_form_keyword("use") {
            self.pos += 3;
            self.skip_trivia()?;
            let name = self.value(WordMode::Command)?;
            return Ok(Expr::new(
                ExprKind::Use(Box::new(name)),
                self.span_from(start),
            ));
        }
        for (word, value) in [
            ("true", Value::Bool(true)),
            ("false", Value::Bool(false)),
            ("null", Value::Null),
        ] {
            if self.at_keyword(word) {
                self.pos += word.len();
                return Ok(Expr::new(ExprKind::Lit(value), self.span_from(start)));
            }
        }
        let first = self.ident().unwrap_or_else(|| unreachable!());
        let mut segments = vec![first];
        while self.peek() == Some('.') && self.peek_nth(1).is_some_and(is_ident_start) {
            self.bump();
            segments.push(self.field_name());
        }
        if self.peek() == Some('(') {
            let mut segments = segments.into_iter();
            let root = segments.next().unwrap_or_else(|| unreachable!());
            let mut callee = Expr::new(ExprKind::Var(root), self.span_from(start));
            for name in segments {
                callee = Expr::new(
                    ExprKind::Field(Box::new(callee), name),
                    self.span_from(start),
                );
            }
            let args = self.call_args()?;
            return Ok(Expr::new(
                ExprKind::Call(Box::new(callee), args),
                self.span_from(start),
            ));
        }
        if self.command_follows()? {
            let mut segments = segments.into_iter();
            let root = segments.next().unwrap_or_else(|| unreachable!());
            let mut callee = Expr::new(ExprKind::Var(root), self.span_from(start));
            for name in segments {
                callee = Expr::new(
                    ExprKind::Field(Box::new(callee), name),
                    self.span_from(start),
                );
            }
            return self.embedded_command(callee, start);
        }
        let text = segments.join(".");
        let lit = Expr::new(ExprKind::Lit(Value::str(text)), self.span_from(start));
        self.glue_tail(lit, start)
    }

    /// Whether whitespace and then a command argument follow on the same line,
    /// making the word before it a command: `(len $list)`, `x = upper hello`,
    /// `(abs -3)`. An operator (`a - b`), a closing bracket or a line break
    /// means it isn't one (so a forgotten `;` stays a clear error).
    fn command_follows(&mut self) -> Result<bool> {
        let save = self.pos;
        if !self.peek().is_some_and(char::is_whitespace) {
            return Ok(false);
        }
        self.skip_trivia()?;
        if self.src[save..self.pos].contains('\n') {
            self.pos = save;
            return Ok(false);
        }
        let follows = match self.peek() {
            None | Some(')' | ']' | '}' | ';' | ',') => false,
            Some('-') => self
                .peek_nth(1)
                .is_some_and(|c| !c.is_whitespace() && c != '='),
            Some(_) => {
                self.peek_operator().is_none()
                    && !self.at_keyword("else")
                    && !self.at_keyword("catch")
            }
        };
        self.pos = save;
        Ok(follows)
    }

    /// Parses the arguments of a command written inside an expression. They run
    /// to the end of the enclosing expression: `)`, `]`, `}`, `;` or `,`.
    fn embedded_command(&mut self, callee: Expr, start: usize) -> Result<Expr> {
        if !self.command_follows()? {
            return Ok(callee);
        }
        let mut args = Vec::new();
        loop {
            self.skip_trivia()?;
            match self.peek() {
                None | Some(')' | ']' | '}' | ';' | ',') => break,
                _ => args.push(self.arg(WordMode::Embedded)?),
            }
        }
        Ok(Expr::new(
            ExprKind::Call(Box::new(callee), args),
            self.span_from(start),
        ))
    }

    /// Glues pieces written directly after an atom: `Hello$name`, `5px`.
    fn glue_tail(&mut self, first: Expr, start: usize) -> Result<Expr> {
        let continues = |c: char| is_ident_char(c) || matches!(c, '$' | '"' | '\'');
        if !self.peek().is_some_and(continues) {
            return Ok(first);
        }
        let rest = self.word(WordMode::Expr)?;
        let mut pieces = vec![first];
        match rest.kind {
            ExprKind::Glue(more) => pieces.extend(more),
            _ => pieces.push(rest),
        }
        Ok(Expr::new(ExprKind::Glue(pieces), self.span_from(start)))
    }

    /// `12`, `2.5`, `1e3`, `6.02e23`.
    fn number(&mut self) -> Result<Expr> {
        let start = self.pos;
        let digits = |p: &mut Self| {
            while p.peek().is_some_and(|c| c.is_ascii_digit() || c == '_') {
                p.bump();
            }
        };
        digits(self);
        let mut float = false;
        if self.peek() == Some('.') && self.peek_nth(1).is_some_and(|c| c.is_ascii_digit()) {
            float = true;
            self.bump();
            digits(self);
        }
        if matches!(self.peek(), Some('e' | 'E'))
            && (self.peek_nth(1).is_some_and(|c| c.is_ascii_digit())
                || (matches!(self.peek_nth(1), Some('+' | '-'))
                    && self.peek_nth(2).is_some_and(|c| c.is_ascii_digit())))
        {
            float = true;
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            digits(self);
        }
        let text: String = self.src[start..self.pos]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        let value = if float {
            text.parse::<f64>().ok().map(Value::num)
        } else {
            text.parse::<i64>()
                .ok()
                .map(Value::Int)
                .or_else(|| text.parse::<f64>().ok().map(Value::num))
        };
        let span = self.span_from(start);
        match value {
            Some(value) => Ok(Expr::new(ExprKind::Lit(value), span)),
            None => Err(self.error_at(format!("invalid number `{text}`"), span)),
        }
    }
}
