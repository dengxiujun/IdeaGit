//! M6 PERF-001 reproducible real-Git benchmark.
//!
//! Run explicitly with:
//! `cargo test -p git-cli --test m6_perf -- --ignored --nocapture`
//!
//! The test is ignored by default because it creates a 100,000-commit repository and is
//! intended for scheduled/per-release runs, not every PR.

use std::{
    io::Write,
    process::Command,
    time::{Duration, Instant},
};

use git_cli::{GitCliHistoryReader, GitCliStatusReader, TokioProcessRunner};
use git_domain::GitErrorKind;
use git_domain::{LogQuery, OperationId, RepositoryId, SessionTarget, WorktreeId};
use git_fixture::TestRepository;
use git_ports::{
    EnvironmentVariable, ExecutionContext, HistoryReader, RepositoryLocation, StatusOptions,
    StatusReader,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const COMMIT_COUNT: usize = 100_000;
const SAMPLES: usize = 11;

fn context() -> ExecutionContext {
    let (progress, _receiver) = mpsc::channel(8);
    ExecutionContext::new(OperationId::new(), CancellationToken::new(), progress)
}

fn cancelled_context() -> ExecutionContext {
    let (progress, _receiver) = mpsc::channel(8);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    ExecutionContext::new(OperationId::new(), cancellation, progress)
}

fn environment(fixture: &TestRepository) -> Vec<EnvironmentVariable> {
    fixture
        .isolated_environment()
        .into_iter()
        .map(|(name, value)| EnvironmentVariable::public(name, value))
        .collect()
}

fn create_large_history(fixture: &TestRepository) {
    let env = fixture.isolated_environment();
    let mut stream = String::with_capacity(COMMIT_COUNT * 96);
    stream.push_str("reset refs/heads/main\n");
    stream.push_str("commit refs/heads/main\nmark :1\nauthor IdeaGit Benchmark <benchmark@ideagit.invalid> 0 +0000\ncommitter IdeaGit Benchmark <benchmark@ideagit.invalid> 0 +0000\ndata 15\ninitial commit\nM 100644 inline benchmark.txt\ndata 7\nstable\n");
    for index in 1..=COMMIT_COUNT {
        let mark = index + 1;
        let message = format!("benchmark commit {index}");
        stream.push_str(&format!(
            "commit refs/heads/main\nmark :{mark}\nauthor IdeaGit Benchmark <benchmark@ideagit.invalid> 0 +0000\ncommitter IdeaGit Benchmark <benchmark@ideagit.invalid> 0 +0000\ndata {}\n{message}\nfrom :{}\n",
            message.len() + 1,
            mark - 1
        ));
    }
    let mut child = Command::new("git")
        .current_dir(&fixture.worktree)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .arg("fast-import")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("start fast-import");
    let write_result = child
        .stdin
        .take()
        .expect("fast-import stdin")
        .write_all(stream.as_bytes());
    let output = child.wait_with_output().expect("wait for fast-import");
    assert!(
        write_result.is_ok() && output.status.success(),
        "fast-import failed (write={write_result:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn percentile(samples: &mut [Duration], percentile: usize) -> Duration {
    samples.sort_unstable();
    let index = ((samples.len() - 1) * percentile).div_ceil(100);
    samples[index]
}

fn print_environment() {
    let git = Command::new("git")
        .args(["--version"])
        .output()
        .expect("git version");
    println!(
        "environment.git={}",
        String::from_utf8_lossy(&git.stdout).trim()
    );
    println!("environment.os={}", std::env::consts::OS);
    println!("environment.arch={}", std::env::consts::ARCH);
    println!(
        "environment.cpu={}",
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
    println!("environment.disk=temporary filesystem (fixture)");
    println!("environment.peak_memory=not collected by this harness");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "scheduled M6 benchmark; creates a 100,000-commit repository"]
async fn perf_001_large_repository_baseline() {
    print_environment();
    let fixture = TestRepository::init().expect("fixture");
    fixture.configure_local_identity().expect("identity");
    let started = Instant::now();
    create_large_history(&fixture);
    println!("dataset.create_ms={}", started.elapsed().as_millis());

    let location = RepositoryLocation::worktree(
        &fixture.worktree,
        fixture.worktree.join(".git"),
        fixture.worktree.join(".git"),
    );
    let target = SessionTarget::worktree(RepositoryId::new(), WorktreeId::new(), 1);
    let env = environment(&fixture);
    let status_reader = GitCliStatusReader::new(TokioProcessRunner).with_environment(env.clone());
    let history_reader = GitCliHistoryReader::new(TokioProcessRunner, location.clone());

    let mut status_cold = Vec::with_capacity(SAMPLES);
    let mut log_cold = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        status_reader
            .status(&location, StatusOptions::default(), context())
            .await
            .expect("status");
        status_cold.push(start.elapsed());

        let start = Instant::now();
        history_reader
            .log(
                target,
                LogQuery {
                    limit: 100,
                    skip: 0,
                },
                context(),
            )
            .await
            .expect("log");
        log_cold.push(start.elapsed());
    }

    let mut status_warm = Vec::with_capacity(SAMPLES);
    let mut log_warm = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        status_reader
            .status(&location, StatusOptions::default(), context())
            .await
            .expect("status");
        status_warm.push(start.elapsed());

        let start = Instant::now();
        history_reader
            .log(
                target,
                LogQuery {
                    limit: 100,
                    skip: 0,
                },
                context(),
            )
            .await
            .expect("log");
        log_warm.push(start.elapsed());
    }

    for (name, mut cold, mut warm) in [
        ("status", status_cold, status_warm),
        ("log_first_page", log_cold, log_warm),
    ] {
        println!(
            "{name}.first_pass_ms.p50={:.3} p95={:.3}",
            percentile(&mut cold, 50).as_secs_f64() * 1000.0,
            percentile(&mut cold, 95).as_secs_f64() * 1000.0
        );
        println!(
            "{name}.subsequent_pass_ms.p50={:.3} p95={:.3}",
            percentile(&mut warm, 50).as_secs_f64() * 1000.0,
            percentile(&mut warm, 95).as_secs_f64() * 1000.0
        );
    }
    println!("coverage.changes_tree=not_measured_no_production_entry");
    println!("coverage.branch_popup=not_measured_no_production_entry");
    println!("coverage.large_diff=not_measured_in_PERF-001");
    println!("cache.cold_os_cache=not_cleared; first_pass_is_not_strict_cold_cache");
    println!("dataset.commit_count={COMMIT_COUNT}");
    println!("dataset.samples={SAMPLES}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "scheduled M6 benchmark"]
async fn perf_001_cancelled_status_is_bounded() {
    let fixture = TestRepository::init().expect("fixture");
    fixture
        .commit_file("status.txt", b"status\n")
        .expect("commit");
    let location = RepositoryLocation::worktree(
        &fixture.worktree,
        fixture.worktree.join(".git"),
        fixture.worktree.join(".git"),
    );
    let reader =
        GitCliStatusReader::new(TokioProcessRunner).with_environment(environment(&fixture));
    let started = Instant::now();
    let error = reader
        .status(&location, StatusOptions::default(), cancelled_context())
        .await
        .expect_err("cancelled status must fail closed");
    let elapsed = started.elapsed();
    assert_eq!(error.kind, GitErrorKind::Cancelled);
    println!("cancelled_status_ms={:.3}", elapsed.as_secs_f64() * 1000.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "scheduled M6 benchmark"]
async fn perf_001_multi_repository_switch_baseline() {
    let fixtures = (0..3)
        .map(|index| {
            let fixture = TestRepository::init().expect("fixture");
            fixture
                .commit_file(&format!("repo-{index}.txt"), b"repository\n")
                .expect("commit");
            fixture
        })
        .collect::<Vec<_>>();
    let locations = fixtures
        .iter()
        .map(|fixture| {
            RepositoryLocation::worktree(
                &fixture.worktree,
                fixture.worktree.join(".git"),
                fixture.worktree.join(".git"),
            )
        })
        .collect::<Vec<_>>();
    let readers = fixtures
        .iter()
        .map(|fixture| {
            GitCliStatusReader::new(TokioProcessRunner).with_environment(environment(fixture))
        })
        .collect::<Vec<_>>();
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        for (reader, location) in readers.iter().zip(&locations) {
            reader
                .status(location, StatusOptions::default(), context())
                .await
                .expect("status");
        }
        samples.push(started.elapsed());
    }
    println!(
        "multi_repository_status_ms.p50={:.3} p95={:.3}",
        percentile(&mut samples, 50).as_secs_f64() * 1000.0,
        percentile(&mut samples, 95).as_secs_f64() * 1000.0
    );
    println!("multi_repository.count=3");
}
