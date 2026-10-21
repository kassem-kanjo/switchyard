#!/usr/bin/env bash
# Native determinism check (ADR 0002): runs every line of expected.txt through
# the release CLI and compares the state hash. Exit 0 = all match, 1 = mismatch.
# Usage: scripts/determinism/check.sh   (SWITCHYARD_BIN overrides the binary)
set -euo pipefail
cd "$(dirname "$0")/../.."
BIN="${SWITCHYARD_BIN:-target/release/switchyard}"
fail=0
while read -r scenario ticks seed expected; do
  case "$scenario" in ''|'#'*) continue ;; esac
  out=$("$BIN" run --scenario "$scenario" --ticks "$ticks" --seed "$seed" | tr -d '\r')
  actual=$(printf '%s\n' "$out" | sed -n 's/^state hash[[:space:]]*//p')
  if [ "$actual" = "$expected" ]; then
    echo "ok       $scenario $ticks $seed $actual"
  else
    echo "MISMATCH $scenario $ticks $seed: expected $expected, actual ${actual:-<none>}"
    fail=1
  fi
done < scripts/determinism/expected.txt
exit $fail
