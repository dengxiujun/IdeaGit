//! M6 PERF-001/003 large unified-diff parser baseline.

use std::time::Instant;

#[test]
#[ignore = "scheduled M6 benchmark"]
fn perf_large_diff_parser_baseline() {
    const LINE_COUNT: usize = 100_000;
    let mut patch = format!(
        "diff --git a/large.txt b/large.txt\n--- a/large.txt\n+++ b/large.txt\n@@ -0,0 +1,{LINE_COUNT} @@\n"
    )
    .into_bytes();
    for index in 0..LINE_COUNT {
        patch.extend_from_slice(format!("+line-{index:06}\n").as_bytes());
    }

    let started = Instant::now();
    let parsed = git_diff::parse(&patch).expect("large patch should parse");
    let elapsed = started.elapsed();
    assert_eq!(parsed.files.len(), 1);
    assert_eq!(parsed.files[0].hunks[0].lines.len(), LINE_COUNT);
    println!("environment.os={}", std::env::consts::OS);
    println!("environment.arch={}", std::env::consts::ARCH);
    println!("dataset.added_lines={LINE_COUNT}");
    println!("dataset.bytes={}", patch.len());
    println!("large_diff_parse_ms={:.3}", elapsed.as_secs_f64() * 1000.0);
    println!("peak_memory=not_collected");
}
