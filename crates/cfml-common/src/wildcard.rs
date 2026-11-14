//! The wildcard name filter of `directoryList()`, `directoryCopy()` and
//! `<cfdirectory>`: a port of Lucee's `WildcardPattern` /
//! `WildcardPatternFilter` (lucee.commons.io.res.util), verified against
//! Lucee 7.1.2.
//!
//! - It matches an entry's NAME, never its path.
//! - The pattern is a list split on any of the delimiter characters (`|` by
//!   default); each token is trimmed, empty tokens are dropped, and a name
//!   matching ANY token is accepted.
//! - `*` is any run of characters, `?` exactly one; everything else is literal.
//! - A leading `!` (on the untrimmed pattern) negates the whole list.
//! - Matching is case-sensitive except on Windows, as Lucee's is.
//! - Lucee's matcher never backtracks: each literal after a `*` is taken at its
//!   LEFTMOST occurrence, so `a*b` does not match `abab`. That also makes a
//!   match linear in the number of tokens, whatever the pattern
//!   (`*a*a*a*a*b` included).
//! - Lucee special-cases two-part tokens: `*x` is `endsWith(x)` and `x*` is
//!   `startsWith(x)` with `x` taken literally, so `*?` matches only names that
//!   end in a `?` and `?` alone matches only the name `?`. Kept for parity.

const CASE_SENSITIVE: bool = !cfg!(windows);

/// A compiled name filter.
#[derive(Debug, Clone)]
pub struct WildcardPattern {
    tokens: Vec<Token>,
    /// `false` when the pattern started with `!`.
    include: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Literal(String),
    Any,
    One,
}

#[derive(Debug, Clone)]
enum Token {
    /// Lucee's one- and two-part shortcuts.
    Everything,
    Exact(String),
    Suffix(String),
    Prefix(String),
    /// Anything else, matched left to right without backtracking.
    General(Vec<Part>),
}

impl WildcardPattern {
    /// The filter for a `directoryList()`/`directoryCopy()` string argument:
    /// `None` (no filtering at all) when the pattern is empty or only
    /// whitespace, as Lucee's `UDFFilter.createResourceAndResourceNameFilter`.
    pub fn for_filter(pattern: &str) -> Option<Self> {
        (!pattern.trim().is_empty()).then(|| Self::new(pattern, "|"))
    }

    /// Lucee's `new WildcardPattern(pattern, caseSensitive, delimiters)`. An
    /// empty or whitespace-only `delimiters` means `|`. A pattern with no
    /// tokens left after trimming (`"|"`, `"  "`) matches nothing, or
    /// everything when negated.
    pub fn new(pattern: &str, delimiters: &str) -> Self {
        let (include, body) = match pattern.strip_prefix('!') {
            Some(rest) => (false, rest),
            None => (true, pattern),
        };
        let delimiters = if delimiters.trim().is_empty() { "|" } else { delimiters };
        let tokens = body
            .split(|c| delimiters.contains(c))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(Token::parse)
            .collect();
        Self { tokens, include }
    }

    /// Does `name` pass the filter?
    pub fn matches(&self, name: &str) -> bool {
        let folded;
        let name = if CASE_SENSITIVE {
            name
        } else {
            folded = name.to_lowercase();
            &folded
        };
        let hit = self.tokens.iter().any(|t| t.matches(name));
        hit == self.include
    }
}

impl Token {
    fn parse(token: &str) -> Self {
        let token = if CASE_SENSITIVE { token.to_string() } else { token.to_lowercase() };
        let mut parts = Vec::new();
        let mut literal = String::new();
        for c in token.chars() {
            let wildcard = match c {
                '*' => Part::Any,
                '?' => Part::One,
                _ => {
                    literal.push(c);
                    continue;
                }
            };
            if !literal.is_empty() {
                parts.push(Part::Literal(std::mem::take(&mut literal)));
            }
            parts.push(wildcard);
        }
        if !literal.is_empty() {
            parts.push(Part::Literal(literal));
        }
        // Lucee compares the wildcard characters themselves literally in the
        // shortcuts (`parts[0].equals(input)`, `endsWith(parts[1])`).
        let text = |p: &Part| match p {
            Part::Literal(s) => s.clone(),
            Part::Any => "*".to_string(),
            Part::One => "?".to_string(),
        };
        match parts.as_slice() {
            [Part::Any] => Token::Everything,
            [only] => Token::Exact(text(only)),
            [Part::Any, last] => Token::Suffix(text(last)),
            [first, Part::Any] => Token::Prefix(text(first)),
            _ => Token::General(parts),
        }
    }

