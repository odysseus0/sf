use std::{
    ffi::{OsStr, OsString},
    path::Path,
};

#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub args: Vec<OsString>,
    /// Optional Rust-side matcher applied to candidates after ignore/hidden filtering.
    ///
    /// This is used for correctness when `mdfind` query mode is looser than our fd-like
    /// semantics (e.g. `mdfind -name` is case-insensitive).
    pub rust_matcher: Option<RustMatcher>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustMatcher {
    CaseSensitiveSubstring { needle: String },
}

impl RustMatcher {
    pub fn matches(&self, path: &Path) -> bool {
        match self {
            RustMatcher::CaseSensitiveSubstring { needle } => path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.contains(needle)),
        }
    }
}

/// Build a query plan for `mdfind`.
///
/// We prefer `mdfind -name <pattern>` for non-glob patterns because it has
/// dramatically lower fixed overhead than a full predicate query on many systems.
///
/// Example (shell):
/// `mdfind -onlyin $BASE -name Cargo.toml`
pub fn build_mdfind_plan(base: &Path, pattern: Option<&str>) -> QueryPlan {
    // Always request NUL-separated output from `mdfind` so we can parse paths robustly
    // (paths may contain newlines).
    let mut args = vec![
        OsString::from("-0"),
        OsString::from("-onlyin"),
        OsString::from(base.as_os_str()),
    ];

    match pattern {
        // "List everything": stick with a predicate query. `-name` doesn't accept globs
        // like `*` in a way we can rely on.
        None => {
            args.push(OsString::from(build_query(None)));
            QueryPlan {
                args,
                rust_matcher: None,
            }
        }
        Some(p) if is_glob(p) => {
            args.push(OsString::from(build_query(Some(p))));
            QueryPlan {
                args,
                rust_matcher: None,
            }
        }
        Some(p) => {
            if should_avoid_name_fast_path(base) {
                args.push(OsString::from(build_query(Some(p))));
                return QueryPlan {
                    args,
                    rust_matcher: None,
                };
            }

            // `mdfind -name` is (effectively) case-insensitive, so we apply a Rust-side
            // matcher for smart-case when the user's pattern contains uppercase.
            args.push(OsString::from("-name"));
            args.push(OsString::from(p));

            let rust_matcher = if has_uppercase(p) {
                Some(RustMatcher::CaseSensitiveSubstring {
                    needle: p.to_owned(),
                })
            } else {
                None
            };

            QueryPlan { args, rust_matcher }
        }
    }
}

fn should_avoid_name_fast_path(base: &Path) -> bool {
    // Empirically, `mdfind -name` may return no results for some ephemeral system paths
    // even when a predicate query scoped with `-onlyin` works. Prefer correctness over
    // speed in these common temporary locations.
    base.starts_with("/var/folders")
        || base.starts_with("/private/var/folders")
        || base.starts_with("/tmp")
        || base.starts_with("/private/tmp")
}

fn build_query(pattern: Option<&str>) -> String {
    let pat = match pattern {
        None => String::from("*"),
        Some(p) if is_glob(p) => p.to_owned(),
        Some(p) => format!("*{p}*"),
    };

    let escaped = escape_query_string(&pat);
    let case_insensitive = pattern.is_some_and(|p| !has_uppercase(p));
    if case_insensitive {
        format!("kMDItemFSName == \"{escaped}\"c")
    } else {
        format!("kMDItemFSName == \"{escaped}\"")
    }
}

fn is_glob(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?')
}

fn has_uppercase(s: &str) -> bool {
    s.chars().any(|c| c.is_uppercase())
}

fn escape_query_string(s: &str) -> String {
    // Spotlight query uses double quotes to delimit strings.
    // Keep escaping minimal and predictable.
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn substring_wraps_in_wildcards() {
        let q = build_query(Some("config"));
        assert_eq!(q, "kMDItemFSName == \"*config*\"c");
    }

    #[test]
    fn glob_used_as_is() {
        let q = build_query(Some("*.ts"));
        assert_eq!(q, "kMDItemFSName == \"*.ts\"c");
    }

    #[test]
    fn smart_case_uppercase_is_case_sensitive() {
        let q = build_query(Some("SPEC"));
        assert_eq!(q, "kMDItemFSName == \"*SPEC*\"");
    }

    #[test]
    fn no_pattern_matches_everything() {
        let q = build_query(None);
        assert_eq!(q, "kMDItemFSName == \"*\"");
    }

    #[test]
    fn plan_uses_predicate_when_no_pattern() {
        let base = PathBuf::from("/tmp");
        let plan = build_mdfind_plan(&base, None);
        assert_eq!(plan.rust_matcher, None);
        assert_eq!(plan.args.len(), 4);
        assert_eq!(plan.args[0], OsString::from("-0"));
        assert_eq!(plan.args[1], OsString::from("-onlyin"));
        assert_eq!(plan.args[2], OsString::from("/tmp"));
        assert_eq!(plan.args[3], OsString::from("kMDItemFSName == \"*\""));
    }

    #[test]
    fn plan_uses_predicate_for_globs() {
        let base = PathBuf::from("/tmp");
        let plan = build_mdfind_plan(&base, Some("*.ts"));
        assert_eq!(plan.rust_matcher, None);
        assert_eq!(plan.args.len(), 4);
        assert_eq!(plan.args[3], OsString::from("kMDItemFSName == \"*.ts\"c"));
    }

    #[test]
    fn plan_uses_name_fast_path_for_substrings() {
        let base = PathBuf::from("/Users/alice");
        let plan = build_mdfind_plan(&base, Some("foo"));
        assert_eq!(plan.rust_matcher, None);
        assert_eq!(plan.args.len(), 5);
        assert_eq!(plan.args[3], OsString::from("-name"));
        assert_eq!(plan.args[4], OsString::from("foo"));
    }

    #[test]
    fn plan_adds_case_sensitive_matcher_for_uppercase_substrings() {
        let base = PathBuf::from("/Users/alice");
        let plan = build_mdfind_plan(&base, Some("Foo"));
        assert!(matches!(
            plan.rust_matcher,
            Some(RustMatcher::CaseSensitiveSubstring { .. })
        ));
    }

    #[test]
    fn plan_avoids_name_fast_path_for_tmp_like_dirs() {
        let base = PathBuf::from("/var/folders/abc");
        let plan = build_mdfind_plan(&base, Some("foo"));
        assert_eq!(plan.args.len(), 4);
        assert!(
            plan.args[3]
                .to_string_lossy()
                .starts_with("kMDItemFSName ==")
        );
    }

    #[test]
    fn escapes_quotes_and_backslashes() {
        let q = build_query(Some("a\"b\\c"));
        assert_eq!(q, "kMDItemFSName == \"*a\\\"b\\\\c*\"c");
    }
}
