//! Byte-level parser for `git status --porcelain=v2 -z --branch`.

use std::{ffi::OsString, str};

use async_trait::async_trait;
use git_domain::{
    BranchHead, BranchOid, ChangeKind, ConflictKind, FileMode, GitError, GitErrorKind, GitResult,
    ObjectId, OrdinaryChange, RefName, RenamedChange, Similarity, SimilarityKind, StatusData,
    StatusEntry, SubmoduleState, UnmergedChange,
};
use git_ports::{
    EnvironmentVariable, ExecutionContext, ProcessRunner, RepositoryLocation, StatusOptions,
    StatusParseError, StatusParser, StatusReader,
};

use crate::executor::{GitCliCommand, GitCliExecutor, GitCliInvocation, sealed};

/// Strict parser for Git porcelain v2 NUL-delimited output.
#[derive(Clone, Copy, Debug, Default)]
pub struct PorcelainV2StatusParser;

/// Git CLI status reader using porcelain v2 as its only machine interface.
#[derive(Debug)]
pub struct GitCliStatusReader<R> {
    executor: GitCliExecutor<R>,
    environment: Vec<EnvironmentVariable>,
}

impl<R> GitCliStatusReader<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self {
            executor: GitCliExecutor::new(runner),
            environment: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_executable(runner: R, executable: impl Into<OsString>) -> Self {
        Self {
            executor: GitCliExecutor::with_executable(runner, executable),
            environment: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_environment(
        mut self,
        environment: impl IntoIterator<Item = EnvironmentVariable>,
    ) -> Self {
        self.environment = environment.into_iter().collect();
        self
    }
}

#[async_trait]
impl<R: ProcessRunner> StatusReader for GitCliStatusReader<R> {
    #[allow(clippy::result_large_err)]
    async fn status(
        &self,
        location: &RepositoryLocation,
        options: StatusOptions,
        context: ExecutionContext,
    ) -> GitResult<StatusData> {
        let operation_id = context.operation_id;
        let worktree = location.worktree_root().ok_or_else(|| {
            GitError::new(
                GitErrorKind::UnsupportedFeature,
                "worktree status is unavailable for a bare repository",
            )
            .with_operation(operation_id)
        })?;
        let mut environment = vec![
            EnvironmentVariable::public("GIT_OPTIONAL_LOCKS", "0"),
            EnvironmentVariable::public("GIT_TERMINAL_PROMPT", "0"),
        ];
        environment.extend(self.environment.iter().cloned());
        let output = self
            .executor
            .execute(
                StatusCommand {
                    cwd: worktree.to_path_buf(),
                    environment,
                    options,
                },
                context,
            )
            .await?;
        if !output.success {
            return Err(
                GitError::new(GitErrorKind::InvalidRepository, "Git status query failed")
                    .with_operation(operation_id)
                    .with_diagnostic(format!("exit_code={:?}", output.exit_code)),
            );
        }
        PorcelainV2StatusParser
            .parse(&output.stdout)
            .map_err(|error| {
                GitError::new(
                    GitErrorKind::Internal,
                    "Git returned invalid porcelain v2 status",
                )
                .with_operation(operation_id)
                .with_diagnostic(error.to_string())
            })
    }
}

struct StatusCommand {
    cwd: std::path::PathBuf,
    environment: Vec<EnvironmentVariable>,
    options: StatusOptions,
}

impl GitCliCommand for StatusCommand {}

impl sealed::Sealed for StatusCommand {
    fn into_invocation(self) -> GitCliInvocation {
        let mut args = vec![
            OsString::from("-c"),
            OsString::from("core.quotepath=false"),
            OsString::from("--no-optional-locks"),
            OsString::from("status"),
            OsString::from("--porcelain=v2"),
            OsString::from("-z"),
            OsString::from("--branch"),
            OsString::from("--renames"),
            OsString::from(if self.options.include_untracked {
                "--untracked-files=all"
            } else {
                "--untracked-files=no"
            }),
        ];
        if self.options.include_ignored {
            args.push(OsString::from("--ignored=matching"));
        }
        GitCliInvocation::new(None, args)
            .with_cwd(self.cwd)
            .with_environment(self.environment)
    }
}

impl StatusParser for PorcelainV2StatusParser {
    fn parse(&self, raw: &[u8]) -> Result<StatusData, StatusParseError> {
        let mut records = NulRecords::new(raw);
        let mut data = StatusData::default();

        while let Some((offset, record)) = records.next()? {
            let marker = record
                .first()
                .copied()
                .ok_or(StatusParseError::InvalidField {
                    offset,
                    field: "record",
                    reason: "empty NUL-delimited record".to_owned(),
                })?;
            match marker {
                b'#' => parse_header(record, offset, &mut data)?,
                b'1' => data
                    .entries
                    .push(StatusEntry::Ordinary(parse_ordinary(record, offset)?)),
                b'2' => {
                    let original = records.next()?.ok_or(StatusParseError::Truncated {
                        offset: raw.len(),
                        context: "type-2 record original path",
                    })?;
                    data.entries.push(StatusEntry::Renamed(parse_renamed(
                        record, original.1, offset, original.0,
                    )?));
                }
                b'u' => data
                    .entries
                    .push(StatusEntry::Unmerged(parse_unmerged(record, offset)?)),
                b'?' => data.entries.push(StatusEntry::Untracked(parse_simple_path(
                    record,
                    offset,
                    b"? ",
                    "untracked path",
                )?)),
                b'!' => data.entries.push(StatusEntry::Ignored(parse_simple_path(
                    record,
                    offset,
                    b"! ",
                    "ignored path",
                )?)),
                marker => return Err(StatusParseError::UnknownRecord { offset, marker }),
            }
        }
        Ok(data)
    }
}

fn parse_header(
    record: &[u8],
    offset: usize,
    data: &mut StatusData,
) -> Result<(), StatusParseError> {
    let Some(header) = record.strip_prefix(b"# ") else {
        return invalid(offset, "header", "expected '# ' prefix");
    };
    if let Some(value) = header.strip_prefix(b"branch.oid ") {
        set_once(&mut data.branch.oid, "branch.oid", offset)?;
        data.branch.oid = Some(if value == b"(initial)" {
            BranchOid::Initial
        } else {
            BranchOid::Object(parse_required_oid(value, offset, "branch.oid")?)
        });
    } else if let Some(value) = header.strip_prefix(b"branch.head ") {
        set_once(&mut data.branch.head, "branch.head", offset)?;
        data.branch.head = Some(if value == b"(detached)" {
            BranchHead::Detached
        } else {
            BranchHead::Named(parse_ref_name(value, offset, "branch.head")?)
        });
    } else if let Some(value) = header.strip_prefix(b"branch.upstream ") {
        set_once(&mut data.branch.upstream, "branch.upstream", offset)?;
        data.branch.upstream = Some(parse_ref_name(value, offset, "branch.upstream")?);
    } else if let Some(value) = header.strip_prefix(b"branch.ab ") {
        if data.branch.ahead.is_some() || data.branch.behind.is_some() {
            return Err(StatusParseError::DuplicateHeader {
                offset,
                header: "branch.ab",
            });
        }
        let mut fields = value.split(|byte| *byte == b' ');
        let ahead = fields.next().unwrap_or_default();
        let behind = fields.next().unwrap_or_default();
        if fields.next().is_some() {
            return invalid(offset, "branch.ab", "expected exactly two counters");
        }
        data.branch.ahead = Some(parse_prefixed_count(ahead, b'+', offset, "ahead")?);
        data.branch.behind = Some(parse_prefixed_count(behind, b'-', offset, "behind")?);
    }
    Ok(())
}

fn parse_ordinary(record: &[u8], offset: usize) -> Result<OrdinaryChange, StatusParseError> {
    let fields = split_fields::<9>(record, offset, "ordinary record")?;
    require_marker(fields[0], b"1", offset)?;
    let (index_status, worktree_status) = parse_xy(fields[1], offset)?;
    if matches!(
        index_status,
        ChangeKind::Renamed | ChangeKind::Copied | ChangeKind::Unmerged
    ) || matches!(
        worktree_status,
        ChangeKind::Renamed | ChangeKind::Copied | ChangeKind::Unmerged
    ) {
        return invalid(
            offset,
            "ordinary XY",
            "status requires a type-2 or unmerged record",
        );
    }
    Ok(OrdinaryChange {
        path: parse_repo_path(fields[8], offset, "path")?,
        index_status,
        worktree_status,
        submodule: parse_submodule(fields[2], offset)?,
        head_mode: parse_mode(fields[3], offset, "HEAD mode")?,
        index_mode: parse_mode(fields[4], offset, "index mode")?,
        worktree_mode: parse_mode(fields[5], offset, "worktree mode")?,
        head_oid: parse_optional_oid(fields[6], offset, "HEAD OID")?,
        index_oid: parse_optional_oid(fields[7], offset, "index OID")?,
    })
}

fn parse_renamed(
    record: &[u8],
    original_path: &[u8],
    offset: usize,
    original_offset: usize,
) -> Result<RenamedChange, StatusParseError> {
    let fields = split_fields::<10>(record, offset, "type-2 record")?;
    require_marker(fields[0], b"2", offset)?;
    let (index_status, worktree_status) = parse_xy(fields[1], offset)?;
    let similarity = parse_similarity(fields[8], offset)?;
    let expected = match similarity.kind {
        SimilarityKind::Rename => ChangeKind::Renamed,
        SimilarityKind::Copy => ChangeKind::Copied,
    };
    if index_status != expected && worktree_status != expected {
        return invalid(offset, "type-2 XY", "status does not match similarity kind");
    }
    Ok(RenamedChange {
        path: parse_repo_path(fields[9], offset, "target path")?,
        original_path: parse_repo_path(original_path, original_offset, "original path")?,
        index_status,
        worktree_status,
        submodule: parse_submodule(fields[2], offset)?,
        head_mode: parse_mode(fields[3], offset, "HEAD mode")?,
        index_mode: parse_mode(fields[4], offset, "index mode")?,
        worktree_mode: parse_mode(fields[5], offset, "worktree mode")?,
        head_oid: parse_optional_oid(fields[6], offset, "HEAD OID")?,
        index_oid: parse_optional_oid(fields[7], offset, "index OID")?,
        similarity,
    })
}

fn parse_unmerged(record: &[u8], offset: usize) -> Result<UnmergedChange, StatusParseError> {
    let fields = split_fields::<11>(record, offset, "unmerged record")?;
    require_marker(fields[0], b"u", offset)?;
    Ok(UnmergedChange {
        path: parse_repo_path(fields[10], offset, "path")?,
        conflict: parse_conflict(fields[1], offset)?,
        submodule: parse_submodule(fields[2], offset)?,
        stage1_mode: parse_mode(fields[3], offset, "stage-1 mode")?,
        stage2_mode: parse_mode(fields[4], offset, "stage-2 mode")?,
        stage3_mode: parse_mode(fields[5], offset, "stage-3 mode")?,
        worktree_mode: parse_mode(fields[6], offset, "worktree mode")?,
        stage1_oid: parse_optional_oid(fields[7], offset, "stage-1 OID")?,
        stage2_oid: parse_optional_oid(fields[8], offset, "stage-2 OID")?,
        stage3_oid: parse_optional_oid(fields[9], offset, "stage-3 OID")?,
    })
}

fn parse_simple_path(
    record: &[u8],
    offset: usize,
    prefix: &[u8],
    field: &'static str,
) -> Result<git_domain::RepoPath, StatusParseError> {
    let path = record
        .strip_prefix(prefix)
        .ok_or_else(|| StatusParseError::InvalidField {
            offset,
            field,
            reason: "invalid record prefix".to_owned(),
        })?;
    parse_repo_path(path, offset, field)
}

fn parse_xy(raw: &[u8], offset: usize) -> Result<(ChangeKind, ChangeKind), StatusParseError> {
    if raw.len() != 2 {
        return invalid(offset, "XY", "expected exactly two bytes");
    }
    Ok((
        parse_change(raw[0], offset)?,
        parse_change(raw[1], offset + 1)?,
    ))
}

fn parse_change(byte: u8, offset: usize) -> Result<ChangeKind, StatusParseError> {
    match byte {
        b'.' => Ok(ChangeKind::Unmodified),
        b'A' => Ok(ChangeKind::Added),
        b'M' => Ok(ChangeKind::Modified),
        b'D' => Ok(ChangeKind::Deleted),
        b'R' => Ok(ChangeKind::Renamed),
        b'C' => Ok(ChangeKind::Copied),
        b'T' => Ok(ChangeKind::TypeChanged),
        b'U' => Ok(ChangeKind::Unmerged),
        _ => invalid(offset, "XY", "unknown change status"),
    }
}

fn parse_conflict(raw: &[u8], offset: usize) -> Result<ConflictKind, StatusParseError> {
    match raw {
        b"DD" => Ok(ConflictKind::BothDeleted),
        b"AU" => Ok(ConflictKind::AddedByUs),
        b"UD" => Ok(ConflictKind::DeletedByThem),
        b"UA" => Ok(ConflictKind::AddedByThem),
        b"DU" => Ok(ConflictKind::DeletedByUs),
        b"AA" => Ok(ConflictKind::BothAdded),
        b"UU" => Ok(ConflictKind::BothModified),
        _ => invalid(offset, "unmerged XY", "unknown conflict combination"),
    }
}

fn parse_submodule(raw: &[u8], offset: usize) -> Result<SubmoduleState, StatusParseError> {
    match raw {
        b"N..." => Ok(SubmoduleState::NOT_SUBMODULE),
        [b'S', commit, modified, untracked]
            if matches!(commit, b'.' | b'C')
                && matches!(modified, b'.' | b'M')
                && matches!(untracked, b'.' | b'U') =>
        {
            Ok(SubmoduleState {
                is_submodule: true,
                commit_changed: *commit == b'C',
                tracked_content_modified: *modified == b'M',
                untracked_content: *untracked == b'U',
            })
        }
        _ => invalid(offset, "submodule state", "expected N... or S[c][m][u]"),
    }
}

fn parse_mode(
    raw: &[u8],
    offset: usize,
    field: &'static str,
) -> Result<Option<FileMode>, StatusParseError> {
    if raw.len() != 6 || !raw.iter().all(|byte| matches!(byte, b'0'..=b'7')) {
        return invalid(offset, field, "expected six octal digits");
    }
    let text = str::from_utf8(raw).expect("octal ASCII was validated");
    let value = u32::from_str_radix(text, 8)
        .map_err(|error| field_error(offset, field, error.to_string()))?;
    Ok((value != 0).then(|| FileMode::new(value)))
}

fn parse_optional_oid(
    raw: &[u8],
    offset: usize,
    field: &'static str,
) -> Result<Option<ObjectId>, StatusParseError> {
    if !matches!(raw.len(), 40 | 64) {
        return invalid(offset, field, "expected a full SHA-1 or SHA-256 object ID");
    }
    if raw.iter().all(|byte| *byte == b'0') {
        return Ok(None);
    }
    parse_required_oid(raw, offset, field).map(Some)
}

fn parse_required_oid(
    raw: &[u8],
    offset: usize,
    field: &'static str,
) -> Result<ObjectId, StatusParseError> {
    let text =
        str::from_utf8(raw).map_err(|error| field_error(offset, field, error.to_string()))?;
    text.parse()
        .map_err(|error: git_domain::ObjectIdError| field_error(offset, field, error.to_string()))
}

fn parse_similarity(raw: &[u8], offset: usize) -> Result<Similarity, StatusParseError> {
    let (kind, digits) = match raw.split_first() {
        Some((b'R', digits)) => (SimilarityKind::Rename, digits),
        Some((b'C', digits)) => (SimilarityKind::Copy, digits),
        _ => return invalid(offset, "similarity", "expected R<score> or C<score>"),
    };
    let text = str::from_utf8(digits)
        .map_err(|error| field_error(offset, "similarity", error.to_string()))?;
    let percent: u8 = text.parse().map_err(|error: std::num::ParseIntError| {
        field_error(offset, "similarity", error.to_string())
    })?;
    if percent > 100 {
        return invalid(offset, "similarity", "score exceeds 100");
    }
    Ok(Similarity { kind, percent })
}

fn parse_prefixed_count(
    raw: &[u8],
    prefix: u8,
    offset: usize,
    field: &'static str,
) -> Result<u64, StatusParseError> {
    let digits = raw
        .strip_prefix(&[prefix])
        .ok_or_else(|| field_error(offset, field, "missing sign prefix"))?;
    let text =
        str::from_utf8(digits).map_err(|error| field_error(offset, field, error.to_string()))?;
    text.parse()
        .map_err(|error: std::num::ParseIntError| field_error(offset, field, error.to_string()))
}

fn parse_repo_path(
    raw: &[u8],
    offset: usize,
    field: &'static str,
) -> Result<git_domain::RepoPath, StatusParseError> {
    git_domain::RepoPath::new(raw.to_vec())
        .map_err(|error| field_error(offset, field, error.to_string()))
}

fn parse_ref_name(
    raw: &[u8],
    offset: usize,
    field: &'static str,
) -> Result<RefName, StatusParseError> {
    RefName::new(raw.to_vec()).map_err(|error| field_error(offset, field, error.to_string()))
}

fn split_fields<'a, const N: usize>(
    record: &'a [u8],
    offset: usize,
    field: &'static str,
) -> Result<[&'a [u8]; N], StatusParseError> {
    let fields: Vec<_> = record.splitn(N, |byte| *byte == b' ').collect();
    fields.try_into().map_err(|fields: Vec<&[u8]>| {
        field_error(
            offset,
            field,
            format!("expected {N} fields, got {}", fields.len()),
        )
    })
}

fn require_marker(raw: &[u8], expected: &[u8], offset: usize) -> Result<(), StatusParseError> {
    if raw == expected {
        Ok(())
    } else {
        invalid(offset, "record marker", "unexpected marker")
    }
}

fn set_once<T>(
    field: &mut Option<T>,
    header: &'static str,
    offset: usize,
) -> Result<(), StatusParseError> {
    if field.is_some() {
        Err(StatusParseError::DuplicateHeader { offset, header })
    } else {
        Ok(())
    }
}

fn invalid<T>(
    offset: usize,
    field: &'static str,
    reason: impl Into<String>,
) -> Result<T, StatusParseError> {
    Err(field_error(offset, field, reason))
}

fn field_error(offset: usize, field: &'static str, reason: impl Into<String>) -> StatusParseError {
    StatusParseError::InvalidField {
        offset,
        field,
        reason: reason.into(),
    }
}

struct NulRecords<'a> {
    raw: &'a [u8],
    position: usize,
}

