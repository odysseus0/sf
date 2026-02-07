#!/bin/bash
set -euo pipefail

DIR="${1:-.}"
PATTERN="${2:-*.ts}"

echo "Searching for '$PATTERN' in $DIR"
echo ""

SF_BIN="${SF_BIN:-}"
if [[ -z "$SF_BIN" ]]; then
  if [[ -x "./target/release/sf" ]]; then
    SF_BIN="./target/release/sf"
  else
    SF_BIN="sf"
  fi
fi

is_glob=0
if [[ "$PATTERN" == *"*"* || "$PATTERN" == *"?"* ]]; then
  is_glob=1
fi

echo "=== sf ==="
/usr/bin/time -p bash -lc "\"$SF_BIN\" \"$PATTERN\" \"$DIR\" >/dev/null 2>/dev/null"

echo ""
echo "=== fd ==="
if [[ "$is_glob" -eq 1 ]]; then
  /usr/bin/time -p bash -lc "fd --glob \"$PATTERN\" \"$DIR\" >/dev/null 2>/dev/null"
else
  /usr/bin/time -p bash -lc "fd --glob \"*$PATTERN*\" \"$DIR\" >/dev/null 2>/dev/null"
fi

echo ""
echo "=== rg ==="
if [[ "$is_glob" -eq 1 ]]; then
  /usr/bin/time -p bash -lc "rg --files -g \"$PATTERN\" \"$DIR\" >/dev/null 2>/dev/null"
else
  # Use ripgrep's smart-case mode to mimic fd's "smart case":
  # case-insensitive unless the pattern contains uppercase.
  /usr/bin/time -p bash -lc "rg --files \"$DIR\" | rg -F -S \"$PATTERN\" >/dev/null 2>/dev/null"
fi

echo ""
echo "=== find ==="
/usr/bin/time -p bash -lc "find \"$DIR\" -name \"$PATTERN\" >/dev/null 2>/dev/null"
