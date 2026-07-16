//! Recursive-descent SQL parser for the Query-of-Queries engine.
//!
//! Precedence (low → high): OR · AND · NOT · predicate (comparison / IS NULL /
//! IN / BETWEEN / LIKE) · `||` concat · `+ - | ^` · `* / % &` · unary `+ -` ·
//! primary (literal, column, function, `*`, `(expr)`, `(SELECT …)`, CASE, CAST).

use cfml_common::dynamic::CfmlValue;

use crate::ast::*;
use crate::lexer::{Lexer, Token};

pub struct Parser {
    lexer: Lexer,
    current: Token,
    /// 0-based counter assigning indices to positional `?` parameters.
    param_index: usize,
}

/// Parse a SQL string into a [`Statement`].
pub fn parse(sql: &str) -> Result<Statement, String> {
    Parser::new(sql).parse()
}

impl Parser {
    pub fn new(input: &str) -> Self {
        let mut lexer = Lexer::new(input);
        let current = lexer.next_token();
        Self {
            lexer,
            current,
            param_index: 0,
        }
    }

    fn advance(&mut self) {
        self.current = self.lexer.next_token();
    }

    fn at(&self, tok: &Token) -> bool {
        &self.current == tok
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.at(tok) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: Token) -> Result<(), String> {
        if self.current == tok {
            self.advance();
            Ok(())
        } else {
            Err(format!("expected {:?}, got {:?}", tok, self.current))
        }
    }

    fn parse_ident(&mut self) -> Result<String, String> {
        match &self.current {
            Token::Identifier(s) => {
                let v = s.clone();
                self.advance();
                Ok(v)
            }
            other => Err(format!("expected identifier, got {:?}", other)),
        }
    }

    fn parse_uint(&mut self) -> Result<usize, String> {
        match &self.current {
            Token::Number(s) => {
                let v = s
                    .parse::<usize>()
                    .map_err(|_| format!("expected a non-negative integer, got '{}'", s))?;
                self.advance();
                Ok(v)
            }
            other => Err(format!("expected an integer, got {:?}", other)),
        }
    }

    /// Parse a full statement and require EOF afterwards.
    pub fn parse(&mut self) -> Result<Statement, String> {
        if !self.at(&Token::Select) {
            return Err(format!("expected SELECT, got {:?}", self.current));
        }
        let stmt = self.parse_select_statement()?;
        if let Token::Error(e) = &self.current {
            return Err(format!("lex error: {}", e));
        }
        if !self.at(&Token::Eof) {
            return Err(format!("unexpected token after statement: {:?}", self.current));
        }
        Ok(Statement::Select(stmt))
    }

    // ── Statement level ────────────────────────────────────────────────

    fn parse_select_statement(&mut self) -> Result<SelectStatement, String> {
        let mut body = self.parse_select_core()?;

        let mut unions = Vec::new();
        while self.eat(&Token::Union) {
            // UNION ALL keeps duplicates; UNION [DISTINCT] removes them.
            let all = if self.eat(&Token::All) {
                true
            } else {
                self.eat(&Token::Distinct);
                false
            };
            let select = self.parse_select_core()?;
            unions.push(Union { all, select });
        }

        let order_by = if self.eat(&Token::Order) {
            self.expect(Token::By)?;
            self.parse_order_by()?
        } else {
            Vec::new()
        };

        let mut limit = if self.eat(&Token::Limit) {
            Some(self.parse_limit()?)
        } else {
            None
        };

        // `SELECT TOP n` on the body applies after ORDER BY → lift it to the
        // statement LIMIT (when there's no explicit LIMIT). Clear it so the
        // executor doesn't also apply it as a per-core cap.
        if limit.is_none() {
            if let Some(n) = body.top.take() {
                limit = Some(LimitClause { offset: 0, count: n });
            }
        }

        Ok(SelectStatement {
            body,
            unions,
            order_by,
            limit,
        })
    }

