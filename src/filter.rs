use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

#[derive(Clone, Debug)]
pub struct FilterConfig {
    /// Current working directory of the `sf` invocation.
    pub cwd: PathBuf,
    /// Absolute search root passed to `mdfind -onlyin`.
    pub search_base: PathBuf,
    /// If false, any candidate with a hidden component under `search_base` is excluded.
    pub include_hidden: bool,
    /// If false, ignore matching is completely disabled (but hidden filtering still applies).
    pub ignore_enabled: bool,
}

/// fd-like ignore/hidden filtering applied to a flat stream of Spotlight candidates.
///
/// Key detail: `sf` does not walk the filesystem. To emulate fd's directory pruning
/// semantics, we also evaluate the ignore/hidden status of ancestor directories under
/// `search_base` and cache "walkability" decisions.
pub struct Filter {
    cfg: FilterConfig,

    // Directory -> whether we can "walk into" it (pruning emulation).
    dir_walkable_cache: HashMap<PathBuf, bool>,

    // Directory -> nearest repo root (requires `.git/HEAD`), or None.
    repo_root_cache: HashMap<PathBuf, Option<PathBuf>>,

    // Ignore file caches keyed by directory that contains the ignore file.
    fdignore_by_dir: HashMap<PathBuf, Option<Gitignore>>,
    ignore_by_dir: HashMap<PathBuf, Option<Gitignore>>,
    gitignore_by_dir: HashMap<PathBuf, Option<Gitignore>>,

    // Repo-root keyed caches.
    info_exclude_by_repo: HashMap<PathBuf, Gitignore>,

    global_gitignore: Gitignore,
    global_fd_ignore: Option<Gitignore>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IgnoreDecision {
    Ignore,
    Whitelist,
}

impl IgnoreDecision {
    fn include(self) -> bool {
        matches!(self, IgnoreDecision::Whitelist)
    }
}

impl Filter {
    pub fn new(cfg: FilterConfig) -> Self {
        let (global_gitignore, _err) = GitignoreBuilder::new(&cfg.cwd).build_global();
        let global_fd_ignore = if cfg.ignore_enabled {
            load_global_fd_ignore(&cfg.cwd)
        } else {
            None
        };

        Self::new_with_globals(cfg, global_gitignore, global_fd_ignore)
    }

    pub(crate) fn new_with_globals(
        cfg: FilterConfig,
        global_gitignore: Gitignore,
        global_fd_ignore: Option<Gitignore>,
    ) -> Self {
        Self {
            cfg,
            dir_walkable_cache: HashMap::new(),
            repo_root_cache: HashMap::new(),
            fdignore_by_dir: HashMap::new(),
            ignore_by_dir: HashMap::new(),
            gitignore_by_dir: HashMap::new(),
            info_exclude_by_repo: HashMap::new(),
            global_gitignore,
            global_fd_ignore,
        }
    }

    pub fn should_include(&mut self, path: &Path) -> bool {
        // Match fd defaults: do not follow symlinks when determining whether something is a dir.
        let is_dir = fs::symlink_metadata(path)
            .map(|m| m.is_dir())
            .unwrap_or(false);

        if !self.cfg.include_hidden && is_hidden_under_base(path, &self.cfg.search_base) {
            return false;
        }

        if !self.is_walkable_to(path, is_dir) {
            return false;
        }

        if !self.cfg.ignore_enabled {
            return true;
        }

        let parent = path.parent().unwrap_or(path);
        self.is_entry_included(path, is_dir, parent)
    }

    fn is_walkable_to(&mut self, path: &Path, is_dir: bool) -> bool {
        let container = if is_dir {
            path
        } else {
            path.parent().unwrap_or(path)
        };

        // `mdfind -onlyin` should ensure everything is under `search_base`,
        // but be defensive. If it's outside, don't try to "walk" parents.
        if !container.starts_with(&self.cfg.search_base) {
            return true;
        }

        // Hot path: most candidates share a lot of directories. Avoid allocating a full
        // root-to-leaf directory list on every call. Instead, walk upward until we find
        // a cached decision, then fill in the missing suffix.
        //
        // Invariant: if a directory is cached as walkable, then all of its ancestors
        // under `search_base` were previously validated as walkable too.
        let mut missing = Vec::new();
        let mut cur = container;
        loop {
            if let Some(&ok) = self.dir_walkable_cache.get(cur) {
                if !ok {
                    return false;
                }
                break;
            }
            missing.push(cur.to_path_buf());

            if cur == self.cfg.search_base {
                break;
            }
            let Some(parent) = cur.parent() else { break };
            cur = parent;
        }

        if missing.is_empty() {
            return true;
        }

        for d in missing.iter().rev() {
            let ok = self.is_dir_walkable_uncached(d);
            self.dir_walkable_cache.insert(d.clone(), ok);
            if !ok {
                return false;
            }
        }

        true
    }

