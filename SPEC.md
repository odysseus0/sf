# sf — Spotlight Find

## Context

macOS maintains a filesystem index (Spotlight) 24/7 via the `mds` daemon. Every dev tool (`fd`, `find`) ignores it and walks the filesystem from scratch. `sf` bridges that gap: index-powered file finding with `.gitignore` support.

macOS-only boutique tool. Small, focused, fast.

**Repo:** `~/projects/sf`

## CLI Interface

```
sf [pattern] [path] [-I]
```

That's the whole interface.

- `pattern` — glob or substring to match filenames. If contains `*` or `?`, treated as glob. Otherwise, substring match (wrapped in `*pattern*`). Optional — omit to list all files.
- `path` — directory to scope search (default: cwd)
- `-I, --no-ignore` — don't respect `.gitignore` or default exclusions

No `--exec`, no regex, no `--color`, no `--limit`, no `--stats`, no `--type`, no `--hidden`, no `--extension`. Use `xargs`, `fd`, `head`, `wc`, the Unix ecosystem. Want `.ts` files? `sf "*.ts"`. That's what globs are for.

## Examples

```bash
sf config                     # files with "config" in name
sf "*.ts"                     # all .ts files
sf "*.ts" ~/projects          # scoped
sf -I config                  # include gitignored results

# compose
sf "*.ts" | wc -l             # count
sf "*.ts" | head -5           # first 5
sf "*.ts" | xargs rg import   # narrow then search content
```

## Architecture

```
CLI (clap) → Query Builder → mdfind subprocess → Filter → stdout
                                                    ↑
                                           ignore crate (.gitignore)
                                           + hardcoded defaults
```

### 1. Query Builder (`query.rs`)

Translates CLI args into mdfind query:

| Input | mdfind query |
|-------|-------------|
| `sf config` | `mdfind -onlyin $PWD 'kMDItemFSName == "*config*"'` |
| `sf "*.ts"` | `mdfind -onlyin $PWD 'kMDItemFSName == "*.ts"'` |
| `sf` (no pattern) | `mdfind -onlyin $PWD 'kMDItemFSName == "*"'` |

Rules:
- Pattern has glob chars (`*`, `?`) → use as-is in `kMDItemFSName == "pattern"`
- Otherwise → wrap: `kMDItemFSName == "*pattern*"`

### 2. mdfind Execution (`mdfind.rs`)

Stream mdfind stdout line-by-line. Don't buffer.

```rust
let mut child = Command::new("mdfind")
    .args(&query_args)
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()?;

let reader = BufReader::new(child.stdout.take().unwrap());
for line in reader.lines() {
    let path = PathBuf::from(line?);
    if filter.should_include(&path) {
        println!("{}", make_relative(&path, &base));
    }
}
```

If `mdfind` not found → print `"sf requires macOS Spotlight"`, exit 1.

### 3. Filter (`filter.rs`)

Two layers, both skipped with `-I`:

**a. Hardcoded default exclusions**

Applied by checking if any path component matches these names:

```
node_modules, .git, target, build, dist, __pycache__,
.tox, vendor, Pods, .build, DerivedData, .DS_Store
```

Implementation: split path into components, check against a `HashSet<&str>`.

**b. Per-repo `.gitignore`**

- For each result, find git repo root by walking up from the path looking for `.git/`
- Two-level cache:
  - `HashMap<PathBuf, Option<PathBuf>>` — parent directory → repo root (cache intermediate dirs for high hit rate since mdfind results from the same repo share parent paths)
  - `HashMap<PathBuf, Gitignore>` — repo root → parsed gitignore matcher
- Build gitignore with `ignore::gitignore::GitignoreBuilder`:
  - Add root `.gitignore`
  - Add `.git/info/exclude`
  - Add nested `.gitignore` files between repo root and result path
  - Add global gitignore via `ignore::gitignore::Gitignore::global()`
- Match with `gitignore.matched_path_or_any_parents(path, is_dir).is_ignore()`
- Not in a git repo → only hardcoded defaults apply

**c. Hidden file filtering**

- Filter out paths where any component starts with `.` (except the path components that are part of the search base)
- `.git` is also caught by hardcoded defaults, but `.env`, `.config`, etc. are hidden too
- No flag to show hidden files in v1 — add later if asked

### 4. Output (`output.rs`)

- Paths relative to cwd (or to the `path` argument if specified)
- One per line
- No color, no decoration
- Pipe-friendly: no trailing newline on last line? Actually, match fd behavior — trailing newline on every line including last.