    fn parse_select_core(&mut self) -> Result<SelectCore, String> {
        self.expect(Token::Select)?;

        let distinct = self.eat(&Token::Distinct);
        let top = if self.eat(&Token::Top) {
            Some(self.parse_uint()?)
        } else {
            None
        };

        let columns = self.parse_select_columns()?;

        let (from, joins) = if self.eat(&Token::From) {
            self.parse_from()?
        } else {
            (None, Vec::new())
        };

        let where_clause = if self.eat(&Token::Where) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        let group_by = if self.eat(&Token::Group) {
            self.expect(Token::By)?;
            self.parse_expr_list()?
        } else {
            Vec::new()
        };

        let having = if self.eat(&Token::Having) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(SelectCore {
            distinct,
            top,
            columns,
            from,
            joins,
            where_clause,
            group_by,
            having,
        })
    }

    fn parse_select_columns(&mut self) -> Result<Vec<SelectColumn>, String> {
        let mut cols = vec![self.parse_select_column()?];
        while self.eat(&Token::Comma) {
            cols.push(self.parse_select_column()?);
        }
        Ok(cols)
    }

    fn parse_select_column(&mut self) -> Result<SelectColumn, String> {
        let expr = self.parse_expr()?;
        let alias = self.parse_optional_alias()?;
        Ok(SelectColumn { expr, alias })
    }

    /// `[AS] identifier`, or a bare identifier as an implicit alias.
    fn parse_optional_alias(&mut self) -> Result<Option<String>, String> {
        if self.eat(&Token::As) {
            Ok(Some(self.parse_ident()?))
        } else if matches!(self.current, Token::Identifier(_)) {
            Ok(Some(self.parse_ident()?))
        } else {
            Ok(None)
        }
    }

    // ── FROM / JOIN ────────────────────────────────────────────────────

    fn parse_from(&mut self) -> Result<(Option<TableRef>, Vec<JoinClause>), String> {
        let seed = self.parse_table_ref()?;
        let mut joins = Vec::new();

        loop {
            let join = match &self.current {
                Token::Comma => {
                    self.advance();
                    JoinClause {
                        join_type: JoinType::Cross,
                        table: self.parse_table_ref()?,
                        on: None,
                    }
                }
                Token::Cross => {
                    self.advance();
                    self.expect(Token::Join)?;
                    JoinClause {
                        join_type: JoinType::Cross,
                        table: self.parse_table_ref()?,
                        on: None,
                    }
                }
                Token::Join | Token::Inner => {
                    self.eat(&Token::Inner);
                    self.expect(Token::Join)?;
                    self.parse_join_with_on(JoinType::Inner)?
                }
                Token::Left => {
                    self.advance();
                    self.eat(&Token::Outer);
                    self.expect(Token::Join)?;
                    self.parse_join_with_on(JoinType::Left)?
                }
                Token::Right => {
                    self.advance();
                    self.eat(&Token::Outer);
                    self.expect(Token::Join)?;
                    self.parse_join_with_on(JoinType::Right)?
                }
                Token::Full => {
                    self.advance();
                    self.eat(&Token::Outer);
                    self.expect(Token::Join)?;
                    self.parse_join_with_on(JoinType::Full)?
                }
                _ => break,
            };
            joins.push(join);
        }

        Ok((Some(seed), joins))
    }

    fn parse_join_with_on(&mut self, join_type: JoinType) -> Result<JoinClause, String> {
        let table = self.parse_table_ref()?;
        self.expect(Token::On)?;
        let on = Some(self.parse_expr()?);
        Ok(JoinClause {
            join_type,
            table,
            on,
        })
    }

    fn parse_table_ref(&mut self) -> Result<TableRef, String> {
        if self.eat(&Token::LParen) {
            let select = self.parse_select_statement()?;
            self.expect(Token::RParen)?;
            // A derived table requires an alias.
            self.eat(&Token::As);
            let alias = self.parse_ident()?;
            Ok(TableRef::Derived {
                select: Box::new(select),
                alias,
            })
        } else {
            // A QoQ source can be a query held at a nested struct path, so the
            // table name may be DOTTED — e.g. `FROM variables.internal.iplog`
            // (Masa's front-end IP block-list check). Accumulate `.segment`s
            // into the full name; the VM resolves the path against scope.
            let mut name = self.parse_ident()?;
            while self.eat(&Token::Dot) {
                name.push('.');
                name.push_str(&self.parse_ident()?);
            }
            let alias = if self.eat(&Token::As) {
                Some(self.parse_ident()?)
            } else if matches!(self.current, Token::Identifier(_)) {
                Some(self.parse_ident()?)
            } else {
                None
            };
            Ok(TableRef::Named { name, alias })
        }
    }