impl<'a> NulRecords<'a> {
    const fn new(raw: &'a [u8]) -> Self {
        Self { raw, position: 0 }
    }

    fn next(&mut self) -> Result<Option<(usize, &'a [u8])>, StatusParseError> {
        if self.position == self.raw.len() {
            return Ok(None);
        }
        let offset = self.position;
        let remaining = &self.raw[offset..];
        let length =
            remaining
                .iter()
                .position(|byte| *byte == 0)
                .ok_or(StatusParseError::Truncated {
                    offset,
                    context: "NUL record terminator",
                })?;
        self.position += length + 1;
        Ok(Some((offset, &remaining[..length])))
    }
}

#[cfg(test)]
mod tests {
    use git_domain::{BranchHead, BranchOid, ObjectFormat, OperationId, StatusEntry};
    use git_fixture::TestRepository;
    use git_ports::{
        EnvironmentVariable, ProcessProgress, RepositoryReader, StatusOptions, StatusReader,
    };
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{GitCliRepositoryReader, TokioProcessRunner};

    const OID_A: &str = "0123456789abcdef0123456789abcdef01234567";
    const OID_B: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const OID_C: &str = "fedcba9876543210fedcba9876543210fedcba98";
    const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn payload(records: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
        let mut raw = Vec::new();
        for record in records {
            raw.extend(record);
            raw.push(0);
        }
        raw
    }