## Dependencies

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
ignore = "0.4"
anyhow = "1"

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
tempfile = "3"
```

Three runtime deps. That's it.

## Project Structure

```
sf/
├── Cargo.toml
├── README.md
├── LICENSE                # MIT
├── bench.sh               # Benchmark script (sf vs fd vs find)
├── src/
│   ├── main.rs            # Entry point, clap derive, orchestration
│   ├── query.rs           # Pattern → mdfind query args
│   ├── mdfind.rs          # Subprocess spawn + stdout streaming
│   ├── filter.rs          # Ignore logic (defaults + gitignore + hidden)
│   └── output.rs          # Path relativization
└── tests/
    ├── integration.rs     # End-to-end (requires macOS + Spotlight)
    └── fixtures/
        ├── repo/
        │   ├── .git/          # (empty dir, enough to be detected as repo)
        │   ├── .gitignore     # "ignored_dir/"
        │   ├── src/
        │   │   └── config.ts
        │   ├── ignored_dir/
        │   │   └── junk.ts
        │   └── node_modules/
        │       └── dep/
        │           └── index.js
        └── plain/
            ├── file.txt
            └── .hidden_file
```

## Testing

### Unit Tests

**query.rs:**
- Substring → wrapped in wildcards
- Glob → used as-is
- No pattern → matches everything
- Path scoping → `-onlyin` arg

**filter.rs:**
- Default exclusions: `node_modules/foo.js` excluded, `src/foo.js` included
- Gitignore: path matching works against parsed `.gitignore`
- Cache: same directory → same cache hit for repo root
- Hidden: `.env` excluded, `src/config.ts` included
- `--no-ignore`: everything passes through

**output.rs:**
- Absolute → relative conversion
- Path under cwd → relative works
- Path outside cwd → keep absolute

### Integration Tests

Require macOS. Use `mdimport` to force Spotlight indexing of test fixtures.

- `sf "*.ts"` in fixture repo → returns `src/config.ts`, not `node_modules/dep/index.js` or `ignored_dir/junk.ts`
- `sf -I "*.ts"` → returns all `.ts` files including ignored
- `sf "*.ts" tests/fixtures/repo` → scoped results
- `sf nonexistent` → exit 0, no output

**Note:** Spotlight indexing of fixtures may be flaky. `mdimport -d1 <dir>` forces import. CI should use macOS runner. If tests are too flaky, gate behind an env var: `SF_INTEGRATION_TESTS=1`.

### Benchmark Script (`bench.sh`)

```bash
#!/bin/bash
set -euo pipefail

DIR="${1:-.}"
PATTERN="${2:-*.ts}"

echo "Searching for '$PATTERN' in $DIR"
echo ""

echo "=== sf ==="
time sf "$PATTERN" "$DIR" > /dev/null 2>&1

echo ""
echo "=== fd ==="
time fd "$PATTERN" "$DIR" > /dev/null 2>&1

echo ""
echo "=== find ==="
time find "$DIR" -name "$PATTERN" > /dev/null 2>&1
```

## Edge Cases

1. **mdfind not found** → clear error message, exit 1
2. **Spotlight disabled / still indexing** → results may be incomplete. Not our problem. Could add a note in `--help`.
3. **Symlinks** → mdfind follows them. Show the path mdfind returns (may be symlink path).
4. **Unicode filenames** → mdfind handles these. Use `OsString`/`PathBuf` throughout, never lossy string conversion.
5. **No results** → exit 0, no output (match fd)
6. **iCloud Drive paths** → mdfind returns `~/Library/Mobile Documents/...`. Works fine, just long relative paths.

## README

```markdown
# sf — Spotlight Find

Index-powered file search for macOS.

Your Mac indexes every file 24/7. `sf` searches that index instead
of walking the filesystem. It respects `.gitignore`.

## Install

    cargo install sf

## Usage

    sf config                   # find files with "config" in name
    sf "*.ts"                   # find all .ts files
    sf "*.ts" ~/projects        # search specific directory
    sf -I config                # include gitignored files

    sf "*.ts" | xargs rg import # compose with other tools

## Speed

Searching 157,000 files:

| Tool  | Time   |
|-------|--------|
| sf    | ~50ms  |
| fd    | ~85ms  |
| find  | ~3.2s  |

sf queries a pre-built index. fd and find walk the filesystem.

## How It Works

sf wraps macOS `mdfind` (Spotlight CLI) and filters results through
the same `.gitignore` logic that `rg` and `fd` use (via the
[ignore](https://crates.io/crates/ignore) crate).

## Limitations

- macOS only
- Filename search only (use `rg` for content search)
- No regex (use `fd`)
- Results depend on Spotlight's index being up to date
```

## Implementation Order

1. `cargo init sf && cd sf` — scaffold with clap derive struct
2. `query.rs` — pattern → mdfind args translation
3. `mdfind.rs` — subprocess spawn, stdout streaming
4. `filter.rs` — hardcoded defaults + hidden file filtering
5. `output.rs` — path relativization
6. Wire it up in `main.rs` — end to end working
7. Add `ignore` crate integration to `filter.rs` — gitignore support
8. Unit tests for each module
9. Integration tests with fixtures
10. `bench.sh` + run benchmarks for README
11. README + LICENSE
