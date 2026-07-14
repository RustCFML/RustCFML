//! CFML Parser - Converts tokens to AST

use crate::ast::*;
use crate::lexer::{Lexer, TokenWithLoc};
use crate::token::Token;
use cfml_common::position::SourceLocation;
use std::convert::TryFrom;

pub struct Parser {
    tokens: Vec<TokenWithLoc>,
    current: usize,
    /// Map from token index → javadoc comment text immediately preceding it.
    doc_comments: std::collections::HashMap<usize, String>,
}

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl Parser {
    pub fn new(source: String) -> Self {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize();
        let doc_comments = lexer.doc_comments().iter().cloned().collect();
        Self {
            tokens,
            current: 0,
            doc_comments,
        }
    }

    pub fn parse(&mut self) -> Result<Program, ParseError> {
        // Surface any lexer error token at its own (correct) location before
        // the grammar trips over it and reports a misleading downstream error
        // (e.g. an unterminated '#' interpolation that ran to EOF — GitHub #181).
        for tok in &self.tokens {
            if let Token::Error(msg) = &tok.token {
                return Err(ParseError {
                    message: msg.clone(),
                    line: tok.location.start.line,
                    column: tok.location.start.column,
                });
            }
        }

        let mut statements = Vec::new();

        while !self.is_at_end() {
            statements.push(self.parse_statement()?);
        }

        Ok(Program {
            statements,
            location: self.current_location(),
        })
    }

    fn is_at_end(&self) -> bool {
        matches!(&self.tokens[self.current].token, Token::Eof)
    }

    fn peek(&self, offset: usize) -> &Token {
        let idx = self.current + offset;
        if idx >= self.tokens.len() {
            return &Token::Eof;
        }
        &self.tokens[idx].token
    }

    fn current_location(&self) -> SourceLocation {
        if self.current < self.tokens.len() {
            self.tokens[self.current].location
        } else {
            SourceLocation::default()
        }
    }

    fn advance(&mut self) -> TokenWithLoc {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn previous(&self) -> TokenWithLoc {
        self.tokens[self.current - 1].clone()
    }

    fn check(&self, token: &Token) -> bool {
        if self.is_at_end() {
            return false;
        }
        std::mem::discriminant(self.peek(0)) == std::mem::discriminant(token)
    }

    fn match_token(&mut self, token: &Token) -> bool {
        if self.check(token) {
            self.advance();
            return true;
        }
        false
    }

    #[allow(dead_code)]
    fn match_any(&mut self, tokens: &[Token]) -> Option<Token> {
        for token in tokens {
            if self.check(token) {
                let t = self.advance().token.clone();
                return Some(t);
            }
        }
        None
    }

    /// True if the token at `offset` is an identifier equal (case-insensitive)
    /// to `word`. Used to recognise the verbose, multi-word comparison operators
    /// (GREATER THAN, DOES NOT CONTAIN, EQUAL, ...) whose words are NOT reserved
    /// keywords — they lex as identifiers so they remain usable as variable names.
    fn peek_word(&self, offset: usize, word: &str) -> bool {
        matches!(self.peek(offset), Token::Identifier(s) if s.eq_ignore_ascii_case(word))
    }

    /// Consume an identifier matching `word` (case-insensitive); returns whether
    /// it was consumed.
    fn match_word(&mut self, word: &str) -> bool {
        if self.peek_word(0, word) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn parse_error(&self, message: &str) -> ParseError {
        let loc = self.current_location();
        ParseError {
            message: format!("{} (found {:?})", message, self.peek(0)),
            line: loc.start.line,
            column: loc.start.column,
        }
    }

    // ---- Statement Parsing ----

    fn parse_statement(&mut self) -> Result<CfmlNode, ParseError> {
        let stmt_loc = self.current_location();

        // Check for access modifiers before function — but only if followed by
        // function, static, or a return type + function.  Otherwise "private" etc.
        // are valid as variable names in CFML.
        if matches!(
            self.peek(0),
            Token::Public | Token::Private | Token::Remote | Token::Package
        ) && self.is_access_modifier_for_function()
        {
            let access = self.parse_access_modifier();
            // Capture optional return type annotation (e.g. "private array function ..."
            // or "public MachII.framework.AppManager function ..." or
            // "public component function init()") so it is not dropped — the
            // declared return type must surface in getMetadata().
            let captured_rt = self.capture_dotted_return_type_before_function();
            if self.match_token(&Token::Function) {
                let mut func = self.parse_function()?;
                func.access = access;
                if func.return_type.is_none() {
                    func.return_type = captured_rt;
                }
                return Ok(CfmlNode::Statement(Statement::FunctionDecl(FunctionDecl {
                    func,
                })));
            }
            if self.match_token(&Token::Static) {
                // Capture optional return type after static (including dotted names)
                let captured_rt = self.capture_dotted_return_type_before_function();
                if self.match_token(&Token::Function) {
                    let mut func = self.parse_function()?;
                    func.access = access;
                    func.is_static = true;
                    if func.return_type.is_none() {
                        func.return_type = captured_rt;
                    }
                    return Ok(CfmlNode::Statement(Statement::FunctionDecl(FunctionDecl {
                        func,
                    })));
                }
            }
        }

        if self.match_token(&Token::Var) {
            return Ok(CfmlNode::Statement(self.parse_var()?));
        }

        if self.match_token(&Token::If) {
            return Ok(CfmlNode::Statement(Statement::If(self.parse_if()?)));
        }

        if self.match_token(&Token::For) {
            return self.parse_for_statement();
        }

        if self.match_token(&Token::While) {
            return Ok(CfmlNode::Statement(Statement::While(self.parse_while()?)));
        }

        if self.match_token(&Token::Do) {
            return Ok(CfmlNode::Statement(Statement::Do(self.parse_do()?)));
        }

        if self.match_token(&Token::Switch) {
            return Ok(CfmlNode::Statement(Statement::Switch(self.parse_switch()?)));
        }

        if self.match_token(&Token::Try) {
            return Ok(CfmlNode::Statement(Statement::Try(self.parse_try()?)));
        }

        if self.match_token(&Token::Throw) {
            // throw(...) with parens = function call form (VM-intercepted)
            if self.check(&Token::LParen) {
                self.consume(&Token::LParen)?;

                // Check if first arg is named (identifier followed by = or :)
                let is_named = matches!(self.peek(0), Token::Identifier(_))
                    && (matches!(self.peek(1), Token::Equal | Token::Colon));

                let arguments = if is_named {
                    // Parse named args like throw(message="oops", type="custom") or throw(message : "oops", type : "custom")
                    let named = self.parse_throw_named_attrs(true)?;
                    self.throw_named_to_positional(named, stmt_loc)
                } else {
                    self.parse_arguments()?
                };

                self.consume(&Token::RParen)?;
                self.match_token(&Token::Semicolon);

                return Ok(self.build_throw_call(arguments, stmt_loc));
            }

            // Single bare-attribute statement form `throw object=expr;` /
            // `throw message="…";`. Lucee compiles the SINGLE-attribute tag
            // statement (its `_singleAttrStatement` path) — Preside's
            // `Bootstrap.onError` relies on `throw object=arguments.exception;`.
            // (Lucee rejects the MULTI-attribute form `throw message=… type=…;`,
            // but accepting it here is a harmless superset.) Gated on a known
            // throw attribute name so a stray `throw x=…` isn't mis-routed.
            let is_attr_form = matches!(self.peek(0), Token::Identifier(n)
                if matches!(n.to_lowercase().as_str(),
                    "message" | "type" | "detail" | "errorcode" | "extendedinfo" | "object"))
                && matches!(self.peek(1), Token::Equal | Token::Colon);
            if is_attr_form {
                let named = self.parse_throw_named_attrs(false)?;
                let arguments = self.throw_named_to_positional(named, stmt_loc);
                self.match_token(&Token::Semicolon);
                return Ok(self.build_throw_call(arguments, stmt_loc));
            }

            // Bare expression form `throw "msg";` / `throw structVar;` — the VM
            // wraps a non-struct value into a proper cfcatch struct so
            // `.message`/`.type` resolve. (`throw(message=…)` / `cfthrow(…)`
            // are handled above.)
            return Ok(CfmlNode::Statement(Statement::Throw(self.parse_throw()?)));
        }

        // `cfthrow(...)` — the cfXXX() script alias for the <cfthrow> tag (Lucee
        // exposes every tag as a `cfName()` function). Route it through the very
        // same lowering as `throw(...)` above so named attrs AND
        // `attributeCollection=` both work. Taffy's Factory throws bean-not-found
        // via `cfthrow(attributecollection=arguments)`; without this `cfthrow`
        // resolved as an undefined variable.
        if matches!(self.peek(0), Token::Identifier(n) if n.eq_ignore_ascii_case("cfthrow"))
            && matches!(self.peek(1), Token::LParen)
        {
            self.advance(); // consume `cfthrow`
            self.consume(&Token::LParen)?;
            let is_named = matches!(self.peek(0), Token::Identifier(_))
                && matches!(self.peek(1), Token::Equal | Token::Colon);
            let arguments = if is_named {
                let named = self.parse_throw_named_attrs(true)?;
                self.throw_named_to_positional(named, stmt_loc)
            } else {
                self.parse_arguments()?
            };
            self.consume(&Token::RParen)?;
            self.match_token(&Token::Semicolon);
            return Ok(self.build_throw_call(arguments, stmt_loc));
        }

        if self.match_token(&Token::Rethrow) {
            self.match_token(&Token::Semicolon);
            return Ok(CfmlNode::Statement(Statement::Rethrow(stmt_loc)));
        }

        if self.match_token(&Token::Return) {
            return Ok(CfmlNode::Statement(Statement::Return(self.parse_return()?)));
        }

        if self.match_token(&Token::Break) {
            self.match_token(&Token::Semicolon);
            return Ok(CfmlNode::Statement(Statement::Break(Break {
                label: None,
                location: stmt_loc,
            })));
        }

        if self.match_token(&Token::Continue) {
            self.match_token(&Token::Semicolon);
            return Ok(CfmlNode::Statement(Statement::Continue(Continue {
                label: None,
                location: stmt_loc,
            })));
        }

        if self.match_token(&Token::Function) {
            return Ok(CfmlNode::Statement(Statement::FunctionDecl(FunctionDecl {
                func: self.parse_function()?,
            })));
        }

        // Bare-typed top-level function: `struct function f() {...}` (no access
        // modifier). Lucee/ACF/BoxLang accept return-type annotations on
        // top-level cfscript functions exactly like component methods. Restrict
        // to plain Identifier so `component`/`interface` declaration branches
        // below still fire.
        if matches!(self.peek(0), Token::Identifier(_)) {
            let mut lookahead = 1;
            while matches!(self.peek(lookahead), Token::Dot) && matches!(self.peek(lookahead + 1), Token::Identifier(_)) {
                lookahead += 2;
            }
            if matches!(self.peek(lookahead), Token::Function) {
                for _ in 0..lookahead {
                    self.advance();
                }
                self.advance(); // consume `function`
                return Ok(CfmlNode::Statement(Statement::FunctionDecl(FunctionDecl {
                    func: self.parse_function()?,
                })));
            }
        }

        // `component` is a SOFT keyword: it introduces a CFC only when it actually
        // begins a declaration (`component {`, `component Name ...`, `component
        // extends=...`, `component output="false" ...`). Used anywhere else — a
        // bare `component = x` assignment, `component.foo`, `component[1]`, or as
        // the `component` attribute of a script-statement cfinvoke — it is an
        // ordinary identifier and must fall through to expression parsing. This
        // matches Lucee/ACF/BoxLang, which all treat `component` as soft.
        if self.check(&Token::Component) && self.is_component_declaration() {
            let comp_doc_idx = self.current;
            self.advance(); // consume `component`
            let mut component = self.parse_component()?;
            self.apply_doc_to_metadata(comp_doc_idx, &mut component.metadata);
            return Ok(CfmlNode::Statement(Statement::ComponentDecl(
                ComponentDecl { component },
            )));
        }

        if self.match_token(&Token::Interface) {
            return Ok(CfmlNode::Statement(Statement::InterfaceDecl(
                InterfaceDecl {
                    interface: self.parse_interface()?,
                },
            )));
        }

        // cfscript param statement: param name="varName" default="value";
        // or shorthand: param varName = defaultValue;  /  param type varName = defaultValue;
        // `param` is a SOFT keyword on Lucee/ACF/BoxLang: it only opens cfparam
        // when followed by another identifier-like token (the var name, a type,
        // or an attribute name like `name=`). Followed by `=`, `[`, `.`, `(`,
        // etc., it is an ordinary identifier — fall through to the expression
        // statement path. Wheels' vendor/wheels/public/helpers.cfm uses
        // `param` as a for-in loop var with `param['default'] = ...` in the body.
        if matches!(self.peek(0), Token::Param) && self.is_identifier_like_at(1) {
            self.advance(); // consume `param`
            return self.parse_param_statement(stmt_loc, false);
        }

        // `cfparam(name="x", default="y", type="…")` — the cf-prefixed function
        // call form of <cfparam>/`param name=…`. Lowers through the identical
        // param machinery (default-assign + type validation) so all three forms
        // stay in sync. Only the unambiguous `cf`-prefixed name is matched here;
        // bare `param(` stays an ordinary identifier (Wheels uses `param` as a
        // variable name).
        if matches!(self.peek(0), Token::Identifier(ref s) if s.eq_ignore_ascii_case("cfparam"))
            && matches!(self.peek(1), Token::LParen)
        {
            self.advance(); // consume `cfparam`
            self.consume(&Token::LParen)?;
            return self.parse_param_statement(stmt_loc, true);
        }

        // cfscript lock block: lock name="x" type="exclusive" timeout="5" { body }
        if self.match_token(&Token::Lock) {
            return self.parse_lock(stmt_loc);
        }

        // cfscript-statement `loop` form (the script equivalent of <cfloop> with
        // attributes): `loop query=q { … }`, `loop list="a,b" item="x" { … }`,
        // `loop array=[…] item="x" { … }`, `loop from=1 to=3 index="i" { … }`,
        // `loop times=3 { … }`, `loop condition="…" { … }`, `loop collection=…`.
        // Only fires when `loop` leads either a `{` body (infinite loop) or an
        // `attr=` pair, so `loop = 5;` (assignment) and `loop(…)` (call) are
        // untouched. (Issue #183 — blocks Preside SqlSchemaSynchronizer.)
        if matches!(self.peek(0), Token::Identifier(ref s) if s.eq_ignore_ascii_case("loop"))
            && (matches!(self.peek(1), Token::LBrace)
                || (self.is_identifier_like_at(1) && matches!(self.peek(2), Token::Equal)))
        {
            self.advance(); // consume `loop`
            return self.parse_script_loop(stmt_loc);
        }

        // `cfloop( … ){ … }` — the cf-prefixed paren-call body form (equivalent to
        // the `loop …` script statement above). `cfloop` is never a user function,
        // so the `(`/`{`/`attr=` lookahead is unambiguous. The lucee-spreadsheet
        // library (Preside asset manager) writes `cfloop( array=…, item=… ){ … }`.
        if matches!(self.peek(0), Token::Identifier(ref s) if s.eq_ignore_ascii_case("cfloop"))
            && (matches!(self.peek(1), Token::LParen | Token::LBrace)
                || (self.is_identifier_like_at(1) && matches!(self.peek(2), Token::Equal)))
        {
            self.advance(); // consume `cfloop`
            return self.parse_script_loop(stmt_loc);
        }

        if self.match_token(&Token::Include) {
            // Script include has two forms:
            //   include "path";                          (bare expression)
            //   include template="path" [attr=val ...];  (attribute form)
            // The attribute form must NOT be parsed as a plain expression, or
            // `template = <expr>` is read as an assignment to a variable named
            // `template` whose value never reaches the include — yielding an
            // empty path (Preside Router.cfc does `include template=ext.dir & "/x"`).
            let path = if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                let mut template_expr = None;
                while self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                    let attr = self.extract_identifier()?.to_lowercase();
                    self.advance(); // consume '='
                    let val = self.parse_expression()?;
                    if attr == "template" {
                        template_expr = Some(val);
                    }
                    // Other attributes (e.g. runOnce) are accepted and ignored.
                }
                match template_expr {
                    Some(p) => p,
                    None => {
                        return Err(self.parse_error(
                            "cfinclude requires a 'template' attribute",
                        ))
                    }
                }
            } else {
                self.parse_expression()?
            };
            self.match_token(&Token::Semicolon);
            return Ok(CfmlNode::Statement(Statement::Include(Include {
                path,
                location: stmt_loc,
            })));
        }

        if self.match_token(&Token::Import) {
            let path = self.extract_identifier()?;
            let alias = if self.match_token(&Token::Identifier("as".into())) {
                Some(self.extract_identifier()?)
            } else {
                None
            };
            self.match_token(&Token::Semicolon);
            return Ok(CfmlNode::Statement(Statement::Import(Import {
                path,
                alias,
                location: stmt_loc,
            })));
        }

        // Handle 'http url="..." method="..." result="..." { httpparam...; }' in CFScript
        if matches!(self.peek(0), Token::Identifier(ref s) if s.to_lowercase() == "http")
            && self.is_identifier_like_at(1)
            && matches!(self.peek(2), Token::Equal)
        {
            self.advance(); // consume 'http'

            // Parse attributes: key=value pairs
            let mut attrs: Vec<(String, Expression)> = Vec::new();
            while !self.check(&Token::LBrace) && !self.check(&Token::Semicolon) && !self.is_at_end() {
                if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                    let attr_name = self.extract_identifier()?;
                    self.advance(); // consume =
                    let attr_value = self.parse_expression()?;
                    attrs.push((attr_name.to_lowercase(), attr_value));
                } else {
                    break;
                }
            }

            // Parse httpparam statements if block body present.
            // KNOWN LIMITATION (issue #55): only literal `httpparam` statements are
            // collected here — control flow in the body (e.g. `for (x in coll) {
            // cfhttpparam(...); }`) is not supported and needs runtime param
            // collection, like cfquery got in 28af97d.
            let mut params: Vec<Expression> = Vec::new();
            if self.check(&Token::LBrace) {
                self.advance(); // consume {
                // Track nested-block depth so the body does NOT terminate at a
                // nested `}` — e.g. `http … { if (…) { httpparam …; } }` (Preside
                // elasticsearch ElasticSearchApiWrapper). httpparam statements are
                // still collected flat regardless of nesting (issue #55 limitation:
                // control flow around them isn't honoured), but the block bounds
                // must be matched correctly or the parser desyncs past the tag.
                let mut depth = 0i32;
                loop {
                    if self.is_at_end() {
                        break;
                    }
                    if self.check(&Token::RBrace) {
                        if depth == 0 {
                            break; // closes the http body
                        }
                        depth -= 1;
                        self.advance();
                        continue;
                    }
                    if self.check(&Token::LBrace) {
                        depth += 1;
                        self.advance();
                        continue;
                    }
                    // Expect httpparam statements (at any depth)
                    if matches!(self.peek(0), Token::Identifier(ref s) if s.to_lowercase() == "httpparam") {
                        self.advance(); // consume 'httpparam'
                        let mut param_pairs: Vec<(Expression, Expression)> = Vec::new();
                        while !self.check(&Token::Semicolon) && !self.check(&Token::RBrace) && !self.is_at_end() {
                            if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                                let pname = self.extract_identifier()?;
                                self.advance(); // consume =
                                let pvalue = self.parse_expression()?;
                                param_pairs.push((
                                    Expression::Literal(Literal {
                                        value: LiteralValue::String(pname.to_lowercase()),
                                        location: stmt_loc.clone(),
                                    }),
                                    pvalue,
                                ));
                            } else {
                                break;
                            }
                        }
                        self.match_token(&Token::Semicolon);
                        params.push(Expression::Struct(Struct {
                            pairs: param_pairs,
                            ordered: false,
                            location: stmt_loc.clone(),
                        }));
                    } else {
                        // Skip unknown tokens inside http block
                        self.advance();
                    }
                }
                self.consume(&Token::RBrace)?; // consume }
            } else {
                self.match_token(&Token::Semicolon);
            }

            // Build the struct argument for cfhttp({ url: ..., method: ..., params: [...] })
            let mut struct_pairs: Vec<(Expression, Expression)> = Vec::new();

            // Extract result var (default "cfhttp") and add remaining attrs
            let mut result_var = "cfhttp".to_string();
            for (name, value) in &attrs {
                if name == "result" {
                    // Extract the string value for the result variable name
                    if let Expression::Literal(ref lit) = value {
                        if let LiteralValue::String(ref s) = lit.value {
                            result_var = s.clone();
                        }
                    } else if let Expression::Identifier(ref id) = value {
                        result_var = id.name.clone();
                    }
                } else {
                    struct_pairs.push((
                        Expression::Literal(Literal {
                            value: LiteralValue::String(name.clone()),
                            location: stmt_loc.clone(),
                        }),
                        value.clone(),
                    ));
                }
            }

            // Add default method if not specified
            let has_method = attrs.iter().any(|(n, _)| n == "method");
            if !has_method {
                struct_pairs.push((
                    Expression::Literal(Literal {
                        value: LiteralValue::String("method".to_string()),
                        location: stmt_loc.clone(),
                    }),
                    Expression::Literal(Literal {
                        value: LiteralValue::String("GET".to_string()),
                        location: stmt_loc.clone(),
                    }),
                ));
            }

            // Add params array if any httpparam statements were found
            if !params.is_empty() {
                struct_pairs.push((
                    Expression::Literal(Literal {
                        value: LiteralValue::String("params".to_string()),
                        location: stmt_loc.clone(),
                    }),
                    Expression::Array(Array {
                        elements: params,
                        location: stmt_loc.clone(),
                    }),
                ));
            }

            // Build: result_var = cfhttp({ ... });
            let cfhttp_call = Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "cfhttp".to_string(),
                    location: stmt_loc.clone(),
                })),
                arguments: vec![Expression::Struct(Struct {
                    pairs: struct_pairs,
                    ordered: false,
                    location: stmt_loc.clone(),
                })],
                location: stmt_loc.clone(),
            }));

            return Ok(CfmlNode::Statement(Statement::Assignment(Assignment {
                target: AssignTarget::Variable(result_var),
                value: cfhttp_call,
                operator: AssignOp::Equal,
                location: stmt_loc,
            })));
        }

        // Handle 'cfinvoke'/'invoke' as a CFScript STATEMENT (the tag-in-script
        // form), e.g.
        //     cfinvoke component="Svc" method="m" returnvariable="r" arg=1;
        //     cfinvoke component=obj method="m" { invokeargument name="a" value=1; }
        // -> [<r> =] __cfinvoke(<component>, "<method>", <argStruct-or-collection>)
        // The function-CALL form `cfinvoke(component=...)` (peek(1) == LParen) is
        // NOT matched here and keeps parsing as an ordinary call. `component`,
        // `method`, etc. lex as keyword tokens, hence is_identifier_like_at.
        if matches!(self.peek(0), Token::Identifier(ref s)
                if s.eq_ignore_ascii_case("cfinvoke") || s.eq_ignore_ascii_case("invoke"))
            && self.is_identifier_like_at(1)
            && matches!(self.peek(2), Token::Equal)
        {
            self.advance(); // consume 'cfinvoke' / 'invoke'

            let mut component_expr: Option<Expression> = None;
            let mut method_expr: Option<Expression> = None;
            let mut return_var_expr: Option<Expression> = None;
            let mut arg_collection: Option<Expression> = None;
            let mut extra_pairs: Vec<(Expression, Expression)> = Vec::new();

            while !self.check(&Token::LBrace) && !self.check(&Token::Semicolon) && !self.is_at_end() {
                if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                    let key = self.extract_identifier()?.to_lowercase();
                    self.advance(); // consume =
                    let value = self.parse_expression()?;
                    match key.as_str() {
                        "component" => component_expr = Some(value),
                        "method" => method_expr = Some(value),
                        "returnvariable" => return_var_expr = Some(value),
                        "argumentcollection" => arg_collection = Some(value),
                        _ => extra_pairs.push((
                            Expression::Literal(Literal {
                                value: LiteralValue::String(key),
                                location: stmt_loc.clone(),
                            }),
                            value,
                        )),
                    }
                } else {
                    break;
                }
            }

            // Optional body of `invokeargument`/`cfinvokeargument name=.. value=..;`
            if self.check(&Token::LBrace) {
                self.advance(); // consume {
                while !self.check(&Token::RBrace) && !self.is_at_end() {
                    if matches!(self.peek(0), Token::Identifier(ref s)
                            if s.eq_ignore_ascii_case("invokeargument")
                                || s.eq_ignore_ascii_case("cfinvokeargument"))
                    {
                        self.advance(); // consume keyword
                        let mut arg_name: Option<Expression> = None;
                        let mut arg_value: Option<Expression> = None;
                        while !self.check(&Token::Semicolon) && !self.check(&Token::RBrace) && !self.is_at_end() {
                            if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                                let k = self.extract_identifier()?.to_lowercase();
                                self.advance(); // consume =
                                let v = self.parse_expression()?;
                                if k == "name" {
                                    arg_name = Some(v);
                                } else if k == "value" {
                                    arg_value = Some(v);
                                }
                            } else {
                                break;
                            }
                        }
                        self.match_token(&Token::Semicolon);
                        if let (Some(n), Some(v)) = (arg_name, arg_value) {
                            extra_pairs.push((n, v));
                        }
                    } else {
                        self.advance(); // skip unknown tokens inside the block
                    }
                }
                self.consume(&Token::RBrace)?;
            } else {
                self.match_token(&Token::Semicolon);
            }

            let str_lit = |s: &str| Expression::Literal(Literal {
                value: LiteralValue::String(s.to_string()),
                location: stmt_loc.clone(),
            });

            // Third arg: an explicit argumentcollection wins; otherwise a struct
            // of the remaining attributes + any invokeargument entries.
            let third_arg = arg_collection.unwrap_or_else(|| Expression::Struct(Struct {
                pairs: extra_pairs,
                ordered: false,
                location: stmt_loc.clone(),
            }));

            let call = Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "__cfinvoke".to_string(),
                    location: stmt_loc.clone(),
                })),
                arguments: vec![
                    component_expr.unwrap_or_else(|| str_lit("")),
                    method_expr.unwrap_or_else(|| str_lit("")),
                    third_arg,
                ],
                location: stmt_loc.clone(),
            }));

            // Bind the result if a returnVariable was given.
            match return_var_expr {
                None => {
                    return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                        expr: call,
                        location: stmt_loc,
                    })));
                }
                Some(rv) => {
                    // A STATIC name -> a real lvalue, reusing the normal assignment
                    // path so `local.rv` / `variables.x` / dotted paths all work.
                    let static_name = match &rv {
                        Expression::Literal(Literal { value: LiteralValue::String(s), .. }) => Some(s.clone()),
                        Expression::Identifier(id) => Some(id.name.clone()),
                        _ => None,
                    };
                    if let Some(name) = static_name {
                        if !name.is_empty() {
                            if let Some(lvalue) = self.returnvar_lvalue(&name) {
                                return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                                    expr: Expression::BinaryOp(Box::new(BinaryOp {
                                        left: Box::new(lvalue),
                                        operator: BinaryOpType::Assign,
                                        right: Box::new(call),
                                        location: stmt_loc.clone(),
                                    })),
                                    location: stmt_loc,
                                })));
                            }
                        }
                        // Empty/unusable static name: just invoke, drop the result.
                        return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                            expr: call,
                            location: stmt_loc,
                        })));
                    }
                    // A DYNAMIC returnVariable (e.g. "#arguments.rv#") -> assign by
                    // name at runtime. (setVariable resolves variables/request/
                    // session/application prefixes; other scopes go to variables.)
                    let set_call = Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "setVariable".to_string(),
                            location: stmt_loc.clone(),
                        })),
                        arguments: vec![rv, call],
                        location: stmt_loc.clone(),
                    }));
                    return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                        expr: set_call,
                        location: stmt_loc,
                    })));
                }
            }
        }

        // Handle CFScript tag-as-statement syntax: keyword attr=value attr=value;
        // content type="..."; → __cfcontent({...})
        // header name="..." value="..."; → __cfheader({...})
        // location url="..."; → __cflocation({...})
        // setting requesttimeout="..."; → __cfsetting({...})
        // cookie name="..." value="..."; → __cfcookie({...})
        // log text="..." type="..."; → __cflog({...})
        // dbinfo type="version" name="info" datasource="..."; → cfdbinfo({...})
        //   (the VM intercept binds the result query to the `name` variable via
        //    pending_result_writeback, exactly like the cfdbinfo(...)/<cfdbinfo>
        //    forms — issue #172).
        // Statement ("migration") form of tags in cfscript — `tag attr="v" …;`.
        // Recognized for any supported tag with OR without the `cf` prefix, and
        // only when the tag keyword is immediately followed by `attr=` so it can
        // never shadow a normal expression or assignment. All forms lower to the
        // SAME call the function form `cfTag(attr=…)` produces, so both share one
        // tested code path (struct-bundling + result write-back) in the VM —
        // this is what keeps the two syntaxes from drifting out of sync.
        {
            // How each tag lowers:
            //   StructArg(fn) → fn({ attrs })          (VM intercept takes one struct)
            //   NamedCall(fn) → fn(attr=…, …)          (flows through is_tag_call_builtin)
            //   Dump          → writeDump(attr=…, …)
            enum TagStmt {
                StructArg(&'static str),
                NamedCall(&'static str),
                Dump,
            }
            let strategy = if let Token::Identifier(ref s) = self.peek(0) {
                let lower = s.to_lowercase();
                // Accept the tag name with or without the `cf` prefix.
                let bare = lower.strip_prefix("cf").unwrap_or(lower.as_str());
                match bare {
                    "content" => Some(TagStmt::StructArg("__cfcontent")),
                    "application" => Some(TagStmt::StructArg("__cfapplication")),
                    "header" => Some(TagStmt::StructArg("__cfheader")),
                    "location" => Some(TagStmt::StructArg("__cflocation")),
                    "setting" => Some(TagStmt::StructArg("__cfsetting")),
                    "cookie" => Some(TagStmt::StructArg("__cfcookie")),
                    "log" => Some(TagStmt::StructArg("__cflog")),
                    "htmlhead" => Some(TagStmt::StructArg("__cfhtmlhead")),
                    "htmlbody" => Some(TagStmt::StructArg("__cfhtmlbody")),
                    "dbinfo" => Some(TagStmt::StructArg("cfdbinfo")),
                    // Script-form `module template="x" …;` / `module
                    // attributeCollection=args;` — the CFScript equivalent of
                    // <cfmodule>. Lowers to __cfmodule({attrs}); the VM extracts
                    // template/name (possibly from attributeCollection) at runtime
                    // and runs it through the same custom-tag machinery as the tag
                    // form. Without this the statement silently parsed as a bare
                    // `module` identifier + `template=…` assignment and rendered
                    // nothing — Preside renders every view via
                    // `module attributeCollection=…`, so every view came out empty.
                    "module" => Some(TagStmt::StructArg("__cfmodule")),
                    "directory" => Some(TagStmt::NamedCall("cfdirectory")),
                    "file" => Some(TagStmt::NamedCall("cffile")),
                    "zip" => Some(TagStmt::NamedCall("cfzip")),
                    "dump" => Some(TagStmt::Dump),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(strategy) = strategy {
                if self.is_identifier_like_at(1) && matches!(self.peek(2), Token::Equal) {
                    self.advance(); // consume the tag keyword
                    let mut attrs: Vec<(String, Expression)> = Vec::new();
                    while !self.check(&Token::Semicolon) && !self.is_at_end() {
                        if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                            let attr_name = self.extract_identifier()?.to_lowercase();
                            self.advance(); // consume =
                            let attr_value = self.parse_expression()?;
                            attrs.push((attr_name, attr_value));
                        } else {
                            break;
                        }
                    }
                    self.match_token(&Token::Semicolon);

                    let (func_name, arguments) = match strategy {
                        TagStmt::StructArg(func) => {
                            let pairs = attrs
                                .into_iter()
                                .map(|(k, v)| {
                                    (
                                        Expression::Literal(Literal {
                                            value: LiteralValue::String(k),
                                            location: stmt_loc.clone(),
                                        }),
                                        v,
                                    )
                                })
                                .collect();
                            (
                                func,
                                vec![Expression::Struct(Struct {
                                    pairs,
                                    ordered: false,
                                    location: stmt_loc.clone(),
                                })],
                            )
                        }
                        TagStmt::NamedCall(_) | TagStmt::Dump => {
                            let func = match strategy {
                                TagStmt::NamedCall(f) => f,
                                _ => "writeDump",
                            };
                            let args = attrs
                                .into_iter()
                                .map(|(k, v)| {
                                    Expression::NamedArgument(Box::new(NamedArgument {
                                        name: k,
                                        value: Box::new(v),
                                        location: stmt_loc.clone(),
                                    }))
                                })
                                .collect();
                            (func, args)
                        }
                    };

                    let call = Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: func_name.to_string(),
                            location: stmt_loc.clone(),
                        })),
                        arguments,
                        location: stmt_loc.clone(),
                    }));
                    return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                        expr: call,
                        location: stmt_loc,
                    })));
                }
            }
        }

        // Handle 'thread name="..." action="run" { body }' in CFScript
        if matches!(self.peek(0), Token::Identifier(ref s) if s.to_lowercase() == "thread")
            && self.is_identifier_like_at(1)
            && matches!(self.peek(2), Token::Equal)
        {
            self.advance(); // consume 'thread'
            let mut attrs: Vec<(String, Expression)> = Vec::new();
            while !self.check(&Token::LBrace) && !self.check(&Token::Semicolon) && !self.is_at_end() {
                if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                    let attr_name = self.extract_identifier()?;
                    self.advance(); // consume =
                    let attr_value = self.parse_expression()?;
                    attrs.push((attr_name.to_lowercase(), attr_value));
                } else {
                    break;
                }
            }

            // Extract name and action
            let thread_name = attrs.iter()
                .find(|(k, _)| k == "name")
                .map(|(_, v)| v.clone())
                .unwrap_or(Expression::Literal(Literal {
                    value: LiteralValue::String("thread1".to_string()),
                    location: stmt_loc.clone(),
                }));
            let action = attrs.iter()
                .find(|(k, _)| k == "action")
                .and_then(|(_, v)| if let Expression::Literal(ref lit) = v {
                    if let LiteralValue::String(ref s) = lit.value { Some(s.to_lowercase()) } else { None }
                } else { None })
                .unwrap_or_else(|| "run".to_string());

            match action.as_str() {
                "run" => {
                    // Parse body block
                    let body = self.parse_block()?;
                    // → __cfthread_run(name, function() { body } [, {custom attrs}])
                    let closure = Expression::Closure(Box::new(Closure {
                        params: vec![],
                        body,
                        location: stmt_loc.clone(),
                        metadata: Vec::new(),
                    }));
                    // Every attribute except the reserved control ones becomes the
                    // thread's `attributes` scope, bound from the parent at spawn —
                    // matching the tag form (tag_parser.rs). Without this, a
                    // script-form `thread ... svc="#obj#" { attributes.svc.foo() }`
                    // saw `attributes` undefined inside the body (GH #234).
                    let attr_pairs: Vec<(Expression, Expression)> = attrs
                        .iter()
                        .filter(|(k, _)| {
                            !matches!(k.as_str(), "action" | "name" | "priority" | "timeout")
                        })
                        .map(|(k, v)| {
                            (
                                Expression::Literal(Literal {
                                    value: LiteralValue::String(k.clone()),
                                    location: stmt_loc.clone(),
                                }),
                                v.clone(),
                            )
                        })
                        .collect();
                    let mut arguments = vec![thread_name, closure];
                    if !attr_pairs.is_empty() {
                        arguments.push(Expression::Struct(Struct {
                            pairs: attr_pairs,
                            ordered: false,
                            location: stmt_loc.clone(),
                        }));
                    }
                    let call = Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cfthread_run".to_string(),
                            location: stmt_loc.clone(),
                        })),
                        arguments,
                        location: stmt_loc.clone(),
                    }));
                    return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                        expr: call,
                        location: stmt_loc,
                    })));
                }
                "join" => {
                    self.match_token(&Token::Semicolon);
                    let timeout = attrs.iter()
                        .find(|(k, _)| k == "timeout")
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Expression::Literal(Literal {
                            value: LiteralValue::Int(0),
                            location: stmt_loc.clone(),
                        }));
                    let call = Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cfthread_join".to_string(),
                            location: stmt_loc.clone(),
                        })),
                        arguments: vec![thread_name, timeout],
                        location: stmt_loc.clone(),
                    }));
                    return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                        expr: call,
                        location: stmt_loc,
                    })));
                }
                "terminate" => {
                    self.match_token(&Token::Semicolon);
                    let call = Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cfthread_terminate".to_string(),
                            location: stmt_loc.clone(),
                        })),
                        arguments: vec![thread_name],
                        location: stmt_loc.clone(),
                    }));
                    return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                        expr: call,
                        location: stmt_loc,
                    })));
                }
                _ => {
                    self.match_token(&Token::Semicolon);
                }
            }
        }

        // Handle 'savecontent variable="varname" { body }' in CFScript
        if matches!(self.peek(0), Token::Identifier(ref s) if s.to_lowercase() == "savecontent") {
            self.advance(); // consume 'savecontent'
            // Parse attributes: variable = "name"
            let mut var_target =
                AssignTarget::Variable("__savecontent_result".to_string());
            while !self.check(&Token::LBrace) && !self.is_at_end() {
                if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                    let attr_name = self.extract_identifier()?;
                    self.advance(); // consume =
                    let attr_value = self.parse_expression()?;
                    if attr_name.to_lowercase() == "variable" {
                        // The variable= target may be unqualified ("cap"), scope-
                        // qualified ("local.cap" → a MemberAccess expr), or a string
                        // literal ("variables.cap"). Lower each to a proper
                        // AssignTarget so the capture lands in the right scope.
                        var_target = match attr_value {
                            Expression::Literal(ref lit) => {
                                if let LiteralValue::String(ref s) = lit.value {
                                    self.assign_target_from_dotted(s, stmt_loc.clone())
                                } else {
                                    self.expression_to_assign_target(&attr_value)?
                                }
                            }
                            _ => self.expression_to_assign_target(&attr_value)?,
                        };
                    }
                } else {
                    break;
                }
            }
            // Parse body block
            let body = self.parse_block()?;
            // Convert to: __cfsavecontent_start(); body; varname = __cfsavecontent_end();
            let mut stmts = Vec::new();
            stmts.push(Statement::Expression(ExpressionStatement {
                expr: Expression::FunctionCall(Box::new(FunctionCall {
                    name: Box::new(Expression::Identifier(Identifier {
                        name: "__cfsavecontent_start".to_string(),
                        location: stmt_loc.clone(),
                    })),
                    arguments: vec![],
                    location: stmt_loc.clone(),
                })),
                location: stmt_loc.clone(),
            }));
            stmts.extend(body);
            stmts.push(Statement::Assignment(Assignment {
                target: var_target,
                value: Expression::FunctionCall(Box::new(FunctionCall {
                    name: Box::new(Expression::Identifier(Identifier {
                        name: "__cfsavecontent_end".to_string(),
                        location: stmt_loc.clone(),
                    })),
                    arguments: vec![],
                    location: stmt_loc.clone(),
                })),
                operator: AssignOp::Equal,
                location: stmt_loc.clone(),
            }));
            return Ok(CfmlNode::Statement(Statement::Output(Output {
                body: stmts,
                location: stmt_loc,
            })));
        }

        // Handle bare `silent { body }` keyword form in CFScript (script
        // equivalent of <cfsilent>). Lower to a savecontent capture whose
        // result is discarded — suppressing the body's output. Only treat as a
        // block when an LBrace immediately follows, so `silent = x;`,
        // `silent.foo`, `silent()` etc. stay ordinary identifiers.
        if matches!(self.peek(0), Token::Identifier(ref s) if s.eq_ignore_ascii_case("silent"))
            && matches!(self.peek(1), Token::LBrace)
        {
            self.advance(); // consume 'silent'
            let body = self.parse_block()?;
            let mut stmts = Vec::new();
            stmts.push(Statement::Expression(ExpressionStatement {
                expr: Expression::FunctionCall(Box::new(FunctionCall {
                    name: Box::new(Expression::Identifier(Identifier {
                        name: "__cfsavecontent_start".to_string(),
                        location: stmt_loc.clone(),
                    })),
                    arguments: vec![],
                    location: stmt_loc.clone(),
                })),
                location: stmt_loc.clone(),
            }));
            stmts.extend(body);
            // Discard the captured output (no assignment).
            stmts.push(Statement::Expression(ExpressionStatement {
                expr: Expression::FunctionCall(Box::new(FunctionCall {
                    name: Box::new(Expression::Identifier(Identifier {
                        name: "__cfsavecontent_end".to_string(),
                        location: stmt_loc.clone(),
                    })),
                    arguments: vec![],
                    location: stmt_loc.clone(),
                })),
                location: stmt_loc.clone(),
            }));
            return Ok(CfmlNode::Statement(Statement::Output(Output {
                body: stmts,
                location: stmt_loc,
            })));
        }

        // Handle script-call-with-body forms for body-block tags:
        //   cfsavecontent(variable="x") { body }
        //   cflock(name="x", type="exclusive", timeout=10) { body }
        //   cftransaction(...) { body }
        //   cfmail(to="...", from="...", subject="...") { body }
        // Plus the bare `transaction { body }` keyword form.
        // These lower to the same bytecode as the angle-bracket tag forms.
        if let Token::Identifier(ref nm) = self.peek(0).clone() {
            let nlow = nm.to_lowercase();
            let is_body_tag = matches!(nlow.as_str(),
                "cfsavecontent" | "cflock" | "cftransaction" | "cfmail" | "cfquery" | "cfhttp");
            if is_body_tag && matches!(self.peek(1), Token::LParen) {
                let saved = self.current;
                self.advance(); // tag name
                self.advance(); // (
                let attrs = match self.parse_tag_call_attrs() {
                    Ok(a) => a,
                    Err(_) => { self.current = saved; Vec::new() }
                };
                if self.current != saved && self.check(&Token::RParen) {
                    self.advance(); // )
                    if self.check(&Token::LBrace) {
                        let body = self.parse_block()?;
                        return Ok(self.lower_script_body_tag(&nlow, attrs, body, stmt_loc));
                    } else {
                        self.current = saved;
                    }
                }
            }
            // `transaction { body }` (bare) or `transaction action="begin" { body }`
            // (with space-separated tag attributes). Lucee/Adobe CF/BoxLang accept
            // both. The attribute form mirrors the angle-bracket `<cftransaction
            // action="...">` tag and lowers to the same start/commit/rollback shape.
            if nlow == "transaction"
                && (matches!(self.peek(1), Token::LBrace)
                    || (self.is_identifier_like_at(1) && matches!(self.peek(2), Token::Equal)))
            {
                let saved = self.current;
                self.advance(); // consume 'transaction'
                let mut attrs: Vec<(String, Expression)> = Vec::new();
                while self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                    let key = self.extract_identifier()?;
                    self.advance(); // consume =
                    match self.parse_expression() {
                        Ok(v) => attrs.push((key, v)),
                        Err(_) => break,
                    }
                }
                if self.check(&Token::LBrace) {
                    let body = self.parse_block()?;
                    return Ok(self.lower_script_body_tag("cftransaction", attrs, body, stmt_loc));
                }
                // Body-less statement form (no `{`):
                //   transaction action="commit";   → __cftransaction_commit()
                //   transaction action="rollback";  → __cftransaction_rollback()
                //   transaction action="begin";     → __cftransaction_start("begin"[, isolation][, datasource])
                // Mirrors the angle-bracket <cftransaction action="..."> tag lowering
                // (tag_parser.rs). Wheels migration templates emit the commit/rollback
                // forms, so without this they failed with `transaction is undefined`.
                if !attrs.is_empty() {
                    self.match_token(&Token::Semicolon); // optional trailing ;
                    let action = attrs
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("action"))
                        .and_then(|(_, v)| match v {
                            Expression::Literal(l) => match &l.value {
                                LiteralValue::String(s) => Some(s.to_lowercase()),
                                _ => None,
                            },
                            _ => None,
                        })
                        .unwrap_or_else(|| "begin".to_string());
                    let fn_name = match action.as_str() {
                        "commit" => "__cftransaction_commit",
                        "rollback" => "__cftransaction_rollback",
                        _ => "__cftransaction_start",
                    };
                    let mut arguments: Vec<Expression> = Vec::new();
                    if fn_name == "__cftransaction_start" {
                        arguments.push(Expression::Literal(Literal {
                            value: LiteralValue::String("begin".to_string()),
                            location: stmt_loc,
                        }));
                        if let Some((_, iso)) =
                            attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case("isolation"))
                        {
                            arguments.push(iso.clone());
                        }
                        if let Some((_, ds)) =
                            attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case("datasource"))
                        {
                            arguments.push(ds.clone());
                        }
                    }
                    return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                        expr: Expression::FunctionCall(Box::new(FunctionCall {
                            name: Box::new(Expression::Identifier(Identifier {
                                name: fn_name.to_string(),
                                location: stmt_loc,
                            })),
                            arguments,
                            location: stmt_loc,
                        })),
                        location: stmt_loc,
                    })));
                }
                // No attributes and no body — restore and fall through.
                self.current = saved;
            }
            // cfhttpparam(name=…, value=…, type=…) statement → appends to the
            // runtime params array a surrounding script cfhttp(){ } block set up.
            // Mirrors the tag preprocessor's <cfhttpparam> → arrayAppend lowering,
            // so params declared inside control flow (for/if) in a cfhttp body are
            // collected at runtime (issue #55). Outside a cfhttp block,
            // __cfhttp_params is undefined — exactly as the tag form behaves.
            if nlow == "cfhttpparam" && matches!(self.peek(1), Token::LParen) {
                let saved = self.current;
                self.advance(); // cfhttpparam
                self.advance(); // (
                let attrs = match self.parse_tag_call_attrs() {
                    Ok(a) => a,
                    Err(_) => { self.current = saved; Vec::new() }
                };
                if self.current != saved && self.check(&Token::RParen) {
                    self.advance(); // )
                    self.match_token(&Token::Semicolon); // optional trailing ;
                    let param_struct = Expression::Struct(Struct {
                        pairs: attrs.iter().map(|(k, v)| (
                            Expression::Literal(Literal {
                                value: LiteralValue::String(k.clone()),
                                location: stmt_loc,
                            }),
                            v.clone(),
                        )).collect(),
                        ordered: false,
                        location: stmt_loc,
                    });
                    return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                        expr: Expression::FunctionCall(Box::new(FunctionCall {
                            name: Box::new(Expression::Identifier(Identifier {
                                name: "arrayAppend".to_string(),
                                location: stmt_loc,
                            })),
                            arguments: vec![
                                Expression::Identifier(Identifier {
                                    name: "__cfhttp_params".to_string(),
                                    location: stmt_loc,
                                }),
                                param_struct,
                            ],
                            location: stmt_loc,
                        })),
                        location: stmt_loc,
                    })));
                }
                self.current = saved;
            }
            // cfmailpart(type="text") { body } / cfmailparam(name=…, value=…)
            // statement forms inside a script cfmail(){ } block. They mirror the
            // <cfmailpart>/<cfmailparam> child tags: cfmailpart captures its own
            // body via a nested savecontent and appends {attrs, body} to the
            // runtime __cfmail_parts array; cfmailparam appends {attrs} to
            // __cfmail_params. The surrounding script cfmail(){ } lowering seeds
            // both arrays and forwards them to __cfmail. Wheels' Global.cfc
            // $mail() emits these in script form for every multipart email, so
            // without this the whole file fails to PARSE (issue: PR #138).
            if nlow == "cfmailpart" && matches!(self.peek(1), Token::LParen) {
                let saved = self.current;
                self.advance(); // cfmailpart
                self.advance(); // (
                let attrs = match self.parse_tag_call_attrs() {
                    Ok(a) => a,
                    Err(_) => { self.current = saved; Vec::new() }
                };
                if self.current != saved && self.check(&Token::RParen) {
                    self.advance(); // )
                    if self.check(&Token::LBrace) {
                        let body = self.parse_block()?;
                        return Ok(self.lower_script_mailpart(attrs, body, stmt_loc));
                    } else {
                        self.match_token(&Token::Semicolon);
                        return Ok(self.lower_script_mailpart(attrs, Vec::new(), stmt_loc));
                    }
                }
                self.current = saved;
            }
            if nlow == "cfmailparam" && matches!(self.peek(1), Token::LParen) {
                let saved = self.current;
                self.advance(); // cfmailparam
                self.advance(); // (
                let attrs = match self.parse_tag_call_attrs() {
                    Ok(a) => a,
                    Err(_) => { self.current = saved; Vec::new() }
                };
                if self.current != saved && self.check(&Token::RParen) {
                    self.advance(); // )
                    self.match_token(&Token::Semicolon); // optional trailing ;
                    let param_struct = self.attrs_to_struct_literal(&attrs, stmt_loc);
                    return Ok(self.array_append_stmt("__cfmail_params", param_struct, stmt_loc));
                }
                self.current = saved;
            }
        }

        // Handle 'abort' keyword as __cfabort() call
        if matches!(self.peek(0), Token::Identifier(ref s) if s.to_lowercase() == "abort") {
            self.advance(); // consume 'abort'
            self.match_token(&Token::Semicolon);
            // Build a function call expression to __cfabort()
            let abort_call = Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "__cfabort".to_string(),
                    location: stmt_loc.clone(),
                })),
                arguments: vec![],
                location: stmt_loc.clone(),
            }));
            return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                expr: abort_call,
                location: stmt_loc,
            })));
        }

        // Expression statement (may be assignment)
        let expr = self.parse_expression()?;

        // Check for compound assignment on expressions
        if let Some(assign_op) = self.check_assignment_op() {
            self.advance(); // consume the operator
            let value = self.parse_expression()?;
            self.match_token(&Token::Semicolon);

            let target = self.expression_to_assign_target(&expr)?;
            return Ok(CfmlNode::Statement(Statement::Assignment(Assignment {
                target,
                value,
                operator: assign_op,
                location: stmt_loc,
            })));
        }

        // Check for postfix ++ / --
        if self.match_token(&Token::PlusPlus) || self.match_token(&Token::MinusMinus) {
            let op = match self.previous().token {
                Token::PlusPlus => PostfixOpType::Increment,
                _ => PostfixOpType::Decrement,
            };
            self.match_token(&Token::Semicolon);
            return Ok(CfmlNode::Statement(Statement::Expression(
                ExpressionStatement {
                    expr: Expression::PostfixOp(Box::new(PostfixOp {
                        operand: Box::new(expr),
                        operator: op,
                        location: stmt_loc,
                    })),
                    location: stmt_loc,
                },
            )));
        }

        self.match_token(&Token::Semicolon);

        Ok(CfmlNode::Statement(Statement::Expression(
            ExpressionStatement {
                expr,
                location: stmt_loc,
            },
        )))
    }

    fn check_assignment_op(&self) -> Option<AssignOp> {
        match self.peek(0) {
            Token::PlusEqual => Some(AssignOp::PlusEqual),
            Token::MinusEqual => Some(AssignOp::MinusEqual),
            Token::StarEqual => Some(AssignOp::StarEqual),
            Token::SlashEqual => Some(AssignOp::SlashEqual),
            Token::AmpEqual => Some(AssignOp::ConcatEqual),
            Token::PercentEqual => Some(AssignOp::PercentEqual),
            _ => None,
        }
    }

    /// Map a compound-assignment operator token to the binary op it applies, for
    /// desugaring `lhs OP= rhs` into `lhs = (lhs OP rhs)` (used in the
    /// for-increment clause, which is parsed as an expression).
    fn compound_assign_binop(&self) -> Option<BinaryOpType> {
        match self.peek(0) {
            Token::PlusEqual => Some(BinaryOpType::Add),
            Token::MinusEqual => Some(BinaryOpType::Sub),
            Token::StarEqual => Some(BinaryOpType::Mul),
            Token::SlashEqual => Some(BinaryOpType::Div),
            Token::PercentEqual => Some(BinaryOpType::Mod),
            Token::AmpEqual => Some(BinaryOpType::Concat),
            _ => None,
        }
    }

    /// Turn a STATIC cfinvoke `returnVariable` name ("msg", "local.rv",
    /// "variables.x") into an assignable lvalue expression by sub-parsing it, so
    /// the normal assignment path handles scope prefixes and dotted paths. Returns
    /// None if the name doesn't parse to a simple lvalue (Identifier / member /
    /// index access), in which case the caller falls back to setVariable.
    fn returnvar_lvalue(&self, name: &str) -> Option<Expression> {
        let mut sub = Parser::new(format!("{};", name));
        let program = sub.parse().ok()?;
        let expr = program.statements.into_iter().find_map(|node| match node {
            CfmlNode::Statement(Statement::Expression(es)) => Some(es.expr),
            CfmlNode::Expression(e) => Some(e),
            _ => None,
        })?;
        match expr {
            Expression::Identifier(_)
            | Expression::MemberAccess(_)
            | Expression::ArrayAccess(_) => Some(expr),
            _ => None,
        }
    }

    fn expression_to_assign_target(&self, expr: &Expression) -> Result<AssignTarget, ParseError> {
        match expr {
            Expression::Identifier(id) => Ok(AssignTarget::Variable(id.name.clone())),
            Expression::ArrayAccess(acc) => Ok(AssignTarget::ArrayAccess(
                acc.array.clone(),
                acc.index.clone(),
            )),
            Expression::MemberAccess(acc) => {
                Ok(AssignTarget::StructAccess(acc.object.clone(), acc.member.clone()))
            }
            _ => Err(self.parse_error("Invalid assignment target")),
        }
    }

    fn parse_access_modifier(&mut self) -> AccessModifier {
        let tok = self.advance().token.clone();
        match tok {
            Token::Public => AccessModifier::Public,
            Token::Private => AccessModifier::Private,
            Token::Remote => AccessModifier::Remote,
            Token::Package => AccessModifier::Package,
            _ => AccessModifier::Public,
        }
    }

    fn parse_var(&mut self) -> Result<Statement, ParseError> {
        let loc = self.current_location();
        // `var` was already consumed, so the next token is unambiguously the
        // variable name. Lucee/ACF/BoxLang accept ANY keyword there (`var catch`,
        // `var type`, `var switch`, …); Preside's ErrorLogService uses
        // `var catch = arguments.error`. Use the permissive property-name
        // extractor (identifiers + soft keywords + all reserved words).
        let mut name = self.extract_property_name()?;
        // CFML allows dotted var declarations like: var local.x = 1
        while self.match_token(&Token::Dot) {
            let part = self.extract_property_name()?;
            name.push('.');
            name.push_str(&part);
        }

        // `var m[ key ] = value` — a subscripted (or further member-chained)
        // target, NOT a bare declaration. Preside's Renderer builds its
        // view-mapping struct with exactly this idiom (`var mappings[ mapping ] =
        // …` inside a loop). It must write into the EXISTING local struct via an
        // indexed assignment; treating it as `var m` (declaration) re-declared and
        // wiped `m` every iteration, leaving the map empty — the cause of every
        // Preside front-end view resolving to /app/views and 404ing. Lower to an
        // indexed assignment on `local.<name>` (var semantics: force local, don't
        // clobber). Lucee accepts and honours this form.
        if self.check(&Token::LBracket) {
            return self.parse_var_indexed_assignment(name, loc);
        }

        let value = if self.match_token(&Token::Equal) {
            Some(self.parse_expression()?)
        } else {
            None
        };

        self.match_token(&Token::Semicolon);

        Ok(Statement::Var(Var {
            name,
            value,
            location: loc,
        }))
    }

    /// Lower `var <name>[ idx ]… = value` to an indexed assignment on
    /// `local.<name>` (a bare name is scoped local per `var` semantics; an
    /// explicitly-scoped/dotted name keeps its own path). See `parse_var`.
    fn parse_var_indexed_assignment(
        &mut self,
        name: String,
        loc: SourceLocation,
    ) -> Result<Statement, ParseError> {
        let full = if name.contains('.') { name } else { format!("local.{}", name) };
        let mut parts = full.split('.');
        let root = parts.next().unwrap_or("").to_string();
        let mut expr = Expression::Identifier(Identifier { name: root, location: loc.clone() });
        for part in parts {
            expr = Expression::MemberAccess(Box::new(MemberAccess {
                object: Box::new(expr),
                member: part.to_string(),
                null_safe: false,
                location: loc.clone(),
            }));
        }
        // Consume the postfix subscript / member chain: `[ expr ]` and `.member`.
        loop {
            if self.match_token(&Token::LBracket) {
                let index = self.parse_expression()?;
                self.consume(&Token::RBracket)?;
                expr = Expression::ArrayAccess(Box::new(ArrayAccess {
                    array: Box::new(expr),
                    index: Box::new(index),
                    location: loc.clone(),
                }));
            } else if self.match_token(&Token::Dot) {
                let member = self.extract_property_name()?;
                expr = Expression::MemberAccess(Box::new(MemberAccess {
                    object: Box::new(expr),
                    member,
                    null_safe: false,
                    location: loc.clone(),
                }));
            } else {
                break;
            }
        }
        // Assignment: plain `=` or a compound op. A bare `var m[k];` (no `=`) is
        // degenerate — emit the access as an expression statement so it doesn't error.
        let op = if self.check(&Token::Equal) {
            self.advance();
            Some(AssignOp::Equal)
        } else if let Some(o) = self.check_assignment_op() {
            self.advance();
            Some(o)
        } else {
            None
        };
        if let Some(operator) = op {
            let value = self.parse_expression()?;
            self.match_token(&Token::Semicolon);
            let target = self.expression_to_assign_target(&expr)?;
            return Ok(Statement::Assignment(Assignment {
                target,
                value,
                operator,
                location: loc,
            }));
        }
        self.match_token(&Token::Semicolon);
        Ok(Statement::Expression(ExpressionStatement { expr, location: loc }))
    }

    fn parse_if(&mut self) -> Result<If, ParseError> {
        let loc = self.current_location();
        self.consume(&Token::LParen)?;
        let condition = self.parse_expression()?;
        self.consume(&Token::RParen)?;

        let then_branch = if self.check(&Token::LBrace) {
            self.parse_block()?
        } else {
            // Single statement without braces
            let stmt = self.parse_statement()?;
            if let CfmlNode::Statement(s) = stmt {
                vec![s]
            } else {
                Vec::new()
            }
        };

        let mut else_if = Vec::new();
        let mut else_branch = None;

        // Handle any interleaving of `else if`, `else elseif`, standalone
        // `elseif` (one keyword, no leading `else`), and a terminal `else`.
        // A single unified loop is needed because a standalone `elseif` may be
        // FOLLOWED by an `else` (Lucee accepts `if{} elseif{} else{}`); two
        // separate loops (one for `else…`, one for `elseif`) stranded that
        // trailing `else` → "Expected RBrace" (Preside emailCenter/Layouts.cfc).
        let parse_branch_body = |p: &mut Self| -> Result<Vec<Statement>, ParseError> {
            if p.check(&Token::LBrace) {
                p.parse_block()
            } else {
                let stmt = p.parse_statement()?;
                Ok(if let CfmlNode::Statement(s) = stmt { vec![s] } else { Vec::new() })
            }
        };
        loop {
            if self.match_token(&Token::ElseIf) {
                // standalone `elseif (…) {…}`
                self.consume(&Token::LParen)?;
                let cond = self.parse_expression()?;
                self.consume(&Token::RParen)?;
                let body = parse_branch_body(self)?;
                else_if.push(ElseIf { condition: cond, body });
            } else if self.match_token(&Token::Else) {
                if self.match_token(&Token::If) || self.match_token(&Token::ElseIf) {
                    // `else if (…)` / `else elseif (…)`
                    self.consume(&Token::LParen)?;
                    let cond = self.parse_expression()?;
                    self.consume(&Token::RParen)?;
                    let body = parse_branch_body(self)?;
                    else_if.push(ElseIf { condition: cond, body });
                } else {
                    // terminal `else`
                    else_branch = Some(parse_branch_body(self)?);
                    break;
                }
            } else {
                break;
            }
        }

        Ok(If {
            condition,
            then_branch,
            else_if,
            else_branch,
            location: loc,
        })
    }

    fn parse_for_statement(&mut self) -> Result<CfmlNode, ParseError> {
        let loc = self.current_location();
        self.consume(&Token::LParen)?;

        // Check for for-in: for (var x in collection) or for (x in collection)
        let has_var = self.match_token(&Token::Var);

        // Lookahead to detect for-in: scan past a (possibly dotted) identifier to find 'in'
        {
            let mut la = 0;
            // First token must be an identifier, soft keyword, or `this`
            // (Wheels-style `for (this.x.y in arr)` writes through the
            // component instance — Lucee/ACF/BoxLang all accept it.)
            let is_ident_start =
                self.is_identifier_like_at(la) || matches!(self.peek(la), Token::This);
            if is_ident_start {
                la += 1;
                // Skip dotted parts: .ident .ident ... — keyword-like member
                // names (package, default, ...) are valid here, so reuse the
                // canonical identifier-like check.
                while matches!(self.peek(la), Token::Dot)
                    && self.is_identifier_like_at(la + 1)
                {
                    la += 2;
                }
                if matches!(self.peek(la), Token::In) {
                    // It's a for-in loop — consume the dotted name
                    let mut name = if matches!(self.peek(0), Token::This) {
                        self.advance();
                        "this".to_string()
                    } else {
                        self.extract_identifier()?
                    };
                    while self.match_token(&Token::Dot) {
                        let part = self.extract_identifier()?;
                        name.push('.');
                        name.push_str(&part);
                    }
                    self.advance(); // consume 'in'
                    let iterable = self.parse_expression()?;
                    self.consume(&Token::RParen)?;
                    let body = self.parse_block_or_statement()?;
                    return Ok(CfmlNode::Statement(Statement::ForIn(ForIn {
                        variable: name,
                        iterable,
                        body,
                        location: loc,
                    })));
                }
            }
        }

        // Standard C-style for loop: for (init; condition; increment)
        let init = if has_var {
            Some(Box::new(Statement::Var(self.parse_var_no_semicolon()?)))
        } else if !self.check(&Token::Semicolon) {
            let expr = self.parse_expression()?;
            // Check if it's an assignment
            if self.match_token(&Token::Equal) {
                let value = self.parse_expression()?;
                if let Expression::Identifier(ident) = &expr {
                    Some(Box::new(Statement::Var(Var {
                        name: ident.name.clone(),
                        value: Some(value),
                        location: self.current_location(),
                    })))
                } else {
                    Some(Box::new(Statement::Expression(ExpressionStatement {
                        expr,
                        location: self.current_location(),
                    })))
                }
            } else {
                Some(Box::new(Statement::Expression(ExpressionStatement {
                    expr,
                    location: self.current_location(),
                })))
            }
        } else {
            None
        };

        self.consume(&Token::Semicolon)?;

        let condition = if !self.check(&Token::Semicolon) {
            Some(self.parse_expression()?)
        } else {
            None
        };

        self.consume(&Token::Semicolon)?;

        let increment = if !self.check(&Token::RParen) {
            let expr = self.parse_expression()?;
            // Support compound assignment in the increment clause (`i += 2`).
            // Statement-level compound assignment is handled in the statement
            // parser, but the for-increment is parsed as an expression, so
            // desugar `lhs OP= rhs` to `lhs = (lhs OP rhs)` here for assignable
            // targets — reusing the existing Assign binary op.
            let expr = if let Some(bin_op) = self.compound_assign_binop() {
                if matches!(
                    expr,
                    Expression::Identifier(_)
                        | Expression::MemberAccess(_)
                        | Expression::ArrayAccess(_)
                ) {
                    self.advance(); // consume the compound operator
                    let value = self.parse_expression()?;
                    let combined = Expression::BinaryOp(Box::new(BinaryOp {
                        left: Box::new(expr.clone()),
                        operator: bin_op,
                        right: Box::new(value),
                        location: self.current_location(),
                    }));
                    Expression::BinaryOp(Box::new(BinaryOp {
                        left: Box::new(expr),
                        operator: BinaryOpType::Assign,
                        right: Box::new(combined),
                        location: self.current_location(),
                    }))
                } else {
                    expr
                }
            } else {
                expr
            };
            Some(Box::new(expr))
        } else {
            None
        };

        self.consume(&Token::RParen)?;

        let body = if self.check(&Token::LBrace) {
            self.parse_block()?
        } else {
            let stmt = self.parse_statement()?;
            if let CfmlNode::Statement(s) = stmt {
                vec![s]
            } else {
                Vec::new()
            }
        };

        Ok(CfmlNode::Statement(Statement::For(For {
            init,
            condition,
            increment,
            body,
            location: loc,
        })))
    }

    fn parse_var_no_semicolon(&mut self) -> Result<Var, ParseError> {
        let loc = self.current_location();
        // See parse_var: the name after `var` may be any keyword (Lucee parity).
        let mut name = self.extract_property_name()?;
        // CFML allows dotted var declarations like: var local.i = 1
        while self.match_token(&Token::Dot) {
            let part = self.extract_property_name()?;
            name.push('.');
            name.push_str(&part);
        }
        let value = if self.match_token(&Token::Equal) {
            Some(self.parse_expression()?)
        } else {
            None
        };

        Ok(Var {
            name,
            value,
            location: loc,
        })
    }

    /// Parse the cfscript-statement `loop` form (`loop`-keyword already
    /// consumed). Collects the `attr=value` pairs, parses the `{ … }` body, and
    /// lowers to the equivalent `for`/`for-in`/`while` AST — mirroring the
    /// `<cfloop>` tag preprocessor in tag_parser.rs. (Issue #183.)
    fn parse_script_loop(&mut self, loc: SourceLocation) -> Result<CfmlNode, ParseError> {
        // Collect attributes until the body block (or a stray `;`). Two syntaxes:
        //   loop array=x item="i" { … }          (space-separated attrs)
        //   cfloop( array=x, item="i" ) { … }    (paren-wrapped, comma-separated)
        // The lucee-spreadsheet lib (Preside asset manager) uses the paren form.
        let mut attrs: Vec<(String, Expression)> = Vec::new();
        if self.check(&Token::LParen) {
            self.advance(); // consume `(`
            for (k, v) in self.parse_tag_call_attrs()? {
                attrs.push((k.to_lowercase(), v));
            }
            self.consume(&Token::RParen)?;
        } else {
            while !self.check(&Token::LBrace) && !self.check(&Token::Semicolon) && !self.is_at_end() {
                if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                    let key = self.extract_identifier()?;
                    self.advance(); // consume =
                    let value = self.parse_expression()?;
                    attrs.push((key.to_lowercase(), value));
                } else {
                    break;
                }
            }
        }

        let body = self.parse_block()?;

        // --- attribute helpers ----------------------------------------------
        let get = |name: &str| -> Option<Expression> {
            attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
        };
        // `index`/`item`/`key` name a variable; the value is a string literal
        // (`index="i"`) or a bare identifier (`index=i`). Extract the name.
        let var_name = |e: &Expression| -> Option<String> {
            match e {
                Expression::Literal(Literal { value: LiteralValue::String(s), .. }) => {
                    Some(s.clone())
                }
                Expression::Identifier(id) => Some(id.name.clone()),
                _ => None,
            }
        };

        // --- expression constructors ----------------------------------------
        // A loop variable named with an explicit scope (`item="local.v"`,
        // `index="local.i"` — the lucee-spreadsheet lib does this) must lower to
        // a scoped MemberAccess READ, not an `Identifier` whose name literally
        // contains a dot (codegen can't resolve `local.i` as one opaque name and
        // throws "Variable 'local.i' is undefined"). A dot-free name stays a
        // plain identifier (temps like `__cfloop_array_N`, bare `i`).
        let ident = |name: &str| -> Expression {
            match name.find('.') {
                None => Expression::Identifier(Identifier { name: name.to_string(), location: loc }),
                Some(pos) => {
                    let root = &name[..pos];
                    let mut expr = Expression::Identifier(Identifier {
                        name: root.to_string(),
                        location: loc,
                    });
                    for part in name[pos + 1..].split('.') {
                        expr = Expression::MemberAccess(Box::new(MemberAccess {
                            object: Box::new(expr),
                            member: part.to_string(),
                            null_safe: false,
                            location: loc,
                        }));
                    }
                    expr
                }
            }
        };
        let int = |n: i64| Expression::Literal(Literal { value: LiteralValue::Int(n), location: loc });
        let bin = |l: Expression, op: BinaryOpType, r: Expression| {
            Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(l),
                operator: op,
                right: Box::new(r),
                location: loc,
            }))
        };
        let call = |name: &str, args: Vec<Expression>| {
            Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier { name: name.to_string(), location: loc })),
                arguments: args,
                location: loc,
            }))
        };
        // `name = value;` as an assignment statement. A scoped/dotted name
        // (`local.v`) becomes a StructAccess target, not a Variable named
        // "local.v" — see `ident` above and `assign_target_from_dotted`.
        let assign = |name: String, value: Expression| {
            Statement::Assignment(Assignment {
                target: self.assign_target_from_dotted(&name, loc),
                value,
                operator: AssignOp::Equal,
                location: loc,
            })
        };
        // Unique temp suffix for synthesized loop variables.
        let uniq = self.current;

        let from = get("from");
        let to = get("to");
        let index = get("index");
        let item = get("item");

        // 1) Index loop: from / to / index (+ optional step).
        if let (Some(from), Some(to), Some(index)) = (from.clone(), to.clone(), index.clone()) {
            let idx = var_name(&index).unwrap_or_else(|| "i".to_string());
            let step = get("step").unwrap_or_else(|| int(1));
            // Direction chosen from the sign of step (Lucee): when step < 0 the
            // terminating test flips to `idx >= to`.
            let cond = Expression::Ternary(Box::new(Ternary {
                condition: Box::new(bin(step.clone(), BinaryOpType::Less, int(0))),
                then_expr: Box::new(bin(ident(&idx), BinaryOpType::GreaterEqual, to.clone())),
                else_expr: Box::new(bin(ident(&idx), BinaryOpType::LessEqual, to.clone())),
                location: loc,
            }));
            let init = Statement::Var(Var { name: idx.clone(), value: Some(from), location: loc });
            let increment = bin(ident(&idx), BinaryOpType::Add, step);
            return Ok(CfmlNode::Statement(Statement::For(For {
                init: Some(Box::new(init)),
                condition: Some(cond),
                increment: Some(Box::new(bin(ident(&idx), BinaryOpType::Assign, increment))),
                body,
                location: loc,
            })));
        }

        // 2) times=N — repeat the body N times.
        if let Some(times) = get("times") {
            let counter = format!("__cfloop_times_{}", uniq);
            let init = Statement::Var(Var { name: counter.clone(), value: Some(int(1)), location: loc });
            let cond = bin(ident(&counter), BinaryOpType::LessEqual, times);
            let increment = bin(ident(&counter), BinaryOpType::Assign, bin(ident(&counter), BinaryOpType::Add, int(1)));
            return Ok(CfmlNode::Statement(Statement::For(For {
                init: Some(Box::new(init)),
                condition: Some(cond),
                increment: Some(Box::new(increment)),
                body,
                location: loc,
            })));
        }

        // 3) condition="…" — while loop. The condition attribute is an
        //    *expression written as a string* (`condition="n LT 3"`), so its
        //    contents must be reparsed as code rather than used as a literal
        //    string value (which would be permanently truthy → infinite loop).
        if let Some(cond) = get("condition") {
            let condition = match &cond {
                Expression::Literal(Literal { value: LiteralValue::String(s), .. }) => {
                    Self::reparse_expr_string(s).unwrap_or(cond)
                }
                _ => cond,
            };
            return Ok(CfmlNode::Statement(Statement::While(While {
                condition,
                body,
                location: loc,
            })));
        }

        // 4) array=… — iterate elements (with optional 1-based index).
        if let Some(array) = get("array") {
            // item + index: bind item=element, index=1-based position.
            if let (Some(item), Some(index)) = (item.clone(), index.clone()) {
                let item_n = var_name(&item).unwrap_or_else(|| "item".to_string());
                let idx_n = var_name(&index).unwrap_or_else(|| "index".to_string());
                let arr_tmp = format!("__cfloop_array_{}", uniq);
                let mut for_body = vec![assign(item_n, Expression::ArrayAccess(Box::new(ArrayAccess {
                    array: Box::new(ident(&arr_tmp)),
                    index: Box::new(ident(&idx_n)),
                    location: loc,
                })))];
                for_body.extend(body);
                let init = Statement::Var(Var { name: idx_n.clone(), value: Some(int(1)), location: loc });
                let cond = bin(ident(&idx_n), BinaryOpType::LessEqual, call("arrayLen", vec![ident(&arr_tmp)]));
                let increment = bin(ident(&idx_n), BinaryOpType::Assign, bin(ident(&idx_n), BinaryOpType::Add, int(1)));
                // Hoist the array into a temp so it is evaluated once.
                return Ok(self.wrap_with_preamble(
                    vec![assign(arr_tmp.clone(), array)],
                    Statement::For(For {
                        init: Some(Box::new(init)),
                        condition: Some(cond),
                        increment: Some(Box::new(increment)),
                        body: for_body,
                        location: loc,
                    }),
                    loc,
                ));
            }
            // item (or index) alone names the element binding.
            if let Some(binding) = item.clone().or(index.clone()) {
                let name = var_name(&binding).unwrap_or_else(|| "item".to_string());
                return Ok(CfmlNode::Statement(Statement::ForIn(ForIn {
                    variable: name,
                    iterable: array,
                    body,
                    location: loc,
                })));
            }
        }

        // 5) list="a,b,c" item|index="x" (+ optional delimiters).
        if let Some(list) = get("list") {
            if let Some(binding) = index.clone().or(item.clone()) {
                let name = var_name(&binding).unwrap_or_else(|| "item".to_string());
                let delims = get("delimiters").unwrap_or_else(|| {
                    Expression::Literal(Literal { value: LiteralValue::String(",".to_string()), location: loc })
                });
                return Ok(CfmlNode::Statement(Statement::ForIn(ForIn {
                    variable: name,
                    iterable: call("listToArray", vec![list, delims]),
                    body,
                    location: loc,
                })));
            }
        }

        // 6) collection=… (struct iteration). item names the value; key/index
        //    names the key.
        if let Some(collection) = get("collection") {
            let item_n = item.as_ref().and_then(|e| var_name(e)).unwrap_or_else(|| "item".to_string());
            let key = get("key").or(index.clone());
            if let Some(key) = key {
                let key_n = var_name(&key).unwrap_or_else(|| "key".to_string());
                let coll_tmp = format!("__cfloop_coll_{}", uniq);
                let mut for_body = vec![assign(item_n, Expression::ArrayAccess(Box::new(ArrayAccess {
                    array: Box::new(ident(&coll_tmp)),
                    index: Box::new(ident(&key_n)),
                    location: loc,
                })))];
                for_body.extend(body);
                return Ok(self.wrap_with_preamble(
                    vec![assign(coll_tmp.clone(), collection.clone())],
                    Statement::ForIn(ForIn {
                        variable: key_n,
                        iterable: call("structKeyArray", vec![ident(&coll_tmp)]),
                        body: for_body,
                        location: loc,
                    }),
                    loc,
                ));
            }
            return Ok(CfmlNode::Statement(Statement::ForIn(ForIn {
                variable: item_n,
                iterable: collection,
                body,
                location: loc,
            })));
        }

        // 7) query=q (+ optional index|item). Without a binding, reassign the
        //    query variable to the current row each iteration so `q.col` works.
        if let Some(query) = get("query") {
            if let Some(binding) = index.clone().or(item.clone()) {
                let name = var_name(&binding).unwrap_or_else(|| "row".to_string());
                return Ok(CfmlNode::Statement(Statement::ForIn(ForIn {
                    variable: name,
                    iterable: query,
                    body,
                    location: loc,
                })));
            }
            let row_tmp = format!("__cfloop_qrow_{}", uniq);
            let mut for_body = Vec::new();
            // `q = __qrow;` only when the query is a plain variable (an lvalue).
            if let Expression::Identifier(id) = &query {
                for_body.push(assign(id.name.clone(), ident(&row_tmp)));
            }
            for_body.extend(body);
            return Ok(CfmlNode::Statement(Statement::ForIn(ForIn {
                variable: row_tmp,
                iterable: query,
                body: for_body,
                location: loc,
            })));
        }

        // 8) No recognised attributes — infinite loop.
        Ok(CfmlNode::Statement(Statement::While(While {
            condition: Expression::Literal(Literal { value: LiteralValue::Bool(true), location: loc }),
            body,
            location: loc,
        })))
    }

    /// Reparse a string's contents as a standalone expression (used for the
    /// `loop condition="…"` attribute, whose value is CFML code written as a
    /// string). Returns None if the string isn't a valid expression.
    fn reparse_expr_string(src: &str) -> Option<Expression> {
        let mut sub = Parser::new(src.to_string());
        sub.parse_expression().ok()
    }

    /// Emit a synthesized loop that needs preamble statements (e.g. hoisting an
    /// iterable into a temp) by wrapping the preamble + loop in a zero-iteration
    /// scaffold. We have no `Block` statement node, so the preamble is run once
    /// via a `for(init; once; ){ … }`-free construct: prepend the preamble to a
    /// single-iteration wrapper. Simpler: use a `for` with the preamble folded
    /// into a leading `if(true){}` is not available either — instead we return
    /// the loop wrapped so the preamble executes first.
    fn wrap_with_preamble(
        &self,
        preamble: Vec<Statement>,
        loop_stmt: Statement,
        loc: SourceLocation,
    ) -> CfmlNode {
        // Run the preamble once, then the loop, by nesting both inside a
        // single-pass `for` whose body is [preamble…, loop]. A one-shot
        // `for(var __once=0; __once<1; __once=__once+1)` executes its body
        // exactly once and introduces no observable variable beyond the temp.
        let once = format!("__cfloop_once_{}", self.current);
        let mut body = preamble;
        body.push(loop_stmt);
        let init = Statement::Var(Var {
            name: once.clone(),
            value: Some(Expression::Literal(Literal { value: LiteralValue::Int(0), location: loc })),
            location: loc,
        });
        let cond = Expression::BinaryOp(Box::new(BinaryOp {
            left: Box::new(Expression::Identifier(Identifier { name: once.clone(), location: loc })),
            operator: BinaryOpType::Less,
            right: Box::new(Expression::Literal(Literal { value: LiteralValue::Int(1), location: loc })),
            location: loc,
        }));
        let inc = Expression::BinaryOp(Box::new(BinaryOp {
            left: Box::new(Expression::Identifier(Identifier { name: once.clone(), location: loc })),
            operator: BinaryOpType::Assign,
            right: Box::new(Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(Expression::Identifier(Identifier { name: once.clone(), location: loc })),
                operator: BinaryOpType::Add,
                right: Box::new(Expression::Literal(Literal { value: LiteralValue::Int(1), location: loc })),
                location: loc,
            }))),
            location: loc,
        }));
        CfmlNode::Statement(Statement::For(For {
            init: Some(Box::new(init)),
            condition: Some(cond),
            increment: Some(Box::new(inc)),
            body,
            location: loc,
        }))
    }

    fn parse_while(&mut self) -> Result<While, ParseError> {
        let loc = self.current_location();
        self.consume(&Token::LParen)?;
        let condition = self.parse_expression()?;
        self.consume(&Token::RParen)?;

        let body = if self.check(&Token::LBrace) {
            self.parse_block()?
        } else {
            let stmt = self.parse_statement()?;
            if let CfmlNode::Statement(s) = stmt {
                vec![s]
            } else {
                Vec::new()
            }
        };

        Ok(While {
            condition,
            body,
            location: loc,
        })
    }

    fn parse_do(&mut self) -> Result<Do, ParseError> {
        let loc = self.current_location();
        let body = self.parse_block()?;
        self.consume(&Token::While)?;
        self.consume(&Token::LParen)?;
        let condition = self.parse_expression()?;
        self.consume(&Token::RParen)?;
        self.match_token(&Token::Semicolon);

        Ok(Do {
            body,
            condition,
            location: loc,
        })
    }

    fn parse_switch(&mut self) -> Result<Switch, ParseError> {
        let loc = self.current_location();
        self.consume(&Token::LParen)?;
        let expression = self.parse_expression()?;
        self.consume(&Token::RParen)?;
        self.consume(&Token::LBrace)?;

        let mut cases = Vec::new();
        let mut default_case = None;

        while !self.check(&Token::RBrace) && !self.is_at_end() {
            if self.match_token(&Token::Case) {
                let mut values = vec![self.parse_expression()?];
                while self.match_token(&Token::Comma) {
                    values.push(self.parse_expression()?);
                }
                self.consume(&Token::Colon)?;

                let mut body = Vec::new();
                while !self.check(&Token::Case)
                    && !self.check(&Token::Default)
                    && !self.check(&Token::RBrace)
                    && !self.is_at_end()
                {
                    // A case body may be wrapped in braces (`case "x": { … }`).
                    // A bare `{` at statement position would otherwise be misread
                    // as a struct literal, so treat it as a block and flatten its
                    // statements (CFML blocks introduce no scope of their own).
                    if self.check(&Token::LBrace) {
                        body.extend(self.parse_block()?);
                    } else {
                        let node = self.parse_statement()?;
                        if let CfmlNode::Statement(s) = node {
                            body.push(s);
                        }
                    }
                }

                cases.push(SwitchCase { values, body });
            } else if self.match_token(&Token::Default) {
                self.consume(&Token::Colon)?;

                let mut body = Vec::new();
                while !self.check(&Token::Case)
                    && !self.check(&Token::RBrace)
                    && !self.is_at_end()
                {
                    // See the `case` body above: a braced default body would be
                    // misread as a struct literal, so flatten a leading block.
                    if self.check(&Token::LBrace) {
                        body.extend(self.parse_block()?);
                    } else {
                        let node = self.parse_statement()?;
                        if let CfmlNode::Statement(s) = node {
                            body.push(s);
                        }
                    }
                }

                default_case = Some(body);
            } else {
                self.advance(); // skip unknown token
            }
        }

        self.consume(&Token::RBrace)?;

        Ok(Switch {
            expression,
            cases,
            default_case,
            location: loc,
        })
    }

    fn parse_try(&mut self) -> Result<Try, ParseError> {
        let loc = self.current_location();
        let body = self.parse_block()?;
        let mut catches = Vec::new();
        let mut finally_body = None;

        while self.match_token(&Token::Catch) {
            self.consume(&Token::LParen)?;

            // catch (type varname) or catch (varname) or catch (any e)
            // The exception type may be a bare (optionally dotted) identifier
            // (`catch (FW1.AbortControllerException e)`) OR a quoted string literal
            // (`catch ("My.Custom.Type" e)`) — the idiomatic way to name a dotted,
            // namespaced custom exception. Lucee/Adobe CF/BoxLang accept both forms.
            let first = if let Token::String(s) = self.peek(0).clone() {
                self.advance();
                s
            } else {
                // A dotted exception type may contain reserved words as segments
                // (`cfflow.workflow.does.not.exist`) — Lucee/ACF/BoxLang allow it.
                // extract_dotted_identifier extracts each `.segment` via
                // extract_property_name, which accepts the full reserved-word set
                // (not/eq/is/…). Within a catch head a `.` always continues the
                // type (the variable name is whitespace-separated, never dotted).
                self.extract_dotted_identifier()?
            };

            let (var_type, var_name) = if self.check(&Token::RParen) {
                (None, first)
            } else {
                let name = self.extract_identifier()?;
                (Some(first), name)
            };

            self.consume(&Token::RParen)?;
            let catch_body = self.parse_block()?;

            catches.push(Catch {
                var_type,
                var_name,
                body: catch_body,
            });
        }

        if self.match_token(&Token::Finally) {
            finally_body = Some(self.parse_block()?);
        }

        Ok(Try {
            body,
            catches,
            finally_body,
            location: loc,
        })
    }

    /// Parse cfscript lock block: lock name="x" type="exclusive" timeout="5" { body }
    /// Desugars to: __cflock_start({name:"x", type:"exclusive", timeout:5}); try { body } finally { __cflock_end("x"); }
    /// Lower a `param` with a (possibly dotted) variable name + default value into
    ///   if ( !isDefined("var.name") ) { var.name = default; }
    /// Dotted names build a MemberAccess chain with a final StructAccess target so
    /// the mutation flows through the normal struct-assignment codegen (which
    /// propagates Arc mutations back to the caller's locals, exactly like
    /// `arguments.obj.foo = bar`). Shared by both the `name="..."` attribute form
    /// and the `param a.b.c = default` shorthand.
    fn lower_param_default(
        &self,
        var_name: &str,
        default_val: Expression,
        loc: SourceLocation,
    ) -> CfmlNode {
        let condition = Expression::FunctionCall(Box::new(FunctionCall {
            name: Box::new(Expression::Identifier(Identifier {
                name: "isDefined".to_string(),
                location: loc,
            })),
            arguments: vec![Expression::Literal(Literal {
                value: LiteralValue::String(var_name.to_string()),
                location: loc,
            })],
            location: loc,
        }));

        let assign_stmt = if let Some(dot_pos) = var_name.find('.') {
            let root = var_name[..dot_pos].to_string();
            let rest = &var_name[dot_pos + 1..];
            let parts: Vec<&str> = rest.split('.').collect();
            let mut expr = Expression::Identifier(Identifier {
                name: root.clone(),
                location: loc,
            });
            for (i, part) in parts.iter().enumerate() {
                if i < parts.len() - 1 {
                    expr = Expression::MemberAccess(Box::new(MemberAccess {
                        object: Box::new(expr),
                        member: part.to_string(),
                        null_safe: false,
                        location: loc,
                    }));
                }
            }
            let last_part = parts.last().unwrap().to_string();
            Statement::Assignment(Assignment {
                target: if parts.len() == 1 {
                    AssignTarget::StructAccess(
                        Box::new(Expression::Identifier(Identifier {
                            name: root,
                            location: loc,
                        })),
                        last_part,
                    )
                } else {
                    AssignTarget::StructAccess(Box::new(expr), last_part)
                },
                value: default_val,
                operator: AssignOp::Equal,
                location: loc,
            })
        } else {
            Statement::Assignment(Assignment {
                target: AssignTarget::Variable(var_name.to_string()),
                value: default_val,
                operator: AssignOp::Equal,
                location: loc,
            })
        };

        CfmlNode::Statement(Statement::If(If {
            condition: Expression::UnaryOp(Box::new(UnaryOp {
                operator: UnaryOpType::Not,
                operand: Box::new(condition),
                location: loc,
            })),
            then_branch: vec![assign_stmt],
            else_if: vec![],
            else_branch: None,
            location: loc,
        }))
    }

    /// Parse cfscript `param` statement:
    ///   param name="varName" default="value" type="string";
    ///   param varName = defaultValue;
    ///   param a.b.c = defaultValue;   (dotted lvalue shorthand)
    /// Converts to: if (!isDefined("varName")) varName = defaultValue;
    fn parse_param_statement(&mut self, loc: SourceLocation, in_parens: bool) -> Result<CfmlNode, ParseError> {
        // Check if it's the named-attribute form: param name="..." default="..."
        let is_named_form = matches!(self.peek(0), Token::Identifier(ref s) if s.to_lowercase() == "name")
            && matches!(self.peek(1), Token::Equal);

        if is_named_form {
            // Parse name=value attributes and emit __cfparam(name, default) call
            let mut name_expr: Option<Expression> = None;
            let mut default_expr: Option<Expression> = None;
            let mut type_name: Option<String> = None;
            let mut min_expr: Option<Expression> = None;
            let mut max_expr: Option<Expression> = None;
            let mut pattern_expr: Option<Expression> = None;
            while (self.is_identifier_like() || matches!(self.peek(0), Token::Identifier(_)))
                && matches!(self.peek(1), Token::Equal) {
                let attr_name = self.extract_identifier()?.to_lowercase();
                self.advance(); // consume =
                let attr_value = self.parse_expression()?;
                match attr_name.as_str() {
                    "name" => name_expr = Some(attr_value),
                    "default" => default_expr = Some(attr_value),
                    // A literal `type="numeric"` drives validation; a non-literal
                    // type expression is left unvalidated (rare).
                    "type" => {
                        if let Expression::Literal(Literal {
                            value: LiteralValue::String(s),
                            ..
                        }) = &attr_value
                        {
                            type_name = Some(s.clone());
                        }
                    }
                    "min" => min_expr = Some(attr_value),
                    "max" => max_expr = Some(attr_value),
                    "pattern" => pattern_expr = Some(attr_value),
                    _ => {}
                }
                // Paren call form `cfparam(name=…, default=…)` separates
                // attributes with commas; the statement form does not.
                if in_parens {
                    self.match_token(&Token::Comma);
                }
            }
            if in_parens {
                self.consume(&Token::RParen)?;
            }
            self.match_token(&Token::Semicolon);

            let name_val = name_expr.unwrap_or(Expression::Literal(Literal {
                value: LiteralValue::String(String::new()),
                location: loc,
            }));

            // For simple string literal names, try to do compile-time expansion
            if let Expression::Literal(ref lit) = name_val {
                if let LiteralValue::String(ref var_name) = lit.value {
                    if !var_name.is_empty() {
                        let default_val = default_expr.unwrap_or(Expression::Literal(Literal {
                            value: LiteralValue::String(String::new()),
                            location: loc,
                        }));
                        let base = self.lower_param_default(var_name, default_val, loc);
                        return Ok(self.append_param_validation(
                            base,
                            var_name,
                            type_name.as_deref(),
                            min_expr,
                            max_expr,
                            pattern_expr,
                            loc,
                        ));
                    }
                }
            }

            let default_val = default_expr.unwrap_or(Expression::Literal(Literal {
                value: LiteralValue::String(String::new()),
                location: loc,
            }));

            // Narrow lowering: recognise the common pattern
            //   param name="a.b.c['#expr#']" default=...;
            // and emit a structKeyExists guard + bracket assignment so the
            // mutation flows through normal codegen (which propagates Arc
            // mutations back to the caller's locals — same path as
            // `arguments.obj.foo = bar`). Falls through to the generic
            // __cfparam dispatch for anything more exotic.
            if let Expression::StringInterpolation(ref interp) = name_val {
                if let Some(stmt) =
                    try_lower_dynamic_param(interp, &default_val, loc)
                {
                    return Ok(CfmlNode::Statement(stmt));
                }
            }

            // Dynamic name (e.g., string interpolation) — emit __cfparam(nameExpr, defaultExpr)
            let call = Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "__cfparam".to_string(),
                    location: loc,
                })),
                arguments: vec![name_val, default_val],
                location: loc,
            }));
            return Ok(CfmlNode::Statement(Statement::Expression(ExpressionStatement {
                expr: call,
                location: loc,
            })));
        }

        // Shorthand form: param varName = defaultValue;
        //   or typed:      param type varName = defaultValue;
        //   or dotted:     param arguments.obj.key = defaultValue;
        //
        // A leading type is present only when TWO identifier-like tokens appear
        // back to back (`param numeric x`). For a dotted lvalue (`param a.b`)
        // peek(1) is a Dot, so the first token is the variable, not a type —
        // this is why the old `peek(1) != Equal|Semicolon` heuristic wrongly
        // consumed `arguments` as a type and then choked on the `.`.
        let mut type_name: Option<String> = None;
        if self.is_identifier_like_at(0) && self.is_identifier_like_at(1) {
            type_name = Some(self.extract_identifier()?); // the declared type
        }

        // Parse the (possibly dotted) lvalue path into a string, e.g.
        // "arguments.target.$wbDelegateMap". Segments after a dot use
        // extract_property_name so keywords / $-prefixed names are accepted.
        let mut var_name = self.extract_identifier()?;
        while self.match_token(&Token::Dot) {
            var_name.push('.');
            var_name.push_str(&self.extract_property_name()?);
        }

        let default_value = if self.match_token(&Token::Equal) {
            self.parse_expression()?
        } else {
            Expression::Literal(Literal {
                value: LiteralValue::String(String::new()),
                location: loc,
            })
        };
        self.match_token(&Token::Semicolon);

        let base = self.lower_param_default(&var_name, default_value, loc);
        Ok(self.append_param_validation(base, &var_name, type_name.as_deref(), None, None, None, loc))
    }

    /// Wrap a lowered `param` default-assignment so the resulting value is
    /// validated against `type` (with optional `min`/`max`/`pattern`). Emits
    /// `if (true) { <default-assignment>; __cfparam_validate(<path>, ...); }`
    /// — there is no block statement, so a `true` If supplies the statement
    /// list. A missing / `any` type is a no-op (returns `base` unchanged).
    #[allow(clippy::too_many_arguments)]
    fn append_param_validation(
        &self,
        base: CfmlNode,
        var_name: &str,
        type_name: Option<&str>,
        min_expr: Option<Expression>,
        max_expr: Option<Expression>,
        pattern_expr: Option<Expression>,
        loc: SourceLocation,
    ) -> CfmlNode {
        let ty = match type_name {
            Some(t) if !t.is_empty() && !t.eq_ignore_ascii_case("any") => t.to_lowercase(),
            _ => return base,
        };
        let base_stmt = match base {
            CfmlNode::Statement(s) => s,
            other => return other,
        };
        // Build a (possibly dotted) lvalue expression for the variable so the
        // validator receives its current value.
        let mut parts = var_name.split('.');
        let mut value_expr = Expression::Identifier(Identifier {
            name: parts.next().unwrap_or("").to_string(),
            location: loc,
        });
        for part in parts {
            value_expr = Expression::MemberAccess(Box::new(MemberAccess {
                object: Box::new(value_expr),
                member: part.to_string(),
                null_safe: false,
                location: loc,
            }));
        }
        let str_lit = |s: &str| {
            Expression::Literal(Literal {
                value: LiteralValue::String(s.to_string()),
                location: loc,
            })
        };
        let empty = || str_lit("");
        let validate_call = Expression::FunctionCall(Box::new(FunctionCall {
            name: Box::new(Expression::Identifier(Identifier {
                name: "__cfparam_validate".to_string(),
                location: loc,
            })),
            arguments: vec![
                value_expr,
                str_lit(&ty),
                str_lit(var_name),
                min_expr.unwrap_or_else(empty),
                max_expr.unwrap_or_else(empty),
                pattern_expr.unwrap_or_else(empty),
            ],
            location: loc,
        }));
        let validate_stmt = Statement::Expression(ExpressionStatement {
            expr: validate_call,
            location: loc,
        });
        CfmlNode::Statement(Statement::If(If {
            condition: Expression::Literal(Literal {
                value: LiteralValue::Bool(true),
                location: loc,
            }),
            then_branch: vec![base_stmt, validate_stmt],
            else_if: vec![],
            else_branch: None,
            location: loc,
        }))
    }

    /// Parse a comma-separated `key = expr` list inside the parens of a
    /// script-call-with-body tag invocation, e.g. `variable="x", timeout=10`.
    /// Caller has already consumed the opening `(`; this stops at (but does
    /// not consume) the matching `)`.
    fn parse_tag_call_attrs(&mut self) -> Result<Vec<(String, Expression)>, ParseError> {
        let mut attrs = Vec::new();
        while !self.check(&Token::RParen) && !self.is_at_end() {
            if !(self.is_identifier_like() && matches!(self.peek(1), Token::Equal)) {
                return Err(ParseError {
                    message: "expected key=value in tag call attributes".to_string(),
                    line: 0, column: 0,
                });
            }
            let key = self.extract_identifier()?;
            self.consume(&Token::Equal)?;
            let value = self.parse_expression()?;
            attrs.push((key, value));
            if !self.match_token(&Token::Comma) {
                break;
            }
        }
        Ok(attrs)
    }

    /// Build an assignment target from a (possibly scoped/dotted) name —
    /// "q" → Variable, "local.q"/"request.data.q" → a StructAccess chain — so
    /// the write flows through normal struct-assignment codegen. Mirrors the
    /// dotted-name handling in `lower_param_default`.
    fn assign_target_from_dotted(&self, var_name: &str, loc: SourceLocation) -> AssignTarget {
        match var_name.find('.') {
            None => AssignTarget::Variable(var_name.to_string()),
            Some(dot_pos) => {
                let root = var_name[..dot_pos].to_string();
                let rest = &var_name[dot_pos + 1..];
                let parts: Vec<&str> = rest.split('.').collect();
                let mut expr = Expression::Identifier(Identifier { name: root.clone(), location: loc });
                for part in parts.iter().take(parts.len() - 1) {
                    expr = Expression::MemberAccess(Box::new(MemberAccess {
                        object: Box::new(expr),
                        member: part.to_string(),
                        null_safe: false,
                        location: loc,
                    }));
                }
                let last_part = parts.last().unwrap().to_string();
                if parts.len() == 1 {
                    AssignTarget::StructAccess(
                        Box::new(Expression::Identifier(Identifier { name: root, location: loc })),
                        last_part,
                    )
                } else {
                    AssignTarget::StructAccess(Box::new(expr), last_part)
                }
            }
        }
    }

    /// If `expr` is a call to `cfqueryparam(...)` (script-statement form),
    /// return the param-struct expression to bind. The attributeCollection
    /// form (`cfqueryparam(attributeCollection = qp)`) yields the struct
    /// itself (it already carries value/cfsqltype/null/list/…); otherwise a
    /// struct is built from the args — named args keep their name, and a bare
    /// positional maps to `value` then `cfsqltype` (cfqueryparam's attribute
    /// order). Returns None for any other expression.
    fn cfqueryparam_call_struct(&self, expr: &Expression) -> Option<Expression> {
        let Expression::FunctionCall(fc) = expr else { return None; };
        let Expression::Identifier(id) = fc.name.as_ref() else { return None; };
        if !id.name.eq_ignore_ascii_case("cfqueryparam") {
            return None;
        }
        let loc = fc.location;
        if fc.arguments.len() == 1 {
            if let Expression::NamedArgument(na) = &fc.arguments[0] {
                if na.name.eq_ignore_ascii_case("attributecollection") {
                    return Some((*na.value).clone());
                }
            }
        }
        let mut pairs: Vec<(Expression, Expression)> = Vec::with_capacity(fc.arguments.len());
        let mut positional = 0;
        for arg in &fc.arguments {
            let (key, value) = match arg {
                Expression::NamedArgument(na) => (na.name.clone(), (*na.value).clone()),
                other => {
                    let k = if positional == 0 { "value" } else { "cfsqltype" };
                    positional += 1;
                    (k.to_string(), other.clone())
                }
            };
            pairs.push((
                Expression::Literal(Literal {
                    value: LiteralValue::String(key),
                    location: loc,
                }),
                value,
            ));
        }
        Some(Expression::Struct(Struct {
            pairs,
            ordered: false,
            location: loc,
        }))
    }

    /// Rewrite `cfqueryparam(...)` script statements appearing anywhere in a
    /// script `cfquery(){}` body into the same
    /// `writeOutput("?"); arrayAppend(__cfquery_params, <param-struct>);`
    /// shape the `<cfqueryparam>` TAG lowers to (see tag_parser.rs). Recurses
    /// through control flow so a cfqueryparam inside a loop/if — the Wheels
    /// databaseAdapters/Base.cfc bind-loop shape — is lowered too. Without
    /// this the call site was an undefined identifier and threw
    /// `Variable 'cfqueryparam' is undefined`.
    fn rewrite_cfqueryparam_stmts(&self, stmts: Vec<Statement>) -> Vec<Statement> {
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            match stmt {
                Statement::Expression(es) => {
                    if let Some(param_expr) = self.cfqueryparam_call_struct(&es.expr) {
                        let loc = es.location;
                        let call = |name: &str, arguments: Vec<Expression>| {
                            Expression::FunctionCall(Box::new(FunctionCall {
                                name: Box::new(Expression::Identifier(Identifier {
                                    name: name.to_string(),
                                    location: loc,
                                })),
                                arguments,
                                location: loc,
                            }))
                        };
                        out.push(Statement::Expression(ExpressionStatement {
                            expr: call(
                                "writeOutput",
                                vec![Expression::Literal(Literal {
                                    value: LiteralValue::String("?".to_string()),
                                    location: loc,
                                })],
                            ),
                            location: loc,
                        }));
                        out.push(Statement::Expression(ExpressionStatement {
                            expr: call(
                                "arrayAppend",
                                vec![
                                    Expression::Identifier(Identifier {
                                        name: "__cfquery_params".to_string(),
                                        location: loc,
                                    }),
                                    param_expr,
                                ],
                            ),
                            location: loc,
                        }));
                    } else {
                        out.push(Statement::Expression(es));
                    }
                }
                Statement::If(mut i) => {
                    i.then_branch = self.rewrite_cfqueryparam_stmts(i.then_branch);
                    i.else_if = i
                        .else_if
                        .into_iter()
                        .map(|mut e| {
                            e.body = self.rewrite_cfqueryparam_stmts(e.body);
                            e
                        })
                        .collect();
                    i.else_branch = i.else_branch.map(|b| self.rewrite_cfqueryparam_stmts(b));
                    out.push(Statement::If(i));
                }
                Statement::For(mut f) => {
                    f.body = self.rewrite_cfqueryparam_stmts(f.body);
                    out.push(Statement::For(f));
                }
                Statement::ForIn(mut f) => {
                    f.body = self.rewrite_cfqueryparam_stmts(f.body);
                    out.push(Statement::ForIn(f));
                }
                Statement::While(mut w) => {
                    w.body = self.rewrite_cfqueryparam_stmts(w.body);
                    out.push(Statement::While(w));
                }
                Statement::Do(mut d) => {
                    d.body = self.rewrite_cfqueryparam_stmts(d.body);
                    out.push(Statement::Do(d));
                }
                Statement::Switch(mut s) => {
                    s.cases = s
                        .cases
                        .into_iter()
                        .map(|mut c| {
                            c.body = self.rewrite_cfqueryparam_stmts(c.body);
                            c
                        })
                        .collect();
                    s.default_case =
                        s.default_case.map(|b| self.rewrite_cfqueryparam_stmts(b));
                    out.push(Statement::Switch(s));
                }
                Statement::Try(mut t) => {
                    t.body = self.rewrite_cfqueryparam_stmts(t.body);
                    t.catches = t
                        .catches
                        .into_iter()
                        .map(|mut c| {
                            c.body = self.rewrite_cfqueryparam_stmts(c.body);
                            c
                        })
                        .collect();
                    t.finally_body =
                        t.finally_body.map(|b| self.rewrite_cfqueryparam_stmts(b));
                    out.push(Statement::Try(t));
                }
                Statement::Output(mut o) => {
                    o.body = self.rewrite_cfqueryparam_stmts(o.body);
                    out.push(Statement::Output(o));
                }
                other => out.push(other),
            }
        }
        out
    }

    /// Lower a script-call-with-body form (`cfsavecontent(...) { ... }`,
    /// `cflock(...) { ... }`, `cftransaction(...) { ... }`, `cfmail(...) { ... }`)
    /// into the same AST shape the angle-bracket tag forms produce.
    fn lower_script_body_tag(
        &self,
        tag: &str,
        attrs: Vec<(String, Expression)>,
        body: Vec<Statement>,
        loc: SourceLocation,
    ) -> CfmlNode {
        match tag {
            "cfsavecontent" => {
                // The variable= target may be unqualified ("cap") or scope-qualified
                // ("local.cap"); lower it to a proper AssignTarget so the capture
                // lands in the right scope rather than being silently discarded.
                let var_target = attrs.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("variable"))
                    .map(|(_, v)| match v {
                        Expression::Literal(Literal { value: LiteralValue::String(s), .. }) => {
                            self.assign_target_from_dotted(s, loc)
                        }
                        other => self.expression_to_assign_target(other)
                            .unwrap_or_else(|_| AssignTarget::Variable("__savecontent_result".to_string())),
                    })
                    .unwrap_or_else(|| AssignTarget::Variable("__savecontent_result".to_string()));
                let mut stmts = vec![Statement::Expression(ExpressionStatement {
                    expr: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cfsavecontent_start".to_string(),
                            location: loc,
                        })),
                        arguments: vec![],
                        location: loc,
                    })),
                    location: loc,
                })];
                stmts.extend(body);
                stmts.push(Statement::Assignment(Assignment {
                    target: var_target,
                    value: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cfsavecontent_end".to_string(),
                            location: loc,
                        })),
                        arguments: vec![],
                        location: loc,
                    })),
                    operator: AssignOp::Equal,
                    location: loc,
                }));
                CfmlNode::Statement(Statement::Output(Output { body: stmts, location: loc }))
            }
            "cfquery" => {
                // Script cfquery(name="q", datasource="ds", ...) { body } mirrors
                // the dynamic angle-bracket <cfquery> path: the body runs inside a
                // savecontent capture (so its output builds the SQL string) while
                // any cfqueryparam appends to __cfquery_params, then queryExecute
                // runs the captured SQL. Without this, the trailing `{ ... }` was
                // mis-parsed as a struct literal (issue #68).
                // Options struct: EVERY attribute (name, result, datasource,
                // attributeCollection, …) — queryExecute's third argument.
                // The VM intercept expands attributeCollection and delivers
                // `name`/`result` into the caller's scope at runtime (they
                // can be dotted, e.g. Wheels' name="local.query" +
                // result="local.wheels.result", and may only be known at
                // runtime inside an attributeCollection). The __cfquery_tag
                // marker tells the intercept this call came from cfquery, so
                // `name` is honored (direct queryExecute ignores it, like
                // Lucee) and only set when a resultset came back (Lucee
                // leaves `name` untouched for INSERT/UPDATE/DDL).
                let mut pairs: Vec<(Expression, Expression)> = attrs.iter()
                    .map(|(k, v)| {
                        // A bare identifier as the name attr (name=q1) targets
                        // the variable called q1 — normalize to its string.
                        let value = if k.eq_ignore_ascii_case("name") {
                            match v {
                                Expression::Identifier(id) => Expression::Literal(Literal {
                                    value: LiteralValue::String(id.name.clone()),
                                    location: loc,
                                }),
                                other => other.clone(),
                            }
                        } else {
                            v.clone()
                        };
                        (
                            Expression::Literal(Literal {
                                value: LiteralValue::String(k.clone()),
                                location: loc,
                            }),
                            value,
                        )
                    })
                    .collect();
                if !attrs.iter().any(|(k, _)| {
                    k.eq_ignore_ascii_case("name") || k.eq_ignore_ascii_case("attributecollection")
                }) {
                    // Pre-#90 leniency: a nameless script cfquery still
                    // populates `queryResult` (Lucee would require name).
                    pairs.push((
                        Expression::Literal(Literal {
                            value: LiteralValue::String("name".to_string()),
                            location: loc,
                        }),
                        Expression::Literal(Literal {
                            value: LiteralValue::String("queryResult".to_string()),
                            location: loc,
                        }),
                    ));
                }
                pairs.push((
                    Expression::Literal(Literal {
                        value: LiteralValue::String("__cfquery_tag".to_string()),
                        location: loc,
                    }),
                    Expression::Literal(Literal {
                        value: LiteralValue::Bool(true),
                        location: loc,
                    }),
                ));
                let opts_struct = Expression::Struct(Struct {
                    pairs,
                    ordered: false,
                    location: loc,
                });
                let call = |name: &str, arguments: Vec<Expression>| Expression::FunctionCall(Box::new(FunctionCall {
                    name: Box::new(Expression::Identifier(Identifier { name: name.to_string(), location: loc })),
                    arguments,
                    location: loc,
                }));
                let mut stmts = vec![
                    // __cfquery_params = [];
                    Statement::Assignment(Assignment {
                        target: AssignTarget::Variable("__cfquery_params".to_string()),
                        value: Expression::Array(Array { elements: vec![], location: loc }),
                        operator: AssignOp::Equal,
                        location: loc,
                    }),
                    // __cfsavecontent_start();
                    Statement::Expression(ExpressionStatement {
                        expr: call("__cfsavecontent_start", vec![]),
                        location: loc,
                    }),
                ];
                // Lower any script-statement cfqueryparam(...) in the body to
                // writeOutput("?") + arrayAppend(__cfquery_params, …), matching
                // the <cfqueryparam> tag path (recurses through control flow).
                stmts.extend(self.rewrite_cfqueryparam_stmts(body));
                // queryExecute(__cfsavecontent_end(), __cfquery_params, OPTS);
                // — the VM intercept delivers name=/result= from OPTS.
                stmts.push(Statement::Expression(ExpressionStatement {
                    expr: call("queryExecute", vec![
                        call("__cfsavecontent_end", vec![]),
                        Expression::Identifier(Identifier { name: "__cfquery_params".to_string(), location: loc }),
                        opts_struct,
                    ]),
                    location: loc,
                }));
                CfmlNode::Statement(Statement::Output(Output { body: stmts, location: loc }))
            }
            "cfhttp" => {
                // Script cfhttp(url=…, method=…, result=…) { body } mirrors the
                // dynamic angle-bracket <cfhttp> path: the body runs after
                // seeding __cfhttp_params = [], each cfhttpparam(...) appends to
                // it (incl. inside for/if), then cfhttp({...attrs, params}) runs.
                // Without this the trailing `{ ... }` was mis-parsed as a struct
                // literal, dropping params in control flow (issue #55).
                let result_name = attrs.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("result"))
                    .and_then(|(_, v)| match v {
                        Expression::Literal(Literal { value: LiteralValue::String(s), .. }) => Some(s.clone()),
                        Expression::Identifier(id) => Some(id.name.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "cfhttp".to_string());
                // Options struct: every attribute except `result`, plus the
                // runtime-collected params array.
                let mut pairs: Vec<(Expression, Expression)> = attrs.iter()
                    .filter(|(k, _)| !k.eq_ignore_ascii_case("result"))
                    .map(|(k, v)| (
                        Expression::Literal(Literal {
                            value: LiteralValue::String(k.clone()),
                            location: loc,
                        }),
                        v.clone(),
                    ))
                    .collect();
                pairs.push((
                    Expression::Literal(Literal {
                        value: LiteralValue::String("params".to_string()),
                        location: loc,
                    }),
                    Expression::Identifier(Identifier { name: "__cfhttp_params".to_string(), location: loc }),
                ));
                let opts_struct = Expression::Struct(Struct { pairs, ordered: false, location: loc });
                let call = |name: &str, arguments: Vec<Expression>| Expression::FunctionCall(Box::new(FunctionCall {
                    name: Box::new(Expression::Identifier(Identifier { name: name.to_string(), location: loc })),
                    arguments,
                    location: loc,
                }));
                let mut stmts = vec![
                    // __cfhttp_params = [];
                    Statement::Assignment(Assignment {
                        target: AssignTarget::Variable("__cfhttp_params".to_string()),
                        value: Expression::Array(Array { elements: vec![], location: loc }),
                        operator: AssignOp::Equal,
                        location: loc,
                    }),
                ];
                stmts.extend(body);
                // RESULT = cfhttp({ ...attrs, params: __cfhttp_params });
                stmts.push(Statement::Assignment(Assignment {
                    target: self.assign_target_from_dotted(&result_name, loc),
                    value: call("cfhttp", vec![opts_struct]),
                    operator: AssignOp::Equal,
                    location: loc,
                }));
                CfmlNode::Statement(Statement::Output(Output { body: stmts, location: loc }))
            }
            "cflock" => {
                let attrs_struct = Expression::Struct(Struct {
                    pairs: attrs.iter().map(|(k, v)| (
                        Expression::Literal(Literal {
                            value: LiteralValue::String(k.clone()),
                            location: loc,
                        }),
                        v.clone(),
                    )).collect(),
                    ordered: false,
                    location: loc,
                });
                let lock_name_expr = attrs.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("name"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Expression::Literal(Literal {
                        value: LiteralValue::String("default".to_string()),
                        location: loc,
                    }));
                let lock_start = Statement::Expression(ExpressionStatement {
                    expr: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cflock_start".to_string(), location: loc,
                        })),
                        arguments: vec![attrs_struct],
                        location: loc,
                    })),
                    location: loc,
                });
                let lock_end = Statement::Expression(ExpressionStatement {
                    expr: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cflock_end".to_string(), location: loc,
                        })),
                        arguments: vec![lock_name_expr],
                        location: loc,
                    })),
                    location: loc,
                });
                let try_stmt = Statement::Try(Try {
                    body,
                    catches: vec![],
                    finally_body: Some(vec![lock_end]),
                    location: loc,
                });
                CfmlNode::Statement(Statement::Output(Output {
                    body: vec![lock_start, try_stmt],
                    location: loc,
                }))
            }
            "cftransaction" => {
                // __cftransaction_start("begin"); try { body; __cftransaction_commit(); }
                // catch (any e) { __cftransaction_rollback(); throw e; }
                let action_expr = attrs.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("action"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Expression::Literal(Literal {
                        value: LiteralValue::String("begin".to_string()),
                        location: loc,
                    }));
                let start = Statement::Expression(ExpressionStatement {
                    expr: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cftransaction_start".to_string(), location: loc,
                        })),
                        arguments: vec![action_expr],
                        location: loc,
                    })),
                    location: loc,
                });
                let commit = Statement::Expression(ExpressionStatement {
                    expr: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cftransaction_commit".to_string(), location: loc,
                        })),
                        arguments: vec![],
                        location: loc,
                    })),
                    location: loc,
                });
                let rollback = Statement::Expression(ExpressionStatement {
                    expr: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cftransaction_rollback".to_string(), location: loc,
                        })),
                        arguments: vec![],
                        location: loc,
                    })),
                    location: loc,
                });
                let rethrow = Statement::Throw(Throw {
                    message: Some(Expression::Identifier(Identifier {
                        name: "__txn_e".to_string(), location: loc,
                    })),
                    type_: None,
                    location: loc,
                });
                let mut try_body = body;
                try_body.push(commit);
                let try_stmt = Statement::Try(Try {
                    body: try_body,
                    catches: vec![Catch {
                        var_type: Some("any".to_string()),
                        var_name: "__txn_e".to_string(),
                        body: vec![rollback, rethrow],
                    }],
                    finally_body: None,
                    location: loc,
                });
                CfmlNode::Statement(Statement::Output(Output {
                    body: vec![start, try_stmt],
                    location: loc,
                }))
            }
            "cfmail" => {
                // Capture the body text via savecontent, then call __cfmail({...attrs, body: captured}).
                let capture_var = "__cfmail_body_capture".to_string();
                let mut stmts: Vec<Statement> = Vec::new();
                // Seed the runtime arrays that script-form cfmailpart/cfmailparam
                // statements in the body append to (mirrors the angle-bracket
                // runtime cfmail path). __cfmail forwards them as parts/params.
                for arr in ["__cfmail_params", "__cfmail_parts"] {
                    stmts.push(Statement::Assignment(Assignment {
                        target: AssignTarget::Variable(arr.to_string()),
                        value: Expression::Array(Array { elements: vec![], location: loc }),
                        operator: AssignOp::Equal,
                        location: loc,
                    }));
                }
                stmts.push(Statement::Expression(ExpressionStatement {
                    expr: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cfsavecontent_start".to_string(), location: loc,
                        })),
                        arguments: vec![],
                        location: loc,
                    })),
                    location: loc,
                }));
                stmts.extend(body);
                stmts.push(Statement::Assignment(Assignment {
                    target: AssignTarget::Variable(capture_var.clone()),
                    value: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cfsavecontent_end".to_string(), location: loc,
                        })),
                        arguments: vec![],
                        location: loc,
                    })),
                    operator: AssignOp::Equal,
                    location: loc,
                }));
                // Build {attrs..., body: __cfmail_body_capture}
                let mut pairs: Vec<(Expression, Expression)> = attrs.iter().map(|(k, v)| (
                    Expression::Literal(Literal {
                        value: LiteralValue::String(k.clone()),
                        location: loc,
                    }),
                    v.clone(),
                )).collect();
                pairs.push((
                    Expression::Literal(Literal {
                        value: LiteralValue::String("body".to_string()),
                        location: loc,
                    }),
                    Expression::Identifier(Identifier {
                        name: capture_var, location: loc,
                    }),
                ));
                for arr in ["params", "parts"] {
                    pairs.push((
                        Expression::Literal(Literal {
                            value: LiteralValue::String(arr.to_string()),
                            location: loc,
                        }),
                        Expression::Identifier(Identifier {
                            name: format!("__cfmail_{}", arr), location: loc,
                        }),
                    ));
                }
                let opts_struct = Expression::Struct(Struct {
                    pairs, ordered: false, location: loc,
                });
                stmts.push(Statement::Expression(ExpressionStatement {
                    expr: Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "__cfmail".to_string(), location: loc,
                        })),
                        arguments: vec![opts_struct],
                        location: loc,
                    })),
                    location: loc,
                }));
                CfmlNode::Statement(Statement::Output(Output { body: stmts, location: loc }))
            }
            _ => unreachable!("unknown body tag: {}", tag),
        }
    }

    /// Build a `{ key: value, ... }` struct literal from parsed tag-call attrs.
    fn attrs_to_struct_literal(
        &self,
        attrs: &[(String, Expression)],
        loc: SourceLocation,
    ) -> Expression {
        Expression::Struct(Struct {
            pairs: attrs.iter().map(|(k, v)| (
                Expression::Literal(Literal {
                    value: LiteralValue::String(k.clone()),
                    location: loc,
                }),
                v.clone(),
            )).collect(),
            ordered: false,
            location: loc,
        })
    }

    /// Build an `arrayAppend(<array_var>, <value>)` statement-expression.
    fn array_append_stmt(
        &self,
        array_var: &str,
        value: Expression,
        loc: SourceLocation,
    ) -> CfmlNode {
        CfmlNode::Statement(Statement::Expression(ExpressionStatement {
            expr: Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "arrayAppend".to_string(),
                    location: loc,
                })),
                arguments: vec![
                    Expression::Identifier(Identifier {
                        name: array_var.to_string(),
                        location: loc,
                    }),
                    value,
                ],
                location: loc,
            })),
            location: loc,
        }))
    }

    /// Lower a script-statement `cfmailpart(...) { body }`: capture the part's
    /// own body via a nested savecontent, then append `{ ...attrs, body }` to
    /// the runtime `__cfmail_parts` array seeded by the surrounding script
    /// `cfmail(){ }` block. Mirrors the tag-form runtime cfmailpart lowering.
    fn lower_script_mailpart(
        &self,
        attrs: Vec<(String, Expression)>,
        body: Vec<Statement>,
        loc: SourceLocation,
    ) -> CfmlNode {
        let mut stmts: Vec<Statement> = Vec::new();
        // __cfsavecontent_start();
        stmts.push(Statement::Expression(ExpressionStatement {
            expr: Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "__cfsavecontent_start".to_string(), location: loc,
                })),
                arguments: vec![],
                location: loc,
            })),
            location: loc,
        }));
        stmts.extend(body);
        // { ...attrs, body: __cfsavecontent_end() } — the end() call is
        // evaluated during struct construction, i.e. after the body output.
        let mut pairs: Vec<(Expression, Expression)> = attrs.iter().map(|(k, v)| (
            Expression::Literal(Literal {
                value: LiteralValue::String(k.clone()),
                location: loc,
            }),
            v.clone(),
        )).collect();
        pairs.push((
            Expression::Literal(Literal {
                value: LiteralValue::String("body".to_string()),
                location: loc,
            }),
            Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "__cfsavecontent_end".to_string(), location: loc,
                })),
                arguments: vec![],
                location: loc,
            })),
        ));
        let part_struct = Expression::Struct(Struct { pairs, ordered: false, location: loc });
        if let CfmlNode::Statement(s) = self.array_append_stmt("__cfmail_parts", part_struct, loc) {
            stmts.push(s);
        }
        CfmlNode::Statement(Statement::Output(Output { body: stmts, location: loc }))
    }

    fn parse_lock(&mut self, loc: SourceLocation) -> Result<CfmlNode, ParseError> {
        // Parse key=value attributes before the block
        let mut attrs: Vec<(String, Expression)> = Vec::new();
        while let Token::Identifier(_) = self.peek(0) {
            if matches!(self.peek(1), Token::Equal) {
                let key = self.extract_identifier()?;
                self.consume(&Token::Equal)?;
                let value = self.parse_expression()?;
                attrs.push((key, value));
            } else {
                break;
            }
        }

        // Parse the block body
        let body = self.parse_block()?;

        // Extract lock name for __cflock_end
        let lock_name_expr = attrs.iter()
            .find(|(k, _)| k.to_lowercase() == "name")
            .map(|(_, v)| v.clone())
            .unwrap_or(Expression::Literal(Literal {
                value: LiteralValue::String("default".to_string()),
                location: loc,
            }));

        // Build struct literal for __cflock_start argument
        let struct_pairs: Vec<(Expression, Expression)> = attrs.iter().map(|(k, v)| {
            (Expression::Literal(Literal {
                value: LiteralValue::String(k.clone()),
                location: loc,
            }), v.clone())
        }).collect();

        let attrs_struct = Expression::Struct(Struct {
            pairs: struct_pairs,
            ordered: false,
            location: loc,
        });

        // __cflock_start(attrs)
        let lock_start = Statement::Expression(ExpressionStatement {
            expr: Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "__cflock_start".to_string(),
                    location: loc,
                })),
                arguments: vec![attrs_struct],
                location: loc,
            })),
            location: loc,
        });

        // __cflock_end(name)
        let lock_end = Statement::Expression(ExpressionStatement {
            expr: Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "__cflock_end".to_string(),
                    location: loc,
                })),
                arguments: vec![lock_name_expr],
                location: loc,
            })),
            location: loc,
        });

        // try { body } finally { __cflock_end(name) }
        let try_stmt = Statement::Try(Try {
            body,
            catches: vec![],
            finally_body: Some(vec![lock_end]),
            location: loc,
        });

        // Wrap as Output block: __cflock_start; try { ... } finally { __cflock_end }
        let output = Statement::Output(Output {
            body: vec![lock_start, try_stmt],
            location: loc,
        });

        Ok(CfmlNode::Statement(output))
    }

    /// Parse `key=value` / `key:value` pairs for a `throw` call. When
    /// `comma_separated` is true (paren form) pairs are separated by commas;
    /// otherwise (bare statement form) they are whitespace-separated and the
    /// list ends at the first token that isn't `identifier =`.
    fn parse_throw_named_attrs(
        &mut self,
        comma_separated: bool,
    ) -> Result<Vec<(String, Expression)>, ParseError> {
        let mut named: Vec<(String, Expression)> = Vec::new();
        loop {
            let key = self.extract_identifier()?.to_lowercase();
            if !self.match_token(&Token::Equal) {
                self.consume(&Token::Colon)?;
            }
            let value = self.parse_expression()?;
            named.push((key, value));
            if comma_separated && !self.match_token(&Token::Comma) {
                break;
            }
            // Continue only while another `identifier =/:` pair follows.
            if !(matches!(self.peek(0), Token::Identifier(_))
                && matches!(self.peek(1), Token::Equal | Token::Colon))
            {
                break;
            }
        }
        Ok(named)
    }

    /// Map named `throw` attributes to the fixed positional order the VM's
    /// throw intercept expects: message, type, detail, errorcode, extendedinfo,
    /// object. Missing attributes become empty strings. (`extendedinfo`/`object`
    /// matter for the re-throw idiom `throw(object=e)` — GitHub issue #158.)
    fn throw_named_to_positional(
        &self,
        named: Vec<(String, Expression)>,
        loc: SourceLocation,
    ) -> Vec<Expression> {
        let get_arg = |name: &str| -> Expression {
            named
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or(Expression::Literal(Literal {
                    value: LiteralValue::String(String::new()),
                    location: loc,
                }))
        };
        vec![
            get_arg("message"),
            get_arg("type"),
            get_arg("detail"),
            get_arg("errorcode"),
            get_arg("extendedinfo"),
            get_arg("object"),
            // Index 6: `attributeCollection` — a runtime struct can't be expanded
            // at parse time, so it rides through as a 7th positional arg and the
            // VM throw intercept seeds the exception from its keys (explicit
            // positional attrs still win). Empty string when absent.
            get_arg("attributecollection"),
        ]
    }

    /// Build a `throw(...)` function-call statement (VM-intercepted).
    fn build_throw_call(&self, arguments: Vec<Expression>, loc: SourceLocation) -> CfmlNode {
        let throw_ident = Expression::Identifier(Identifier {
            name: "throw".to_string(),
            location: loc,
        });
        let call_expr = Expression::FunctionCall(Box::new(FunctionCall {
            name: Box::new(throw_ident),
            arguments,
            location: loc,
        }));
        CfmlNode::Statement(Statement::Expression(ExpressionStatement {
            expr: call_expr,
            location: loc,
        }))
    }

    fn parse_throw(&mut self) -> Result<Throw, ParseError> {
        let loc = self.current_location();
        let message = if !self.check(&Token::Semicolon) && !self.is_at_end() {
            Some(self.parse_expression()?)
        } else {
            None
        };
        self.match_token(&Token::Semicolon);

        Ok(Throw {
            message,
            type_: None,
            location: loc,
        })
    }

    fn parse_return(&mut self) -> Result<Return, ParseError> {
        let loc = self.current_location();
        let value = if !self.check(&Token::Semicolon)
            && !self.check(&Token::RBrace)
            && !self.is_at_end()
        {
            Some(self.parse_expression()?)
        } else {
            None
        };

        self.match_token(&Token::Semicolon);

        Ok(Return {
            value,
            location: loc,
        })
    }

    fn parse_function(&mut self) -> Result<Function, ParseError> {
        let loc = self.current_location();
        // Optional return type before function name
        let mut return_type = None;
        let name;

        // Parse a (possibly dotted) run. A function name is always immediately
        // followed by `(`, so if `(` does not follow the first run, that run was
        // a return type and the real name is the next run. Names accept soft
        // keywords and other keyword identifiers (e.g. a method named `new`);
        // in tag-based CFCs they may also be dotted (`upload.profile_id`).
        let first = self.extract_function_name()?;
        let mut dotted = first;
        while self.match_token(&Token::Dot) {
            dotted.push('.');
            dotted.push_str(&self.extract_function_name()?);
        }

        if self.check(&Token::LParen) {
            name = dotted;
        } else {
            return_type = Some(dotted);
            let mut n = self.extract_function_name()?;
            while self.match_token(&Token::Dot) {
                n.push('.');
                n.push_str(&self.extract_function_name()?);
            }
            name = n;
        }

        self.consume(&Token::LParen)?;
        let params = self.parse_param_list()?;
        self.consume(&Token::RParen)?;

        // Parse function metadata attributes (e.g., httpmethod="GET" restpath="/users",
        // output="false", hint="..."). Accepts both identifiers and keyword tokens as keys.
        let mut metadata = Vec::new();
        loop {
            let head_is_keyish = matches!(self.peek(0), Token::Identifier(_))
                || self.token_as_string(&self.peek(0).clone()).is_some();
            if !head_is_keyish {
                break;
            }
            let is_attr_key = matches!(self.peek(1), Token::Equal);
            // Bare boolean attribute: `function foo() skip { … }` — an attribute
            // with no `="value"`, equivalent to `skip="true"`. Recognised when the
            // next token ends the header (`{`) or begins another attribute key.
            // TestBox uses these (`skip`, `output`) on spec methods; Lucee/ACF/
            // BoxLang accept them too. Without this, the bare attr was left before
            // the `{`, so the body never parsed.
            let is_bare_bool = !is_attr_key
                && matches!(self.peek(1), Token::LBrace | Token::Identifier(_));
            if !is_attr_key && !is_bare_bool {
                break;
            }
            let key = if let Token::Identifier(ref s) = self.peek(0) {
                let s = s.clone();
                self.advance();
                s
            } else if let Some(s) = self.token_as_string(&self.peek(0).clone()) {
                self.advance();
                s
            } else {
                break;
            };
            if is_bare_bool {
                metadata.push((key, "true".to_string()));
                continue;
            }
            self.consume(&Token::Equal)?;
            // Attribute values may be quoted OR unquoted (e.g. `output=true`),
            // matching component/interface headers.
            let val = match self.parse_decl_attr_value() {
                Some(v) => v,
                None => break,
            };
            metadata.push((key, val));
        }

        let body = if self.check(&Token::LBrace) {
            self.parse_block()?
        } else {
            Vec::new()
        };

        Ok(Function {
            name,
            params,
            return_type,
            access: AccessModifier::Public,
            is_static: false,
            is_abstract: false,
            body,
            location: loc,
            metadata,
        })
    }

    /// Parse a javadoc comment body into `(declaration-level annotations,
    /// per-parameter annotations keyed by lowercased param name)`.
    /// `@key value` → declaration annotation; `@name.sub value` → the parameter
    /// named `name` gets annotation `sub`. A bare `@key` (no value) yields
    /// `"true"` (Lucee parity). Leading non-`@` text becomes the `hint`
    /// annotation unless an explicit `@hint` is present.
    fn parse_doc_comment(
        text: &str,
    ) -> (
        Vec<(String, String)>,
        std::collections::HashMap<String, Vec<(String, String)>>,
    ) {
        let mut decl: Vec<(String, String)> = Vec::new();
        let mut params: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        let mut hint_lines: Vec<String> = Vec::new();
        let mut seen_at = false;
        for raw in text.lines() {
            let mut line = raw.trim();
            // Strip the leading '*' (and following space) of javadoc continuation lines.
            if let Some(rest) = line.strip_prefix('*') {
                line = rest.trim_start();
            }
            if let Some(rest) = line.strip_prefix('@') {
                seen_at = true;
                let rest = rest.trim();
                if rest.is_empty() {
                    continue;
                }
                let (key, val) = match rest.find(char::is_whitespace) {
                    Some(i) => (rest[..i].to_string(), rest[i..].trim().to_string()),
                    None => (rest.to_string(), String::new()),
                };
                let value = if val.is_empty() {
                    // A bare annotation with no value (`@foo`) is boolean true.
                    "true".to_string()
                } else {
                    // Strip a single matching pair of surrounding quotes, matching
                    // Lucee: `@k "v"` -> v, `@k ""` -> "" (empty, NOT "true"),
                    // `@k 'v'` -> v, `@k v` -> v. (The quote chars are ASCII, so
                    // the byte-index slice lands on char boundaries.)
                    let b = val.as_bytes();
                    if val.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[val.len() - 1] == b[0]
                    {
                        val[1..val.len() - 1].to_string()
                    } else {
                        val
                    }
                };
                if let Some(dot) = key.find('.') {
                    let pname = key[..dot].to_lowercase();
                    let anno = key[dot + 1..].to_string();
                    params.entry(pname).or_default().push((anno, value));
                } else {
                    decl.push((key, value));
                }
            } else if !seen_at && !line.is_empty() {
                hint_lines.push(line.to_string());
            }
        }
        if !hint_lines.is_empty()
            && !decl.iter().any(|(k, _)| k.eq_ignore_ascii_case("hint"))
        {
            decl.push(("hint".to_string(), hint_lines.join(" ")));
        }
        (decl, params)
    }

    /// If a javadoc comment precedes the declaration starting at token `idx`,
    /// fold its annotations into the function metadata and parameter annotations.
    /// Explicit inline attributes already present win over javadoc.
    fn apply_doc_to_function(&self, idx: usize, func: &mut Function) {
        let Some(text) = self.doc_comments.get(&idx) else {
            return;
        };
        let (decl, params) = Self::parse_doc_comment(text);
        for (k, v) in decl {
            if !func.metadata.iter().any(|(ek, _)| ek.eq_ignore_ascii_case(&k)) {
                func.metadata.push((k, v));
            }
        }
        for p in func.params.iter_mut() {
            if let Some(annos) = params.get(&p.name.to_lowercase()) {
                for (k, v) in annos {
                    if !p.annotations.iter().any(|(ek, _)| ek.eq_ignore_ascii_case(k)) {
                        p.annotations.push((k.clone(), v.clone()));
                    }
                }
            }
        }
    }

    /// Fold a preceding javadoc comment's declaration-level annotations into a
    /// component/interface metadata list.
    fn apply_doc_to_metadata(&self, idx: usize, metadata: &mut Vec<(String, String)>) {
        let Some(text) = self.doc_comments.get(&idx) else {
            return;
        };
        let (decl, _params) = Self::parse_doc_comment(text);
        for (k, v) in decl {
            if !metadata.iter().any(|(ek, _)| ek.eq_ignore_ascii_case(&k)) {
                metadata.push((k, v));
            }
        }
    }

    fn parse_component(&mut self) -> Result<Component, ParseError> {
        let loc = self.current_location();
        // Parse component name: can be `component Name extends=...` (rare,
        // RustCFML-tolerant — CFML normally derives the name from the
        // filename). Consume an identifier as the name only when it's
        // followed by `extends` or `implements` — otherwise a bare ident
        // before `{` or another ident is a bare-bool attribute (e.g.
        // `component singleton {`, `component singleton serializable {`),
        // which we let the metadata loop below pick up.
        let mut name = if matches!(self.peek(0), Token::Identifier(_))
            && matches!(self.peek(1), Token::Extends | Token::Implements)
            && !matches!(self.peek(0), Token::Extends | Token::Implements)
        {
            self.extract_identifier().unwrap_or_else(|_| "Anonymous".to_string())
        } else {
            "Anonymous".to_string()
        };

        // Check for name="..." attribute before other metadata
        if name == "Anonymous" && matches!(self.peek(0), Token::Identifier(_))
            && self.peek(0).to_string() == "name"
            && matches!(self.peek(1), Token::Equal)
        {
            self.advance(); // consume "name"
            self.consume(&Token::Equal)?;
            if let Token::String(val) = self.peek(0).clone() {
                self.advance();
                name = val.clone();
            }
        }

        let mut extends = None;
        let mut implements = Vec::new();

        if self.match_token(&Token::Extends) {
            // Handle `extends Animal`, `extends="Animal"`, and the unquoted
            // dotted form `extends=testbox.system.BaseSpec` (TestBox specs use
            // this; Lucee/ACF/BoxLang accept an unquoted dotted path here).
            if self.match_token(&Token::Equal) {
                if let Token::String(val) = self.peek(0).clone() {
                    self.advance();
                    extends = Some(val);
                } else {
                    extends = self.extract_dotted_identifier().ok();
                }
            } else {
                extends = self.extract_dotted_identifier().ok();
            }
        }

        if self.match_token(&Token::Implements) {
            // Handle both `implements IFoo` and `implements="IFoo"` syntax
            if self.match_token(&Token::Equal) {
                if let Token::String(val) = self.peek(0).clone() {
                    self.advance();
                    // May be comma-separated: "IFoo,IBar"
                    for iface in val.split(',') {
                        let trimmed = iface.trim().to_string();
                        if !trimmed.is_empty() {
                            implements.push(trimmed);
                        }
                    }
                } else {
                    // Unquoted dotted form: `implements=foo.IBar`
                    loop {
                        if let Ok(iface) = self.extract_dotted_identifier() {
                            implements.push(iface);
                        } else {
                            break;
                        }
                        if !self.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }
            } else {
                loop {
                    if let Ok(iface) = self.extract_dotted_identifier() {
                        implements.push(iface);
                    }
                    if !self.match_token(&Token::Comma) {
                        break;
                    }
                }
            }
        }

        // Parse component metadata attributes (e.g., taffy_uri="/users/{id}", output="false", hint="...", accessors="true")
        // Accepts both identifiers and keyword tokens as attribute keys, plus
        // namespaced colon-separated keys like `taffy:uri` (lex'd as three
        // tokens: ident, `:`, ident).
        let mut metadata = Vec::new();
        loop {
            // Determine whether the upcoming tokens form a metadata attribute.
            // Patterns we recognise as a key:
            //   <ident>            =
            //   <ident> : <ident>  =       (namespaced, e.g. taffy:uri)
            // `extends`/`implements` are accepted here too: component header
            // attributes are order-independent on Lucee/ACF/BoxLang, so they may
            // appear AFTER another attribute (e.g. `component output="false"
            // extends="Foo" {`) and still populate the dedicated fields.
            let head_is_keyish =
                matches!(self.peek(0), Token::Identifier(_) | Token::Extends | Token::Implements)
                    || self.token_as_string(&self.peek(0).clone()).is_some();
            if !head_is_keyish {
                break;
            }
            let is_simple = matches!(self.peek(1), Token::Equal);
            let is_namespaced = matches!(self.peek(1), Token::Colon)
                && (matches!(self.peek(2), Token::Identifier(_))
                    || self.token_as_string(&self.peek(2).clone()).is_some())
                && matches!(self.peek(3), Token::Equal);
            // Bare boolean attribute: `singleton`, `serializable`, … — an
            // attribute with no `="value"`, equivalent to `name="true"`.
            // Recognised when the next token ends the header (`{`) or begins
            // another attribute key. Matches Lucee/ACF/BoxLang. Without this,
            // `component accessors="true" singleton {` left `singleton` before
            // the `{`, failing the body parse → silent null component.
            let is_bare_bool = !is_simple
                && !is_namespaced
                && matches!(
                    self.peek(1),
                    Token::LBrace | Token::Identifier(_) | Token::Extends | Token::Implements
                );
            if !is_simple && !is_namespaced && !is_bare_bool {
                break;
            }
            if is_bare_bool {
                let key = match self.peek(0).clone() {
                    Token::Identifier(s) => {
                        self.advance();
                        s
                    }
                    ref t => {
                        if let Some(s) = self.token_as_string(t) {
                            self.advance();
                            s
                        } else {
                            break;
                        }
                    }
                };
                metadata.push((key, "true".to_string()));
                continue;
            }
            let mut key = match self.peek(0).clone() {
                Token::Identifier(s) => {
                    self.advance();
                    s
                }
                Token::Extends => {
                    self.advance();
                    "extends".to_string()
                }
                Token::Implements => {
                    self.advance();
                    "implements".to_string()
                }
                ref t => {
                    if let Some(s) = self.token_as_string(t) {
                        self.advance();
                        s
                    } else {
                        break;
                    }
                }
            };
            if is_namespaced {
                self.advance(); // consume ':'
                let suffix = if let Token::Identifier(ref s) = self.peek(0) {
                    let s = s.clone();
                    self.advance();
                    s
                } else if let Some(s) = self.token_as_string(&self.peek(0).clone()) {
                    self.advance();
                    s
                } else {
                    break;
                };
                key.push('_');
                key.push_str(&suffix);
            }
            self.consume(&Token::Equal)?;
            // The value may be quoted OR unquoted (e.g. `output=false`).
            let val = match self.parse_decl_attr_value() {
                Some(v) => v,
                None => break,
            };
            if key.eq_ignore_ascii_case("extends") {
                // Only fill from the attribute form if a leading `extends` (handled
                // above) didn't already set it.
                if extends.is_none() {
                    extends = Some(val);
                }
            } else if key.eq_ignore_ascii_case("implements") {
                for iface in val.split(',') {
                    let trimmed = iface.trim().to_string();
                    if !trimmed.is_empty() {
                        implements.push(trimmed);
                    }
                }
            } else {
                // If this is the name attribute and we have an Anonymous component, use it as the name
                if key.eq_ignore_ascii_case("name") && name == "Anonymous" {
                    name = val.clone();
                }
                metadata.push((key, val));
            }
        }

        self.consume(&Token::LBrace)?;

        let mut properties = Vec::new();
        let mut functions = Vec::new();
        let mut body = Vec::new();
        let mut static_body = Vec::new();

        while !self.check(&Token::RBrace) && !self.is_at_end() {
            // Index of the first token of this member — a preceding javadoc
            // comment is anchored here.
            let member_start = self.current;
            // Check for access modifiers — only consume if followed by function/property/static
            let access = if matches!(
                self.peek(0),
                Token::Public | Token::Private | Token::Remote | Token::Package
            ) && self.is_access_modifier_for_member()
            {
                self.parse_access_modifier()
            } else {
                AccessModifier::Public
            };

            let is_static = self.match_token(&Token::Static);

            // `static { ... }` class-initialization block (BoxLang/Lucee). The
            // body runs once per component type to populate the shared `static`
            // scope. Distinguished from `static function` / `static property`
            // by the immediately-following `{`.
            if is_static && self.check(&Token::LBrace) {
                self.consume(&Token::LBrace)?;
                while !self.check(&Token::RBrace) && !self.is_at_end() {
                    let node = self.parse_statement()?;
                    if let CfmlNode::Statement(s) = node {
                        static_body.push(s);
                    }
                }
                self.consume(&Token::RBrace)?;
                continue;
            }

            // Capture optional return type annotation (e.g. "array function ...",
            // "component function init()", or a dotted component-path type) so it
            // surfaces as meta.returnType.
            let captured_rt = self.capture_dotted_return_type_before_function();

            if self.match_token(&Token::Property) {
                properties.push(self.parse_property()?);
            } else if self.match_token(&Token::Function) {
                let mut func = self.parse_function()?;
                func.access = access;
                func.is_static = is_static;
                if func.return_type.is_none() {
                    func.return_type = captured_rt;
                }
                self.apply_doc_to_function(member_start, &mut func);
                functions.push(func);
            } else if self.match_token(&Token::Var) {
                body.push(self.parse_var()?);
            } else {
                let node = self.parse_statement()?;
                if let CfmlNode::Statement(s) = node {
                    body.push(s);
                }
            }
        }

        self.consume(&Token::RBrace)?;

        // Check for accessors="true" attribute
        let accessors = metadata.iter()
            .any(|(key, value)| key.to_lowercase() == "accessors" && value.to_lowercase() == "true");

        Ok(Component {
            name,
            extends,
            implements,
            properties,
            functions,
            body,
            static_body,
            location: loc,
            metadata,
            accessors,
        })
    }

    fn parse_interface(&mut self) -> Result<Interface, ParseError> {
        let loc = self.current_location();
        // Optional name (same logic as component — skip if followed by '=')
        let name = if matches!(self.peek(0), Token::Identifier(_))
            && !matches!(self.peek(1), Token::Equal)
            && !matches!(self.peek(0), Token::Extends)
        {
            self.extract_identifier().unwrap_or_else(|_| "Anonymous".to_string())
        } else {
            "Anonymous".to_string()
        };

        // interfaces can extend multiple other interfaces, in either the
        // bareword form (`interface extends Base, Other`) or the attribute form
        // (`interface extends="Base"` / `extends="A,B"`), matching Lucee/ACF.
        let mut extends = Vec::new();
        if self.match_token(&Token::Extends) {
            if self.match_token(&Token::Equal) {
                if let Some(val) = self.parse_decl_attr_value() {
                    for parent in val.split(',') {
                        let trimmed = parent.trim().to_string();
                        if !trimmed.is_empty() {
                            extends.push(trimmed);
                        }
                    }
                }
            } else {
                loop {
                    if let Ok(parent) = self.extract_dotted_identifier() {
                        extends.push(parent);
                    }
                    if !self.match_token(&Token::Comma) {
                        break;
                    }
                }
            }
        }

        // Parse metadata attributes (same order-independent rules as component:
        // values may be quoted or unquoted, and `extends` may appear here too —
        // e.g. `interface displayname="x" extends="Base" {`).
        let mut metadata = Vec::new();
        loop {
            let head_is_keyish = matches!(self.peek(0), Token::Identifier(_) | Token::Extends)
                || self.token_as_string(&self.peek(0).clone()).is_some();
            if !matches!(self.peek(1), Token::Equal) || !head_is_keyish {
                break;
            }
            let key = match self.peek(0).clone() {
                Token::Identifier(s) => {
                    self.advance();
                    s
                }
                Token::Extends => {
                    self.advance();
                    "extends".to_string()
                }
                ref t => {
                    if let Some(s) = self.token_as_string(t) {
                        self.advance();
                        s
                    } else {
                        break;
                    }
                }
            };
            self.consume(&Token::Equal)?;
            let val = match self.parse_decl_attr_value() {
                Some(v) => v,
                None => break,
            };
            if key.eq_ignore_ascii_case("extends") {
                for parent in val.split(',') {
                    let trimmed = parent.trim().to_string();
                    if !trimmed.is_empty() {
                        extends.push(trimmed);
                    }
                }
            } else {
                metadata.push((key, val));
            }
        }

        self.consume(&Token::LBrace)?;

        let mut functions = Vec::new();

        while !self.check(&Token::RBrace) && !self.is_at_end() {
            // Consume optional semicolons between signatures
            if self.match_token(&Token::Semicolon) {
                continue;
            }

            // Parse access modifier — only if followed by function declaration
            let access = if matches!(
                self.peek(0),
                Token::Public | Token::Private | Token::Remote | Token::Package
            ) && self.is_access_modifier_for_function()
            {
                self.parse_access_modifier()
            } else {
                AccessModifier::Public
            };

            // Capture optional return type annotation (incl. dotted component-path)
            let captured_rt = self.capture_dotted_return_type_before_function();

            if self.match_token(&Token::Function) {
                let mut func = self.parse_function()?;
                func.access = access;
                if func.return_type.is_none() {
                    func.return_type = captured_rt;
                }
                functions.push(func);
            } else {
                // Skip unexpected tokens
                self.advance();
            }
        }

        self.consume(&Token::RBrace)?;

        Ok(Interface {
            name,
            extends,
            functions,
            metadata,
            location: loc,
        })
    }

    fn parse_property(&mut self) -> Result<Property, ParseError> {
        let loc = self.current_location();

        // Detect key-value syntax: property name="x" [type="y"] [inject="z"] ...;
        // Key-value syntax is detected when an identifier is followed by = and a string
        let is_kv = {
            let has_ident = matches!(self.peek(0), Token::Identifier(_))
                || self.token_as_string(&self.peek(0).clone()).is_some();
            has_ident && matches!(self.peek(1), Token::Equal) && matches!(self.peek(2), Token::String(_))
        };

        if is_kv {
            return self.parse_property_kv(loc);
        }

        // Positional syntax: property [required] [type] name [= default];
        let mut prop_type = None;
        let mut required = false;

        if self.match_token(&Token::Required) {
            required = true;
        }

        let first = self
            .extract_identifier()
            .unwrap_or_else(|_| "unknown".to_string());

        let name = if let Token::Identifier(_) = self.peek(0) {
            prop_type = Some(first);
            self.extract_identifier()
                .unwrap_or_else(|_| "unknown".to_string())
        } else {
            first
        };

        let default = if self.match_token(&Token::Equal) {
            Some(self.parse_expression()?)
        } else {
            None
        };

        self.match_token(&Token::Semicolon);

        Ok(Property {
            name,
            prop_type,
            default,
            required,
            attributes: Vec::new(),
            location: loc,
        })
    }

    /// Parse key-value property syntax: property name="x" type="string" inject="Service" default="val";
    fn parse_property_kv(&mut self, loc: SourceLocation) -> Result<Property, ParseError> {
        let mut name = String::new();
        let mut prop_type = None;
        let mut default = None;
        let mut required = false;
        let mut attributes = Vec::new();

        loop {
            let next_is_eq = matches!(self.peek(1), Token::Equal);

            // The attribute key: an identifier in any position; a keyword-as-string
            // only in the key=value form (e.g. `type="x"`). A token that is not a
            // valid attribute name (`;`, `}`, EOF, …) ends the attribute list.
            let key = if let Token::Identifier(ref s) = self.peek(0) {
                s.clone()
            } else if next_is_eq {
                match self.token_as_string(&self.peek(0).clone()) {
                    Some(s) => s,
                    None => break,
                }
            } else {
                break;
            };

            if next_is_eq {
                self.advance(); // consume key
                self.advance(); // consume =

                // For "default", parse as an expression (number, string, boolean, …).
                if key.eq_ignore_ascii_case("default") {
                    let expr = self.parse_expression()?;
                    default = Some(expr);
                } else {
                    // name, type, inject="X", delegateSuffix="disk", … — string value.
                    // Lucee also accepts UNQUOTED scalar values on property attributes
                    // (e.g. `required=false`, `maxlength=100`, `ondelete=cascade`); these
                    // must NOT terminate the attribute list, or every later attribute
                    // (e.g. a trailing `feature="…"`) is silently dropped.
                    let val = match self.peek(0).clone() {
                        Token::String(v) => {
                            self.advance();
                            v
                        }
                        Token::Integer(i) => {
                            self.advance();
                            i.to_string()
                        }
                        Token::Double(d) => {
                            self.advance();
                            d.to_string()
                        }
                        Token::True => {
                            self.advance();
                            "true".to_string()
                        }
                        Token::False => {
                            self.advance();
                            "false".to_string()
                        }
                        Token::Identifier(s) => {
                            self.advance();
                            s
                        }
                        _ => break,
                    };

                    match key.to_lowercase().as_str() {
                        "name" => name = val,
                        "type" => prop_type = Some(val),
                        // `required` with an explicit value (`required="false"`)
                        // is metadata that must be PRESERVED verbatim in
                        // getMetadata().properties — Lucee keeps `required:
                        // "false"`. Storing it as a normal attribute (rather than
                        // collapsing to the bool field, which codegen only emits
                        // when true) keeps the literal and its declared position.
                        // The bool field stays false here, so codegen won't also
                        // emit a duplicate `required` key.
                        "required" => attributes.push(("required".to_string(), val)),
                        _ => attributes.push((key.to_lowercase(), val)),
                    }
                }
            } else {
                // Bare annotation with no value (e.g. `inject delegate delegatePrefix`).
                // Lucee records these with an empty-string value; parsing MUST continue
                // past them so following annotations aren't dropped.
                self.advance(); // consume key
                match key.to_lowercase().as_str() {
                    "required" => required = true,
                    _ => attributes.push((key.to_lowercase(), String::new())),
                }
            }
        }

        self.match_token(&Token::Semicolon);

        Ok(Property {
            name,
            prop_type,
            default,
            required,
            attributes,
            location: loc,
        })
    }

    fn parse_param_list(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();

        if self.check(&Token::RParen) {
            return Ok(params);
        }

        loop {
            let required = self.match_token(&Token::Required);
            let mut param_type = None;

            // The leading token(s) are EITHER the parameter name, OR a type
            // annotation followed by the name. A type may be a dotted FQN
            // (`wheels.system.TestResult`), so parse it as a dotted word; the
            // name itself is always a single word. Both are keyword-tolerant:
            // CFML allows reserved words (`in`, `do`, `for`, `eq`, `is`, …) as
            // parameter names on Lucee/ACF/BoxLang (e.g. ColdBox Util.cfc's
            // `arrayToStruct(in)`), so extract_param_word accepts them.
            let first = self
                .extract_param_word()
                .unwrap_or_else(|_| "arg".to_string());

            // If next is also a word (identifier, soft keyword, or reserved
            // word), then first was the type annotation and next is the name.
            let name = if self.is_param_word_at(0) {
                param_type = Some(first);
                self.extract_param_word()
                    .unwrap_or_else(|_| "arg".to_string())
            } else {
                first
            };

            let default = if self.match_token(&Token::Equal) {
                Some(self.parse_expression()?)
            } else {
                None
            };

            // Capture optional per-parameter attributes (e.g. `inject="..."`,
            // `hint="..."`). These mirror the javadoc `@param.key value` form and
            // are surfaced as parameter annotations in getMetadata() (WireBox DI
            // reads `param.inject`).
            let mut annotations: Vec<(String, String)> = Vec::new();
            // A bareword token usable as an annotation key: a plain identifier
            // or a soft keyword (public, required, default, …).
            let key_string = |p: &Self, n: usize| -> Option<String> {
                if let Token::Identifier(ref s) = p.peek(n) {
                    Some(s.clone())
                } else {
                    p.token_as_string(&p.peek(n).clone())
                }
            };
            loop {
                if key_string(self, 0).is_none() {
                    break;
                }
                // Namespaced attribute keys — `colddoc:generic="x.Y"` from a tag
                // attribute like `<cfargument colddoc:generic="...">`. Lucee
                // accepts these and surfaces them in metadata; without this the
                // `:` fell through as an unexpected token ("Expected RParen,
                // found Colon"). Detect `ident : ident =`.
                let namespaced = matches!(self.peek(1), Token::Colon)
                    && key_string(self, 2).is_some()
                    && matches!(self.peek(3), Token::Equal);
                let plain = matches!(self.peek(1), Token::Equal);
                if !namespaced && !plain {
                    break;
                }
                let mut key = key_string(self, 0).unwrap();
                self.advance();
                if namespaced {
                    self.advance(); // consume ':'
                    key.push(':');
                    key.push_str(&key_string(self, 0).unwrap());
                    self.advance();
                }
                self.consume(&Token::Equal)?;
                if let Token::String(ref v) = self.peek(0) {
                    let v = v.clone();
                    self.advance();
                    annotations.push((key, v));
                } else {
                    // Tolerate bare identifier / number / etc. as attribute value
                    let _ = self.parse_expression();
                }
            }

            params.push(Param {
                name,
                param_type,
                default,
                required,
                annotations,
            });

            if !self.match_token(&Token::Comma) {
                // Lucee/ACF/BoxLang treat a newline between parameters as a soft
                // separator and tolerate a missing comma. Mirror that: only stop
                // looping at the closing paren / EOF, or when the next token
                // clearly isn't another parameter.
                if self.check(&Token::RParen) || self.is_at_end() {
                    break;
                }
                if !(self.is_param_word_at(0) || self.check(&Token::Required)) {
                    break;
                }
                // otherwise fall through and parse the next param
            }
        }

        Ok(params)
    }

    fn parse_block(&mut self) -> Result<Vec<Statement>, ParseError> {
        self.consume(&Token::LBrace)?;
        let mut statements = Vec::new();

        while !self.check(&Token::RBrace) && !self.is_at_end() {
            let node = self.parse_statement()?;
            if let CfmlNode::Statement(s) = node {
                statements.push(s);
            }
        }

        self.consume(&Token::RBrace)?;
        Ok(statements)
    }

    /// Parse either a braced block or a single statement (CFML allows braceless for/if/while bodies)
    fn parse_block_or_statement(&mut self) -> Result<Vec<Statement>, ParseError> {
        if self.check(&Token::LBrace) {
            self.parse_block()
        } else {
            let node = self.parse_statement()?;
            if let CfmlNode::Statement(s) = node {
                Ok(vec![s])
            } else {
                Ok(Vec::new())
            }
        }
    }

    /// Check whether an access modifier (public/private/remote/package) at peek(0)
    /// is used as a function declaration prefix vs. a plain identifier.
    /// Returns true if the modifier is followed by: function, static, or a return-type + function.
    /// A token usable as the (leading) return-type name in a function signature.
    /// Most type names lex as plain identifiers (`array`, `string`, `query`…),
    /// but `component` and `interface` are reserved keyword tokens — yet both are
    /// legal return types (`public component function init()`, as in Wheels'
    /// Seeder.cfc). Used for return-type lookahead and skipping.
    fn is_return_type_token(&self, offset: usize) -> bool {
        matches!(
            self.peek(offset),
            Token::Identifier(_) | Token::Component | Token::Interface
        )
    }

    /// If the current token begins a (possibly dotted) return-type annotation
    /// that is immediately followed by `function`, advance past the whole
    /// annotation and return the captured (possibly dotted) return-type string,
    /// so the declared return type in `access returnType function name(...)` is
    /// not dropped (it must surface as `meta.returnType` in getMetadata()).
    /// Returns `None` (consuming nothing) when no return type precedes
    /// `function`. A single-token check (`type function`) misses dotted
    /// component-path return types like
    /// `testbox.system.mockutils.MockGenerator function getMockGenerator()`
    /// (MockBox / any CFC returning a typed object) — silently dropping the
    /// whole method declaration. (GitHub #177)
    fn capture_dotted_return_type_before_function(&mut self) -> Option<String> {
        if !self.is_return_type_token(0) {
            return None;
        }
        let mut lookahead = 1;
        while matches!(self.peek(lookahead), Token::Dot)
            && matches!(self.peek(lookahead + 1), Token::Identifier(_))
        {
            lookahead += 2;
        }
        if matches!(self.peek(lookahead), Token::Function) {
            let mut parts = String::new();
            for i in 0..lookahead {
                let tok = self.peek(i).clone();
                self.advance();
                match tok {
                    Token::Dot => parts.push('.'),
                    Token::Identifier(s) => parts.push_str(&s),
                    Token::Component => parts.push_str("component"),
                    Token::Interface => parts.push_str("interface"),
                    _ => {}
                }
            }
            return Some(parts);
        }
        None
    }

    fn is_access_modifier_for_function(&self) -> bool {
        // peek(0) is the access modifier token itself; check what follows at peek(1)+
        let mut la = 1;
        // Skip optional "static"
        if matches!(self.peek(la), Token::Static) {
            la += 1;
        }
        // Direct "function" after modifier (or static)
        if matches!(self.peek(la), Token::Function) {
            return true;
        }
        // Return-type annotation: type-name(.Identifier)* followed by "function"
        if self.is_return_type_token(la) {
            la += 1;
            while matches!(self.peek(la), Token::Dot) && matches!(self.peek(la + 1), Token::Identifier(_)) {
                la += 2;
            }
            if matches!(self.peek(la), Token::Function) {
                return true;
            }
        }
        false
    }

    /// Check whether an access modifier at peek(0) precedes a component member
    /// (function, property, static) vs. being used as a plain identifier.
    fn is_access_modifier_for_member(&self) -> bool {
        let mut la = 1;
        // Skip optional "static"
        if matches!(self.peek(la), Token::Static) {
            la += 1;
        }
        // Direct function/property after modifier
        if matches!(self.peek(la), Token::Function | Token::Property) {
            return true;
        }
        // Return-type annotation: type-name(.Identifier)* followed by "function"
        if self.is_return_type_token(la) {
            la += 1;
            while matches!(self.peek(la), Token::Dot) && matches!(self.peek(la + 1), Token::Identifier(_)) {
                la += 2;
            }
            if matches!(self.peek(la), Token::Function) {
                return true;
            }
        }
        false
    }

    /// Check if the next token can be used as an identifier (true Identifier or soft keyword).
    fn is_identifier_like(&self) -> bool {
        self.is_identifier_like_at(0)
    }

    /// Decide whether a `component` token at the current position begins a CFC
    /// declaration (vs. being used as an ordinary identifier). A declaration is
    /// `component`, optionally followed by a name and/or metadata attributes,
    /// then a `{` body. The token immediately after `component` is decisive:
    ///   - `{`                       -> anonymous component body
    ///   - `extends` / `implements`  -> inheritance clause
    ///   - an identifier-like token  -> the CFC name, or a metadata key such as
    ///                                  `output=` / `displayname=` / `hint=`
    ///                                  (note `output` and friends lex as keyword
    ///                                  tokens, hence is_identifier_like_at, not a
    ///                                  bare Identifier check).
    /// Anything else (`=`, `.`, `[`, `(`, `;`, an operator, EOF) means `component`
    /// is an identifier — e.g. `component = "x"` or a cfinvoke `component="..."`
    /// attribute — so it is NOT a declaration and falls through to expressions.
    fn is_component_declaration(&self) -> bool {
        self.is_identifier_like_at(1)
            || matches!(self.peek(1), Token::LBrace | Token::Extends | Token::Implements)
    }

    /// Check if the token at offset can be used as an identifier.
    fn is_identifier_like_at(&self, offset: usize) -> bool {
        matches!(self.peek(offset),
            Token::Identifier(_) | Token::Local | Token::Param | Token::Output
            | Token::Required | Token::Default | Token::Include | Token::Import
            | Token::Property | Token::Abstract | Token::Final | Token::Static | Token::Lock
            | Token::Function | Token::Var | Token::Throw | Token::Component
            | Token::Interface | Token::Package | Token::Remote
            | Token::Public | Token::Private | Token::Extends | Token::Implements
            | Token::ImpKeyword
        )
    }

    /// Whether the token at `offset` can be consumed by `extract_property_name`
    /// as a dotted path/member segment. CFML lets ANY keyword follow a dot —
    /// including the operator words (`contains`, `eq`, `is`, `mod`, `and`, …) —
    /// so `new pkg.string.Contains()` and `obj.Contains()` are valid (cfflow's
    /// string-condition CFCs are literally named `Contains`/`Not`-style words).
    /// Mirror the exact set `extract_property_name` accepts.
    fn is_property_name_at(&self, offset: usize) -> bool {
        self.is_identifier_like_at(offset)
            || matches!(self.peek(offset),
                Token::If | Token::Else | Token::ElseIf | Token::For | Token::In
                | Token::While | Token::Do | Token::Break | Token::Continue
                | Token::Return | Token::Switch | Token::Case | Token::Try
                | Token::Catch | Token::Finally | Token::Rethrow | Token::New
                | Token::This | Token::Super | Token::True | Token::False | Token::Null
                | Token::Contains | Token::NotKeyword | Token::AndKeyword | Token::OrKeyword
                | Token::XorKeyword | Token::EqKeyword | Token::NeqKeyword
                | Token::GtKeyword | Token::GteKeyword | Token::LtKeyword
                | Token::LteKeyword | Token::ModKeyword | Token::IsKeyword)
    }

    fn extract_identifier(&mut self) -> Result<String, ParseError> {
        match self.peek(0) {
            Token::Identifier(_) => {
                if let Token::Identifier(id) = self.advance().token {
                    Ok(id)
                } else {
                    unreachable!()
                }
            }
            // CFML soft keywords — can be used as identifiers in most contexts
            Token::Local => { self.advance(); Ok("local".to_string()) }
            Token::Param => { self.advance(); Ok("param".to_string()) }
            Token::Output => { self.advance(); Ok("output".to_string()) }
            Token::Required => { self.advance(); Ok("required".to_string()) }
            Token::Default => { self.advance(); Ok("default".to_string()) }
            Token::Include => { self.advance(); Ok("include".to_string()) }
            Token::Import => { self.advance(); Ok("import".to_string()) }
            Token::Property => { self.advance(); Ok("property".to_string()) }
            Token::Abstract => { self.advance(); Ok("abstract".to_string()) }
            Token::Final => { self.advance(); Ok("final".to_string()) }
            Token::Static => { self.advance(); Ok("static".to_string()) }
            Token::Lock => { self.advance(); Ok("lock".to_string()) }
            Token::Function => { self.advance(); Ok("function".to_string()) }
            Token::Var => { self.advance(); Ok("var".to_string()) }
            Token::Throw => { self.advance(); Ok("throw".to_string()) }
            Token::Component => { self.advance(); Ok("component".to_string()) }
            Token::Interface => { self.advance(); Ok("interface".to_string()) }
            Token::Package => { self.advance(); Ok("package".to_string()) }
            Token::Remote => { self.advance(); Ok("remote".to_string()) }
            Token::Public => { self.advance(); Ok("public".to_string()) }
            Token::Private => { self.advance(); Ok("private".to_string()) }
            // `extends` / `implements` are declaration keywords but, like the
            // other soft keywords above, are legal ordinary identifiers (e.g.
            // function parameter names) on Lucee/Adobe CF/BoxLang. Component and
            // interface headers match these tokens explicitly before reaching
            // here, so accepting them as identifiers does not shadow inheritance.
            Token::Extends => { self.advance(); Ok("extends".to_string()) }
            Token::Implements => { self.advance(); Ok("implements".to_string()) }
            // `imp` (logical implication) is an operator only between two
            // operands; as a bare token it is a legal identifier on Lucee/Adobe
            // CF/BoxLang (e.g. `var imp = ...`). The operator is consumed at the
            // parse_imp precedence level before primary/identifier contexts see it.
            Token::ImpKeyword => { self.advance(); Ok("imp".to_string()) }
            _ => Err(self.parse_error("Expected identifier")),
        }
    }

    /// Extract a function/method name. Accepts identifiers, soft keywords, and
    /// other keywords that are legal member names in CFML (e.g. a method named
    /// `new`), mirroring property-name rules.
    fn extract_function_name(&mut self) -> Result<String, ParseError> {
        self.extract_property_name()
    }

    /// Extract a property name after a dot — any keyword or identifier is valid in CFML.
    fn extract_property_name(&mut self) -> Result<String, ParseError> {
        // First try normal identifier extraction (handles identifiers + soft keywords)
        if let Ok(name) = self.extract_identifier() {
            return Ok(name);
        }
        // After a dot, any keyword can be used as a property name in CFML
        let name = match self.peek(0) {
            Token::If => "if", Token::Else => "else", Token::ElseIf => "elseif",
            Token::For => "for", Token::In => "in", Token::While => "while",
            Token::Do => "do", Token::Break => "break", Token::Continue => "continue",
            Token::Return => "return", Token::Switch => "switch", Token::Case => "case",
            Token::Try => "try", Token::Catch => "catch", Token::Finally => "finally",
            Token::Throw => "throw", Token::Rethrow => "rethrow", Token::Function => "function", Token::Var => "var",
            Token::New => "new", Token::This => "this", Token::Super => "super",
            Token::Component => "component", Token::Extends => "extends",
            Token::Implements => "implements", Token::Interface => "interface",
            Token::Public => "public", Token::Private => "private",
            Token::Remote => "remote", Token::Package => "package",
            Token::Static => "static",
            Token::True => "true", Token::False => "false", Token::Null => "null",
            Token::Contains => "contains", Token::NotKeyword => "not",
            Token::AndKeyword => "and", Token::OrKeyword => "or", Token::XorKeyword => "xor",
            Token::EqKeyword => "eq", Token::NeqKeyword => "neq",
            Token::GtKeyword => "gt", Token::GteKeyword => "gte",
            Token::LtKeyword => "lt", Token::LteKeyword => "lte",
            Token::ModKeyword => "mod", Token::IsKeyword => "is",
            _ => return Err(self.parse_error("Expected property name")),
        };
        self.advance();
        Ok(name.to_string())
    }

    /// Extract a component/interface declaration attribute VALUE. CFML accepts
    /// the value quoted OR unquoted: a string literal, a boolean/number keyword,
    /// or a bare (optionally dotted) identifier are all legal value positions on
    /// Lucee/Adobe CF/BoxLang. Returns the value rendered as a string, or None if
    /// the next token cannot begin a value.
    fn parse_decl_attr_value(&mut self) -> Option<String> {
        match self.peek(0).clone() {
            Token::String(s) => {
                self.advance();
                Some(s)
            }
            Token::True => {
                self.advance();
                Some("true".to_string())
            }
            Token::False => {
                self.advance();
                Some("false".to_string())
            }
            Token::Integer(n) => {
                self.advance();
                Some(n.to_string())
            }
            Token::Double(d) => {
                self.advance();
                Some(d.to_string())
            }
            Token::Identifier(_) => self.extract_dotted_identifier().ok(),
            _ => None,
        }
    }

    /// Convert a keyword token to its string representation for use as metadata keys.
    fn token_as_string(&self, token: &Token) -> Option<String> {
        match token {
            Token::Output => Some("output".to_string()),
            Token::Public => Some("public".to_string()),
            Token::Private => Some("private".to_string()),
            Token::Remote => Some("remote".to_string()),
            Token::Package => Some("package".to_string()),
            Token::Static => Some("static".to_string()),
            Token::Abstract => Some("abstract".to_string()),
            Token::Final => Some("final".to_string()),
            Token::Required => Some("required".to_string()),
            Token::Default => Some("default".to_string()),
            Token::Lock => Some("lock".to_string()),
            _ => None,
        }
    }

    fn extract_dotted_identifier(&mut self) -> Result<String, ParseError> {
        let mut path = self.extract_identifier()?;
        while self.match_token(&Token::Dot) {
            let next = self.extract_property_name()?;
            path.push('.');
            path.push_str(&next);
        }
        Ok(path)
    }

    /// Extract a (possibly dotted) word in a function parameter list. Unlike
    /// extract_dotted_identifier, the leading segment is keyword-tolerant:
    /// CFML accepts reserved words (`in`, `do`, `for`, `eq`, `is`, …) as
    /// parameter names on Lucee/Adobe CF/BoxLang. extract_property_name already
    /// accepts identifiers, soft keywords, and the full reserved-word set.
    fn extract_param_word(&mut self) -> Result<String, ParseError> {
        let mut path = self.extract_property_name()?;
        while self.match_token(&Token::Dot) {
            let next = self.extract_property_name()?;
            path.push('.');
            path.push_str(&next);
        }
        Ok(path)
    }

    /// Whether the token at `offset` can begin a parameter name. Mirrors the
    /// keyword set extract_param_word / extract_property_name accept, so a
    /// reserved word used as a parameter name is recognised both as the name
    /// itself and when disambiguating `type name` from a lone `name`.
    fn is_param_word_at(&self, offset: usize) -> bool {
        self.is_identifier_like_at(offset)
            || matches!(self.peek(offset),
                Token::If | Token::Else | Token::ElseIf | Token::For | Token::In
                | Token::While | Token::Do | Token::Break | Token::Continue
                | Token::Return | Token::Switch | Token::Case | Token::Try
                | Token::Catch | Token::Finally | Token::Rethrow | Token::New
                | Token::This | Token::Super | Token::True | Token::False | Token::Null
                | Token::Contains | Token::NotKeyword | Token::AndKeyword | Token::OrKeyword
                | Token::EqKeyword | Token::NeqKeyword | Token::GtKeyword | Token::GteKeyword
                | Token::LtKeyword | Token::LteKeyword | Token::ModKeyword | Token::IsKeyword)
    }

    fn consume(&mut self, token: &Token) -> Result<(), ParseError> {
        if self.check(token) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError {
                message: format!("Expected {:?}, found {:?}", token, self.peek(0)),
                line: self.current_location().start.line,
                column: self.current_location().start.column,
            })
        }
    }

    // ---- Expression Parsing (Pratt-style precedence climbing) ----

    fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        self.parse_assignment_expr()
    }

    fn parse_assignment_expr(&mut self) -> Result<Expression, ParseError> {
        let expr = self.parse_ternary()?;

        if self.check(&Token::Equal) {
            if let Expression::Identifier(ref ident) = expr {
                let name = ident.name.clone();
                self.advance(); // consume =
                let value = self.parse_assignment_rhs()?;
                return Ok(Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(Expression::Identifier(Identifier {
                        name,
                        location: self.current_location(),
                    })),
                    operator: BinaryOpType::Assign,
                    right: Box::new(value),
                    location: self.current_location(),
                })));
            } else if let Expression::MemberAccess(_) | Expression::ArrayAccess(_) = &expr {
                self.advance(); // consume =
                let value = self.parse_assignment_rhs()?;
                return Ok(Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(expr),
                    operator: BinaryOpType::Assign,
                    right: Box::new(value),
                    location: self.current_location(),
                })));
            } else if matches!(
                &expr,
                Expression::Literal(Literal { value: LiteralValue::String(_), .. })
                    | Expression::StringInterpolation(_)
            ) {
                // Dynamic/quoted-string LHS: `"variables.x" = v` or
                // `"#scope#.#prop#" = v`. CFML (Lucee/ACF) treats a string-valued
                // lvalue as a runtime scope path. Codegen emits SetDynamicVar.
                self.advance(); // consume =
                let value = self.parse_assignment_rhs()?;
                return Ok(Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(expr),
                    operator: BinaryOpType::Assign,
                    right: Box::new(value),
                    location: self.current_location(),
                })));
            }
        }

        Ok(expr)
    }

    /// Parse the right-hand side of an assignment. Unlike `parse_assignment_expr`,
    /// this also accepts a *compound* assignment operator (`&=`, `+=`, …) so a
    /// chained assignment like `local.sql = local.sql &= ";"` parses as
    /// `local.sql = (local.sql = local.sql & ";")`. Top-level statement
    /// compound-assignment is intentionally NOT routed here: the statement parser
    /// handles `lhs OP= rhs` directly (check_assignment_op) to keep the
    /// single-evaluation semantics of struct/array targets in codegen
    /// (Statement::Assignment / emit_load_current_target).
    fn parse_assignment_rhs(&mut self) -> Result<Expression, ParseError> {
        let expr = self.parse_ternary()?;

        if self.check(&Token::Equal) {
            if let Expression::Identifier(_) | Expression::MemberAccess(_) | Expression::ArrayAccess(_) = &expr {
                self.advance(); // consume =
                let value = self.parse_assignment_rhs()?;
                return Ok(Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(expr),
                    operator: BinaryOpType::Assign,
                    right: Box::new(value),
                    location: self.current_location(),
                })));
            }
        } else if let Some(bin_op) = self.compound_assign_binop() {
            if matches!(
                expr,
                Expression::Identifier(_)
                    | Expression::MemberAccess(_)
                    | Expression::ArrayAccess(_)
            ) {
                self.advance(); // consume the compound operator
                let value = self.parse_assignment_rhs()?;
                let combined = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(expr.clone()),
                    operator: bin_op,
                    right: Box::new(value),
                    location: self.current_location(),
                }));
                return Ok(Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(expr),
                    operator: BinaryOpType::Assign,
                    right: Box::new(combined),
                    location: self.current_location(),
                })));
            }
        }

        Ok(expr)
    }

    fn parse_ternary(&mut self) -> Result<Expression, ParseError> {
        let expr = self.parse_imp()?;

        if self.match_token(&Token::Question) {
            let then_expr = Box::new(self.parse_expression()?);
            self.consume(&Token::Colon)?;
            let else_expr = Box::new(self.parse_expression()?);

            return Ok(Expression::Ternary(Box::new(Ternary {
                condition: Box::new(expr),
                then_expr,
                else_expr,
                location: self.current_location(),
            })));
        }

        // Elvis operator ?: (null coalescing) and ?? (null coalescing alias)
        if self.match_token(&Token::QuestionColon) || self.match_token(&Token::QuestionQuestion) {
            let right = Box::new(self.parse_expression()?);
            return Ok(Expression::Elvis(Box::new(Elvis {
                left: Box::new(expr),
                right,
                location: self.current_location(),
            })));
        }

        Ok(expr)
    }

    fn parse_imp(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_eqv()?;

        while self.match_token(&Token::ImpKeyword) {
            let right = Box::new(self.parse_eqv()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator: BinaryOpType::Imp,
                right,
                location: self.current_location(),
            }));
        }

        Ok(left)
    }

    fn parse_eqv(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_xor()?;

        while self.match_token(&Token::EqvKeyword) {
            let right = Box::new(self.parse_xor()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator: BinaryOpType::Eqv,
                right,
                location: self.current_location(),
            }));
        }

        Ok(left)
    }

    fn parse_xor(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_or()?;

        while self.match_token(&Token::XorKeyword) {
            let right = Box::new(self.parse_or()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator: BinaryOpType::Xor,
                right,
                location: self.current_location(),
            }));
        }

        Ok(left)
    }

    fn parse_or(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_and()?;

        while self.match_token(&Token::BarBar) || self.match_token(&Token::OrKeyword) {
            let right = Box::new(self.parse_and()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator: BinaryOpType::Or,
                right,
                location: self.current_location(),
            }));
        }

        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_not()?;

        while self.match_token(&Token::AmpAmp) || self.match_token(&Token::AndKeyword) {
            let right = Box::new(self.parse_not()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator: BinaryOpType::And,
                right,
                location: self.current_location(),
            }));
        }

        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expression, ParseError> {
        if self.match_token(&Token::NotKeyword) || self.match_token(&Token::Bang) {
            let operand = Box::new(self.parse_not()?);
            return Ok(Expression::UnaryOp(Box::new(UnaryOp {
                operator: UnaryOpType::Not,
                operand,
                location: self.current_location(),
            })));
        }

        self.parse_equality()
    }

    fn parse_equality(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_comparison()?;

        loop {
            // `EQUAL` / `EQUALS` (verbose alias for EQ) and `==`/`EQ`.
            if self.match_token(&Token::EqualEqualEqual) {
                let right = Box::new(self.parse_comparison()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::StrictEqual,
                    right,
                    location: self.current_location(),
                }));
            } else if self.match_token(&Token::BangEqualEqual) {
                let right = Box::new(self.parse_comparison()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::StrictNotEqual,
                    right,
                    location: self.current_location(),
                }));
            } else if self.match_token(&Token::EqualEqual)
                || self.match_token(&Token::EqKeyword)
                || self.match_word("equal")
                || self.match_word("equals")
            {
                let right = Box::new(self.parse_comparison()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::Equal,
                    right,
                    location: self.current_location(),
                }));
            } else if self.match_token(&Token::IsKeyword) {
                // `IS NOT` is the verbose alias for NEQ; bare `IS` is EQ.
                let operator = if self.match_token(&Token::NotKeyword) {
                    BinaryOpType::NotEqual
                } else {
                    BinaryOpType::Equal
                };
                let right = Box::new(self.parse_comparison()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator,
                    right,
                    location: self.current_location(),
                }));
            } else if self.match_token(&Token::BangEqual) || self.match_token(&Token::NeqKeyword) {
                let right = Box::new(self.parse_comparison()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::NotEqual,
                    right,
                    location: self.current_location(),
                }));
            } else if matches!(self.peek(0), Token::NotKeyword)
                && (self.peek_word(1, "equal") || self.peek_word(1, "equals"))
            {
                // `NOT EQUAL` / `NOT EQUALS` — verbose alias for NEQ. Only consume
                // the `NOT` once the following word confirms the operator, so a
                // leading unary `NOT` elsewhere is untouched.
                self.advance(); // NOT
                self.advance(); // EQUAL / EQUALS
                let right = Box::new(self.parse_comparison()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::NotEqual,
                    right,
                    location: self.current_location(),
                }));
            } else {
                break;
            }
        }

        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_contains()?;

        loop {
            if self.match_token(&Token::Greater) || self.match_token(&Token::GtKeyword) {
                let right = Box::new(self.parse_contains()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::Greater,
                    right,
                    location: self.current_location(),
                }));
            } else if self.match_token(&Token::GreaterEqual) || self.match_token(&Token::GteKeyword) {
                let right = Box::new(self.parse_contains()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::GreaterEqual,
                    right,
                    location: self.current_location(),
                }));
            } else if self.match_token(&Token::Less) || self.match_token(&Token::LtKeyword) {
                let right = Box::new(self.parse_contains()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::Less,
                    right,
                    location: self.current_location(),
                }));
            } else if self.match_token(&Token::LessEqual) || self.match_token(&Token::LteKeyword) {
                let right = Box::new(self.parse_contains()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::LessEqual,
                    right,
                    location: self.current_location(),
                }));
            } else if self.peek_word(0, "greater") && self.peek_word(1, "than") {
                // `GREATER THAN` (GT) and `GREATER THAN OR EQUAL TO` (GTE).
                self.advance(); // greater
                self.advance(); // than
                let operator = self.match_or_equal_to_suffix(
                    BinaryOpType::Greater,
                    BinaryOpType::GreaterEqual,
                );
                let right = Box::new(self.parse_contains()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator,
                    right,
                    location: self.current_location(),
                }));
            } else if self.peek_word(0, "less") && self.peek_word(1, "than") {
                // `LESS THAN` (LT) and `LESS THAN OR EQUAL TO` (LTE).
                self.advance(); // less
                self.advance(); // than
                let operator = self.match_or_equal_to_suffix(
                    BinaryOpType::Less,
                    BinaryOpType::LessEqual,
                );
                let right = Box::new(self.parse_contains()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator,
                    right,
                    location: self.current_location(),
                }));
            } else {
                break;
            }
        }

        Ok(left)
    }

    /// After a `GREATER THAN` / `LESS THAN`, consume an optional `OR EQUAL TO`
    /// suffix. Returns `with_suffix` if it was present, else `base`. (`OR` lexes
    /// as the logical-or keyword; `equal`/`to` are plain identifiers.)
    fn match_or_equal_to_suffix(
        &mut self,
        base: BinaryOpType,
        with_suffix: BinaryOpType,
    ) -> BinaryOpType {
        if matches!(self.peek(0), Token::OrKeyword)
            && self.peek_word(1, "equal")
            && self.peek_word(2, "to")
        {
            self.advance(); // or
            self.advance(); // equal
            self.advance(); // to
            with_suffix
        } else {
            base
        }
    }

    fn parse_contains(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_concatenation()?;

        if self.match_token(&Token::Contains) {
            let right = Box::new(self.parse_concatenation()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator: BinaryOpType::Contains,
                right,
                location: self.current_location(),
            }));
        } else if self.peek_word(0, "does")
            && matches!(self.peek(1), Token::NotKeyword)
            && (self.peek_word(2, "contain")
                || self.peek_word(2, "contains")
                || matches!(self.peek(2), Token::Contains))
        {
            // `DOES NOT CONTAIN` — verbose negated CONTAINS. The positive form is
            // the `CONTAINS` keyword, but the negated form's `CONTAIN` (singular)
            // lexes as a plain identifier.
            self.advance(); // does
            self.advance(); // not
            self.advance(); // contain(s)
            let right = Box::new(self.parse_concatenation()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator: BinaryOpType::DoesNotContain,
                right,
                location: self.current_location(),
            }));
        } else if self.match_token(&Token::NotKeyword) {
            // "NOT CONTAINS" as two-word operator
            if self.match_token(&Token::Contains) {
                let right = Box::new(self.parse_concatenation()?);
                left = Expression::BinaryOp(Box::new(BinaryOp {
                    left: Box::new(left),
                    operator: BinaryOpType::DoesNotContain,
                    right,
                    location: self.current_location(),
                }));
            } else {
                // It was just NOT used as unary, put it back
                self.current -= 1;
            }
        }

        Ok(left)
    }

    fn parse_concatenation(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_term()?;

        while self.match_token(&Token::Amp) {
            let right = Box::new(self.parse_term()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator: BinaryOpType::Concat,
                right,
                location: self.current_location(),
            }));
        }

        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_factor()?;

        while self.match_token(&Token::Plus) || self.match_token(&Token::Minus) {
            let operator = match self.previous().token {
                Token::Plus => BinaryOpType::Add,
                _ => BinaryOpType::Sub,
            };
            let right = Box::new(self.parse_factor()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator,
                right,
                location: self.current_location(),
            }));
        }

        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.parse_power()?;

        while self.match_token(&Token::Star)
            || self.match_token(&Token::Slash)
            || self.match_token(&Token::Percent)
            || self.match_token(&Token::ModKeyword)
            || self.match_token(&Token::Backslash)
        {
            let operator = match self.previous().token {
                Token::Star => BinaryOpType::Mul,
                Token::Slash => BinaryOpType::Div,
                Token::Backslash => BinaryOpType::IntDiv,
                _ => BinaryOpType::Mod,
            };
            let right = Box::new(self.parse_power()?);
            left = Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator,
                right,
                location: self.current_location(),
            }));
        }

        Ok(left)
    }

    fn parse_power(&mut self) -> Result<Expression, ParseError> {
        let left = self.parse_unary()?;

        if self.match_token(&Token::Caret) {
            let right = Box::new(self.parse_unary()?);
            return Ok(Expression::BinaryOp(Box::new(BinaryOp {
                left: Box::new(left),
                operator: BinaryOpType::Pow,
                right,
                location: self.current_location(),
            })));
        }

        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expression, ParseError> {
        if self.match_token(&Token::Minus) {
            let operand = Box::new(self.parse_unary()?);
            return Ok(Expression::UnaryOp(Box::new(UnaryOp {
                operator: UnaryOpType::Minus,
                operand,
                location: self.current_location(),
            })));
        }

        // Unary plus (`+1`, `+x`, `3 + +1`) is a numeric-identity prefix in CFML —
        // parse and return the operand unchanged. Lucee/ACF accept it; we previously
        // only handled unary `-`, so a leading `+` failed with "Expected RParen".
        if self.match_token(&Token::Plus) {
            return self.parse_unary();
        }

        // Prefix ++ / --
        if self.match_token(&Token::PlusPlus) {
            let operand = Box::new(self.parse_call()?);
            return Ok(Expression::UnaryOp(Box::new(UnaryOp {
                operator: UnaryOpType::PrefixIncrement,
                operand,
                location: self.current_location(),
            })));
        }
        if self.match_token(&Token::MinusMinus) {
            let operand = Box::new(self.parse_call()?);
            return Ok(Expression::UnaryOp(Box::new(UnaryOp {
                operator: UnaryOpType::PrefixDecrement,
                operand,
                location: self.current_location(),
            })));
        }

        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expression, ParseError> {
        let mut expr = self.parse_call()?;

        // Postfix ++ / --
        if self.match_token(&Token::PlusPlus) {
            expr = Expression::PostfixOp(Box::new(PostfixOp {
                operand: Box::new(expr),
                operator: PostfixOpType::Increment,
                location: self.current_location(),
            }));
        } else if self.match_token(&Token::MinusMinus) {
            expr = Expression::PostfixOp(Box::new(PostfixOp {
                operand: Box::new(expr),
                operator: PostfixOpType::Decrement,
                location: self.current_location(),
            }));
        }

        Ok(expr)
    }

    fn parse_call(&mut self) -> Result<Expression, ParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            if self.match_token(&Token::Dot) {
                // CFML allows a numeric-literal member key: `a.1` is equivalent
                // to `a[1]` for both arrays (index) and structs (key "1").
                // Lucee/ACF accept this; treat it as bracket access.
                if let Token::Integer(i) = self.peek(0).clone() {
                    self.advance();
                    let loc = self.current_location();
                    expr = Expression::ArrayAccess(Box::new(ArrayAccess {
                        array: Box::new(expr),
                        index: Box::new(Expression::Literal(Literal {
                            value: LiteralValue::Int(i),
                            location: loc.clone(),
                        })),
                        location: loc,
                    }));
                    continue;
                }
                // Chained numeric keys like `m.1.2` lex the `1.2` as a single
                // Double token after the leading dot. Decompose it into two
                // successive integer-key accesses (`m[1][2]`).
                if let Token::Double(d) = self.peek(0).clone() {
                    let text = format!("{}", d);
                    let parts: Vec<&str> = text.split('.').collect();
                    if parts.len() == 2 && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
                        self.advance();
                        let loc = self.current_location();
                        for part in parts {
                            if let Ok(n) = part.parse::<i64>() {
                                expr = Expression::ArrayAccess(Box::new(ArrayAccess {
                                    array: Box::new(expr),
                                    index: Box::new(Expression::Literal(Literal {
                                        value: LiteralValue::Int(n),
                                        location: loc.clone(),
                                    })),
                                    location: loc.clone(),
                                }));
                            }
                        }
                        continue;
                    }
                }
                let method = self.extract_property_name().unwrap_or_default();
                if self.match_token(&Token::LParen) {
                    let args = self.parse_arguments()?;
                    self.consume(&Token::RParen)?;
                    expr = Expression::MethodCall(Box::new(MethodCall {
                        object: Box::new(expr),
                        method,
                        arguments: args,
                        null_safe: false,
                        location: self.current_location(),
                    }));
                } else {
                    expr = Expression::MemberAccess(Box::new(MemberAccess {
                        object: Box::new(expr),
                        member: method,
                        null_safe: false,
                        location: self.current_location(),
                    }));
                }
            } else if self.match_token(&Token::LParen) {
                let args = self.parse_arguments()?;
                self.consume(&Token::RParen)?;
                expr = Expression::FunctionCall(Box::new(FunctionCall {
                    name: Box::new(expr),
                    arguments: args,
                    location: self.current_location(),
                }));
            } else if self.match_token(&Token::LBracket) {
                let index = Box::new(self.parse_expression()?);
                self.consume(&Token::RBracket)?;
                expr = Expression::ArrayAccess(Box::new(ArrayAccess {
                    array: Box::new(expr),
                    index,
                    location: self.current_location(),
                }));
            } else if self.match_token(&Token::ColonColon) {
                // `::` static member access. `Component::member` reads a static
                // member; `Component::method(args)` calls a static method —
                // neither requires an instance.
                let member = self.extract_property_name().unwrap_or_default();
                if self.match_token(&Token::LParen) {
                    let args = self.parse_arguments()?;
                    self.consume(&Token::RParen)?;
                    expr = Expression::StaticCall(Box::new(StaticCall {
                        class: Box::new(expr),
                        method: member,
                        arguments: args,
                        location: self.current_location(),
                    }));
                } else {
                    expr = Expression::StaticMember(Box::new(StaticMember {
                        class: Box::new(expr),
                        member,
                        location: self.current_location(),
                    }));
                }
            } else if self.match_token(&Token::QuestionDot) {
                // Null-safe navigation: obj?.method() or obj?.property
                let member = self.extract_property_name().unwrap_or_default();
                if self.match_token(&Token::LParen) {
                    let args = self.parse_arguments()?;
                    self.consume(&Token::RParen)?;
                    expr = Expression::MethodCall(Box::new(MethodCall {
                        object: Box::new(expr),
                        method: member,
                        arguments: args,
                        null_safe: true,
                        location: self.current_location(),
                    }));
                } else {
                    expr = Expression::MemberAccess(Box::new(MemberAccess {
                        object: Box::new(expr),
                        member,
                        null_safe: true,
                        location: self.current_location(),
                    }));
                }
            } else {
                break;
            }
        }

        Ok(expr)
    }

    /// True when the tokens at the current position form a quoted or
    /// interpolated string literal immediately followed by `=` or `:` — i.e. a
    /// *dynamic* named argument whose name is computed at runtime, like
    /// `func( "#property#" = value )`. Lucee supports these; RustCFML lowers the
    /// whole call to `argumentCollection` (see `parse_arguments`) since a
    /// runtime name cannot live in the static names array.
    fn is_dynamic_named_arg(&self) -> bool {
        match self.peek(0) {
            Token::String(_) => matches!(self.peek(1), Token::Equal | Token::Colon),
            Token::InterpolatedStringStart => {
                let mut k = 1;
                loop {
                    match self.peek(k) {
                        Token::InterpolatedStringEnd => {
                            return matches!(self.peek(k + 1), Token::Equal | Token::Colon);
                        }
                        Token::Eof => return false,
                        _ => {
                            k += 1;
                            if k > 4096 {
                                return false;
                            }
                        }
                    }
                }
            }
            _ => false,
        }
    }

    fn parse_arguments(&mut self) -> Result<Vec<Expression>, ParseError> {
        let mut args = Vec::new();

        if self.check(&Token::RParen) {
            return Ok(args);
        }

        // Each collected entry is (name-key expression, value). A `None` name
        // marks a positional argument. Identifier and literal-string names are
        // stored as string literals; interpolated names keep their expression so
        // they can be resolved at runtime.
        let mut collected: Vec<(Option<Expression>, Expression)> = Vec::new();
        let mut has_dynamic_name = false;

        loop {
            if self.match_token(&Token::DotDotDot) {
                let expr = self.parse_expression()?;
                collected.push((None, Expression::Spread(Box::new(expr))));
            } else {
                // Check for named argument: identifier = value or identifier : value
                // CFML supports foo(name = value, name2 = value2) and foo(name : value)
                // We must detect this before parse_expression consumes `=` as assignment.
                let is_named_arg = (matches!(self.peek(0), Token::Identifier(_)) || self.is_identifier_like())
                    && (matches!(self.peek(1), Token::Equal | Token::Colon));
                if is_named_arg {
                    let name = self.extract_identifier()?;
                    let loc = self.current_location();
                    // Consume either = or :
                    if !self.match_token(&Token::Equal) {
                        self.consume(&Token::Colon)?;
                    }
                    let value = self.parse_expression()?;
                    collected.push((
                        Some(Expression::Literal(Literal {
                            value: LiteralValue::String(name),
                            location: loc,
                        })),
                        value,
                    ));
                } else if self.is_dynamic_named_arg() {
                    // Dynamic named argument: a quoted/interpolated string as the
                    // parameter name, e.g. `func( "#property#" = value )`.
                    let name_expr = self.parse_primary()?;
                    if !self.match_token(&Token::Equal) {
                        self.consume(&Token::Colon)?;
                    }
                    let value = self.parse_expression()?;
                    has_dynamic_name = true;
                    collected.push((Some(name_expr), value));
                } else {
                    collected.push((None, self.parse_expression()?));
                }
            }
            if !self.match_token(&Token::Comma) {
                break;
            }
        }

        let any_positional = collected.iter().any(|(name, _)| name.is_none());

        if has_dynamic_name && !any_positional {
            // Every argument is named and at least one name is runtime-computed.
            // Fold them into a single `argumentCollection` struct literal so the
            // computed names resolve at call time (struct literals already
            // support dynamic/interpolated keys). Matches Lucee.
            let loc = self.current_location();
            let pairs: Vec<(Expression, Expression)> = collected
                .into_iter()
                .map(|(name, value)| (name.expect("all args named here"), value))
                .collect();
            let struct_expr = Expression::Struct(Struct {
                pairs,
                ordered: false,
                location: loc.clone(),
            });
            args.push(Expression::NamedArgument(Box::new(NamedArgument {
                name: "argumentCollection".to_string(),
                value: Box::new(struct_expr),
                location: loc,
            })));
        } else {
            // Fast path (no runtime-computed names): reconstruct the original
            // argument expressions exactly as before.
            for (name, value) in collected {
                match name {
                    Some(Expression::Literal(Literal {
                        value: LiteralValue::String(n),
                        location,
                    })) => {
                        args.push(Expression::NamedArgument(Box::new(NamedArgument {
                            name: n,
                            value: Box::new(value),
                            location,
                        })));
                    }
                    // Interpolated name mixed with positional args (invalid CFML):
                    // emit the value positionally so the runtime raises the
                    // mixed named/positional-arguments error, as Lucee does.
                    Some(_) => args.push(value),
                    None => args.push(value),
                }
            }
        }

        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expression, ParseError> {
        let token = self.advance().token.clone();

        match token {
            // Lucee tolerates redundant `#expr#` around an expression in SCRIPT
            // context and just strips the hashes (e.g. Mura/Masa's
            // formBuilderManager `if(!directoryExists(#expandPath(...)# & "/x"))`
            // written inside a <cfscript> block). Parse the inner expression and
            // consume the matching closing `#`, returning the inner value.
            Token::HashSign => {
                let inner = self.parse_expression()?;
                self.consume(&Token::HashSign)?;
                Ok(inner)
            }
            Token::True => Ok(Expression::Literal(Literal {
                value: LiteralValue::Bool(true),
                location: self.current_location(),
            })),
            Token::False => Ok(Expression::Literal(Literal {
                value: LiteralValue::Bool(false),
                location: self.current_location(),
            })),
            Token::Null => Ok(Expression::Literal(Literal {
                value: LiteralValue::Null,
                location: self.current_location(),
            })),
            Token::Integer(i) => Ok(Expression::Literal(Literal {
                value: LiteralValue::Int(i),
                location: self.current_location(),
            })),
            Token::Double(d) => Ok(Expression::Literal(Literal {
                value: LiteralValue::Double(d),
                location: self.current_location(),
            })),
            Token::String(s) => Ok(Expression::Literal(Literal {
                value: LiteralValue::String(s),
                location: self.current_location(),
            })),
            Token::InterpolatedStringStart => {
                let mut parts: Vec<Expression> = Vec::new();
                while !self.is_at_end() && !self.check(&Token::InterpolatedStringEnd) {
                    let part_token = self.advance().token.clone();
                    match part_token {
                        Token::String(s) => {
                            parts.push(Expression::Literal(Literal {
                                value: LiteralValue::String(s),
                                location: self.current_location(),
                            }));
                        }
                        Token::InterpolatedExpr(expr_str) => {
                            // Add semicolon so sub-parser can parse as a statement
                            let mut sub_parser = Parser::new(format!("{};", expr_str));
                            if let Ok(program) = sub_parser.parse() {
                                let expr = program.statements.into_iter().next().and_then(|node| {
                                    match node {
                                        CfmlNode::Statement(Statement::Expression(es)) => Some(es.expr),
                                        CfmlNode::Expression(expr) => Some(expr),
                                        _ => None,
                                    }
                                });
                                parts.push(expr.unwrap_or(Expression::Empty));
                            } else {
                                // Fallback: treat as identifier
                                parts.push(Expression::Identifier(Identifier {
                                    name: expr_str.trim().to_string(),
                                    location: self.current_location(),
                                }));
                            }
                        }
                        _ => break,
                    }
                }
                self.match_token(&Token::InterpolatedStringEnd);
                Ok(Expression::StringInterpolation(StringInterpolation {
                    parts,
                    location: self.current_location(),
                }))
            }
            Token::Identifier(id) => Ok(Expression::Identifier(Identifier {
                name: id,
                location: self.current_location(),
            })),
            // `throw(...)` in EXPRESSION position. Lucee exposes throw() as a
            // BIF, so idioms like `x ?: throw("msg","type")` are valid (Preside
            // `WebflowSpecLibrary.getWebflow` uses exactly this). The statement
            // form is handled in `parse_statement`; here we mirror its lowering
            // to a VM-intercepted `throw(...)` call, supporting both positional
            // and named args.
            Token::Throw => {
                let loc = self.current_location();
                if self.check(&Token::LParen) {
                    self.consume(&Token::LParen)?;
                    let is_named = matches!(self.peek(0), Token::Identifier(_))
                        && (matches!(self.peek(1), Token::Equal | Token::Colon));
                    let arguments = if is_named {
                        let named = self.parse_throw_named_attrs(true)?;
                        self.throw_named_to_positional(named, loc)
                    } else {
                        self.parse_arguments()?
                    };
                    self.consume(&Token::RParen)?;
                    Ok(Expression::FunctionCall(Box::new(FunctionCall {
                        name: Box::new(Expression::Identifier(Identifier {
                            name: "throw".to_string(),
                            location: loc,
                        })),
                        arguments,
                        location: loc,
                    })))
                } else {
                    // Bare `throw` with no parens — treat as an identifier
                    // (permissive, matching the soft-keyword arms below).
                    Ok(Expression::Identifier(Identifier {
                        name: "throw".to_string(),
                        location: loc,
                    }))
                }
            }
            // CFML soft keywords used as variables in expressions
            Token::Component => Ok(Expression::Identifier(Identifier {
                name: "component".to_string(),
                location: self.current_location(),
            })),
            Token::Local => Ok(Expression::Identifier(Identifier {
                name: "local".to_string(),
                location: self.current_location(),
            })),
            Token::Param => Ok(Expression::Identifier(Identifier {
                name: "param".to_string(),
                location: self.current_location(),
            })),
            Token::Output => Ok(Expression::Identifier(Identifier {
                name: "output".to_string(),
                location: self.current_location(),
            })),
            Token::Required => Ok(Expression::Identifier(Identifier {
                name: "required".to_string(),
                location: self.current_location(),
            })),
            Token::Default => Ok(Expression::Identifier(Identifier {
                name: "default".to_string(),
                location: self.current_location(),
            })),
            Token::Include => Ok(Expression::Identifier(Identifier {
                name: "include".to_string(),
                location: self.current_location(),
            })),
            Token::Import => Ok(Expression::Identifier(Identifier {
                name: "import".to_string(),
                location: self.current_location(),
            })),
            Token::Property => Ok(Expression::Identifier(Identifier {
                name: "property".to_string(),
                location: self.current_location(),
            })),
            Token::Abstract => Ok(Expression::Identifier(Identifier {
                name: "abstract".to_string(),
                location: self.current_location(),
            })),
            Token::Final => Ok(Expression::Identifier(Identifier {
                name: "final".to_string(),
                location: self.current_location(),
            })),
            Token::Static => Ok(Expression::Identifier(Identifier {
                name: "static".to_string(),
                location: self.current_location(),
            })),
            Token::Lock => Ok(Expression::Identifier(Identifier {
                name: "lock".to_string(),
                location: self.current_location(),
            })),
            Token::Private => Ok(Expression::Identifier(Identifier {
                name: "private".to_string(),
                location: self.current_location(),
            })),
            Token::Public => Ok(Expression::Identifier(Identifier {
                name: "public".to_string(),
                location: self.current_location(),
            })),
            Token::Remote => Ok(Expression::Identifier(Identifier {
                name: "remote".to_string(),
                location: self.current_location(),
            })),
            Token::Package => Ok(Expression::Identifier(Identifier {
                name: "package".to_string(),
                location: self.current_location(),
            })),
            Token::Extends => Ok(Expression::Identifier(Identifier {
                name: "extends".to_string(),
                location: self.current_location(),
            })),
            Token::Implements => Ok(Expression::Identifier(Identifier {
                name: "implements".to_string(),
                location: self.current_location(),
            })),
            Token::ImpKeyword => Ok(Expression::Identifier(Identifier {
                name: "imp".to_string(),
                location: self.current_location(),
            })),
            // Error-handling / switch keywords used as ordinary identifiers in
            // EXPRESSION position (Lucee/ACF accept any keyword as a variable
            // name). parse_primary is only reached where a value is expected, so
            // these can't shadow their statement forms. Preside's ErrorLogService
            // does `var catch = arguments.error;` and errorTemplate.cfm reads
            // `catch.message` / `catch.type`. (The `var`-declaration side is
            // handled by parse_var's permissive name extraction.)
            Token::Catch => Ok(Expression::Identifier(Identifier {
                name: "catch".to_string(),
                location: self.current_location(),
            })),
            Token::Try => Ok(Expression::Identifier(Identifier {
                name: "try".to_string(),
                location: self.current_location(),
            })),
            Token::Finally => Ok(Expression::Identifier(Identifier {
                name: "finally".to_string(),
                location: self.current_location(),
            })),
            Token::Rethrow => Ok(Expression::Identifier(Identifier {
                name: "rethrow".to_string(),
                location: self.current_location(),
            })),
            Token::Switch => Ok(Expression::Identifier(Identifier {
                name: "switch".to_string(),
                location: self.current_location(),
            })),
            Token::Case => Ok(Expression::Identifier(Identifier {
                name: "case".to_string(),
                location: self.current_location(),
            })),
            Token::Do => Ok(Expression::Identifier(Identifier {
                name: "do".to_string(),
                location: self.current_location(),
            })),
            Token::This => Ok(Expression::This(This {
                location: self.current_location(),
            })),
            Token::Super => Ok(Expression::Super(Super {
                location: self.current_location(),
            })),
            Token::New => {
                // After `new`, collect a dotted class name: Ident(.Ident)*
                // e.g. `new framework.one()`, `new com.myapp.Service()`
                // Then parse arguments in parens.
                if let Token::Identifier(_) = self.peek(0) {
                    let mut name_parts = Vec::new();
                    if let Token::Identifier(first) = self.advance().token.clone() {
                        name_parts.push(first);
                    }
                    while self.check(&Token::Dot) {
                        // Accept reserved-word segments (public/private/remote/
                        // package/static, …) as ordinary path parts. JVM engines
                        // only reserve these words in modifier/statement position,
                        // never inside a dotted component path after `new`, so
                        // `new pkg.public.Inner()` and `new pkg.Public()` are
                        // valid (GitHub #246). extract_property_name accepts the
                        // full keyword set; guard the dot-consume with the same
                        // set via is_identifier_like_at + the reserved words.
                        let seg_ok = self.is_property_name_at(1);
                        if seg_ok {
                            self.advance(); // consume dot
                            match self.extract_property_name() {
                                Ok(part) => name_parts.push(part),
                                Err(_) => break,
                            }
                        } else {
                            break;
                        }
                    }
                    let class_name = name_parts.join(".");
                    let class = Box::new(Expression::Identifier(Identifier {
                        name: class_name,
                        location: self.current_location(),
                    }));
                    let args = if self.match_token(&Token::LParen) {
                        let a = self.parse_arguments()?;
                        self.consume(&Token::RParen)?;
                        a
                    } else {
                        Vec::new()
                    };
                    Ok(Expression::New(Box::new(NewExpression {
                        class,
                        arguments: args,
                        location: self.current_location(),
                    })))
                } else if self.check(&Token::LParen) {
                    // `new` is a SOFT keyword (Lucee/ACF/BoxLang): with no
                    // component path after it, `new (argumentCollection = {...})`
                    // is a plain call to a function named `new` — the Wheels
                    // model-create shape — not the new-operator. Return the bare
                    // identifier and let parse_call's postfix loop consume the
                    // parens as a normal FunctionCall.
                    Ok(Expression::Identifier(Identifier {
                        name: "new".to_string(),
                        location: self.current_location(),
                    }))
                } else if matches!(
                    self.peek(0),
                    Token::Comma
                        | Token::RParen
                        | Token::RBracket
                        | Token::RBrace
                        | Token::Semicolon
                        | Token::Colon
                        | Token::Eof
                        | Token::Plus
                        | Token::Minus
                        | Token::Star
                        | Token::Slash
                        | Token::Percent
                        | Token::Equal
                        | Token::EqualEqual
                        | Token::BangEqual
                        | Token::Less
                        | Token::Greater
                        | Token::LessEqual
                        | Token::GreaterEqual
                        | Token::Amp
                        | Token::AndKeyword
                        | Token::OrKeyword
                ) {
                    // `new` is followed by a value-terminator or binary operator,
                    // so it's a plain variable named `new` (Lucee/ACF allow this;
                    // Mura/Masa's fileWriter does `var new = FileOpen(...);
                    // FileWrite(new, x)`). Return it as an ordinary identifier.
                    Ok(Expression::Identifier(Identifier {
                        name: "new".to_string(),
                        location: self.current_location(),
                    }))
                } else {
                    // Fallback for new with a non-Identifier class name — a
                    // string (`new "#path#"()`) OR a reserved-word class name
                    // that lexes as a keyword (`new Component()`, `new Query()`).
                    // Parse ONLY the class-name expression as a primary — NOT a
                    // full call — then consume the constructor `(args)` here. If
                    // we used parse_call, its postfix loop would swallow the
                    // constructor parens (calling the class-name string as a
                    // function) and any following `.method()` chain, so
                    // `new "#path#"().setX()` mis-compiled to "(call the string)".
                    // Leaving the chain unconsumed lets the outer postfix loop
                    // apply it to the New result, exactly like `new Ident().m()`.
                    let class = Box::new(self.parse_primary()?);
                    let args = if self.match_token(&Token::LParen) {
                        let a = self.parse_arguments()?;
                        self.consume(&Token::RParen)?;
                        a
                    } else {
                        Vec::new()
                    };
                    Ok(Expression::New(Box::new(NewExpression {
                        class,
                        arguments: args,
                        location: self.current_location(),
                    })))
                }
            }
            Token::Function => self.parse_closure(),
            Token::LParen => {
                // Arrow function check: (params) => expr or regular grouping: (expr)
                // We need to peek ahead to see if this is an arrow function
                // Note: parse_primary() already advanced past the LParen, so self.current points to the next token

                // Look ahead to find the matching `)` and check if `=>` follows.
                // Track bracket depth so default-value expressions
                // (`(a = 1, b = [2]) => ...`) and nested calls don't confuse the
                // scan — we want the close paren that balances the one already
                // consumed, then peek for the fat arrow.
                let mut is_arrow = false;
                {
                    let mut offset = 0;
                    let mut depth: i32 = 1; // the opening `(` is already consumed
                    while self.current + offset < self.tokens.len() {
                        match &self.tokens[self.current + offset].token {
                            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
                            Token::RParen | Token::RBracket | Token::RBrace => {
                                depth -= 1;
                                if depth == 0 {
                                    is_arrow = self.current + offset + 1 < self.tokens.len()
                                        && matches!(
                                            &self.tokens[self.current + offset + 1].token,
                                            Token::FatArrow
                                        );
                                    break;
                                }
                            }
                            _ => {}
                        }
                        offset += 1;
                    }
                }

                if is_arrow {
                    // Parse as arrow function. Reuse the full parameter parser so
                    // default values, type annotations and `required` are honoured
                    // exactly like `function(...)` declarations.
                    let params = self.parse_param_list()?;
                    self.consume(&Token::RParen)?;
                    self.match_token(&Token::FatArrow); // consume =>

                    // Block-bodied arrow function: `(p) => { stmt; stmt; }`
                    // Compile as a closure so statements are supported.
                    if self.check(&Token::LBrace) {
                        let body = self.parse_block()?;
                        return Ok(Expression::Closure(Box::new(Closure {
                            params,
                            body,
                            location: self.current_location(),
                            metadata: Vec::new(),
                        })));
                    }

                    let body = self.parse_expression()?;
                    return Ok(Expression::ArrowFunction(Box::new(ArrowFunction {
                        params,
                        body: Box::new(body),
                        location: self.current_location(),
                    })));
                }

                // Not an arrow function - parse as grouped expression
                let expr = self.parse_expression()?;
                self.consume(&Token::RParen)?;
                Ok(expr)
            }
            Token::LBracket => self.parse_array_literal(),
            Token::LBrace => self.parse_struct_literal(),
            _ => Ok(Expression::Empty),
        }
    }

    fn parse_closure(&mut self) -> Result<Expression, ParseError> {
        // Optional name for named closures
        let _name = if let Token::Identifier(_) = self.peek(0) {
            Some(self.extract_identifier()?)
        } else {
            None
        };

        self.consume(&Token::LParen)?;
        let params = self.parse_param_list()?;
        self.consume(&Token::RParen)?;

        // Capture optional closure metadata attributes (e.g.,
        // `localmode = "classic"`). These appear between the RParen and
        // LBrace. Only literal-string values are stored — anything else is
        // skipped to preserve forward-compat with future attribute kinds.
        let mut metadata: Vec<(String, String)> = Vec::new();
        while !self.check(&Token::LBrace) && !self.is_at_end() {
            if self.is_identifier_like() && matches!(self.peek(1), Token::Equal) {
                let key = self.extract_identifier()?;
                self.advance(); // skip =
                if let Token::String(val) = self.peek(0).clone() {
                    self.advance();
                    metadata.push((key, val));
                } else {
                    // Non-string value — preserve old behaviour and skip.
                    self.parse_expression()?;
                }
            } else {
                break;
            }
        }

        let body = self.parse_block()?;

        Ok(Expression::Closure(Box::new(Closure {
            params,
            body,
            location: self.current_location(),
            metadata,
        })))
    }

    fn parse_array_literal(&mut self) -> Result<Expression, ParseError> {
        let mut elements = Vec::new();

        if !self.check(&Token::RBracket) {
            loop {
                if self.check(&Token::RBracket) {
                    break; // trailing comma
                }
                if self.match_token(&Token::DotDotDot) {
                    let expr = self.parse_expression()?;
                    elements.push(Expression::Spread(Box::new(expr)));
                } else {
                    let expr = self.parse_expression()?;
                    // `[ key: value, ... ]` is an ordered struct literal (Lucee).
                    // We only learn this once we hit the first colon, so hand off
                    // to the bracket-struct parser carrying the key parsed so far.
                    if elements.is_empty() && self.match_token(&Token::Colon) {
                        return self.parse_bracket_struct_literal(expr);
                    }
                    elements.push(expr);
                }
                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }

        self.consume(&Token::RBracket)?;

        Ok(Expression::Array(Array {
            elements,
            location: self.current_location(),
        }))
    }

    /// Finish parsing a bracket-delimited ordered struct literal
    /// (`[ "key": value, ... ]`). `first_key` is the already-parsed key whose
    /// trailing `:` has just been consumed.
    fn parse_bracket_struct_literal(
        &mut self,
        first_key: Expression,
    ) -> Result<Expression, ParseError> {
        let mut pairs = Vec::new();
        let first_value = self.parse_expression()?;
        pairs.push((first_key, first_value));

        while self.match_token(&Token::Comma) {
            if self.check(&Token::RBracket) {
                break; // trailing comma
            }
            if self.match_token(&Token::DotDotDot) {
                let expr = self.parse_expression()?;
                pairs.push((Expression::Spread(Box::new(expr.clone())), expr));
                continue;
            }
            let is_key_eq =
                self.is_identifier_like_at(0) && matches!(self.peek(1), Token::Equal);
            let key = if is_key_eq {
                self.parse_ternary()?
            } else {
                self.parse_expression()?
            };
            if self.match_token(&Token::Colon) || self.match_token(&Token::Equal) {
                let value = self.parse_expression()?;
                pairs.push((key, value));
            } else {
                // Shorthand [x] means [x: x]
                pairs.push((key.clone(), key));
            }
        }

        self.consume(&Token::RBracket)?;

        Ok(Expression::Struct(Struct {
            pairs,
            ordered: true,
            location: self.current_location(),
        }))
    }

    fn parse_struct_literal(&mut self) -> Result<Expression, ParseError> {
        let mut pairs = Vec::new();

        if !self.check(&Token::RBrace) {
            loop {
                if self.check(&Token::RBrace) {
                    break; // trailing comma
                }
                if self.match_token(&Token::DotDotDot) {
                    // Spread: ...expr merges another struct
                    let expr = self.parse_expression()?;
                    // Use a sentinel key to mark this as a spread entry
                    pairs.push((Expression::Spread(Box::new(expr.clone())), expr));
                } else {
                    // Soften `null`/`true`/`false` when they appear as a struct-
                    // literal key (followed by `:` or `=`) — Lucee/ACF/BoxLang
                    // accept them as identifier-style keys here, and the
                    // cfqueryparam compiler emits exactly this shape
                    // (`{value: ..., cfsqltype: ..., null: true}`). Without this
                    // softening, `null:` parses as a Null literal-key, and the
                    // struct ends up missing the "null" entry — so
                    // queryExecute's struct-unwrap can't see null=true.
                    let key = match self.peek(0) {
                        Token::Null if matches!(self.peek(1), Token::Colon | Token::Equal) => {
                            self.advance();
                            Expression::Literal(Literal {
                                value: LiteralValue::String("null".to_string()),
                                location: self.current_location(),
                            })
                        }
                        Token::True if matches!(self.peek(1), Token::Colon | Token::Equal) => {
                            self.advance();
                            Expression::Literal(Literal {
                                value: LiteralValue::String("true".to_string()),
                                location: self.current_location(),
                            })
                        }
                        Token::False if matches!(self.peek(1), Token::Colon | Token::Equal) => {
                            self.advance();
                            Expression::Literal(Literal {
                                value: LiteralValue::String("false".to_string()),
                                location: self.current_location(),
                            })
                        }
                        // In struct literals, `=` is a key-value separator (like
                        // `:`), NOT an assignment operator. parse_expression()
                        // is greedy and would consume `"key" = value` as an
                        // assignment expression (yielding the value, not the
                        // key), so always parse the key at ternary precedence —
                        // that covers identifiers, soft keywords
                        // (`{ component = x }`), string literals
                        // (`{ "default" = x }`), and bracketed/parenthesized
                        // expressions, but stops at `=`/`:`.
                        _ => self.parse_ternary()?,
                    };

                    // Support both : and = for struct initialization
                    if self.match_token(&Token::Colon) || self.match_token(&Token::Equal) {
                        let value = self.parse_expression()?;
                        pairs.push((key, value));
                    } else {
                        // Shorthand {x} means {x: x}
                        pairs.push((key.clone(), key));
                    }
                }

                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }

        self.consume(&Token::RBrace)?;

        Ok(Expression::Struct(Struct {
            pairs,
            ordered: false,
            location: self.current_location(),
        }))
    }
}

/// Try to lower `param name="<dotted.path>['#expr#']" default=<d>;` into:
///
///   if (!structKeyExists(<dotted.path>, <expr>)) {
///       <dotted.path>[<expr>] = <d>;
///   }
///
/// Returns None if the interpolation does not match this exact shape.
/// This is a deliberately narrow optimisation aimed at the Taffy/FW1
/// `mimeExtensions['#ext#']` idiom — anything more general falls through
/// to the runtime `__cfparam` dispatch.
fn try_lower_dynamic_param(
    interp: &StringInterpolation,
    default_val: &Expression,
    loc: SourceLocation,
) -> Option<Statement> {
    // Three accepted shapes:
    //   1. parts.len() == 3 — `<path>['#expr#']` (or "...")
    //   2. parts.len() == 2 — `<path>.#expr#` (interpolated trailing key)
    //   3. parts.len() == 3 — `<path>['#expr#'].<lit>(.<lit>...)` — bracket then dotted literal tail
    let prefix_path: &str;
    let key_part_index: usize;
    let mut trailing_literals: Vec<String> = Vec::new();
    match interp.parts.len() {
        3 => {
            let prefix = match &interp.parts[0] {
                Expression::Literal(Literal { value: LiteralValue::String(s), .. }) => s,
                _ => return None,
            };
            let suffix = match &interp.parts[2] {
                Expression::Literal(Literal { value: LiteralValue::String(s), .. }) => s,
                _ => return None,
            };
            let p = if let Some(p) = prefix.strip_suffix("['") {
                p
            } else if let Some(p) = prefix.strip_suffix("[\"") {
                p
            } else {
                return None;
            };
            // Suffix must start with the matching closing quote+bracket; anything after
            // is a dotted literal tail (shape 3).
            let after_close = if let Some(rest) = suffix.strip_prefix("']") {
                rest
            } else if let Some(rest) = suffix.strip_prefix("\"]") {
                rest
            } else {
                return None;
            };
            if !after_close.is_empty() {
                // Expect `.<ident>(.<ident>)*`
                if !after_close.starts_with('.') {
                    return None;
                }
                for seg in after_close[1..].split('.') {
                    if !is_simple_ident(seg) {
                        return None;
                    }
                    trailing_literals.push(seg.to_string());
                }
            }
            prefix_path = p;
            key_part_index = 1;
        }
        2 => {
            let prefix = match &interp.parts[0] {
                Expression::Literal(Literal { value: LiteralValue::String(s), .. }) => s,
                _ => return None,
            };
            let p = match prefix.strip_suffix('.') {
                Some(p) => p,
                None => return None,
            };
            prefix_path = p;
            key_part_index = 1;
        }
        _ => return None,
    }
    if prefix_path.is_empty() {
        return None;
    }
    // Build a MemberAccess chain from the dotted prefix.
    let segments: Vec<&str> = prefix_path.split('.').collect();
    if segments.iter().any(|s| s.is_empty() || !is_simple_ident(s)) {
        return None;
    }
    let mut base = Expression::Identifier(Identifier {
        name: segments[0].to_string(),
        location: loc,
    });
    for seg in &segments[1..] {
        base = Expression::MemberAccess(Box::new(MemberAccess {
            object: Box::new(base),
            member: (*seg).to_string(),
            null_safe: false,
            location: loc,
        }));
    }
    let key_expr = interp.parts[key_part_index].clone();

    if trailing_literals.is_empty() {
        // Shape 1 & 2: `base[key]` is the leaf.
        // Condition: !structKeyExists(base, key)
        let cond = Expression::UnaryOp(Box::new(UnaryOp {
            operator: UnaryOpType::Not,
            operand: Box::new(Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "structKeyExists".to_string(),
                    location: loc,
                })),
                arguments: vec![base.clone(), key_expr.clone()],
                location: loc,
            }))),
            location: loc,
        }));
        let assign = Statement::Assignment(Assignment {
            target: AssignTarget::ArrayAccess(Box::new(base), Box::new(key_expr)),
            value: default_val.clone(),
            operator: AssignOp::Equal,
            location: loc,
        });
        Some(Statement::If(If {
            condition: cond,
            then_branch: vec![assign],
            else_if: vec![],
            else_branch: None,
            location: loc,
        }))
    } else {
        // Shape 3: `base[key].lit1.lit2...litN` is the leaf.
        // Build expr `base[key].lit1...litN-1` (parent of leaf) and the leaf member name.
        let mut parent = Expression::ArrayAccess(Box::new(ArrayAccess {
            array: Box::new(base),
            index: Box::new(key_expr),
            location: loc,
        }));
        let leaf = trailing_literals.last().unwrap().clone();
        for seg in &trailing_literals[..trailing_literals.len() - 1] {
            parent = Expression::MemberAccess(Box::new(MemberAccess {
                object: Box::new(parent),
                member: seg.clone(),
                null_safe: false,
                location: loc,
            }));
        }
        // Condition: !structKeyExists(parent, "leaf")
        let cond = Expression::UnaryOp(Box::new(UnaryOp {
            operator: UnaryOpType::Not,
            operand: Box::new(Expression::FunctionCall(Box::new(FunctionCall {
                name: Box::new(Expression::Identifier(Identifier {
                    name: "structKeyExists".to_string(),
                    location: loc,
                })),
                arguments: vec![
                    parent.clone(),
                    Expression::Literal(Literal {
                        value: LiteralValue::String(leaf.clone()),
                        location: loc,
                    }),
                ],
                location: loc,
            }))),
            location: loc,
        }));
        // Body: parent.leaf = default
        let assign = Statement::Assignment(Assignment {
            target: AssignTarget::StructAccess(Box::new(parent), leaf),
            value: default_val.clone(),
            operator: AssignOp::Equal,
            location: loc,
        });
        Some(Statement::If(If {
            condition: cond,
            then_branch: vec![assign],
            else_if: vec![],
            else_branch: None,
            location: loc,
        }))
    }
}

fn is_simple_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl TryFrom<CfmlNode> for Statement {
    type Error = ();

    fn try_from(node: CfmlNode) -> Result<Self, Self::Error> {
        match node {
            CfmlNode::Statement(s) => Ok(s),
            _ => Err(()),
        }
    }
}