    fn context() -> ExecutionContext {
        let (progress, _receiver) = mpsc::channel::<ProcessProgress>(64);
        ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
    }

    fn isolated_environment(fixture: &TestRepository) -> Vec<EnvironmentVariable> {
        fixture
            .isolated_environment()
            .into_iter()
            .map(|(name, value)| EnvironmentVariable::public(name, value))
            .collect()
    }

    async fn location(fixture: &TestRepository) -> RepositoryLocation {
        GitCliRepositoryReader::new(TokioProcessRunner)
            .with_environment(isolated_environment(fixture))
            .discover(fixture.worktree.clone(), context())
            .await
            .expect("fixture repository should be discovered")
    }

    fn path_of(entry: &StatusEntry) -> &[u8] {
        match entry {
            StatusEntry::Ordinary(change) => change.path.as_bytes(),
            StatusEntry::Renamed(change) => change.path.as_bytes(),
            StatusEntry::Unmerged(change) => change.path.as_bytes(),
            StatusEntry::Untracked(path) | StatusEntry::Ignored(path) => path.as_bytes(),
        }
    }

    fn ordinary(path: &[u8]) -> Vec<u8> {
        let mut record = format!("1 M. N... 100644 100755 100755 {OID_A} {OID_B} ").into_bytes();
        record.extend_from_slice(path);
        record
    }

