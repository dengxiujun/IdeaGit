//! Immutable unified-diff AST and parser.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub files: Vec<FilePatch>,
    pub binary: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePatch {
    pub old_path: Option<Vec<u8>>,
    pub new_path: Option<Vec<u8>>,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub lines: Vec<Line>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub content: Vec<u8>,
    pub has_newline: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    MissingFileHeader,
    InvalidHunkHeader,
    HunkLineCountMismatch,
}
type PathPair = (Option<Vec<u8>>, Option<Vec<u8>>);
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingFileHeader => "missing file header",
            Self::InvalidHunkHeader => "invalid hunk header",
            Self::HunkLineCountMismatch => "hunk line count mismatch",
        })
    }
}
impl std::error::Error for ParseError {}

/// Parse Git's byte-oriented unified format without converting content to UTF-8.
pub fn parse(input: &[u8]) -> Result<Patch, ParseError> {
    let mut files = Vec::new();
    let mut index = 0;
    while index < input.len() {
        let (line, next) = next_line(input, index);
        if !line.starts_with(b"diff --git ") {
            index = next;
            continue;
        }
        let (old_path, new_path) = parse_git_header(line)?;
        let mut file = FilePatch {
            old_path,
            new_path,
            hunks: Vec::new(),
            binary: false,
        };
        index = next;
        while index < input.len() {
            let (line, following) = next_line(input, index);
            if line.starts_with(b"diff --git ") {
                break;
            }
            if line.starts_with(b"Binary files ") || line == b"GIT binary patch" {
                file.binary = true;
            } else if line.starts_with(b"@@ ") {
                let (hunk, consumed) = parse_hunk(input, index)?;
                file.hunks.push(hunk);
                index = consumed;
                continue;
            }
            index = following;
        }
        files.push(file);
    }
    Ok(Patch {
        binary: files.iter().any(|file| file.binary),
        files,
    })
}
fn next_line(input: &[u8], start: usize) -> (&[u8], usize) {
    let end = input[start..]
        .iter()
        .position(|b| *b == b'\n')
        .map_or(input.len(), |p| start + p);
    let raw = &input[start..end];
    (
        raw.strip_suffix(b"\r").unwrap_or(raw),
        if end < input.len() { end + 1 } else { end },
    )
}
fn parse_git_header(line: &[u8]) -> Result<PathPair, ParseError> {
    let rest = line
        .strip_prefix(b"diff --git ")
        .ok_or(ParseError::MissingFileHeader)?;
    let mut parts = rest.splitn(3, |b| *b == b' ');
    let old = parts.next().ok_or(ParseError::MissingFileHeader)?;
    let new = parts.next().ok_or(ParseError::MissingFileHeader)?;
    Ok((path(old, b"a/"), path(new, b"b/")))
}
fn path(value: &[u8], prefix: &[u8]) -> Option<Vec<u8>> {
    (!value.is_empty() && value != b"/dev/null")
        .then(|| value.strip_prefix(prefix).unwrap_or(value).to_vec())
}
fn parse_hunk(input: &[u8], start: usize) -> Result<(Hunk, usize), ParseError> {
    let (header, mut index) = next_line(input, start);
    let rest = header
        .strip_prefix(b"@@ -")
        .ok_or(ParseError::InvalidHunkHeader)?;
    let mut fields = rest
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty());
    let (old_start, old_count) = range(fields.next().ok_or(ParseError::InvalidHunkHeader)?)?;
    let new_field = fields
        .next()
        .ok_or(ParseError::InvalidHunkHeader)?
        .strip_prefix(b"+")
        .ok_or(ParseError::InvalidHunkHeader)?;
    let (new_start, new_count) = range(new_field)?;
    let mut old = old_start;
    let mut new = new_start;
    let mut lines: Vec<Line> = Vec::new();
    while index < input.len() {
        let (raw, following) = next_line(input, index);
        if raw.starts_with(b"@@ ") || raw.starts_with(b"diff --git ") {
            break;
        }
        if raw == b"\\ No newline at end of file" {
            if let Some(last) = lines.last_mut() {
                last.has_newline = false;
            }
            index = following;
            continue;
        }
        let (kind, content) = match raw.first() {
            Some(b' ') => (LineKind::Context, &raw[1..]),
            Some(b'+') => (LineKind::Added, &raw[1..]),
            Some(b'-') => (LineKind::Removed, &raw[1..]),
            _ => break,
        };
        let line = Line {
            kind,
            old_line: (kind != LineKind::Added).then_some(old),
            new_line: (kind != LineKind::Removed).then_some(new),
            content: content.to_vec(),
            has_newline: true,
        };
        if kind != LineKind::Added {
            old += 1;
        }
        if kind != LineKind::Removed {
            new += 1;
        }
        lines.push(line);
        index = following;
    }
    if old - old_start != old_count || new - new_start != new_count {
        return Err(ParseError::HunkLineCountMismatch);
    }
    Ok((
        Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            lines,
        },
        index,
    ))
}
fn range(value: &[u8]) -> Result<(u32, u32), ParseError> {
    let value = value.trim_ascii();
    let mut parts = value.splitn(2, |b| *b == b',');
    let start = std::str::from_utf8(parts.next().ok_or(ParseError::InvalidHunkHeader)?)
        .ok()
        .and_then(|v| v.parse().ok())
        .ok_or(ParseError::InvalidHunkHeader)?;
    let count = parts.next().map_or(Ok(1), |v| {
        std::str::from_utf8(v)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or(ParseError::InvalidHunkHeader)
    })?;
    Ok((start, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_lines_and_marker() {
        let patch = parse(b"diff --git a/a b/a\n@@ -1,2 +1,2 @@\n-old\n+new\n context\n\\ No newline at end of file\n").expect("valid");
        let h = &patch.files[0].hunks[0];
        assert_eq!(h.lines[0].old_line, Some(1));
        assert_eq!(h.lines[1].new_line, Some(1));
        assert!(!h.lines[2].has_newline);
    }
    #[test]
    fn preserves_binary_and_dev_null() {
        let patch = parse(b"diff --git a/x b/x\nBinary files a/x and b/x differ\n").expect("valid");
        assert!(patch.binary);
        let deletion = parse(b"diff --git a/x /dev/null\n@@ -1 +0,0 @@\n-x\n").expect("valid");
        assert_eq!(deletion.files[0].new_path, None);
    }
    #[test]
    fn rejects_bad_count() {
        assert_eq!(
            parse(b"diff --git a/x b/x\n@@ -1,2 +1,1 @@\n-x\n"),
            Err(ParseError::HunkLineCountMismatch)
        );
    }
}
