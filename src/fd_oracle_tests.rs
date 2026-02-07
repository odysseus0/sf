use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use tempfile::TempDir;

use crate::{
    filter::{Filter, FilterConfig},
    output::OutputStyle,
    test_support,
};

fn oracle_enabled() -> bool {
    std::env::var("SF_FD_ORACLE").ok().as_deref() == Some("1")
}

fn normalize_fd_output(s: &str) -> Vec<String> {
    let mut out = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.trim_end_matches('/').to_string())
        .collect::<Vec<_>>();
    out.sort();
    out
}

fn collect_sf_like(
    root: &Path,
    include_hidden: bool,
    ignore_enabled: bool,
    global_gitignore: Gitignore,
    global_fd_ignore: Option<Gitignore>,
    pattern: &str,
) -> Vec<String> {
    // Keep oracle hermetic: don't read the caller's real global ignore config.
    let mut filter = Filter::new_with_globals(
        FilterConfig {
            cwd: root.to_path_buf(),
            search_base: root.to_path_buf(),
            include_hidden,
            ignore_enabled,
        },
        global_gitignore,
        global_fd_ignore,
    );
    let out_style = OutputStyle::new(root.to_path_buf(), root.to_path_buf(), None);

    let mut out = Vec::new();
    for abs in test_support::enumerate_paths(root) {
        if !test_support::smartcase_basename_contains(&abs, pattern) {
            continue;
        }
        if filter.should_include(&abs) {
            out.push(out_style.render(&abs).to_string_lossy().to_string());
        }
    }
    out.sort();
    out
}

fn find_fd_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SF_FD_BIN") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }

    if let Ok(out) = Command::new("sh")
        .args(["-lc", "command -v fd"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            let p = PathBuf::from(s.trim());
            if p.is_file() {
                return Some(p);
            }
        }
    }

    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".tmp/fd-v10.3.0/target/debug/fd");
    if local.is_file() {
        return Some(local);
    }

    None
}

fn build_fd_from_tmp() -> Option<PathBuf> {
    let fd_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".tmp/fd-v10.3.0");
    if !fd_dir.is_dir() {
        return None;
    }

    let status = Command::new("cargo")
        .args(["build", "-q"])
        .current_dir(&fd_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }

    let p = fd_dir.join("target/debug/fd");
    p.is_file().then_some(p)
}

fn fd_or_skip() -> Option<PathBuf> {
    find_fd_binary().or_else(build_fd_from_tmp)
}

fn run_fd(fd_bin: &Path, root: &Path, args: &[&str], home: &Path, xdg: &Path) -> String {
    let out = Command::new(fd_bin)
        .args(["--color", "never"])
        .current_dir(root)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", xdg)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run fd");
    assert!(
        out.status.success(),
        "fd failed.\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn fd_pattern_args(pattern: &str) -> Vec<String> {
    let pat = if pattern.contains('*') || pattern.contains('?') {
        pattern.to_string()
    } else {
        format!("*{pattern}*")
    };
    vec!["--glob".into(), pat, ".".into()]
}

fn setup_fd_like_tree() -> (TempDir, PathBuf) {
    let tmp = tempfile::Builder::new()
        .prefix("sf-fd-oracle")
        .tempdir()
        .unwrap();
    let root = tmp.path().to_path_buf();

    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

    for d in ["one/two/three", "one/two/three/directory_foo"] {
        fs::create_dir_all(root.join(d)).unwrap();
    }
    for f in [
        "a.foo",
        "one/b.foo",
        "one/two/c.foo",
        "one/two/C.Foo2",
        "one/two/three/d.foo",
        "fdignored.foo",
        "gitignored.foo",
        ".hidden.foo",
        "e1 e2",
    ] {
        let p = root.join(f);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, "x").unwrap();
    }

    fs::write(root.join(".fdignore"), "fdignored.foo\n").unwrap();
    fs::write(root.join(".gitignore"), "gitignored.foo\n").unwrap();

    (tmp, root)
}