    #[test]
    fn parses_headers_all_record_kinds_and_raw_paths() {
        let mut renamed =
            format!("2 R. SC.U 100644 100644 100644 {OID_A} {OID_B} R100 ").into_bytes();
        renamed.extend_from_slice(b"new path\twith\nbytes\xff");
        let unmerged =
            format!("u UU N... 100644 100644 100755 100755 {OID_A} {OID_B} {OID_C} conflict.txt")
                .into_bytes();
        let raw = payload([
            format!("# branch.oid {OID_A}").into_bytes(),
            b"# branch.head feature/raw".to_vec(),
            b"# branch.upstream origin/feature/raw".to_vec(),
            b"# branch.ab +3 -2".to_vec(),
            b"# future.header ignored".to_vec(),
            ordinary(b"directory with spaces/file\tname\n"),
            renamed,
            b"old path\twith\nbytes\xfe".to_vec(),
            unmerged,
            b"? untracked file".to_vec(),
            b"! ignored file".to_vec(),
        ]);

        let data = PorcelainV2StatusParser
            .parse(&raw)
            .expect("complete porcelain v2 payload should parse");

        assert!(matches!(data.branch.oid, Some(BranchOid::Object(_))));
        assert!(matches!(data.branch.head, Some(BranchHead::Named(_))));
        assert_eq!(data.branch.ahead, Some(3));
        assert_eq!(data.branch.behind, Some(2));
        assert_eq!(data.entries.len(), 5);

        let StatusEntry::Ordinary(change) = &data.entries[0] else {
            panic!("first record should be ordinary");
        };
        assert_eq!(change.index_status, ChangeKind::Modified);
        assert_eq!(change.worktree_status, ChangeKind::Unmodified);
        assert_eq!(change.head_mode.map(FileMode::get), Some(0o100644));
        assert_eq!(change.index_mode.map(FileMode::get), Some(0o100755));
        assert_eq!(
            change.path.as_bytes(),
            b"directory with spaces/file\tname\n"
        );

        let StatusEntry::Renamed(change) = &data.entries[1] else {
            panic!("second record should be renamed");
        };
        assert_eq!(change.similarity.percent, 100);
        assert_eq!(change.similarity.kind, SimilarityKind::Rename);
        assert_eq!(change.path.as_bytes(), b"new path\twith\nbytes\xff");
        assert_eq!(
            change.original_path.as_bytes(),
            b"old path\twith\nbytes\xfe"
        );
        assert!(change.submodule.is_submodule);
        assert!(change.submodule.commit_changed);
        assert!(change.submodule.untracked_content);

        let StatusEntry::Unmerged(change) = &data.entries[2] else {
            panic!("third record should be unmerged");
        };
        assert_eq!(change.conflict, ConflictKind::BothModified);
        assert_eq!(change.stage3_mode.map(FileMode::get), Some(0o100755));
        assert!(matches!(data.entries[3], StatusEntry::Untracked(_)));
        assert!(matches!(data.entries[4], StatusEntry::Ignored(_)));
    }

