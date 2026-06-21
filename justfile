# cargo-crap file excludes. Globs are relative to the crate root (`--path` defaults to `.`):
# `xtask/**` skips the dev-only tool the way `tests/**` skips tests, and (unlike `**/main.rs`)
# won't catch a real `src/main.rs`.
crap_excludes := "--exclude 'examples/**' --exclude 'benches/**' --exclude 'tests/**' --exclude 'src/generated.rs' --exclude 'xtask/**'"

# CRAP threshold read straight from `.cargo-crap.toml` (the same source cargo-crap uses), so the
# gate predicate can't drift from the report. Falls back to cargo-crap's built-in default (30).
crap_threshold := shell("v=$(grep -E '^threshold[[:space:]]*=' .cargo-crap.toml 2>/dev/null | sed -E 's/[^0-9]+//g'); echo ${v:-30}")

# Clippy the whole workspace with the pedantic lints (generated.rs self-allows; see CLAUDE.md).
clippy:
  @cargo clippy --workspace --all-targets

# Run the demo CLI (binary `utilars`). Reads creds from env; pass args after the recipe, e.g. `just cli vaults`.
cli *args:
  @cargo run --quiet --example utilars -- {{ args }}

# Generate a cargo-crap CRAP report (cyclomatic complexity x test coverage).
crap:
  #!/usr/bin/env bash
  set -euo pipefail
  mkdir -p tmp
  # Homebrew's rustc ships no llvm-tools component; if a standalone
  # llvm-profdata is on PATH, point cargo-llvm-cov at it.
  if command -v llvm-profdata >/dev/null 2>&1; then
    export LLVM_COV="$(command -v llvm-cov)"
    export LLVM_PROFDATA="$(command -v llvm-profdata)"
  fi
  cargo llvm-cov --workspace --lcov --output-path tmp/lcov.info
  cargo crap --lcov tmp/lcov.info {{ crap_excludes }}

# CI gate: workspace coverage + CRAP against the committed baseline.
crap-ci:
  #!/usr/bin/env bash
  set -euo pipefail
  mkdir -p tmp
  if command -v llvm-profdata >/dev/null 2>&1; then
    export LLVM_COV="$(command -v llvm-cov)"
    export LLVM_PROFDATA="$(command -v llvm-profdata)"
  fi
  # Reuse an already-collected tmp/lcov.info when REUSE_LCOV is set (see `gate`); else
  # run the instrumented suite ourselves.
  if [ -z "${REUSE_LCOV:-}" ]; then
    cargo llvm-cov --workspace --lcov --output-path tmp/lcov.info
  fi
  cargo crap --lcov tmp/lcov.info {{ crap_excludes }} \
    --baseline .cargo-crap.json --format json --output tmp/crap-delta.json
  # Predicate: any "regressed" entry, OR any "new" entry whose CRAP
  # exceeds the threshold.
  BAD=$(jq --argjson t {{ crap_threshold }} \
    '[.entries[] | select(.status == "regressed" or (.status == "new" and .crap > $t))] | length' \
    tmp/crap-delta.json)
  if [ "$BAD" -gt 0 ]; then
    echo "CRAP gate FAILED: $BAD offending entries (regressed or new-over-threshold)"
    jq --argjson t {{ crap_threshold }} \
      '.entries | map(select(.status == "regressed" or (.status == "new" and .crap > $t))) | sort_by(-.crap) | .[] | {status, function, file, line, crap, cyclomatic, coverage}' \
      tmp/crap-delta.json
    exit 1
  fi
  echo "CRAP gate PASSED: no regressions, no new over-threshold functions"

# Both coverage gates off ONE instrumented run: the CRAP baseline gate (`crap-ci`) and the
# per-function coverage histogram + 90% floor (`coverage`). Running those two recipes back to
# back re-runs the slow llvm-cov suite twice; this collects coverage once (`--no-report`) and
# both gates reuse it via REUSE_LCOV.
gate:
  #!/usr/bin/env bash
  set -euo pipefail
  mkdir -p tmp
  if command -v llvm-profdata >/dev/null 2>&1; then
    export LLVM_COV="$(command -v llvm-cov)"
    export LLVM_PROFDATA="$(command -v llvm-profdata)"
  fi
  # Collect coverage once; the `report` calls in the gates below reuse this profile data.
  cargo llvm-cov --workspace --no-report
  cargo llvm-cov report --lcov --output-path tmp/lcov.info
  REUSE_LCOV=1 just crap-ci
  REUSE_LCOV=1 just coverage

