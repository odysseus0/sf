# sf — Spotlight Find

Index-powered file search for macOS, with fd-like ignore semantics.

Your Mac indexes every file 24/7. `sf` searches that index instead
of walking the filesystem. It respects `.fdignore`, `.ignore`, and git ignore
rules (fd v10.3.0 behavior, high-ROI subset).

## Install

```bash
cargo install sf
```

## Usage

```bash
sf config                   # find files with "config" in name
sf "*.ts"                   # find all .ts files
sf "*.ts" ~/projects        # search specific directory
sf -I config                # include ignored files (still hides dotfiles unless -H)

sf "*.ts" | xargs rg import # compose with other tools
sf -0 "*.rs" | xargs -0 rg "unsafe"  # safe piping (handles weird filenames)
```

`pattern` is a glob if it contains `*` or `?`. Otherwise it’s treated as a
substring match (equivalent to `*pattern*`). To list everything under a path,
use `sf "*" /some/dir`.

## How It Works

sf wraps macOS `mdfind` (Spotlight CLI) and filters results through
the same ignore logic that `rg` and `fd` use (via the
[ignore](https://crates.io/crates/ignore) crate).

## Limitations

- macOS only
- Filename search only (use `rg` for content search)
- No regex: Spotlight's filename predicate supports glob-style matching, not full regex. Use `fd` for regex searches.
- Results depend on Spotlight's index being up to date

## Development

Integration tests are macOS-only and skipped by default due to Spotlight
indexing flakiness. Run them explicitly:

```bash
SF_INTEGRATION_TESTS=1 cargo test
```

Optional fd oracle tests (compare `sf` filtering against a real `fd` binary, no Spotlight):

```bash
SF_FD_ORACLE=1 cargo test fd_oracle -- --nocapture
# optionally:
SF_FD_ORACLE=1 SF_FD_BIN=/path/to/fd cargo test fd_oracle -- --nocapture
```

## Performance Notes

`sf` is most compelling when the alternative is walking a huge, messy, non-repo tree (the macOS "junk drawer"), where tools like `fd` and `rg --files` must traverse everything:

- `~/Library` (Application Support, Caches, Containers, etc.)
- `/Applications`

Real example (on a machine with a large `~/Library`):

```bash
sf settings.json ~/Library         # ~50ms
fd --glob '*settings.json*' ~/Library  # seconds
```

In typical git repos, `fd`/`rg` can be competitive or faster, especially for globs like `*.rs`.

To benchmark locally:

```bash
cargo build --release
./bench.sh ~/Library Cargo.toml
./bench.sh /Applications Info.plist
```
