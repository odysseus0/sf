use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::path::PathBuf;
use std::process::Stdio;

fn integration_tests_enabled() -> bool {
    std::env::var("SF_INTEGRATION_TESTS").ok().as_deref() == Some("1")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn mdimport_best_effort(path: &std::path::Path) {
    // Best-effort attempt to encourage Spotlight indexing. Don't fail tests if this fails.
    // Keep stdout/stderr quiet to avoid test log noise.
    let _ = std::process::Command::new("mdimport")
        .arg("-i")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[test]
#[cfg(target_os = "macos")]
fn sf_respects_gitignore_and_defaults() {
    if !integration_tests_enabled() {
        eprintln!("skipping (set SF_INTEGRATION_TESTS=1 to enable)");
        return;
    }

    let fixtures = fixtures_dir();
    let repo = fixtures.join("repo");

    mdimport_best_effort(&repo);

    let mut cmd = cargo_bin_cmd!("sf");
    cmd.current_dir(&repo).arg("*.ts");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("src/config.ts\n"))
        .stdout(predicate::str::contains("ignored_dir/junk.ts").not());
}

#[test]
#[cfg(target_os = "macos")]
fn sf_no_ignore_includes_ignored_results() {
    if !integration_tests_enabled() {
        eprintln!("skipping (set SF_INTEGRATION_TESTS=1 to enable)");
        return;
    }

    let fixtures = fixtures_dir();
    let repo = fixtures.join("repo");
    mdimport_best_effort(&repo);

    let mut cmd = cargo_bin_cmd!("sf");
    cmd.current_dir(&repo).args(["-I", "*.ts"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("src/config.ts\n"))
        .stdout(predicate::str::contains("ignored_dir/junk.ts\n"));
}

#[test]
#[cfg(target_os = "macos")]
fn sf_scopes_to_explicit_path() {
    if !integration_tests_enabled() {
        eprintln!("skipping (set SF_INTEGRATION_TESTS=1 to enable)");
        return;
    }

    let fixtures = fixtures_dir();
    let repo = fixtures.join("repo");
    mdimport_best_effort(&repo);

    let mut cmd = cargo_bin_cmd!("sf");
    cmd.current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("*.ts")
        .arg(&repo);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("src/config.ts\n"))
        .stdout(predicate::str::contains("ignored_dir/junk.ts").not());
}

#[test]
#[cfg(target_os = "macos")]
fn sf_nonexistent_pattern_exits_zero_with_no_output() {
    if !integration_tests_enabled() {
        eprintln!("skipping (set SF_INTEGRATION_TESTS=1 to enable)");
        return;
    }

    let fixtures = fixtures_dir();
    let repo = fixtures.join("repo");
    mdimport_best_effort(&repo);

    let mut cmd = cargo_bin_cmd!("sf");
    cmd.current_dir(&repo).arg("definitely-does-not-exist");
    cmd.assert().success().stdout("");
}
