//! SQL tokenizer for the Query-of-Queries engine.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Select,
    From,
    Where,
    And,
    Or,
    Not,
    In,
    Between,
    Like,
    Escape,
    Is,
    Null,
    True,
    False,
    As,
    On,
    Order,
    By,
    Asc,
    Desc,
    Group,
    Having,
    Limit,
    Offset,
    Top,
    Distinct,
    Case,
    When,
    Then,
    Else,
    End,
    Exists,
    Cast,
    Convert,
    // Joins / set ops
    Join,
    Inner,
    Left,
    Right,
    Full,
    Outer,
    Cross,
    Union,
    All,
    // Identifiers & literals
    Identifier(String),
    String(String),
    Number(String),
    NamedParam(String), // :name
    // Symbols
    Eq,    // =
    Neq,   // <>
    Neq2,  // !=
    Lt,    // <
    Lte,   // <=
    Gt,    // >
    Gte,   // >=
    Plus,  // +
    Minus, // -
    Star,  // *
    Slash, // /
    Mod,   // %
    Concat, // ||
    Amp,   // &  (bitwise and)
    Pipe,  // |  (bitwise or)
    Caret, // ^  (bitwise xor)
    LParen, // (
    RParen, // )
    Comma,  // ,
    Dot,    // .
    Param,  // ?
    // Special
    Eof,
    Error(String),
}

/// SQL lexer — tokenizes a SQL string into [`Token`]s.
pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        self.pos += 1;
        c
    }

    /// Skip whitespace and SQL comments (`-- line` and `/* block */`).
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                }
                Some('-') if self.peek_at(1) == Some('-') => {
                    // line comment to end of line
                    while let Some(c) = self.peek() {
                        self.advance();
                        if c == '\n' {
                            break;
                        }
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    self.advance(); // /
                    self.advance(); // *
                    while let Some(c) = self.advance() {
                        if c == '*' && self.peek() == Some('/') {
                            self.advance();
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn read_number(&mut self, first: char) -> Token {
        let mut s = String::new();
        s.push(first);
        let mut seen_dot = first == '.';
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.advance();
            } else if c == '.' && !seen_dot {
                seen_dot = true;
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        Token::Number(s)
    }

    /// Read a `'…'` string literal. Doubled quotes (`''`) are an escaped quote.
    fn read_string(&mut self) -> Token {
        let mut s = String::new();
        while let Some(c) = self.advance() {
            if c == '\'' {
                if self.peek() == Some('\'') {
                    s.push('\'');
                    self.advance();
                } else {
                    return Token::String(s);
                }
            } else {
                s.push(c);
            }
        }
        Token::Error("unterminated string literal".to_string())
    }

    /// Read a quoted identifier delimited by `close` (`"…"`, `` `…` ``, `[…]`).
    /// Doubled closing chars escape one (ANSI `""`).
    fn read_quoted_ident(&mut self, close: char) -> Token {
        let mut s = String::new();
        while let Some(c) = self.advance() {
            if c == close {
                if self.peek() == Some(close) {
                    s.push(close);
                    self.advance();
                } else {
                    return Token::Identifier(s);
                }
            } else {
                s.push(c);
            }
        }
        Token::Error("unterminated quoted identifier".to_string())
    }

    fn read_identifier_or_keyword(&mut self, first: char) -> Token {
        let mut s = String::new();
        s.push(first);
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' || c == '$' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        match s.to_uppercase().as_str() {
            "SELECT" => Token::Select,
            "FROM" => Token::From,
            "WHERE" => Token::Where,
            "AND" => Token::And,
            "OR" => Token::Or,
            "NOT" => Token::Not,
            "IN" => Token::In,
            "BETWEEN" => Token::Between,
            "LIKE" => Token::Like,
            "ESCAPE" => Token::Escape,
            "IS" => Token::Is,
            "NULL" => Token::Null,
            "TRUE" => Token::True,
            "FALSE" => Token::False,
            "AS" => Token::As,
            "ON" => Token::On,
            "ORDER" => Token::Order,
            "BY" => Token::By,
            "ASC" => Token::Asc,
            "DESC" => Token::Desc,
            "GROUP" => Token::Group,
            "HAVING" => Token::Having,
            "LIMIT" => Token::Limit,
            "OFFSET" => Token::Offset,
            "TOP" => Token::Top,
            "DISTINCT" => Token::Distinct,
            "CASE" => Token::Case,
            "WHEN" => Token::When,
            "THEN" => Token::Then,
            "ELSE" => Token::Else,
            "END" => Token::End,
            "EXISTS" => Token::Exists,
            "CAST" => Token::Cast,
            "CONVERT" => Token::Convert,
            "JOIN" => Token::Join,
            "INNER" => Token::Inner,
            "LEFT" => Token::Left,
            "RIGHT" => Token::Right,
            "FULL" => Token::Full,
            "OUTER" => Token::Outer,
            "CROSS" => Token::Cross,
            "UNION" => Token::Union,
            "ALL" => Token::All,
            _ => Token::Identifier(s),
        }
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_trivia();
        let Some(c) = self.advance() else {
            return Token::Eof;
        };

        match c {
            '=' => Token::Eq,
            '<' => match self.peek() {
                Some('>') => {
                    self.advance();
                    Token::Neq
                }
                Some('=') => {
                    self.advance();
                    Token::Lte
                }
                _ => Token::Lt,
            },
            '>' => match self.peek() {
                Some('=') => {
                    self.advance();
                    Token::Gte
                }
                _ => Token::Gt,
            },
            '!' => match self.peek() {
                Some('=') => {
                    self.advance();
                    Token::Neq2
                }
                _ => Token::Error("unexpected '!'".to_string()),
            },
            '+' => Token::Plus,
            '-' => Token::Minus,
            '*' => Token::Star,
            '/' => Token::Slash,
            '%' => Token::Mod,
            '|' => match self.peek() {
                Some('|') => {
                    self.advance();
                    Token::Concat
                }
                _ => Token::Pipe,
            },
            '&' => Token::Amp,
            '^' => Token::Caret,
            '(' => Token::LParen,
            ')' => Token::RParen,
            ',' => Token::Comma,
            '.' => {
                // A dot followed by a digit starts a number (e.g. `.5`).
                if self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    self.read_number('.')
                } else {
                    Token::Dot
                }
            }
            '?' => Token::Param,
            ':' => {
                // Named parameter `:name`.
                let mut name = String::new();
                while let Some(c) = self.peek() {
                    if c.is_alphanumeric() || c == '_' {
                        name.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
                if name.is_empty() {
                    Token::Error("expected name after ':'".to_string())
                } else {
                    Token::NamedParam(name)
                }
            }
            '\'' => self.read_string(),
            '"' => self.read_quoted_ident('"'),
            '`' => self.read_quoted_ident('`'),
            '[' => self.read_quoted_ident(']'),
            c if c.is_ascii_digit() => self.read_number(c),
            c if c.is_alphabetic() || c == '_' => self.read_identifier_or_keyword(c),
            _ => Token::Error(format!("unexpected character '{}'", c)),
        }
    }

}