    fn is_dir_walkable_uncached(&mut self, dir: &Path) -> bool {
        if !self.cfg.include_hidden && is_hidden_under_base(dir, &self.cfg.search_base) {
            return false;
        }
        if !self.cfg.ignore_enabled {
            return true;
        }
        let parent = dir.parent().unwrap_or(dir);
        self.is_entry_included(dir, true, parent)
    }

    fn is_entry_included(&mut self, path: &Path, is_dir: bool, parent_dir: &Path) -> bool {
        // Precedence: .fdignore > .ignore > git ignores (repo only) > global fd ignore.
        if let Some(dec) = self.match_fdignore(path, is_dir, parent_dir) {
            return dec.include();
        }
        if let Some(dec) = self.match_dot_ignore(path, is_dir, parent_dir) {
            return dec.include();
        }
        if let Some(dec) = self.match_git_ignores(path, is_dir, parent_dir) {
            return dec.include();
        }
        if let Some(dec) = self.match_global_fd_ignore(path, is_dir) {
            return dec.include();
        }
        true
    }

    fn match_fdignore(
        &mut self,
        path: &Path,
        is_dir: bool,
        start: &Path,
    ) -> Option<IgnoreDecision> {
        self.match_from_ancestors(path, is_dir, start, IgnoreKind::FdIgnore)
    }

    fn match_dot_ignore(
        &mut self,
        path: &Path,
        is_dir: bool,
        start: &Path,
    ) -> Option<IgnoreDecision> {
        self.match_from_ancestors(path, is_dir, start, IgnoreKind::DotIgnore)
    }

    fn match_git_ignores(
        &mut self,
        path: &Path,
        is_dir: bool,
        parent_dir: &Path,
    ) -> Option<IgnoreDecision> {
        let repo_root = self.repo_root_for_dir(parent_dir)?;

        // Closest `.gitignore` wins (deepest directory has highest precedence).
        let mut cur = parent_dir;
        loop {
            if let Some(gi) = self.gitignore_in_dir(cur)
                && let Some(dec) = match_to_decision(gi.matched(path, is_dir))
            {
                return Some(dec);
            }
            if cur == repo_root {
                break;
            }
            let Some(p) = cur.parent() else { break };
            cur = p;
        }

        let info = self.info_exclude_for_repo(&repo_root);
        if let Some(dec) = match_to_decision(info.matched(path, is_dir)) {
            return Some(dec);
        }

        if let Some(dec) = match_to_decision(self.global_gitignore.matched(path, is_dir)) {
            return Some(dec);
        }

        None
    }

    fn match_global_fd_ignore(&mut self, path: &Path, is_dir: bool) -> Option<IgnoreDecision> {
        let gi = self.global_fd_ignore.as_ref()?;
        match_to_decision(gi.matched(path, is_dir))
    }

    fn match_from_ancestors(
        &mut self,
        path: &Path,
        is_dir: bool,
        start: &Path,
        kind: IgnoreKind,
    ) -> Option<IgnoreDecision> {
        // fd default behavior reads ignore files in parent directories too. We don't implement
        // `--no-ignore-parent`, so this always walks to the filesystem root.
        for cur in std::iter::successors(Some(start), |p| p.parent()) {
            let gi = match kind {
                IgnoreKind::FdIgnore => self.fdignore_in_dir(cur),
                IgnoreKind::DotIgnore => self.ignore_in_dir(cur),
            };
            if let Some(gi) = gi
                && let Some(dec) = match_to_decision(gi.matched(path, is_dir))
            {
                return Some(dec);
            }
        }
        None
    }

    fn fdignore_in_dir(&mut self, dir: &Path) -> Option<&Gitignore> {
        get_or_build_ignore_file(&mut self.fdignore_by_dir, dir, ".fdignore")
    }

    fn ignore_in_dir(&mut self, dir: &Path) -> Option<&Gitignore> {
        get_or_build_ignore_file(&mut self.ignore_by_dir, dir, ".ignore")
    }

    fn gitignore_in_dir(&mut self, dir: &Path) -> Option<&Gitignore> {
        get_or_build_ignore_file(&mut self.gitignore_by_dir, dir, ".gitignore")
    }