    #[test]
    fn parses_unborn_and_sha256_object_ids() {
        let raw = payload([
            b"# branch.oid (initial)".to_vec(),
            b"# branch.head main".to_vec(),
            format!(
                "1 A. N... 000000 100644 100644 {} {SHA256} new.txt",
                "0".repeat(64)
            )
            .into_bytes(),
        ]);

        let data = PorcelainV2StatusParser
            .parse(&raw)
            .expect("SHA-256 and unborn status should parse");
        assert_eq!(data.branch.oid, Some(BranchOid::Initial));
        let StatusEntry::Ordinary(change) = &data.entries[0] else {
            panic!("record should be ordinary");
        };
        assert!(change.head_oid.is_none());
        assert!(change.head_mode.is_none());
        assert_eq!(
            change.index_oid.expect("index OID should exist").format(),
            ObjectFormat::Sha256
        );
    }

    #[test]
    fn rejects_truncation_missing_rename_source_and_unknown_records() {
        let truncated = ordinary(b"file.txt");
        assert!(matches!(
            PorcelainV2StatusParser.parse(&truncated),
            Err(StatusParseError::Truncated { .. })
        ));

        let rename_without_source =
            payload([
                format!("2 R. N... 100644 100644 100644 {OID_A} {OID_B} R90 renamed.txt")
                    .into_bytes(),
            ]);
        assert!(matches!(
            PorcelainV2StatusParser.parse(&rename_without_source),
            Err(StatusParseError::Truncated {
                context: "type-2 record original path",
                ..
            })
        ));

        assert!(matches!(
            PorcelainV2StatusParser.parse(&payload([b"x future".to_vec()])),
            Err(StatusParseError::UnknownRecord { marker: b'x', .. })
        ));
    }

