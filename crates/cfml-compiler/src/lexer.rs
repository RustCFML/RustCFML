//! CFML Lexer - Tokenizes CFML source code

use crate::token::Token;
use cfml_common::position::{Position, SourceLocation};

pub struct Lexer {
    source: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
    tokens: Vec<TokenWithLoc>,
    /// Start position of the current token being scanned
    token_start_line: usize,
    token_start_column: usize,
    /// Javadoc (`/** ... */`) comments captured during scanning, each paired
    /// with the index of the token it immediately precedes (`tokens.len()` at
    /// capture time). The parser consults this to attach `@annotations` to the
    /// following component / function / property declaration.
    doc_comments: Vec<(usize, String)>,
}

#[derive(Debug, Clone)]
pub struct TokenWithLoc {
    pub token: Token,
    pub location: SourceLocation,
}

impl Lexer {
    pub fn new(source: String) -> Self {
        Self {
            source: source.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
            tokens: Vec::new(),
            token_start_line: 1,
            token_start_column: 1,
            doc_comments: Vec::new(),
        }
    }

    /// Javadoc comments captured during the most recent `tokenize()`, paired
    /// with the index of the token each precedes.
    pub fn doc_comments(&self) -> &[(usize, String)] {
        &self.doc_comments
    }

    pub fn tokenize(&mut self) -> Vec<TokenWithLoc> {
        while !self.is_at_end() {
            self.scan_token();
        }
        self.tokens.push(TokenWithLoc {
            token: Token::Eof,
            location: SourceLocation::new(
                Position::new(self.line, self.column),
                Position::new(self.line, self.column),
            ),
        });
        self.tokens.clone()
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.source.len()
    }

    fn current(&self) -> char {
        if self.is_at_end() {
            '\0'
        } else {
            self.source[self.pos]
        }
    }

    /// True when the most recently emitted token is something a following `.`
    /// would be member access on (an identifier, a closing `)`/`]`, or a
    /// literal). Used to disambiguate `.5` (leading-dot number, e.g. `x * .5`)
    /// from member access (`q.5`): only when the previous token is NOT one of
    /// these do we treat `.<digit>` as a number literal.
    fn prev_is_member_target(&self) -> bool {
        matches!(
            self.tokens.last().map(|t| &t.token),
            Some(
                Token::Identifier(_)
                    | Token::RParen
                    | Token::RBracket
                    | Token::Integer(_)
                    | Token::Double(_)
                    | Token::String(_)
            )
        )
    }

    fn peek(&self, offset: usize) -> char {
        let idx = self.pos + offset;
        if idx >= self.source.len() {
            '\0'
        } else {
            self.source[idx]
        }
    }