    fn info_exclude_for_repo(&mut self, repo_root: &Path) -> &Gitignore {
        self.info_exclude_by_repo
            .entry(repo_root.to_path_buf())
            .or_insert_with(|| build_info_exclude_matcher(repo_root))
    }

    fn repo_root_for_dir(&mut self, dir: &Path) -> Option<PathBuf> {
        let mut cur = dir.to_path_buf();
        let mut visited = Vec::new();

        loop {
            if let Some(cached) = self.repo_root_cache.get(&cur) {
                let root = cached.clone();
                for v in visited {
                    self.repo_root_cache.insert(v, root.clone());
                }
                return root;
            }

            visited.push(cur.clone());

            if cur.join(".git").join("HEAD").is_file() {
                let root = Some(cur.clone());
                for v in visited {
                    self.repo_root_cache.insert(v, root.clone());
                }
                return root;
            }

            let Some(parent) = cur.parent() else {
                for v in visited {
                    self.repo_root_cache.insert(v, None);
                }
                return None;
            };
            cur = parent.to_path_buf();
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum IgnoreKind {
    FdIgnore,
    DotIgnore,
}

fn get_or_build_ignore_file<'a>(
    cache: &'a mut HashMap<PathBuf, Option<Gitignore>>,
    dir: &Path,
    filename: &str,
) -> Option<&'a Gitignore> {
    if !cache.contains_key(dir) {
        let p = dir.join(filename);
        let gi = if p.is_file() {
            let mut builder = GitignoreBuilder::new(dir);
            let _ = builder.add(&p);
            builder.build().ok()
        } else {
            None
        };
        cache.insert(dir.to_path_buf(), gi);
    }
    cache.get(dir).and_then(|o| o.as_ref())
}

fn build_info_exclude_matcher(repo_root: &Path) -> Gitignore {
    let exclude = repo_root.join(".git").join("info").join("exclude");
    if !exclude.is_file() {
        return Gitignore::empty();
    }

    let mut builder = GitignoreBuilder::new(repo_root);
    let _ = builder.add(&exclude);
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

fn match_to_decision(m: ignore::Match<&ignore::gitignore::Glob>) -> Option<IgnoreDecision> {
    match m {
        ignore::Match::Ignore(_) => Some(IgnoreDecision::Ignore),
        ignore::Match::Whitelist(_) => Some(IgnoreDecision::Whitelist),
        ignore::Match::None => None,
    }
}

fn is_hidden_path(path: &Path) -> bool {
    path.components()
        .any(|c| is_hidden_component(c.as_os_str()))
}

fn is_hidden_under_base(path: &Path, base: &Path) -> bool {
    if let Ok(rest) = path.strip_prefix(base) {
        return rest
            .components()
            .any(|c| is_hidden_component(c.as_os_str()));
    }
    // Shouldn't happen with `mdfind -onlyin`, but be defensive.
    is_hidden_path(path)
}

fn is_hidden_component(comp: &std::ffi::OsStr) -> bool {
    #[cfg(unix)]
    {
        let b = comp.as_bytes();
        if b == b"." || b == b".." {
            return false;
        }
        b.first().is_some_and(|c| *c == b'.')
    }

    #[cfg(not(unix))]
    {
        comp.to_str()
            .is_some_and(|s| s != "." && s != ".." && s.starts_with('.'))
    }
}

fn load_global_fd_ignore(cwd: &Path) -> Option<Gitignore> {
    let p = global_fd_ignore_path()?;
    if !p.is_file() {
        return None;
    }

    let mut builder = GitignoreBuilder::new(cwd);
    let _ = builder.add(&p);
    builder.build().ok()
}

fn global_fd_ignore_path() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").and_then(|s| {
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(s))
        }
    });
    if let Some(xdg) = xdg {
        return Some(xdg.join("fd").join("ignore"));
    }
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("fd")
            .join("ignore"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn filter_for_test(root: &Path, include_hidden: bool, ignore_enabled: bool) -> Filter {
        Filter::new_with_globals(
            FilterConfig {
                cwd: root.to_path_buf(),
                search_base: root.to_path_buf(),
                include_hidden,
                ignore_enabled,
            },
            Gitignore::empty(),
            None,
        )
    }

    fn filter_for_test_with_global_fd_ignore(
        root: &Path,
        include_hidden: bool,
        ignore_enabled: bool,
        global_ignore_content: &str,
    ) -> Filter {
        let ignore_file = root.join("global-fd-ignore");
        fs::write(&ignore_file, global_ignore_content).unwrap();

        let mut builder = GitignoreBuilder::new(root);
        let _ = builder.add(&ignore_file);
        let global_fd_ignore = builder.build().ok();

        Filter::new_with_globals(
            FilterConfig {
                cwd: root.to_path_buf(),
                search_base: root.to_path_buf(),
                include_hidden,
                ignore_enabled,
            },
            Gitignore::empty(),
            global_fd_ignore,
        )
    }

    #[test]
    fn hidden_files_excluded_by_default() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".env"), "x").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/config.ts"), "x").unwrap();

        let mut f = filter_for_test(root, false, true);
        assert!(!f.should_include(&root.join(".env")));
        assert!(f.should_include(&root.join("src/config.ts")));
    }

    #[test]
    fn no_ignore_disables_ignores_but_not_hidden() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".env"), "x").unwrap();
        fs::write(root.join("ignored.foo"), "x").unwrap();
        fs::write(root.join(".gitignore"), "ignored.foo\n").unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        let mut f = filter_for_test(root, false, false);
        assert!(!f.should_include(&root.join(".env")));
        assert!(f.should_include(&root.join("ignored.foo")));
    }

    #[test]
    fn require_git_head_for_gitignore() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::create_dir_all(root.join(".git")).unwrap();
        // Intentionally do not create `.git/HEAD`.
        fs::write(root.join(".gitignore"), "ignored.foo\n").unwrap();
        fs::write(root.join("ignored.foo"), "x").unwrap();

        let mut f = filter_for_test(root, true, true);
        assert!(f.should_include(&root.join("ignored.foo")));

        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let mut f = filter_for_test(root, true, true);
        assert!(!f.should_include(&root.join("ignored.foo")));
    }

    #[test]
    fn fdignore_has_highest_precedence() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        fs::create_dir_all(root.join("inner")).unwrap();
        fs::write(root.join("inner/foo"), "x").unwrap();

        fs::write(root.join("inner/.gitignore"), "foo\n").unwrap();
        fs::write(root.join(".fdignore"), "!foo\n").unwrap();

        let mut f = filter_for_test(root, true, true);
        assert!(f.should_include(&root.join("inner/foo")));
    }

    #[test]
    fn dot_ignore_overrides_gitignore() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        fs::write(root.join("foo"), "x").unwrap();
        fs::write(root.join(".gitignore"), "foo\n").unwrap();
        fs::write(root.join(".ignore"), "!foo\n").unwrap();

        let mut f = filter_for_test(root, true, true);
        assert!(f.should_include(&root.join("foo")));
    }

    #[test]
    fn ignored_directory_prunes_descendants_even_if_file_is_whitelisted_locally() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        fs::create_dir_all(root.join("ignored_dir")).unwrap();
        fs::write(root.join(".gitignore"), "ignored_dir/\n").unwrap();
        fs::write(root.join("ignored_dir/.gitignore"), "!keep.ts\n").unwrap();
        fs::write(root.join("ignored_dir/keep.ts"), "x").unwrap();

        let mut f = filter_for_test(root, true, true);
        assert!(!f.should_include(&root.join("ignored_dir/keep.ts")));
    }

    #[test]
    fn unignoring_directory_chain_allows_whitelisted_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        fs::create_dir_all(root.join("ignored_dir")).unwrap();
        fs::write(
            root.join(".gitignore"),
            "ignored_dir/\n!ignored_dir/\nignored_dir/*\n!ignored_dir/keep.ts\n",
        )
        .unwrap();
        fs::write(root.join("ignored_dir/keep.ts"), "x").unwrap();
        fs::write(root.join("ignored_dir/junk.ts"), "x").unwrap();

        let mut f = filter_for_test(root, true, true);
        assert!(f.should_include(&root.join("ignored_dir/keep.ts")));
        assert!(!f.should_include(&root.join("ignored_dir/junk.ts")));
    }

    #[test]
    fn global_fd_ignore_is_lowest_precedence() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("foo"), "x").unwrap();
        fs::write(root.join("bar"), "x").unwrap();
        fs::write(root.join(".ignore"), "!foo\n").unwrap();

        let mut f = filter_for_test_with_global_fd_ignore(root, true, true, "foo\nbar\n");
        assert!(f.should_include(&root.join("foo")));
        assert!(!f.should_include(&root.join("bar")));
    }

    #[test]
    fn no_ignore_disables_global_fd_ignore() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("bar"), "x").unwrap();

        let mut f = filter_for_test_with_global_fd_ignore(root, true, false, "bar\n");
        assert!(f.should_include(&root.join("bar")));
    }
}
