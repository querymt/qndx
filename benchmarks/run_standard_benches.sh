#!/usr/bin/env bash
#
# run_standard_benches.sh — Run real_corpus benchmarks against all standard corpora.
#
# Iterates over downloaded corpora in benchmarks/corpora/ and runs the
# real_corpus benchmark for each one, with the appropriate patterns file.
#
# Usage:
#   ./benchmarks/run_standard_benches.sh                   # all downloaded corpora
#   ./benchmarks/run_standard_benches.sh rust linux         # specific corpora
#   ./benchmarks/run_standard_benches.sh --save-baseline main  # pass extra args to criterion
#   ./benchmarks/run_standard_benches.sh --summary-only     # just print the summary tables
#
# Prerequisites:
#   ./benchmarks/fetch_corpora.sh    # download corpora first

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CORPORA_DIR="${QNDX_CORPORA_DIR:-${SCRIPT_DIR}/corpora}"

# ── Helpers ─────────────────────────────────────────────────────────

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
RESET='\033[0m'

info()  { printf "${BLUE}[info]${RESET}  %s\n" "$*"; }
ok()    { printf "${GREEN}[ok]${RESET}    %s\n" "$*"; }
warn()  { printf "${YELLOW}[warn]${RESET}  %s\n" "$*"; }
err()   { printf "${RED}[error]${RESET} %s\n" "$*" >&2; }

# ── Parse arguments ─────────────────────────────────────────────────

CORPORA=()
CRITERION_ARGS=()
SUMMARY_ONLY=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --summary-only)
      SUMMARY_ONLY=true
      shift
      ;;
    --save-baseline|--baseline)
      CRITERION_ARGS+=("$1" "$2")
      shift 2
      ;;
    --*)
      CRITERION_ARGS+=("$1")
      shift
      ;;
    *)
      CORPORA+=("$1")
      shift
      ;;
  esac
done

# ── Discover corpora ────────────────────────────────────────────────

# Known corpora and their pattern files (mirrors corpora.toml for shell use)
declare -A PATTERNS_MAP
PATTERNS_MAP[rust]="benchmarks/patterns/rust.txt"
PATTERNS_MAP[linux]="benchmarks/patterns/linux.txt"
PATTERNS_MAP[kubernetes]="benchmarks/patterns/kubernetes.txt"

# Default corpus order (matches corpora.toml [defaults].standard)
DEFAULT_CORPORA=(rust linux kubernetes)

if [[ ${#CORPORA[@]} -eq 0 ]]; then
  # Auto-discover: use defaults, filtered to what's actually downloaded
  for name in "${DEFAULT_CORPORA[@]}"; do
    if [[ -d "${CORPORA_DIR}/${name}" ]]; then
      CORPORA+=("${name}")
    fi
  done
fi

if [[ ${#CORPORA[@]} -eq 0 ]]; then
  err "No corpora found in ${CORPORA_DIR}/"
  echo ""
  echo "Download corpora first:"
  echo "  ./benchmarks/fetch_corpora.sh"
  echo ""
  echo "Or specify QNDX_BENCH_CORPUS directly:"
  echo "  QNDX_BENCH_CORPUS=/path/to/repo cargo bench --bench real_corpus"
  exit 1
fi

# ── Run benchmarks ──────────────────────────────────────────────────

printf "\n${BOLD}Standard Corpus Benchmarks${RESET}\n"
printf "Corpora dir: ${CORPORA_DIR}\n"
printf "Corpora:     ${CORPORA[*]}\n"
if [[ ${#CRITERION_ARGS[@]} -gt 0 ]]; then
  printf "Extra args:  ${CRITERION_ARGS[*]}\n"
fi
printf "\n"

passed=0
failed=0
skipped=0

for name in "${CORPORA[@]}"; do
  corpus_path="${CORPORA_DIR}/${name}"

  if [[ ! -d "${corpus_path}" ]]; then
    warn "${name}: not downloaded, skipping"
    ((skipped++))
    continue
  fi

  # Determine patterns file
  patterns_file="${PATTERNS_MAP[${name}]:-}"
  patterns_path=""
  if [[ -n "${patterns_file}" && -f "${PROJECT_ROOT}/${patterns_file}" ]]; then
    patterns_path="${PROJECT_ROOT}/${patterns_file}"
  fi

  printf "${BOLD}=== ${name} ===${RESET}\n"
  info "Corpus: ${corpus_path}"
  if [[ -n "${patterns_path}" ]]; then
    info "Patterns: ${patterns_path}"
  fi
  printf "\n"

  # Build the environment
  export QNDX_BENCH_CORPUS="${corpus_path}"
  export QNDX_BENCH_NAME="${name}"
  if [[ -n "${patterns_path}" ]]; then
    export QNDX_BENCH_PATTERNS="${patterns_path}"
  else
    unset QNDX_BENCH_PATTERNS 2>/dev/null || true
  fi

  if [[ "${SUMMARY_ONLY}" == "true" ]]; then
    # Run with a minimal sample to just get the summary table
    export QNDX_BENCH_SUMMARY_ONLY=1
    if cargo bench -p qndx-bench --bench real_corpus -- --test "${CRITERION_ARGS[@]+"${CRITERION_ARGS[@]}"}"; then
      ((passed++))
    else
      ((failed++))
    fi
  else
    if cargo bench -p qndx-bench --bench real_corpus -- ${CRITERION_ARGS[@]+"${CRITERION_ARGS[@]}"}; then
      ((passed++))
    else
      err "${name}: benchmark failed"
      ((failed++))
    fi
  fi

  printf "\n"
done

# ── Summary ─────────────────────────────────────────────────────────

printf "${BOLD}=== Summary ===${RESET}\n"
printf "  Passed:  ${passed}\n"
printf "  Failed:  ${failed}\n"
printf "  Skipped: ${skipped}\n"
printf "\n"

if (( failed > 0 )); then
  exit 1
fi