    // ── ORDER BY / LIMIT ───────────────────────────────────────────────

    fn parse_order_by(&mut self) -> Result<Vec<OrderByExpr>, String> {
        let mut items = vec![self.parse_order_by_item()?];
        while self.eat(&Token::Comma) {
            items.push(self.parse_order_by_item()?);
        }
        Ok(items)
    }

    fn parse_order_by_item(&mut self) -> Result<OrderByExpr, String> {
        let expr = self.parse_expr()?;
        let direction = if self.eat(&Token::Asc) {
            SortDirection::Asc
        } else if self.eat(&Token::Desc) {
            SortDirection::Desc
        } else {
            SortDirection::Asc
        };
        Ok(OrderByExpr { expr, direction })
    }

    fn parse_limit(&mut self) -> Result<LimitClause, String> {
        let first = self.parse_uint()?;
        if self.eat(&Token::Comma) {
            // LIMIT offset, count
            let count = self.parse_uint()?;
            Ok(LimitClause {
                offset: first,
                count,
            })
        } else if self.eat(&Token::Offset) {
            // LIMIT count OFFSET offset
            let offset = self.parse_uint()?;
            Ok(LimitClause {
                offset,
                count: first,
            })
        } else {
            Ok(LimitClause {
                offset: 0,
                count: first,
            })
        }
    }

    // ── Expressions ────────────────────────────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }

