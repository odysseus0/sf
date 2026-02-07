use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Delimiter {
    Newline,
    Nul,
}

#[derive(Clone, Debug)]
pub struct OutputStyle {
    cwd: PathBuf,
    search_base: PathBuf,
    path_arg: Option<PathBuf>,
}

impl OutputStyle {
    pub fn new(cwd: PathBuf, search_base: PathBuf, path_arg: Option<&Path>) -> Self {
        Self {
            cwd,
            search_base,
            path_arg: path_arg.map(|p| p.to_path_buf()),
        }
    }

    pub fn render(&self, abs_path: &Path) -> PathBuf {
        match self.path_arg.as_deref() {
            None => {
                // Omitted `path`: print relative to CWD, but without a leading "./".
                strip_prefix_or_abs(abs_path, &self.cwd)
            }
            Some(p) if p.is_absolute() => abs_path.to_path_buf(),
            Some(p) => {
                // Explicit relative `path`: preserve the prefix (including "./" when `path` is ".").
                let rel_to_base = strip_prefix_or_abs(abs_path, &self.search_base);
                if rel_to_base.as_os_str().is_empty() || rel_to_base == Path::new(".") {
                    return p.to_path_buf();
                }
                p.join(rel_to_base)
            }
        }
    }
}

fn strip_prefix_or_abs(path: &Path, base: &Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix(base) {
        if rest.as_os_str().is_empty() {
            return PathBuf::from(".");
        }
        return rest.to_path_buf();
    }
    path.to_path_buf()
}

pub fn write_path(out: &mut dyn Write, path: &Path, delim: Delimiter) -> io::Result<()> {
    let suffix: &[u8] = match delim {
        Delimiter::Newline => b"\n",
        Delimiter::Nul => b"\0",
    };

    #[cfg(unix)]
    {
        out.write_all(path.as_os_str().as_bytes())?;
        out.write_all(suffix)?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        use std::fmt::Write as _;

        let mut s = String::new();
        s.push_str(&path.display().to_string());
        // Lossy for non-unix targets; `sf` is macOS-only anyway.
        s.push_str(&String::from_utf8_lossy(suffix));
        out.write_all(s.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print0_writes_nul_delimited_output() {
        let mut buf = Vec::new();
        write_path(&mut buf, Path::new("a b"), Delimiter::Nul).unwrap();
        assert_eq!(buf, b"a b\0");
    }

    #[test]
    fn omitted_path_is_relative_to_cwd_without_dot_slash() {
        let style = OutputStyle::new(PathBuf::from("/a/b"), PathBuf::from("/a/b"), None);
        assert_eq!(
            style.render(Path::new("/a/b/c/d.txt")),
            PathBuf::from("c/d.txt")
        );
    }

    #[test]
    fn explicit_dot_path_preserves_dot_slash_prefix() {
        let style = OutputStyle::new(
            PathBuf::from("/a/b"),
            PathBuf::from("/a/b"),
            Some(Path::new(".")),
        );
        assert_eq!(
            style.render(Path::new("/a/b/c.txt")),
            PathBuf::from("./c.txt")
        );
    }

    #[test]
    fn explicit_relative_path_preserves_prefix() {
        let style = OutputStyle::new(
            PathBuf::from("/a/b"),
            PathBuf::from("/a/b/src"),
            Some(Path::new("src")),
        );
        assert_eq!(
            style.render(Path::new("/a/b/src/lib.rs")),
            PathBuf::from("src/lib.rs")
        );
    }

    #[test]
    fn explicit_absolute_path_outputs_absolute() {
        let style = OutputStyle::new(
            PathBuf::from("/a/b"),
            PathBuf::from("/x/y"),
            Some(Path::new("/x/y")),
        );
        assert_eq!(style.render(Path::new("/x/y/z")), PathBuf::from("/x/y/z"));
    }
}
