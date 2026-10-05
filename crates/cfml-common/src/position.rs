//! Source code position tracking

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Position {
    /// `u32`, not `usize`: a position sits in every token and every syntax-tree
    /// node (twice, start and end), and halving it shrank a big component's
    /// compile-time allocation. No source file reaches 4 billion lines.
    pub line: u32,
    pub column: u32,
}

impl Position {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line: line as u32, column: column as u32 }
    }

    pub fn start() -> Self {
        Self { line: 1, column: 1 }
    }
}

impl Default for Position {
    fn default() -> Self {
        Self::start()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    pub start: Position,
    pub end: Position,
}

impl SourceLocation {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    pub fn merge(self, other: Self) -> Self {
        Self {
            start: self.start,
            end: other.end,
        }
    }
}

impl Default for SourceLocation {
    fn default() -> Self {
        Self {
            start: Position::default(),
            end: Position::default(),
        }
    }
}
