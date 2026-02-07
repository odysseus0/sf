use std::{
    ffi::OsString,
    io::{self, BufRead, BufReader, Write},
    process::{Command, Stdio},
};

use anyhow::{Context, Result};

use crate::{filter::Filter, output, query};

#[derive(Debug)]
pub struct MdfindNotFound;

impl std::fmt::Display for MdfindNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mdfind not found")
    }
}

impl std::error::Error for MdfindNotFound {}

pub fn run(
    plan: &query::QueryPlan,
    filter: &mut Filter,
    out_style: &output::OutputStyle,
    delimiter: output::Delimiter,
    out: &mut dyn Write,
) -> Result<()> {
    let mut child = Command::new("mdfind")
        .args(&plan.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                anyhow::Error::new(MdfindNotFound)
            } else {
                anyhow::Error::new(e)
            }
        })
        .context("failed to spawn mdfind")?;

    let stdout = child
        .stdout
        .take()
        .context("failed to capture mdfind stdout")?;
    let mut reader = BufReader::new(stdout);

    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = reader.read_until(b'\0', &mut buf)?;
        if n == 0 {
            break;
        }

        // Strip trailing NUL (and optional CR, just in case).
        while matches!(buf.last(), Some(b'\0' | b'\r')) {
            buf.pop();
        }
        if buf.is_empty() {
            continue;
        }

        // Avoid an extra allocation: `read_until` gives us a Vec<u8> already.
        let bytes = std::mem::take(&mut buf);
        let path = std::path::PathBuf::from(os_string_from_vec(bytes));
        if filter.should_include(&path)
            && plan.rust_matcher.as_ref().is_none_or(|m| m.matches(&path))
        {
            let rendered = out_style.render(&path);
            output::write_path(out, &rendered, delimiter)?;
        }
    }

    // Ensure we don't leave zombies (and propagate any execution failure).
    let status = child.wait().context("failed to wait for mdfind")?;
    if !status.success() {
        anyhow::bail!("mdfind exited with status {status}");
    }

    Ok(())
}

fn os_string_from_vec(bytes: Vec<u8>) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(bytes)
    }

    #[cfg(not(unix))]
    {
        // Best-effort fallback for non-Unix targets.
        String::from_utf8_lossy(&bytes).into_owned().into()
    }
}