    fn advance(&mut self) -> char {
        let c = self.current();
        self.pos += 1;
        // Count a new line on LF, and on a *bare* CR (classic-Mac endings).
        // In CRLF the CR is skipped here and the following LF does the count,
        // so the pair increments the line exactly once and pos is never
        // double-advanced.
        if c == '\n' || (c == '\r' && self.current() != '\n') {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        c
    }

    fn add_token(&mut self, token: Token) {
        let location = SourceLocation::new(
            Position::new(self.token_start_line, self.token_start_column),
            Position::new(self.line, self.column),
        );
        self.tokens.push(TokenWithLoc { token, location });
    }

    fn scan_token(&mut self) {
        self.token_start_line = self.line;
        self.token_start_column = self.column;
        let c = self.advance();

        match c {
            '(' => self.add_token(Token::LParen),
            ')' => self.add_token(Token::RParen),
            '{' => self.add_token(Token::LBrace),
            '}' => self.add_token(Token::RBrace),
            '[' => self.add_token(Token::LBracket),
            ']' => self.add_token(Token::RBracket),
            ',' => self.add_token(Token::Comma),
            '.' => {
                if self.peek(0) == '.' && self.peek(1) == '.' {
                    self.advance(); // consume second dot
                    self.advance(); // consume third dot
                    self.add_token(Token::DotDotDot);
                } else if self.peek(0).is_ascii_digit() && !self.prev_is_member_target() {
                    // Leading-dot decimal literal (`.5`, `.000001`) — valid CFML
                    // where a value is expected (after an operator, `(`, `,`,
                    // `=`, …). Route through number() so `x * .000001` lexes as
                    // multiplication by 0.000001 instead of a stray Dot + Integer
                    // (Mura/Masa utility.cfc renderFileSize). We only treat it as
                    // a number when the previous token is NOT a member-access
                    // target, so `q.5`-style access still lexes as Dot + number.
                    self.number('.');
                } else {
                    self.add_token(Token::Dot);
                }
            }
            ';' => self.add_token(Token::Semicolon),
            ':' => {
                if self.match_char(':') {
                    self.add_token(Token::ColonColon);
                } else {
                    self.add_token(Token::Colon);
                }
            }
            '^' => self.add_token(Token::Caret),
            '#' => self.add_token(Token::HashSign),
            '\\' => self.add_token(Token::Backslash),

            '?' => {
                if self.match_char('.') {
                    self.add_token(Token::QuestionDot);
                } else if self.match_char(':') {
                    self.add_token(Token::QuestionColon);
                } else if self.match_char('?') {
                    self.add_token(Token::QuestionQuestion);
                } else {
                    self.add_token(Token::Question);
                }
            }

            '+' => {
                if self.match_char('=') {
                    self.add_token(Token::PlusEqual);
                } else if self.match_char('+') {
                    self.add_token(Token::PlusPlus);
                } else {
                    self.add_token(Token::Plus);
                }
            }
            '-' => {
                if self.match_char('=') {
                    self.add_token(Token::MinusEqual);
                } else if self.match_char('-') {
                    self.add_token(Token::MinusMinus);
                } else if self.match_char('>') {
                    self.add_token(Token::Arrow);
                } else {
                    self.add_token(Token::Minus);
                }
            }
            '*' => {
                if self.match_char('=') {
                    self.add_token(Token::StarEqual);
                } else {
                    self.add_token(Token::Star);
                }
            }
            '/' => {
                if self.match_char('/') {
                    self.single_line_comment();
                } else if self.match_char('*') {
                    self.multi_line_comment();
                } else if self.match_char('=') {
                    self.add_token(Token::SlashEqual);
                } else {
                    self.add_token(Token::Slash);
                }
            }
            '%' => {
                if self.match_char('=') {
                    self.add_token(Token::PercentEqual);
                } else {
                    self.add_token(Token::Percent);
                }
            }

            '&' => {
                if self.match_char('&') {
                    self.add_token(Token::AmpAmp);
                } else if self.match_char('=') {
                    self.add_token(Token::AmpEqual);
                } else {
                    self.add_token(Token::Amp); // String concatenation
                }
            }
            '|' => {
                if self.match_char('|') {
                    self.add_token(Token::BarBar);
                }
                // Single | is not a valid CFML operator, ignore
            }

            '=' => {
                if self.match_char('=') {
                    // `===` strict-equality (Lucee/ACF/BoxLang): same-type
                    // equality, no cross-type coercion. `==` stays loose.
                    if self.match_char('=') {
                        self.add_token(Token::EqualEqualEqual);
                    } else {
                        self.add_token(Token::EqualEqual);
                    }
                } else if self.match_char('>') {
                    self.add_token(Token::FatArrow);
                } else {
                    self.add_token(Token::Equal);
                }
            }
            '!' => {
                if self.match_char('=') {
                    if self.match_char('=') {
                        self.add_token(Token::BangEqualEqual);
                    } else {
                        self.add_token(Token::BangEqual);
                    }
                } else {
                    self.add_token(Token::Bang);
                }
            }
            '>' => {
                if self.match_char('=') {
                    self.add_token(Token::GreaterEqual);
                } else {
                    self.add_token(Token::Greater);
                }
            }
            '<' => {
                // CFML tag-style comment `<!--- ... --->`. In tag mode the tag
                // preprocessor strips these, but a script-based `.cfc` body is
                // lexed directly as CFScript, so the comment reaches the lexer.
                // Lucee tolerates these inside script components (Preside's
                // preside-objects use `<!--- properties --->` markers); without
                // this the whole component fails to parse.
                if self.peek(0) == '!'
                    && self.peek(1) == '-'
                    && self.peek(2) == '-'
                    && self.peek(3) == '-'
                {
                    self.advance(); // '!'
                    self.advance(); // '-'
                    self.advance(); // '-'
                    self.advance(); // '-'
                    self.tag_comment();
                } else if self.match_char('=') {
                    self.add_token(Token::LessEqual);
                } else if self.match_char('>') {
                    self.add_token(Token::BangEqual); // <> is != in CFML
                } else {
                    self.add_token(Token::Less);
                }
            }

            '"' => self.string('"'),
            '\'' => self.string('\''),

            '0'..='9' => self.number(c),

            'a'..='z' | 'A'..='Z' | '_' | '$' => self.identifier(c),

            ' ' | '\t' | '\r' | '\n' => {} // Whitespace already handled by advance()

            _ => {}
        }
    }

    fn match_char(&mut self, expected: char) -> bool {
        if self.is_at_end() || self.current() != expected {
            false
        } else {
            self.advance();
            true
        }
    }

    /// Consume a quoted string literal verbatim into `out`, starting at the
    /// opening quote (`current()` is the quote). Handles doubled-quote escapes
    /// (`''`/`""`), `##` hash escapes, and `#...#` interpolation segments whose
    /// expressions may contain further nested strings with their own
    /// interpolation. Leaves the lexer positioned just after the closing quote
    /// (or at EOF if the string is unterminated). Used while scanning the
    /// expression of an outer `#...#` interpolation so a nested string is
    /// captured as one opaque unit.
    fn consume_string_literal_into(&mut self, out: &mut String) {
        let quote = self.current();
        out.push(quote);
        self.advance(); // opening quote
        while !self.is_at_end() {
            let c = self.current();
            if c == quote {
                if self.peek(1) == quote {
                    // Doubled quote — escaped quote, not the end of the string.
                    out.push(quote);
                    out.push(quote);
                    self.advance();
                    self.advance();
                    continue;
                }
                out.push(quote);
                self.advance(); // closing quote
                return;
            }
            if c == '#' {
                if self.peek(1) == '#' {
                    // ## — escaped literal hash.
                    out.push('#');
                    out.push('#');
                    self.advance();
                    self.advance();
                    continue;
                }
                out.push('#');
                self.advance(); // opening #
                self.consume_interpolation_into(out);
                continue;
            }
            out.push(c);
            self.advance();
        }
    }

    /// Consume an interpolation expression into `out`, starting just after the
    /// opening `#`, up to and including the matching depth-0 closing `#`. Nested
    /// `()`/`[]` add depth; nested string literals are consumed whole (which may
    /// recurse back into interpolation) so a `#` inside a nested string is never
    /// mistaken for the closing `#`. Best-effort on EOF (degrades rather than
    /// erroring — the top-level scanner owns unterminated-interpolation errors).
    fn consume_interpolation_into(&mut self, out: &mut String) {
        let mut depth = 0i32;
        let mut expr_buf = String::new(); // tracked for the string-start guard
        while !self.is_at_end() {
            let c = self.current();
            if c == '#' && depth == 0 {
                out.push('#');
                self.advance(); // closing #
                return;
            }
            if (c == '"' || c == '\'') && self.can_start_interpolation_string_literal(&expr_buf) {
                self.consume_string_literal_into(out);
                // A nested string just closed; reflect that for the next guard
                // check (a quote can't immediately follow a string).
                expr_buf.push(c);
                continue;
            }
            if c == '(' || c == '[' {
                depth += 1;
            }
            if c == ')' || c == ']' {
                depth -= 1;
            }
            out.push(c);
            expr_buf.push(c);
            self.advance();
        }
    }

    fn string(&mut self, quote: char) {
        let start_line = self.line;
        let start_column = self.column;

        // Handle #expr# interpolation in both single and double-quoted strings
        {
            let mut parts: Vec<(bool, String)> = Vec::new(); // (is_expr, content)
            let mut current_str = String::new();
            let mut has_interpolation = false;

            while !self.is_at_end() {
                // Check for closing quote (but doubled quote "" is an escape)
                if self.current() == quote {
                    if self.peek(1) == quote {
                        // Doubled quote: "" → literal "
                        current_str.push(quote);
                        self.advance(); // skip first quote
                        self.advance(); // skip second quote
                        continue;
                    } else {
                        break; // End of string
                    }
                }
                if self.current() == '\\' {
                    // CFML does NOT use backslash escape sequences in string
                    // literals — a backslash is always a literal backslash.
                    // (Quotes are escaped by doubling: "" / ''; hashes by ##.)
                    // This matches Lucee/ACF/BoxLang, and lets regex patterns
                    // like "tests(\\|/)$" and Windows paths survive verbatim.
                    current_str.push('\\');
                    self.advance(); // consume the backslash only
                } else if self.current() == '#' && self.peek(1) == '#' {
                    // ## is an escaped # literal
                    current_str.push('#');
                    self.advance();
                    self.advance();
                } else if self.current() == '#' && self.peek(1) == quote {
                    // # immediately before closing quote — treat as literal #
                    current_str.push('#');
                    self.advance();
                } else if self.current() == '#' {
                    // Start of interpolation expression
                    let hash_line = self.line;
                    let hash_column = self.column;
                    has_interpolation = true;
                    if !current_str.is_empty() {
                        parts.push((false, current_str.clone()));
                        current_str.clear();
                    }
                    self.advance(); // skip opening #
                    let mut expr_str = String::new();
                    let mut depth = 0;
                    // Scan to the matching (depth-0) '#'. A lone, unbalanced '#'
                    // must NOT run past the end of the string literal: if we reach
                    // the closing quote (at depth <= 0, i.e. not inside a nested
                    // call) the interpolation was never terminated. Report it at
                    // the opening '#', not at some far-off EOF (GitHub #181).
                    //
                    // Quoted literals inside the interpolation expression are still
                    // part of that expression, even when they use the same quote as
                    // the outer string (e.g. "#flag ? "YES" : ""#"). Consume them
                    // whole so a nested string can't be mistaken for the outer
                    // string's closing quote (GitHub #189).
                    while !self.is_at_end() && !(self.current() == '#' && depth == 0) {
                        if (self.current() == '"' || self.current() == '\'')
                            && self.can_start_interpolation_string_literal(&expr_str)
                        {
                            // Consume the whole nested string literal. It may itself
                            // contain `#...#` interpolation whose expressions hold
                            // further nested strings (e.g. Preside's
                            // `'Right( #fn( 'col' )# - ? )'` inside an outer string),
                            // so this must recurse rather than scan flatly for the
                            // next matching quote.
                            self.consume_string_literal_into(&mut expr_str);
                            continue;
                        }
                        if self.current() == '(' || self.current() == '[' { depth += 1; }
                        if self.current() == ')' || self.current() == ']' { depth -= 1; }
                        if self.current() == quote && depth <= 0 {
                            self.tokens.push(TokenWithLoc {
                                token: Token::Error(format!(
                                    "Unterminated '#' interpolation in string: a '#' opened an interpolation that was never closed before the end of the string literal. Escape a literal hash as '##'."
                                )),
                                location: SourceLocation::new(
                                    Position::new(hash_line, hash_column),
                                    Position::new(self.line, self.column),
                                ),
                            });
                            self.advance(); // consume the closing quote
                            return;
                        }
                        expr_str.push(self.current());
                        self.advance();
                    }
                    if self.is_at_end() {
                        // Ran to EOF without a closing '#'.
                        self.tokens.push(TokenWithLoc {
                            token: Token::Error(format!(
                                "Unterminated '#' interpolation in string: a '#' opened an interpolation that was never closed before end of file. Escape a literal hash as '##'."
                            )),
                            location: SourceLocation::new(
                                Position::new(hash_line, hash_column),
                                Position::new(self.line, self.column),
                            ),
                        });
                        return;
                    }
                    self.advance(); // skip closing #
                    if !expr_str.is_empty() {
                        parts.push((true, expr_str));
                    }
                } else {
                    current_str.push(self.current());
                    self.advance();
                }
            }

            if !self.is_at_end() {
                self.advance(); // closing quote
            }

            if has_interpolation {
                if !current_str.is_empty() {
                    parts.push((false, current_str));
                }
                // Emit InterpolatedStringStart, then parts, then InterpolatedStringEnd
                self.tokens.push(TokenWithLoc {
                    token: Token::InterpolatedStringStart,
                    location: SourceLocation::new(
                        Position::new(start_line, start_column),
                        Position::new(self.line, self.column),
                    ),
                });
                for (is_expr, content) in parts {
                    if is_expr {
                        self.tokens.push(TokenWithLoc {
                            token: Token::InterpolatedExpr(content),
                            location: SourceLocation::new(
                                Position::new(start_line, start_column),
                                Position::new(self.line, self.column),
                            ),
                        });
                    } else {
                        self.tokens.push(TokenWithLoc {
                            token: Token::String(content),
                            location: SourceLocation::new(
                                Position::new(start_line, start_column),
                                Position::new(self.line, self.column),
                            ),
                        });
                    }
                }
                self.tokens.push(TokenWithLoc {
                    token: Token::InterpolatedStringEnd,
                    location: SourceLocation::new(
                        Position::new(start_line, start_column),
                        Position::new(self.line, self.column),
                    ),
                });
            } else {
                // No interpolation, emit as regular string
                self.tokens.push(TokenWithLoc {
                    token: Token::String(current_str),
                    location: SourceLocation::new(
                        Position::new(start_line, start_column),
                        Position::new(self.line, self.column),
                    ),
                });
            }
        }
    }

    /// Decide whether a quote character encountered while scanning an
    /// interpolation expression begins a nested string literal (rather than
    /// closing the outer string). It does when the preceding non-space token
    /// is something a string can legally follow: an open bracket/paren/brace,
    /// a separator, an operator, or a keyword operator (EQ, AND, ...). If the
    /// preceding char is a value-terminator (identifier/number/closing
    /// bracket with no operator between), the quote is treated as the outer
    /// string's closing quote instead. (GitHub #189)
    fn can_start_interpolation_string_literal(&self, expr_str: &str) -> bool {
        let trimmed = expr_str.trim_end();
        let Some(previous) = trimmed.chars().last() else {
            return true;
        };

        if matches!(
            previous,
            '(' | '['
                | '{'
                | ','
                | ':'
                | '?'
                | '='
                | '!'
                | '<'
                | '>'
                | '+'
                | '-'
                | '*'
                | '/'
                | '%'
                | '&'
                | '|'
                | '^'
        ) {
            return true;
        }

        if previous.is_ascii_alphanumeric() || previous == '_' || previous == '$' {
            let word = trimmed
                .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            return matches!(
                word.as_str(),
                "and"
                    | "or"
                    | "xor"
                    | "eq"
                    | "neq"
                    | "ne"
                    | "gt"
                    | "gte"
                    | "lt"
                    | "lte"
                    | "contains"
                    | "is"
                    | "mod"
                    | "eqv"
                    | "imp"
            );
        }

        false
    }

    fn number(&mut self, first: char) {
        let start_column = self.column - 1;
        let mut value = String::new();
        value.push(first);

        while !self.is_at_end() && self.current().is_ascii_digit() {
            value.push(self.current());
            self.advance();
        }

        // Check for decimal point (but not if followed by a letter - could be method call)
        if !self.is_at_end() && self.current() == '.' && self.peek(1).is_ascii_digit() {
            value.push(self.current());
            self.advance();
            while !self.is_at_end() && self.current().is_ascii_digit() {
                value.push(self.current());
                self.advance();
            }
        }

        // Scientific notation
        if !self.is_at_end() && (self.current() == 'e' || self.current() == 'E') {
            value.push(self.current());
            self.advance();
            if !self.is_at_end() && (self.current() == '+' || self.current() == '-') {
                value.push(self.current());
                self.advance();
            }
            while !self.is_at_end() && self.current().is_ascii_digit() {
                value.push(self.current());
                self.advance();
            }
        }

        let token = if value.contains('.') || value.contains('e') || value.contains('E') {
            Token::Double(value.parse().unwrap_or(0.0))
        } else {
            Token::Integer(value.parse().unwrap_or(0))
        };

        self.tokens.push(TokenWithLoc {
            token,
            location: SourceLocation::new(
                Position::new(self.line, start_column),
                Position::new(self.line, self.column),
            ),
        });
    }

    fn identifier(&mut self, first: char) {
        let start_column = self.column - 1;
        let mut value = String::new();
        value.push(first);

        while !self.is_at_end()
            && (self.current().is_ascii_alphanumeric()
                || self.current() == '_'
                || self.current() == '$')
        {
            value.push(self.current());
            self.advance();
        }

        let token = Token::keyword(&value).unwrap_or_else(|| Token::Identifier(value));

        self.tokens.push(TokenWithLoc {
            token,
            location: SourceLocation::new(
                Position::new(self.line, start_column),
                Position::new(self.line, self.column),
            ),
        });
    }

    fn single_line_comment(&mut self) {
        // Stop at either LF or a bare CR. Classic-Mac (CR-only) line endings
        // have no '\n', so a '\n'-only terminator would let a `// ...` comment
        // run to EOF and swallow the rest of the file — including closing
        // braces ("Expected RBrace, found Eof"). ColdBox EventHandler.cfc ships
        // with CR-only endings and 18 line comments.
        while !self.is_at_end() && self.current() != '\n' && self.current() != '\r' {
            self.advance();
        }
    }

    fn tag_comment(&mut self) {
        // Consume through the matching `--->`. CFML tag comments nest, so track
        // depth. The opening `<!---` has already been consumed by the caller.
        let mut depth = 1usize;
        while !self.is_at_end() {
            if self.peek(0) == '<'
                && self.peek(1) == '!'
                && self.peek(2) == '-'
                && self.peek(3) == '-'
                && self.peek(4) == '-'
            {
                self.advance();
                self.advance();
                self.advance();
                self.advance();
                self.advance();
                depth += 1;
            } else if self.peek(0) == '-'
                && self.peek(1) == '-'
                && self.peek(2) == '-'
                && self.peek(3) == '>'
            {
                self.advance();
                self.advance();
                self.advance();
                self.advance();
                depth -= 1;
                if depth == 0 {
                    break;
                }
            } else {
                self.advance();
            }
        }
    }

    fn multi_line_comment(&mut self) {
        // A `/**` (but not the empty `/**/`) opens a javadoc comment whose
        // `@key value` annotations are attached to the following declaration.
        let is_doc = self.current() == '*' && self.peek(1) != '/';
        let mut content = String::new();
        if is_doc {
            self.advance(); // consume the leading '*' of '/**'
        }
        while !self.is_at_end() {
            if self.current() == '*' && self.peek(1) == '/' {
                self.advance();
                self.advance();
                break;
            }
            if is_doc {
                content.push(self.current());
            }
            self.advance();
        }
        if is_doc {
            // Anchor to the index the next emitted token will occupy.
            self.doc_comments.push((self.tokens.len(), content));
        }
    }
}

pub fn tokenize(source: String) -> Vec<TokenWithLoc> {
    let mut lexer = Lexer::new(source);
    lexer.tokenize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lone_hash_error(toks: &[TokenWithLoc]) -> Option<&TokenWithLoc> {
        toks.iter().find(|t| matches!(t.token, Token::Error(_)))
    }

    #[test]
    fn lone_hash_in_string_is_bounded_error_not_run_to_eof() {
        // GitHub #181: a lone '#' must NOT scan past the closing quote.
        let toks = tokenize(r#"foo("a label (GitHub #180)"); bar(1, 2);"#.to_string());
        let err = lone_hash_error(&toks).expect("expected a lexer Error token");
        // Reported at the opening '#', not at EOF.
        assert_eq!(err.location.start.line, 1);
        assert_eq!(err.location.start.column, 22);
        // It must not have devoured the trailing bar(...) call.
        assert!(
            toks.iter().any(|t| matches!(&t.token, Token::Identifier(s) if s == "bar")),
            "interpolation ran past the string and swallowed later tokens"
        );
    }

    #[test]
    fn lone_hash_running_to_eof_is_error() {
        let toks = tokenize(r#"x = "GitHub #180 issue"#.to_string());
        assert!(lone_hash_error(&toks).is_some());
    }

    #[test]
    fn escaped_and_valid_interpolation_do_not_error() {
        // Escaped ## literal, normal #var#, and same-quote nested interpolation.
        for src in [
            r#""escaped ##180 ok""#,
            r#""y is #y# done""#,
            r#""val=#ucase("ab")#!""#,
            // Same-quote nested string inside a bracket subscript in an
            // interpolation — ColdBox RoutingService.cfc:966.
            r#""^#results[ "scriptName" ]#\/""#,
        ] {
            let toks = tokenize(src.to_string());
            assert!(
                lone_hash_error(&toks).is_none(),
                "valid string wrongly flagged: {src}"
            );
        }
    }

    #[test]
    fn same_quote_string_literals_inside_interpolation_do_not_error() {
        for src in [
            r###""value=#flag ? "YES" : ""#""###,
            r###""statement=#flag ? "UNIQUE" : ""# INDEX #name#""###,
            r###""enabled=#status EQ "active" ? "yes" : "no"#""###,
            r###""quoted=#flag ? "A ""quoted"" value" : "fallback"#""###,
            r###""hash=#replace("a##b", "##", "-")#""###,
            r###"'value=#flag ? 'YES' : ''#'"###,
        ] {
            let toks = tokenize(src.to_string());
            assert!(
                lone_hash_error(&toks).is_none(),
                "valid same-quote interpolation wrongly flagged: {src}"
            );
        }
    }
}