#[test]
fn fd_oracle_default_and_hidden_and_no_ignore() {
    if !oracle_enabled() {
        eprintln!("skipping (set SF_FD_ORACLE=1 to enable)");
        return;
    }
    let Some(fd_bin) = fd_or_skip() else {
        eprintln!("skipping (fd not found; set SF_FD_BIN=/path/to/fd or ensure fd is in PATH)");
        return;
    };

    let (_tmp, root) = setup_fd_like_tree();
    let env = tempfile::Builder::new()
        .prefix("sf-fd-oracle-env")
        .tempdir()
        .unwrap();
    let home = env.path().join("home");
    let xdg = env.path().join("xdg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&xdg).unwrap();

    // Default.
    let args = fd_pattern_args("foo");
    let args_ref = args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let fd = normalize_fd_output(&run_fd(&fd_bin, &root, &args_ref, &home, &xdg));
    let sf = collect_sf_like(&root, false, true, Gitignore::empty(), None, "foo");
    assert_eq!(sf, fd);

    // Hidden.
    let mut args = vec!["--hidden".to_string()];
    args.extend(fd_pattern_args("foo"));
    let args_ref = args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let fd = normalize_fd_output(&run_fd(&fd_bin, &root, &args_ref, &home, &xdg));
    let sf = collect_sf_like(&root, true, true, Gitignore::empty(), None, "foo");
    assert_eq!(sf, fd);

    // No ignore (but still not hidden).
    let mut args = vec!["--no-ignore".to_string()];
    args.extend(fd_pattern_args("foo"));
    let args_ref = args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let fd = normalize_fd_output(&run_fd(&fd_bin, &root, &args_ref, &home, &xdg));
    let sf = collect_sf_like(&root, false, false, Gitignore::empty(), None, "foo");
    assert_eq!(sf, fd);
}

#[test]
fn fd_oracle_precedence_fdignore_over_gitignore() {
    if !oracle_enabled() {
        eprintln!("skipping (set SF_FD_ORACLE=1 to enable)");
        return;
    }
    let Some(fd_bin) = fd_or_skip() else {
        eprintln!("skipping (fd not found; set SF_FD_BIN=/path/to/fd or ensure fd is in PATH)");
        return;
    };

    let tmp = tempfile::Builder::new()
        .prefix("sf-fd-oracle")
        .tempdir()
        .unwrap();
    let root = tmp.path().to_path_buf();

    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

    fs::create_dir_all(root.join("inner")).unwrap();
    fs::write(root.join("inner/foo"), "x").unwrap();

    fs::write(root.join("inner/.gitignore"), "foo\n").unwrap();
    fs::write(root.join(".fdignore"), "!foo\n").unwrap();

    let env = tempfile::Builder::new()
        .prefix("sf-fd-oracle-env")
        .tempdir()
        .unwrap();
    let home = env.path().join("home");
    let xdg = env.path().join("xdg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&xdg).unwrap();

    let args = fd_pattern_args("foo");
    let args_ref = args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let fd = normalize_fd_output(&run_fd(&fd_bin, &root, &args_ref, &home, &xdg));
    let sf = collect_sf_like(&root, true, true, Gitignore::empty(), None, "foo");
    assert_eq!(sf, fd);
}

#[test]
fn fd_oracle_global_fd_ignore_lowest_precedence() {
    if !oracle_enabled() {
        eprintln!("skipping (set SF_FD_ORACLE=1 to enable)");
        return;
    }
    let Some(fd_bin) = fd_or_skip() else {
        eprintln!("skipping (fd not found; set SF_FD_BIN=/path/to/fd or ensure fd is in PATH)");
        return;
    };

    let tmp = tempfile::Builder::new()
        .prefix("sf-fd-oracle")
        .tempdir()
        .unwrap();
    let root = tmp.path().to_path_buf();

    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

    fs::write(root.join("foo"), "x").unwrap();
    fs::write(root.join("bar"), "x").unwrap();
    fs::write(root.join(".ignore"), "!foo\n").unwrap();

    let config_dir = tempfile::Builder::new()
        .prefix("sf-fd-config")
        .tempdir()
        .unwrap();
    fs::create_dir_all(config_dir.path().join("fd")).unwrap();
    fs::write(config_dir.path().join("fd/ignore"), "foo\nbar\n").unwrap();

    // fd reads global ignore via XDG_CONFIG_HOME.
    let home = config_dir.path().join("home");
    fs::create_dir_all(&home).unwrap();

    let args = fd_pattern_args("o");
    let args_ref = args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let fd = normalize_fd_output(&run_fd(&fd_bin, &root, &args_ref, &home, config_dir.path()));

    // sf: wire the same global ignore file explicitly.
    let mut builder = GitignoreBuilder::new(&root);
    builder.add(config_dir.path().join("fd/ignore"));
    let global_fd_ignore = builder.build().ok();
    let sf = collect_sf_like(&root, true, true, Gitignore::empty(), global_fd_ignore, "o");

    assert_eq!(sf, fd);
}

#[test]
fn fd_oracle_global_gitignore_inside_repo() {
    if !oracle_enabled() {
        eprintln!("skipping (set SF_FD_ORACLE=1 to enable)");
        return;
    }
    let Some(fd_bin) = fd_or_skip() else {
        eprintln!("skipping (fd not found; set SF_FD_BIN=/path/to/fd or ensure fd is in PATH)");
        return;
    };

    let tmp = tempfile::Builder::new()
        .prefix("sf-fd-oracle")
        .tempdir()
        .unwrap();
    let root = tmp.path().to_path_buf();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(root.join("bar"), "x").unwrap();

    let env = tempfile::Builder::new()
        .prefix("sf-fd-oracle-env")
        .tempdir()
        .unwrap();
    let home = env.path().join("home");
    let xdg = env.path().join("xdg");
    fs::create_dir_all(xdg.join("git")).unwrap();
    fs::write(xdg.join("git/ignore"), "bar\n").unwrap();
    fs::create_dir_all(&home).unwrap();

    let args = fd_pattern_args("a");
    let args_ref = args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let fd = normalize_fd_output(&run_fd(&fd_bin, &root, &args_ref, &home, &xdg));

    let mut builder = GitignoreBuilder::new(&root);
    builder.add(xdg.join("git/ignore"));
    let global_gitignore = builder.build().unwrap_or_else(|_| Gitignore::empty());
    let sf = collect_sf_like(&root, true, true, global_gitignore, None, "a");

    assert_eq!(sf, fd);
}