    fn matches(&self, name: &str) -> bool {
        match self {
            Token::Everything => true,
            Token::Exact(s) => name == s,
            Token::Suffix(s) => name.ends_with(s.as_str()),
            Token::Prefix(s) => name.starts_with(s.as_str()),
            Token::General(parts) => match_general(name, parts),
        }
    }
}

/// `ParsedPattern.isMatch` past its shortcuts: a literal after a `*` is found
/// at its leftmost occurrence, any other literal must sit exactly at the
/// cursor; `?` steps one character; a trailing `*` takes the rest.
fn match_general(name: &str, parts: &[Part]) -> bool {
    let mut pos = 0;
    let mut after_any = false;
    for part in parts {
        match part {
            Part::Any => after_any = true,
            Part::One => {
                // Lucee steps past the end without failing here: only a later
                // literal or the final length check can reject it, so `a?*`
                // matches `a`. A cursor past the end makes `get(pos..)` None.
                after_any = false;
                pos += name.get(pos..).and_then(|rest| rest.chars().next()).map_or(1, char::len_utf8);
            }
            Part::Literal(lit) => {
                let Some(rest) = name.get(pos..) else { return false };
                if after_any {
                    match rest.find(lit.as_str()) {
                        Some(at) => pos += at + lit.len(),
                        None => return false,
                    }
                } else if rest.starts_with(lit.as_str()) {
                    pos += lit.len();
                } else {
                    return false;
                }
                after_any = false;
            }
        }
    }
    parts.last() == Some(&Part::Any) || pos == name.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pattern: &str, name: &str) -> bool {
        WildcardPattern::new(pattern, "|").matches(name)
    }

    #[test]
    fn literals_and_wildcards() {
        assert!(m("test.txt", "test.txt"));
        assert!(!m("test.txt", "other.txt"));
        assert!(m("*.txt", "a.txt"));
        assert!(!m("*.txt", "a.cfm"));
        assert!(m("test*", "testempty"));
        assert!(m("?.txt", "a.txt"));
        assert!(!m("?.txt", "ab.txt"));
        assert!(!m("?.txt", ".txt"));
        assert!(m("a*b*c", "axxbyyc"));
        assert!(m("a?c*", "abcdef"));
        assert!(!m("a?c*", "ac"));
    }

    #[test]
    fn lists_trimming_and_negation() {
        assert!(m(" *.txt | *.cfm ", "x.cfm"));
        assert!(!m(" *.txt | *.cfm ", "x.cfc"));
        assert!(!m("!*.txt", "a.txt"));
        assert!(m("!*.txt", "a.cfm"));
        // `!` only counts as the very first character.
        assert!(!m(" !*.txt", "a.cfm"));
        assert!(WildcardPattern::new("*.txt,*.cfm", ",").matches("a.cfm"));
    }

    #[test]
    fn empty_patterns() {
        assert!(WildcardPattern::for_filter("").is_none());
        assert!(WildcardPattern::for_filter("   ").is_none());
        assert!(!m("|", "anything"));
        assert!(!m("  ", "anything"));
        assert!(m("!", "anything"));
    }

    #[test]
    fn lucee_quirks() {
        // Never backtracks.
        assert!(!m("a*b", "abab"));
        assert!(m("a*b", "axxb"));
        // `?` alone, and in the two-part shortcuts, is literal.
        assert!(m("?", "?"));
        assert!(!m("?", "a"));
        assert!(!m("*?", "ab"));
        assert!(m("*?", "a?"));
        // A `?` after a `*` pins the next literal to the cursor.
        assert!(m("*?.txt", "a.txt"));
        assert!(!m("*?.txt", "abc.txt"));
        // `?` may step past the end when only a trailing `*` follows.
        assert!(m("a?*", "a"));
        assert!(!m("a??", "a"));
    }

    #[test]
    fn multibyte_names_never_panic() {
        assert!(m("?.txt", "é.txt"));
        assert!(!m("é?", "é"));
        assert!(m("*é*x", "aéx"));
        assert!(!m("a?b", "aé"));
    }

    #[test]
    fn no_exponential_backtracking() {
        let name = "a".repeat(10_000);
        let started = std::time::Instant::now();
        assert!(!m("*a*a*a*a*a*a*a*a*a*a*b", &name));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[cfg(not(windows))]
    #[test]
    fn case_sensitive_off_windows() {
        assert!(m("*.TXT", "B.TXT"));
        assert!(!m("*.TXT", "b.txt"));
    }

    #[cfg(windows)]
    #[test]
    fn case_insensitive_on_windows() {
        assert!(m("*.TXT", "b.txt"));
    }
}