    fn parse_expr_list(&mut self) -> Result<Vec<Expr>, String> {
        let mut list = vec![self.parse_expr()?];
        while self.eat(&Token::Comma) {
            list.push(self.parse_expr()?);
        }
        Ok(list)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.eat(&Token::Or) {
            let right = self.parse_and()?;
            left = bin(left, BinaryOp::Or, right);
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_not()?;
        while self.eat(&Token::And) {
            let right = self.parse_not()?;
            left = bin(left, BinaryOp::And, right);
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Not) {
            let expr = self.parse_predicate()?;
            Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
            })
        } else {
            self.parse_predicate()
        }
    }

    /// A comparison or a postfix predicate (IS NULL / IN / BETWEEN / LIKE).
    fn parse_predicate(&mut self) -> Result<Expr, String> {
        let left = self.parse_concat()?;

        let op = match &self.current {
            Token::Eq => Some(BinaryOp::Eq),
            Token::Neq | Token::Neq2 => Some(BinaryOp::Neq),
            Token::Lt => Some(BinaryOp::Lt),
            Token::Lte => Some(BinaryOp::Lte),
            Token::Gt => Some(BinaryOp::Gt),
            Token::Gte => Some(BinaryOp::Gte),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let right = self.parse_concat()?;
            return Ok(bin(left, op, right));
        }

        match &self.current {
            Token::Is => {
                self.advance();
                let negated = self.eat(&Token::Not);
                self.expect(Token::Null)?;
                Ok(Expr::IsNull {
                    expr: Box::new(left),
                    negated,
                })
            }
            Token::In => self.parse_in(left, false),
            Token::Between => self.parse_between(left, false),
            Token::Like => self.parse_like(left, false),
            Token::Not => {
                self.advance();
                match &self.current {
                    Token::In => self.parse_in(left, true),
                    Token::Between => self.parse_between(left, true),
                    Token::Like => self.parse_like(left, true),
                    other => Err(format!(
                        "expected IN, BETWEEN or LIKE after NOT, got {:?}",
                        other
                    )),
                }
            }
            _ => Ok(left),
        }
    }

    fn parse_in(&mut self, left: Expr, negated: bool) -> Result<Expr, String> {
        self.expect(Token::In)?;
        self.expect(Token::LParen)?;
        if self.at(&Token::Select) {
            let select = self.parse_select_statement()?;
            self.expect(Token::RParen)?;
            return Ok(Expr::InSubquery {
                expr: Box::new(left),
                negated,
                select: Box::new(select),
            });
        }
        let list = self.parse_expr_list()?;
        self.expect(Token::RParen)?;
        Ok(Expr::InList {
            expr: Box::new(left),
            negated,
            list,
        })
    }

    fn parse_between(&mut self, left: Expr, negated: bool) -> Result<Expr, String> {
        self.expect(Token::Between)?;
        // Operands at concat precedence so the BETWEEN's AND isn't swallowed.
        let low = self.parse_concat()?;
        self.expect(Token::And)?;
        let high = self.parse_concat()?;
        Ok(Expr::Between {
            expr: Box::new(left),
            negated,
            low: Box::new(low),
            high: Box::new(high),
        })
    }

    fn parse_like(&mut self, left: Expr, negated: bool) -> Result<Expr, String> {
        self.expect(Token::Like)?;
        let pattern = self.parse_concat()?;
        let escape = if self.eat(&Token::Escape) {
            Some(Box::new(self.parse_concat()?))
        } else {
            None
        };
        Ok(Expr::Like {
            expr: Box::new(left),
            negated,
            pattern: Box::new(pattern),
            escape,
            compiled: None,
        })
    }

    fn parse_concat(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;
        while self.eat(&Token::Concat) {
            let right = self.parse_additive()?;
            left = bin(left, BinaryOp::Concat, right);
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_term()?;
        loop {
            let op = match &self.current {
                Token::Plus => BinaryOp::Add,
                Token::Minus => BinaryOp::Sub,
                Token::Pipe => BinaryOp::BitOr,
                Token::Caret => BinaryOp::BitXor,
                _ => break,
            };
            self.advance();
            let right = self.parse_term()?;
            left = bin(left, op, right);
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match &self.current {
                Token::Star => BinaryOp::Mul,
                Token::Slash => BinaryOp::Div,
                Token::Mod => BinaryOp::Mod,
                Token::Amp => BinaryOp::BitAnd,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = bin(left, op, right);
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Minus) {
            let expr = self.parse_unary()?;
            Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
            })
        } else if self.eat(&Token::Plus) {
            let expr = self.parse_unary()?;
            Ok(Expr::Unary {
                op: UnaryOp::Plus,
                expr: Box::new(expr),
            })
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.current.clone() {
            Token::Number(s) => {
                self.advance();
                Ok(Expr::Literal(number_literal(&s)?))
            }
            Token::String(s) => {
                self.advance();
                Ok(Expr::Literal(CfmlValue::string(s)))
            }
            Token::Null => {
                self.advance();
                Ok(Expr::Literal(CfmlValue::Null))
            }
            Token::True => {
                self.advance();
                Ok(Expr::Literal(CfmlValue::Bool(true)))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Literal(CfmlValue::Bool(false)))
            }
            Token::Star => {
                self.advance();
                Ok(Expr::Star { table: None })
            }
            Token::Param => {
                self.advance();
                let idx = self.param_index;
                self.param_index += 1;
                Ok(Expr::Param(ParamRef::Positional(idx)))
            }
            Token::NamedParam(name) => {
                self.advance();
                Ok(Expr::Param(ParamRef::Named(name)))
            }
            Token::Case => self.parse_case(),
            Token::Cast => self.parse_cast(),
            Token::Convert => self.parse_convert(),
            Token::LParen => {
                self.advance();
                if self.at(&Token::Select) {
                    let select = self.parse_select_statement()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::ScalarSubquery(Box::new(select)))
                } else {
                    let expr = self.parse_expr()?;
                    self.expect(Token::RParen)?;
                    Ok(expr)
                }
            }
            // LEFT / RIGHT double as join keywords and string functions; in an
            // expression position they can only be function calls.
            Token::Left | Token::Right => {
                let name = if self.at(&Token::Left) { "left" } else { "right" };
                self.advance();
                self.expect(Token::LParen)?;
                let (args, distinct) = self.parse_function_args()?;
                self.expect(Token::RParen)?;
                Ok(Expr::Function {
                    name: name.to_string(),
                    args,
                    distinct,
                })
            }
            Token::Identifier(name) => {
                self.advance();
                if self.eat(&Token::LParen) {
                    let (args, distinct) = self.parse_function_args()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::Function {
                        name,
                        args,
                        distinct,
                    })
                } else if self.eat(&Token::Dot) {
                    if self.eat(&Token::Star) {
                        Ok(Expr::Star { table: Some(name) })
                    } else {
                        let col = self.parse_ident()?;
                        Ok(Expr::Column {
                            table: Some(name),
                            name: col,
                        })
                    }
                } else {
                    Ok(Expr::Column {
                        table: None,
                        name,
                    })
                }
            }
            other => Err(format!("unexpected token in expression: {:?}", other)),
        }
    }

    /// Function arguments: optional leading DISTINCT (for `COUNT(DISTINCT x)`),
    /// then a comma list (or `*` for `COUNT(*)`).
    fn parse_function_args(&mut self) -> Result<(Vec<Expr>, bool), String> {
        let distinct = self.eat(&Token::Distinct);
        let mut args = Vec::new();
        if !self.at(&Token::RParen) {
            args.push(self.parse_expr()?);
            while self.eat(&Token::Comma) {
                args.push(self.parse_expr()?);
            }
        }
        Ok((args, distinct))
    }

    fn parse_case(&mut self) -> Result<Expr, String> {
        self.expect(Token::Case)?;
        // Simple CASE has an operand before the first WHEN.
        let operand = if self.at(&Token::When) {
            None
        } else {
            Some(Box::new(self.parse_expr()?))
        };

        let mut whens = Vec::new();
        while self.eat(&Token::When) {
            let when = self.parse_expr()?;
            self.expect(Token::Then)?;
            let then = self.parse_expr()?;
            whens.push(WhenThen { when, then });
        }
        if whens.is_empty() {
            return Err("CASE requires at least one WHEN".to_string());
        }

        let else_expr = if self.eat(&Token::Else) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        self.expect(Token::End)?;

        Ok(Expr::Case {
            operand,
            whens,
            else_expr,
        })
    }

    fn parse_cast(&mut self) -> Result<Expr, String> {
        self.expect(Token::Cast)?;
        self.expect(Token::LParen)?;
        let expr = self.parse_expr()?;
        self.expect(Token::As)?;
        let ty = self.parse_type_name()?;
        self.expect(Token::RParen)?;
        Ok(Expr::Cast {
            expr: Box::new(expr),
            ty,
        })
    }

    fn parse_convert(&mut self) -> Result<Expr, String> {
        self.expect(Token::Convert)?;
        self.expect(Token::LParen)?;
        let expr = self.parse_expr()?;
        self.expect(Token::Comma)?;
        let ty = self.parse_type_name()?;
        self.expect(Token::RParen)?;
        Ok(Expr::Cast {
            expr: Box::new(expr),
            ty,
        })
    }

    /// A type name for CAST/CONVERT — an identifier with an optional `(n[,m])`
    /// length/precision, which is parsed and discarded.
    fn parse_type_name(&mut self) -> Result<String, String> {
        let name = self.parse_ident()?;
        if self.eat(&Token::LParen) {
            // skip n [, m]
            let _ = self.parse_uint();
            if self.eat(&Token::Comma) {
                let _ = self.parse_uint();
            }
            self.expect(Token::RParen)?;
        }
        Ok(name.to_lowercase())
    }
}

fn bin(left: Expr, op: BinaryOp, right: Expr) -> Expr {
    Expr::Binary {
        left: Box::new(left),
        op,
        right: Box::new(right),
    }
}

fn number_literal(s: &str) -> Result<CfmlValue, String> {
    if s.contains('.') {
        s.parse::<f64>()
            .map(CfmlValue::Double)
            .map_err(|e| format!("invalid number '{}': {}", s, e))
    } else {
        match s.parse::<i64>() {
            Ok(n) => Ok(CfmlValue::Int(n)),
            // Out-of-range integer literal → fall back to f64.
            Err(_) => s
                .parse::<f64>()
                .map(CfmlValue::Double)
                .map_err(|e| format!("invalid number '{}': {}", s, e)),
        }
    }
}