    #[test]
    fn rejects_malformed_fields_and_unsafe_paths() {
        let cases = [
            format!("1 Z. N... 100644 100644 100644 {OID_A} {OID_B} file"),
            format!("1 M. BAD! 100644 100644 100644 {OID_A} {OID_B} file"),
            format!("1 M. N... 10064x 100644 100644 {OID_A} {OID_B} file"),
            format!("1 M. N... 100644 100644 100644 not-an-oid {OID_B} file"),
            format!("1 M. N... 100644 100644 100644 {OID_A} {OID_B} ../escape"),
            format!("2 R. N... 100644 100644 100644 {OID_A} {OID_B} R101 target"),
            format!("u XX N... 100644 100644 100644 100644 {OID_A} {OID_B} {OID_C} file"),
        ];

        for case in cases {
            let mut records = vec![case.into_bytes()];
            if records[0].starts_with(b"2 ") {
                records.push(b"source".to_vec());
            }
            assert!(
                PorcelainV2StatusParser.parse(&payload(records)).is_err(),
                "malformed case must fail"
            );
        }
    }

    #[test]
    fn rejects_duplicate_known_headers_but_ignores_unknown_headers() {
        let duplicate = payload([
            b"# branch.head main".to_vec(),
            b"# future.one value".to_vec(),
            b"# branch.head other".to_vec(),
        ]);
        assert!(matches!(
            PorcelainV2StatusParser.parse(&duplicate),
            Err(StatusParseError::DuplicateHeader {
                header: "branch.head",
                ..
            })
        ));

        let unknown = payload([b"# future.one value".to_vec()]);
        assert_eq!(
            PorcelainV2StatusParser
                .parse(&unknown)
                .expect("unknown header should be ignored"),
            StatusData::default()
        );
    }