# Regenerate the committed CRAP baseline (`.cargo-crap.json`).
crap-baseline:
  #!/usr/bin/env bash
  set -euo pipefail
  mkdir -p tmp
  if command -v llvm-profdata >/dev/null 2>&1; then
    export LLVM_COV="$(command -v llvm-cov)"
    export LLVM_PROFDATA="$(command -v llvm-profdata)"
  fi
  cargo llvm-cov --workspace --lcov --output-path tmp/lcov.info
  cargo crap --lcov tmp/lcov.info {{ crap_excludes }} \
    --format json --output tmp/crap-full.json
  WS_ROOT="$(pwd)"
  jq --argjson t {{ crap_threshold }} --arg ws "$WS_ROOT/" \
    '.entries |= (map(select(.crap > $t)) | map(.file |= sub("^" + $ws; "")))' \
    tmp/crap-full.json > .cargo-crap.json
  KEPT=$(jq '.entries | length' .cargo-crap.json)
  echo "Wrote .cargo-crap.json with $KEPT over-threshold entries"

# Per-file + per-function coverage histogram; exits non-zero if any function is below 90%.
coverage:
  #!/usr/bin/env bash
  set -euo pipefail
  # NOTE: cargo-crap's `coverage` column is a PERCENTAGE (0-100), not a fraction — the threshold
  # predicate is `.coverage < 90`, NOT `< 0.90` (which tests 0.9% and passes everything).
  mkdir -p tmp
  if command -v llvm-profdata >/dev/null 2>&1; then
    export LLVM_COV="$(command -v llvm-cov)"
    export LLVM_PROFDATA="$(command -v llvm-profdata)"
  fi
  # Reuse an already-collected tmp/lcov.info + profile when REUSE_LCOV is set (see `gate`);
  # else run the instrumented suite ourselves. The `report` below reuses the profile either way.
  if [ -z "${REUSE_LCOV:-}" ]; then
    cargo llvm-cov --workspace --lcov --output-path tmp/lcov.info >/dev/null
  fi
  cargo crap --lcov tmp/lcov.info {{ crap_excludes }} --format json --output tmp/crap-now.json >/dev/null
  echo "PER-FILE LINE COVERAGE (source, excl. generated, examples, tests)"
  echo "─────────────────────────────────────────────────────────────────────"
  cargo llvm-cov report --summary-only --ignore-filename-regex 'generated.rs|xtask|tests/' \
  | awk '$1 ~ /\.rs$/ { cov=$10; gsub(/%/,"",cov); b=int(cov/2.5); s=""; for(i=0;i<b;i++)s=s"█"; printf "  %-25s %6.2f  %s\n",$1,cov,s }'
  echo ""
  echo "PER-FUNCTION COVERAGE DISTRIBUTION (threshold = 90%)"
  echo "─────────────────────────────────────────────────────────────────"
  # Bar = bucket's share of all functions, scaled to a 50-wide axis: round(50 * count/total).
  jq -r '
     def lpad($w): tostring as $s | ($w - ($s|length)) as $p | (if $p>0 then " "*$p else "" end) + $s;
     [.entries[].coverage] as $c | ($c|length) as $tot | [
       ["  <60%",([$c[]|select(.<60)]|length)],
       ["60-69%",([$c[]|select(.>=60 and .<70)]|length)],
       ["70-79%",([$c[]|select(.>=70 and .<80)]|length)],
       ["80-89%",([$c[]|select(.>=80 and .<90)]|length)],
       ["90-99%",([$c[]|select(.>=90 and .<100)]|length)],
       ["  100%",([$c[]|select(.>=100)]|length)]
     ][] | .[1] as $n | (if $tot>0 then (50*$n/$tot|round) else 0 end) as $bars
     | "  \(.[0]) \($n|lpad(4))  \(if $bars>0 then "▓"*$bars else "" end)"
   ' tmp/crap-now.json
  echo ""
  UNDER=$(jq -r '[.entries[]|select(.coverage<90)]|length' tmp/crap-now.json)
  if [ "$UNDER" -gt 0 ]; then
    echo "THRESHOLD VIOLATIONS (<90% line coverage):"
    jq -r '.entries[]|select(.coverage<90)|"  \(.coverage)%  \(.function)  [\(.file|sub(".*/";""))]"' tmp/crap-now.json | sort -n
    exit 1
  fi
  echo "Threshold OK: every function ≥90% line coverage"

# List hand-written .rs files (src, xtask, tests) over 30k — a "consider splitting" smell.
large-files:
  #!/usr/bin/env bash
  set -euo pipefail
  # src/generated.rs is excluded — machine-generated and intentionally large.
  found=$(find src xtask/src tests -type f -name '*.rs' ! -name 'generated.rs' -size +30k)
  if [ -z "$found" ]; then
    echo "(no hand-written source file over 30k)"
  else
    # `wc -lc` → lines + bytes; sort by size (bytes), print both.
    echo "$found" | xargs wc -lc | grep -v ' total$' | sort -rnk2 \
      | awk '{printf "  %6s lines  %7.1f KB  %s\n",$1,$2/1024,$3}'
  fi

# Clean ./tmp directory.
clean:
  @rm -rf tmp build
  @mkdir -p tmp build
