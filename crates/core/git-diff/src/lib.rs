//! Immutable unified-diff AST and parser.

use std::fmt;

use git_domain::{ObjectId, RepoPath};

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

/// A stable source for a read-only diff.  HEAD, Index and Worktree are deliberately distinct
/// even when their content currently has the same object identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentSource {
    Head,
    Index,
    Worktree,
    Commit(ObjectId),
    Tree(ObjectId),
    ConflictStage(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionPair {
    pub before: ContentSource,
    pub after: ContentSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreeVersionFile {
    pub path: RepoPath,
    pub head: Option<VersionIdentity>,
    pub index: Option<VersionIdentity>,
    pub worktree: Option<VersionIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionIdentity {
    pub source: ContentSource,
    pub oid: Option<ObjectId>,
    pub mode: Option<u32>,
}

impl ThreeVersionFile {
    #[must_use]
    pub fn source(&self, source: ContentSource) -> Option<VersionIdentity> {
        match source {
            ContentSource::Head => self.head,
            ContentSource::Index => self.index,
            ContentSource::Worktree => self.worktree,
            ContentSource::Commit(_) | ContentSource::Tree(_) | ContentSource::ConflictStage(_) => {
                None
            }
        }
    }
}

/// The three comparisons needed by staging UI: HEAD↔Index, Index↔Worktree and HEAD↔Worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreeVersionDocument {
    pub path: RepoPath,
    pub file: ThreeVersionFile,
    pub head_to_index: Option<Patch>,
    pub index_to_worktree: Option<Patch>,
    pub head_to_worktree: Option<Patch>,
}

impl ThreeVersionDocument {
    #[must_use]
    pub fn comparison(&self, pair: VersionPair) -> Option<&Patch> {
        match (pair.before, pair.after) {
            (ContentSource::Head, ContentSource::Index) => self.head_to_index.as_ref(),
            (ContentSource::Index, ContentSource::Worktree) => self.index_to_worktree.as_ref(),
            (ContentSource::Head, ContentSource::Worktree) => self.head_to_worktree.as_ref(),
            _ => None,
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffLineRef {
    pub hunk_id: u32,
    pub line_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignedRowKind {
    Context,
    Added,
    Deleted,
    Modified,
    Spacer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlignedDiffRow {
    pub left: Option<DiffLineRef>,
    pub right: Option<DiffLineRef>,
    pub kind: AlignedRowKind,
}

/// Align one hunk by logical lines.  It does not inspect rendered height or UI scroll offsets.
pub fn align_hunk(hunk_id: u32, hunk: &Hunk) -> Vec<AlignedDiffRow> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < hunk.lines.len() {
        let line = &hunk.lines[index];
        match line.kind {
            LineKind::Context => {
                rows.push(AlignedDiffRow {
                    left: Some(DiffLineRef {
                        hunk_id,
                        line_index: index,
                    }),
                    right: Some(DiffLineRef {
                        hunk_id,
                        line_index: index,
                    }),
                    kind: AlignedRowKind::Context,
                });
                index += 1;
            }
            LineKind::Removed => {
                let start = index;
                while index < hunk.lines.len() && hunk.lines[index].kind == LineKind::Removed {
                    index += 1;
                }
                let removed_end = index;
                let added_start = index;
                while index < hunk.lines.len() && hunk.lines[index].kind == LineKind::Added {
                    index += 1;
                }
                let added_end = index;
                let width = (removed_end - start).max(added_end - added_start);
                for offset in 0..width {
                    let left = (start + offset < removed_end).then_some(DiffLineRef {
                        hunk_id,
                        line_index: start + offset,
                    });
                    let right = (added_start + offset < added_end).then_some(DiffLineRef {
                        hunk_id,
                        line_index: added_start + offset,
                    });
                    rows.push(AlignedDiffRow {
                        left,
                        right,
                        kind: match (left, right) {
                            (Some(_), Some(_)) => AlignedRowKind::Modified,
                            (Some(_), None) => AlignedRowKind::Deleted,
                            (None, Some(_)) => AlignedRowKind::Added,
                            (None, None) => AlignedRowKind::Spacer,
                        },
                    });
                }
            }
            LineKind::Added => {
                rows.push(AlignedDiffRow {
                    left: None,
                    right: Some(DiffLineRef {
                        hunk_id,
                        line_index: index,
                    }),
                    kind: AlignedRowKind::Added,
                });
                index += 1;
            }
        }
    }
    rows
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

    #[test]
    fn aligns_context_and_replacement_lines_by_identity() {
        let patch = parse(
            b"diff --git a/x b/x\n@@ -1,4 +1,3 @@\n same\n-old one\n-old two\n+new one\n tail\n",
        )
        .expect("valid");
        let rows = align_hunk(7, &patch.files[0].hunks[0]);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].kind, AlignedRowKind::Context);
        assert_eq!(rows[1].kind, AlignedRowKind::Modified);
        assert_eq!(
            rows[1].left,
            Some(DiffLineRef {
                hunk_id: 7,
                line_index: 1
            })
        );
        assert_eq!(
            rows[1].right,
            Some(DiffLineRef {
                hunk_id: 7,
                line_index: 3
            })
        );
        assert_eq!(rows[2].kind, AlignedRowKind::Deleted);
        assert_eq!(rows[3].kind, AlignedRowKind::Context);
    }

    #[test]
    fn aligns_pure_addition_and_deletion_with_spacers() {
        let added =
            parse(b"diff --git a/x b/x\n@@ -1,1 +1,3 @@\n base\n+one\n+two\n").expect("valid");
        let rows = align_hunk(1, &added.files[0].hunks[0]);
        assert_eq!(rows[1].kind, AlignedRowKind::Added);
        assert_eq!(rows[2].kind, AlignedRowKind::Added);

        let deleted =
            parse(b"diff --git a/x b/x\n@@ -1,3 +1,1 @@\n base\n-one\n-two\n").expect("valid");
        let rows = align_hunk(2, &deleted.files[0].hunks[0]);
        assert_eq!(rows[1].kind, AlignedRowKind::Deleted);
        assert_eq!(rows[2].kind, AlignedRowKind::Deleted);
    }

    #[test]
    fn keeps_head_index_and_worktree_as_distinct_sources() {
        let path = RepoPath::try_from("file.txt").expect("valid path");
        let file = ThreeVersionFile {
            path: path.clone(),
            head: Some(VersionIdentity {
                source: ContentSource::Head,
                oid: None,
                mode: Some(0o100644),
            }),
            index: Some(VersionIdentity {
                source: ContentSource::Index,
                oid: None,
                mode: Some(0o100644),
            }),
            worktree: Some(VersionIdentity {
                source: ContentSource::Worktree,
                oid: None,
                mode: Some(0o100644),
            }),
        };
        assert_ne!(
            file.head.map(|version| version.source),
            file.index.map(|version| version.source)
        );
        assert_eq!(file.source(ContentSource::Worktree), file.worktree);
        assert_eq!(file.path, path);
    }

    #[test]
    fn selects_only_supported_three_version_comparisons() {
        let patch = parse(b"diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n").expect("valid");
        let path = RepoPath::try_from("x").expect("valid path");
        let document = ThreeVersionDocument {
            path: path.clone(),
            file: ThreeVersionFile {
                path,
                head: None,
                index: None,
                worktree: None,
            },
            head_to_index: Some(patch),
            index_to_worktree: None,
            head_to_worktree: None,
        };
        assert!(
            document
                .comparison(VersionPair {
                    before: ContentSource::Head,
                    after: ContentSource::Index,
                })
                .is_some()
        );
        assert!(
            document
                .comparison(VersionPair {
                    before: ContentSource::Index,
                    after: ContentSource::Head,
                })
                .is_none()
        );
    }
}