    #[test]
    fn parses_copy_scores_and_every_conflict_combination() {
        let copied = payload([
            format!("2 C. N... 100644 100644 100644 {OID_A} {OID_B} C75 copied.txt").into_bytes(),
            b"source.txt".to_vec(),
        ]);
        let data = PorcelainV2StatusParser
            .parse(&copied)
            .expect("copy record should parse");
        let StatusEntry::Renamed(change) = &data.entries[0] else {
            panic!("copy uses the type-2 status variant");
        };
        assert_eq!(change.similarity.kind, SimilarityKind::Copy);
        assert_eq!(change.similarity.percent, 75);

        for (xy, expected) in [
            ("DD", ConflictKind::BothDeleted),
            ("AU", ConflictKind::AddedByUs),
            ("UD", ConflictKind::DeletedByThem),
            ("UA", ConflictKind::AddedByThem),
            ("DU", ConflictKind::DeletedByUs),
            ("AA", ConflictKind::BothAdded),
            ("UU", ConflictKind::BothModified),
        ] {
            let raw = payload([format!(
                "u {xy} N... 100644 100644 100644 100644 {OID_A} {OID_B} {OID_C} file"
            )
            .into_bytes()]);
            let parsed = PorcelainV2StatusParser
                .parse(&raw)
                .expect("known conflict combination should parse");
            assert!(matches!(
                &parsed.entries[0],
                StatusEntry::Unmerged(change) if change.conflict == expected
            ));
        }
    }

    #[tokio::test]
    async fn reads_real_fixture_status_with_index_worktree_rename_and_ignored_entries() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("tracked.txt", b"base\n")
            .expect("tracked base should commit");
        fixture
            .commit_file("rename-source.txt", b"rename me\n")
            .expect("rename source should commit");
        fixture
            .commit_file(".gitignore", b"*.ignored\n")
            .expect("ignore rules should commit");

        fixture
            .write_file("tracked.txt", b"modified\n")
            .expect("tracked file should change");
        fixture
            .write_file("staged.txt", b"staged\n")
            .expect("staged file should be written");
        fixture
            .stage_file("staged.txt")
            .expect("new file should stage");
        fixture
            .write_file("mixed.txt", b"index version\n")
            .expect("mixed file should be written");
        fixture
            .stage_file("mixed.txt")
            .expect("mixed file should stage");
        fixture
            .write_file("mixed.txt", b"worktree version\n")
            .expect("mixed file should change after staging");
        fixture
            .rename_file("rename-source.txt", "renamed.txt")
            .expect("tracked file should rename");
        fixture
            .write_file("untracked.txt", b"untracked\n")
            .expect("untracked file should be written");
        fixture
            .write_file("cache.ignored", b"ignored\n")
            .expect("ignored file should be written");

