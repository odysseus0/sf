#![forbid(unsafe_code)]

mod filter;
mod mdfind;
mod output;
mod query;

#[cfg(test)]
mod fd_oracle_tests;
#[cfg(test)]
mod fd_parity_tests;
#[cfg(test)]
mod test_support;

use std::{
    io,
    path::{Path, PathBuf},
    process,
};

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "sf",
    about = "Spotlight-powered file finding with fd-like ignore semantics (macOS only).",
    version
)]
struct Args {
    /// Glob (contains '*' or '?') or substring match.
    ///
    /// If omitted, lists all files under the search path.
    ///
    /// Matching is fd-like "smart case": case-insensitive unless the pattern contains any
    /// uppercase character.
    #[arg(value_name = "pattern")]
    pattern: Option<String>,

    /// Directory to scope search (default: current directory).
    #[arg(value_name = "path")]
    path: Option<PathBuf>,

    /// Include hidden files and directories (names starting with '.').
    #[arg(short = 'H', long = "hidden")]
    hidden: bool,

    /// Don't respect ignore files (.gitignore/.ignore/.fdignore/global ignores).
    ///
    /// Does not imply `--hidden`.
    #[arg(short = 'I', long = "no-ignore")]
    no_ignore: bool,

    /// Print NUL ('\\0') after each result instead of '\\n'.
    #[arg(short = '0', long = "print0")]
    print0: bool,
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("sf is macOS-only (requires Spotlight)");
    process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    if let Err(err) = run() {
        // Match common Unix CLI behavior: don't print scary errors on broken pipes
        // (e.g. `sf "*.rs" | head`).
        if is_broken_pipe(&err) {
            process::exit(0);
        }

        if is_mdfind_not_found(&err) {
            eprintln!("sf requires macOS Spotlight");
            process::exit(1);
        }

        eprintln!("{err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    let cwd = std::env::current_dir().context("failed to read current directory")?;
    let base = make_absolute_dir(&cwd, args.path.as_deref())?;

    let query_plan = query::build_mdfind_plan(&base, args.pattern.as_deref());
    let mut filter = filter::Filter::new(filter::FilterConfig {
        cwd: cwd.clone(),
        search_base: base.clone(),
        include_hidden: args.hidden,
        ignore_enabled: !args.no_ignore,
    });
    let out_style = output::OutputStyle::new(cwd, base, args.path.as_deref());
    let delimiter = if args.print0 {
        output::Delimiter::Nul
    } else {
        output::Delimiter::Newline
    };

    let stdout = io::stdout();
    let mut out = stdout.lock();
    mdfind::run(&query_plan, &mut filter, &out_style, delimiter, &mut out)?;
    Ok(())
}

fn make_absolute_dir(cwd: &Path, path: Option<&Path>) -> Result<PathBuf> {
    let base = match path {
        None => cwd.to_path_buf(),
        Some(p) if p.is_absolute() => p.to_path_buf(),
        Some(p) => cwd.join(p),
    };

    match std::fs::metadata(&base) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => anyhow::bail!("path is not a directory: {}", base.display()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            anyhow::bail!("path does not exist: {}", base.display())
        }
        Err(e) => return Err(anyhow::Error::new(e)).context("failed to stat path"),
    }

    Ok(base)
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|ioe| ioe.kind() == io::ErrorKind::BrokenPipe)
    })
}

fn is_mdfind_not_found(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.downcast_ref::<mdfind::MdfindNotFound>().is_some())
}
