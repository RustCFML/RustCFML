//! CFML Token definitions

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Identifier(String),
    Integer(i64),
    Double(f64),
    String(String),
    True,
    False,
    Null,

    // Arithmetic operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Backslash, // Integer division

    // String concatenation
    Amp, // & (string concatenation in CFML)

    // Comparison operators
    Equal,
    EqualEqual,
    EqualEqualEqual,
    Bang,
    BangEqual,
    BangEqualEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,

    // Logical operators
    AmpAmp,
    BarBar,

    // Compound assignment
    PlusEqual,
    MinusEqual,
    StarEqual,
    SlashEqual,
    AmpEqual,
    PercentEqual,
    PlusPlus,
    MinusMinus,

    // Delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Semicolon,
    Colon,
    ColonColon, // :: (static member access, e.g. Component::member)
    Question,
    QuestionDot,  // ?. (null-safe navigation)
    QuestionColon, // ?: (elvis operator)
    QuestionQuestion, // ?? (null coalescing, alias for ?:)
    HashSign,     // # (for string interpolation)

    // Control flow
    If,
    Else,
    ElseIf,
    Switch,
    Case,
    Default,
    For,
    In,
    While,
    Do,
    Break,
    Continue,
    Return,
    Try,
    Catch,
    Throw,
    Rethrow,
    Finally,

    // OOP
    Component,
    Property,
    Function,
    Public,
    Private,
    Remote,
    Package,
    Static,
    Abstract,
    Final,
    Extends,
    Implements,
    New,
    This,
    Super,
    Interface,

    // Variable scope
    Var,
    Local,

    // CFML-specific keywords (case-insensitive operators)
    Contains,
    NotKeyword, // NOT (logical not, keyword form)
    AndKeyword, // AND
    OrKeyword,  // OR
    XorKeyword, // XOR
    EqKeyword,  // EQ
    NeqKeyword, // NEQ
    GtKeyword,  // GT
    GteKeyword, // GTE
    LtKeyword,  // LT
    LteKeyword, // LTE
    ModKeyword, // MOD
    EqvKeyword, // EQV
    ImpKeyword, // IMP
    IsKeyword,  // IS

    // Other
    Lock,
    Include,
    Import,
    Output,
    Arrow,    // ->
    FatArrow, // =>
    Param,    // param keyword
    Required, // required keyword

    // Spread operator
    DotDotDot,  // ... (spread)

    // String interpolation
    InterpolatedStringStart,       // Beginning of interpolated string
    InterpolatedStringEnd,         // End of interpolated string
    InterpolatedExpr(String),      // Expression inside #...#

    Eof,
    Error(String),
}

impl Token {
    pub fn keyword(s: &str) -> Option<Token> {
        match s.to_lowercase().as_str() {
            "if" => Some(Token::If),
            "else" => Some(Token::Else),
            "elseif" => Some(Token::ElseIf),
            "switch" => Some(Token::Switch),
            "case" => Some(Token::Case),
            "default" => Some(Token::Default),
            "for" => Some(Token::For),
            "in" => Some(Token::In),
            "while" => Some(Token::While),
            "do" => Some(Token::Do),
            "break" => Some(Token::Break),
            "continue" => Some(Token::Continue),
            "return" => Some(Token::Return),
            "try" => Some(Token::Try),
            "catch" => Some(Token::Catch),
            "finally" => Some(Token::Finally),
            "throw" => Some(Token::Throw),
            "rethrow" => Some(Token::Rethrow),
            "component" => Some(Token::Component),
            "property" => Some(Token::Property),
            "function" => Some(Token::Function),
            "public" => Some(Token::Public),
            "private" => Some(Token::Private),
            "remote" => Some(Token::Remote),
            "package" => Some(Token::Package),
            "static" => Some(Token::Static),
            "abstract" => Some(Token::Abstract),
            "final" => Some(Token::Final),
            "extends" => Some(Token::Extends),
            "implements" => Some(Token::Implements),
            "new" => Some(Token::New),
            "this" => Some(Token::This),
            "super" => Some(Token::Super),
            "interface" => Some(Token::Interface),
            "var" => Some(Token::Var),
            "local" => Some(Token::Local),
            // `yes`/`no` are NOT boolean literals in CFML script — Lucee/ACF treat
            // a bare `yes`/`no` as an ordinary identifier (variable reference):
            // `x = yes` throws "variable [YES] doesn't exist". Only `true`/`false`
            // are literals. Lexing them as identifiers also keeps them usable as
            // struct-literal KEYS (`{ yes=1, no=2 }` → keys "yes"/"no", not
            // "true"/"false"). String coercion (`isBoolean("yes")`, boolean tag
            // attributes) is handled at the value level, not the lexer.
            "true" => Some(Token::True),
            "false" => Some(Token::False),
            "null" => Some(Token::Null),
            "lock" => Some(Token::Lock),
            "include" => Some(Token::Include),
            "import" => Some(Token::Import),
            "output" => Some(Token::Output),
            "param" => Some(Token::Param),
            "required" => Some(Token::Required),
            // CFML keyword operators (case-insensitive)
            "contains" => Some(Token::Contains),
            "not" => Some(Token::NotKeyword),
            "and" => Some(Token::AndKeyword),
            "or" => Some(Token::OrKeyword),
            "xor" => Some(Token::XorKeyword),
            "eq" => Some(Token::EqKeyword),
            "neq" => Some(Token::NeqKeyword),
            "gt" => Some(Token::GtKeyword),
            "gte" | "ge" => Some(Token::GteKeyword),
            "lt" => Some(Token::LtKeyword),
            "lte" | "le" => Some(Token::LteKeyword),
            "mod" => Some(Token::ModKeyword),
            "eqv" => Some(Token::EqvKeyword),
            "imp" => Some(Token::ImpKeyword),
            "is" => Some(Token::IsKeyword),
            _ => None,
        }
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Identifier(s) => write!(f, "{}", s),
            Token::Integer(i) => write!(f, "{}", i),
            Token::Double(d) => write!(f, "{}", d),
            Token::String(s) => write!(f, "\"{}\"", s),
            Token::True => write!(f, "true"),
            Token::False => write!(f, "false"),
            Token::Null => write!(f, "null"),
            _ => write!(f, "{:?}", self),
        }
    }
}