        let location = location(&fixture).await;
        let data = GitCliStatusReader::new(TokioProcessRunner)
            .with_environment(isolated_environment(&fixture))
            .status(
                &location,
                StatusOptions {
                    include_untracked: true,
                    include_ignored: true,
                },
                context(),
            )
            .await
            .expect("real porcelain v2 status should parse");

        assert!(matches!(data.branch.head, Some(BranchHead::Named(_))));
        let tracked = data
            .entries
            .iter()
            .find(|entry| path_of(entry) == b"tracked.txt")
            .expect("tracked modification should appear");
        assert!(matches!(
            tracked,
            StatusEntry::Ordinary(OrdinaryChange {
                index_status: ChangeKind::Unmodified,
                worktree_status: ChangeKind::Modified,
                ..
            })
        ));
        let staged = data
            .entries
            .iter()
            .find(|entry| path_of(entry) == b"staged.txt")
            .expect("staged addition should appear");
        assert!(matches!(
            staged,
            StatusEntry::Ordinary(OrdinaryChange {
                index_status: ChangeKind::Added,
                worktree_status: ChangeKind::Unmodified,
                ..
            })
        ));
        let mixed = data
            .entries
            .iter()
            .find(|entry| path_of(entry) == b"mixed.txt")
            .expect("mixed change should appear");
        assert!(matches!(
            mixed,
            StatusEntry::Ordinary(OrdinaryChange {
                index_status: ChangeKind::Added,
                worktree_status: ChangeKind::Modified,
                ..
            })
        ));
        let renamed = data
            .entries
            .iter()
            .find(|entry| path_of(entry) == b"renamed.txt")
            .expect("rename should appear");
        assert!(matches!(
            renamed,
            StatusEntry::Renamed(RenamedChange { original_path, .. })
                if original_path.as_bytes() == b"rename-source.txt"
        ));
        assert!(data.entries.iter().any(
            |entry| matches!(entry, StatusEntry::Untracked(path) if path.as_bytes() == b"untracked.txt")
        ));
        assert!(data.entries.iter().any(
            |entry| matches!(entry, StatusEntry::Ignored(path) if path.as_bytes() == b"cache.ignored")
        ));
    }

    #[tokio::test]
    async fn reads_real_unmerged_conflict_and_honors_untracked_option() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .create_conflict("conflict.txt", b"base\n", b"current\n", b"other\n")
            .expect("fixture should create a real conflict");
        fixture
            .write_file("untracked.txt", b"untracked\n")
            .expect("untracked file should be written");
        let location = location(&fixture).await;
        let reader = GitCliStatusReader::new(TokioProcessRunner)
            .with_environment(isolated_environment(&fixture));

        let data = reader
            .status(
                &location,
                StatusOptions {
                    include_untracked: false,
                    include_ignored: false,
                },
                context(),
            )
            .await
            .expect("conflict status should parse");

        assert!(data.entries.iter().any(|entry| matches!(
            entry,
            StatusEntry::Unmerged(UnmergedChange {
                conflict: ConflictKind::BothModified,
                path,
                ..
            }) if path.as_bytes() == b"conflict.txt"
        )));
        assert!(
            !data
                .entries
                .iter()
                .any(|entry| matches!(entry, StatusEntry::Untracked(_)))
        );
    }

    #[tokio::test]
    async fn rejects_worktree_status_for_bare_repository() {
        let fixture = TestRepository::init().expect("fixture should initialize");
        fixture
            .commit_file("README.md", b"seed\n")
            .expect("fixture commit should succeed");
        let bare = fixture
            .create_local_remote()
            .expect("bare remote should be created");
        let bare_location = GitCliRepositoryReader::new(TokioProcessRunner)
            .with_environment(isolated_environment(&fixture))
            .discover(bare.path().to_path_buf(), context())
            .await
            .expect("bare repository should be discovered");

        let error = GitCliStatusReader::new(TokioProcessRunner)
            .with_environment(isolated_environment(&fixture))
            .status(&bare_location, StatusOptions::default(), context())
            .await
            .expect_err("bare repository has no worktree status");

        assert_eq!(error.kind, GitErrorKind::UnsupportedFeature);
    }
}
