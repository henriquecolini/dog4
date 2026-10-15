//! Byte spans into source text and the table of loaded sources.

use std::rc::Rc;

/// A half-open byte range `start..end` into one source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// First byte.
    pub start: u32,
    /// One past the last byte.
    pub end: u32,
}

impl Span {
    /// Creates a span from byte offsets.
    pub fn new(start: usize, end: usize) -> Span {
        Span {
            start: start as u32,
            end: end.max(start) as u32,
        }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// Identifies a source registered in a [`SourceMap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceId(pub u32);

/// One loaded source file (or eval'd snippet).
#[derive(Debug)]
pub struct Source {
    /// Display name, e.g. the module id or `<eval>`.
    pub name: Rc<str>,
    /// Full text.
    pub text: Rc<str>,
}

/// Every source loaded by a runtime, so errors can be rendered with context.
#[derive(Debug, Default)]
pub struct SourceMap {
    sources: Vec<Source>,
}

/// A span resolved to human coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// Source display name.
    pub file: Rc<str>,
    /// 1-based line.
    pub line: u32,
    /// 1-based column, in characters.
    pub col: u32,
    /// The full text of the line containing the span start.
    pub line_text: String,
    /// Number of characters to underline (at least 1).
    pub width: u32,
}

impl SourceMap {
    /// Registers a source and returns its id.
    pub fn add(&mut self, name: impl Into<Rc<str>>, text: impl Into<Rc<str>>) -> SourceId {
        self.sources.push(Source {
            name: name.into(),
            text: text.into(),
        });
        SourceId(self.sources.len() as u32 - 1)
    }

    /// Looks up a source.
    pub fn get(&self, id: SourceId) -> &Source {
        &self.sources[id.0 as usize]
    }

    /// Resolves a span to line/column information.
    pub fn locate(&self, id: SourceId, span: Span) -> Location {
        let source = self.get(id);
        let text = &*source.text;
        let start = (span.start as usize).min(text.len());
        let end = (span.end as usize).clamp(start, text.len());
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = text[start..].find('\n').map_or(text.len(), |i| start + i);
        let line = text[..start].matches('\n').count() as u32 + 1;
        let col = text[line_start..start].chars().count() as u32 + 1;
        let width = text[start..end.min(line_end)].chars().count().max(1) as u32;
        Location {
            file: source.name.clone(),
            line,
            col,
            line_text: text[line_start..line_end].to_string(),
            width,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_lines_and_columns() {
        let mut map = SourceMap::default();
        let id = map.add("t", "ab\ncão x\n");
        let loc = map.locate(id, Span::new(8, 9));
        assert_eq!((loc.line, loc.col, loc.width), (2, 5, 1));
        assert_eq!(loc.line_text, "cão x");
        let first = map.locate(id, Span::new(0, 2));
        assert_eq!((first.line, first.col, first.width), (1, 1, 2));
    }

    #[test]
    fn locate_at_end_of_text() {
        let mut map = SourceMap::default();
        let id = map.add("t", "abc");
        let loc = map.locate(id, Span::new(3, 3));
        assert_eq!((loc.line, loc.col, loc.width), (1, 4, 1));
    }
}
