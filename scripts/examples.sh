#!/usr/bin/env bash
# Runs the in-process examples and checks what they print.
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

# Runs example $1 and checks that every line after it appears in its output.
expect() {
  local example=$1
  shift
  echo "== $example"
  local output
  output=$(cargo run -q -p "$example")
  echo "$output" | sed 's/^/   /'
  for line in "$@"; do
    grep -qF -- "$line" <<<"$output" || { echo "missing: $line" >&2; exit 1; }
  done
}

expect namu-basic "fibonacci: 144" "sum_of_squares: 385"
expect namu-advanced "200 records labeled, 167 confidently, 67 of those about billing in"
echo "examples passed"
