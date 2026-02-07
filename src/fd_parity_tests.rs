use std::{
    fs,
    path::{Path, PathBuf},
};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use tempfile::TempDir;

use crate::{
    filter::{Filter, FilterConfig},
    output::{Delimiter, OutputStyle},
    test_support,
};

static DEFAULT_DIRS: &[&str] = &["one/two/three", "one/two/three/directory_foo"];

static DEFAULT_FILES: &[&str] = &[
    "a.foo",
    "one/b.foo",
    "one/two/c.foo",
    "one/two/C.Foo2",
    "one/two/three/d.foo",
    "fdignored.foo",
    "gitignored.foo",
    ".hidden.foo",
    "e1 e2",
];

struct TestTree {
    _tmp: TempDir,
    root: PathBuf,
}

impl TestTree {
    fn new(dirs: &[&str], files: &[&str]) -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("sf-fd-parity")
            .tempdir()
            .unwrap();
        let root = tmp.path().to_path_buf();

        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        for d in dirs {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in files {
            let p = root.join(f);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(p, "x").unwrap();
        }

        // Match fd testenv defaults.
        fs::write(root.join(".fdignore"), "fdignored.foo\n").unwrap();
        fs::write(root.join(".gitignore"), "gitignored.foo\n").unwrap();

        Self { _tmp: tmp, root }
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn remove_git_head(&self) {
        let _ = fs::remove_file(self.root.join(".git/HEAD"));
    }

    fn ensure_git_head(&self) {
        fs::create_dir_all(self.root.join(".git")).unwrap();
        fs::write(self.root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    }

    fn write_file(&self, rel: &str, content: &str) {
        let p = self.root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }
}

fn build_gitignore(root: &Path, patterns: &str) -> Gitignore {
    let ignore_path = root.join("gitignore-fixture");
    fs::write(&ignore_path, patterns).unwrap();
    let mut builder = GitignoreBuilder::new(root);
    builder.add(ignore_path);
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

fn collect_matches(
    root: &Path,
    filter: &mut Filter,
    out_style: &OutputStyle,
    pattern_substr: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for abs_path in test_support::enumerate_paths(root) {
        if !test_support::smartcase_basename_contains(&abs_path, pattern_substr) {
            continue;
        }
        if filter.should_include(&abs_path) {
            let rendered = out_style.render(&abs_path);
            out.push(rendered.to_string_lossy().to_string());
        }
    }
    out.sort();
    out
}

fn make_filter(
    root: &Path,
    include_hidden: bool,
    ignore_enabled: bool,
    global_gitignore: Gitignore,
    global_fd_ignore: Option<Gitignore>,
) -> Filter {
    Filter::new_with_globals(
        FilterConfig {
            cwd: root.to_path_buf(),
            search_base: root.to_path_buf(),
            include_hidden,
            ignore_enabled,
        },
        global_gitignore,
        global_fd_ignore,
    )
}

fn make_out_style(root: &Path) -> OutputStyle {
    OutputStyle::new(root.to_path_buf(), root.to_path_buf(), None)
}

// Port/adapted from fd v10.3.0: `test_hidden`.
#[test]
fn fd_hidden_adapted() {
    let tree = TestTree::new(DEFAULT_DIRS, DEFAULT_FILES);
    let root = tree.root();

    let mut f = make_filter(root, true, true, Gitignore::empty(), None);
    let out_style = make_out_style(root);
    let got = collect_matches(root, &mut f, &out_style, "foo");

    // fd includes a trailing slash for directories; sf prints plain paths.
    assert_eq!(
        got,
        vec![
            ".hidden.foo",
            "a.foo",
            "one/b.foo",
            "one/two/C.Foo2",
            "one/two/c.foo",
            "one/two/three/d.foo",
            "one/two/three/directory_foo",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
    );
}

// Port/adapted from fd v10.3.0: `test_no_ignore`.
#[test]
fn fd_no_ignore_adapted() {
    let tree = TestTree::new(DEFAULT_DIRS, DEFAULT_FILES);
    let root = tree.root();

    // --no-ignore does not imply --hidden.
    let mut f = make_filter(root, false, false, Gitignore::empty(), None);
    let out_style = make_out_style(root);
    let got = collect_matches(root, &mut f, &out_style, "foo");

    assert_eq!(
        got,
        vec![
            "a.foo",
            "fdignored.foo",
            "gitignored.foo",
            "one/b.foo",
            "one/two/C.Foo2",
            "one/two/c.foo",
            "one/two/three/d.foo",
            "one/two/three/directory_foo",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
    );
}

// Port/adapted from fd v10.3.0: `test_gitignore_and_fdignore` (subset we implement).
#[test]
fn fd_gitignore_and_fdignore_adapted() {
    let files = &[
        "ignored-by-nothing",
        "ignored-by-fdignore",
        "ignored-by-gitignore",
        "ignored-by-both",
    ];
    let tree = TestTree::new(&[], files);
    let root = tree.root();

    tree.write_file(".fdignore", "ignored-by-fdignore\nignored-by-both\n");
    tree.write_file(".gitignore", "ignored-by-gitignore\nignored-by-both\n");

    let mut f = make_filter(root, true, true, Gitignore::empty(), None);
    let out_style = make_out_style(root);

    // In fd: `fd ignored` should only show ignored-by-nothing.
    let got = collect_matches(root, &mut f, &out_style, "ignored");
    assert_eq!(got, vec!["ignored-by-nothing".to_string()]);

    // In fd: `--no-ignore` shows everything.
    let mut f = make_filter(root, true, false, Gitignore::empty(), None);
    let got = collect_matches(root, &mut f, &out_style, "ignored");
    assert_eq!(
        got,
        vec![
            "ignored-by-both",
            "ignored-by-fdignore",
            "ignored-by-gitignore",
            "ignored-by-nothing",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
    );
}

// Port/adapted from fd v10.3.0: `test_custom_ignore_precedence`.
#[test]
fn fd_custom_ignore_precedence_adapted() {
    let tree = TestTree::new(&["inner"], &["inner/foo"]);
    let root = tree.root();

    // Ignore 'foo' via .gitignore in the leaf dir.
    tree.write_file("inner/.gitignore", "foo\n");
    // Whitelist 'foo' via .fdignore in root, which should override gitignore.
    tree.write_file(".fdignore", "!foo\n");

    let mut f = make_filter(root, true, true, Gitignore::empty(), None);
    let out_style = make_out_style(root);
    let got = collect_matches(root, &mut f, &out_style, "foo");
    assert_eq!(got, vec!["inner/foo".to_string()]);
}

// Port/adapted from fd v10.3.0: `test_respect_ignore_files` (require git).
#[test]
fn fd_require_git_adapted() {
    let tree = TestTree::new(DEFAULT_DIRS, DEFAULT_FILES);
    let root = tree.root();

    // Not a "real" repo anymore for sf: remove .git/HEAD.
    tree.remove_git_head();

    let mut f = make_filter(root, true, true, Gitignore::empty(), None);
    let out_style = make_out_style(root);
    let got = collect_matches(root, &mut f, &out_style, "foo");

    // fdignored.foo is still ignored by `.fdignore`, but gitignored.foo should re-appear.
    assert!(got.contains(&"gitignored.foo".to_string()));
    assert!(!got.contains(&"fdignored.foo".to_string()));

    // Restore .git/HEAD: gitignored.foo should now be ignored.
    tree.ensure_git_head();
    let mut f = make_filter(root, true, true, Gitignore::empty(), None);
    let got = collect_matches(root, &mut f, &out_style, "foo");
    assert!(!got.contains(&"gitignored.foo".to_string()));
}

#[test]
fn global_gitignore_only_applies_inside_real_repo() {
    let tree = TestTree::new(&[], &["foo", "bar"]);
    let root = tree.root();

    let gg = build_gitignore(root, "bar\n");

    // Outside repo (missing HEAD) => ignore should not apply.
    tree.remove_git_head();
    let mut f = make_filter(root, true, true, gg.clone(), None);
    let out_style = make_out_style(root);
    let got = collect_matches(root, &mut f, &out_style, "a");
    assert!(got.contains(&"bar".to_string()));

    // Inside repo => it should apply.
    tree.ensure_git_head();
    let mut f = make_filter(root, true, true, gg, None);
    let got = collect_matches(root, &mut f, &out_style, "a");
    assert!(!got.contains(&"bar".to_string()));
}

#[test]
fn print0_emits_nul_and_does_not_touch_diagnostics() {
    let mut buf = Vec::new();
    crate::output::write_path(&mut buf, Path::new("x"), Delimiter::Nul).unwrap();
    assert_eq!(buf, b"x\0");
}
