#!/usr/bin/env bash
#
# fetch_corpora.sh — Download standard benchmark corpora for qndx.
#
# Clones well-known open-source repositories at --depth 1 into
# benchmarks/corpora/<name>/.  Skips any corpus that already exists.
#
# Usage:
#   ./benchmarks/fetch_corpora.sh              # all corpora
#   ./benchmarks/fetch_corpora.sh rust linux    # specific corpora
#   ./benchmarks/fetch_corpora.sh --list        # show available corpora
#   ./benchmarks/fetch_corpora.sh --clean       # remove all downloaded corpora
#
# The downloaded directories are gitignored (benchmarks/corpora/).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORPORA_DIR="${SCRIPT_DIR}/corpora"

# ── Corpus definitions ──────────────────────────────────────────────
# Format: name|git_url|description|approx_size
CORPORA=(
  "rust|https://github.com/rust-lang/rust.git|Rust compiler and stdlib (~35K files, ~500 MB)|small"
  "linux|https://github.com/torvalds/linux.git|Linux kernel (~75K files, ~1.2 GB)|medium"
  "kubernetes|https://github.com/kubernetes/kubernetes.git|Kubernetes (~15K files, ~200 MB)|medium"
  "chromium|https://chromium.googlesource.com/chromium/src.git|Chromium browser (~400K files, ~20 GB)|large"
)

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

corpus_field() {
  local entry="$1" field="$2"
  echo "$entry" | cut -d'|' -f"$field"
}

list_corpora() {
  printf "\n${BOLD}Available standard corpora:${RESET}\n\n"
  printf "  %-14s %-8s %s\n" "NAME" "TIER" "DESCRIPTION"
  printf "  %-14s %-8s %s\n" "----" "----" "-----------"
  for entry in "${CORPORA[@]}"; do
    local name desc tier
    name="$(corpus_field "$entry" 1)"
    desc="$(corpus_field "$entry" 3)"
    tier="$(corpus_field "$entry" 4)"
    local status=""
    if [[ -d "${CORPORA_DIR}/${name}" ]]; then
      status=" ${GREEN}(downloaded)${RESET}"
    fi
    printf "  %-14s %-8s %b\n" "$name" "$tier" "${desc}${status}"
  done
  printf "\n"
}

clean_corpora() {
  if [[ -d "${CORPORA_DIR}" ]]; then
    info "Removing ${CORPORA_DIR}..."
    rm -rf "${CORPORA_DIR}"
    ok "Cleaned."
  else
    info "Nothing to clean."
  fi
}

fetch_one() {
  local entry="$1"
  local name url desc tier
  name="$(corpus_field "$entry" 1)"
  url="$(corpus_field "$entry" 2)"
  desc="$(corpus_field "$entry" 3)"
  tier="$(corpus_field "$entry" 4)"

  local dest="${CORPORA_DIR}/${name}"

  if [[ -d "${dest}" ]]; then
    ok "${name}: already present at ${dest}"
    return 0
  fi

  info "${name}: cloning (${desc})..."
  mkdir -p "${CORPORA_DIR}"

  # Use --depth 1 to avoid downloading full history.
  # --filter=blob:none would be faster but some servers don't support it.
  if git clone --depth 1 --single-branch "${url}" "${dest}" 2>&1 | \
     sed 's/^/    /'; then
    # Count files and total size
    local file_count total_bytes
    file_count="$(find "${dest}" -type f -not -path '*/.git/*' | wc -l | tr -d ' ')"
    total_bytes="$(find "${dest}" -type f -not -path '*/.git/*' -exec stat -f%z {} + 2>/dev/null | awk '{s+=$1} END {print s}' || \
                   find "${dest}" -type f -not -path '*/.git/*' -printf '%s\n' 2>/dev/null | awk '{s+=$1} END {print s}' || \
                   echo "unknown")"
    ok "${name}: ${file_count} files, $(human_bytes "${total_bytes}")"
  else
    err "${name}: clone failed"
    rm -rf "${dest}"
    return 1
  fi
}

human_bytes() {
  local bytes="${1:-0}"
  if [[ "${bytes}" == "unknown" ]]; then
    echo "unknown size"
    return
  fi
  if (( bytes >= 1073741824 )); then
    awk "BEGIN { printf \"%.1f GB\", ${bytes}/1073741824 }"
  elif (( bytes >= 1048576 )); then
    awk "BEGIN { printf \"%.1f MB\", ${bytes}/1048576 }"
  elif (( bytes >= 1024 )); then
    awk "BEGIN { printf \"%.1f KB\", ${bytes}/1024 }"
  else
    echo "${bytes} B"
  fi
}

find_entry() {
  local target="$1"
  for entry in "${CORPORA[@]}"; do
    local name
    name="$(corpus_field "$entry" 1)"
    if [[ "${name}" == "${target}" ]]; then
      echo "${entry}"
      return 0
    fi
  done
  return 1
}

# ── Main ────────────────────────────────────────────────────────────

if [[ $# -eq 1 && "$1" == "--list" ]]; then
  list_corpora
  exit 0
fi

if [[ $# -eq 1 && "$1" == "--clean" ]]; then
  clean_corpora
  exit 0
fi

if [[ $# -eq 1 && "$1" == "--help" ]]; then
  cat <<'EOF'
Usage: fetch_corpora.sh [OPTIONS] [CORPUS...]

Download standard benchmark corpora for qndx.

Arguments:
  CORPUS...          Names of corpora to download (default: all except large tier)

Options:
  --list             List available corpora and their status
  --clean            Remove all downloaded corpora
  --all              Download all corpora including large tier
  --help             Show this help

Corpora are cloned with --depth 1 into benchmarks/corpora/<name>/.

Environment:
  QNDX_CORPORA_DIR   Override the download directory
                      (default: benchmarks/corpora/)

Examples:
  ./benchmarks/fetch_corpora.sh                  # rust + linux + kubernetes
  ./benchmarks/fetch_corpora.sh rust             # just rust
  ./benchmarks/fetch_corpora.sh --all            # everything including chromium
  QNDX_BENCH_CORPUS=benchmarks/corpora/linux cargo bench --bench real_corpus
EOF
  exit 0
fi

# Override corpora dir from env
if [[ -n "${QNDX_CORPORA_DIR:-}" ]]; then
  CORPORA_DIR="${QNDX_CORPORA_DIR}"
fi

printf "\n${BOLD}qndx benchmark corpora${RESET}\n"
printf "Download directory: ${CORPORA_DIR}\n\n"

failed=0

if [[ $# -eq 0 ]]; then
  # Default: fetch everything except large tier
  for entry in "${CORPORA[@]}"; do
    tier="$(corpus_field "$entry" 4)"
    if [[ "${tier}" == "large" ]]; then
      name="$(corpus_field "$entry" 1)"
      warn "${name}: skipping large corpus (use --all or name it explicitly)"
      continue
    fi
    fetch_one "${entry}" || ((failed++))
  done
elif [[ $# -eq 1 && "$1" == "--all" ]]; then
  for entry in "${CORPORA[@]}"; do
    fetch_one "${entry}" || ((failed++))
  done
else
  # Fetch named corpora
  for name in "$@"; do
    entry="$(find_entry "${name}" || true)"
    if [[ -z "${entry}" ]]; then
      err "Unknown corpus: ${name}"
      warn "Run with --list to see available corpora"
      ((failed++))
      continue
    fi
    fetch_one "${entry}" || ((failed++))
  done
fi

printf "\n"
if (( failed > 0 )); then
  err "${failed} corpus download(s) failed"
  exit 1
else
  ok "Done. Run benchmarks with:"
  printf "\n"
  # Show example for first downloaded corpus
  for entry in "${CORPORA[@]}"; do
    name="$(corpus_field "$entry" 1)"
    if [[ -d "${CORPORA_DIR}/${name}" ]]; then
      printf "  QNDX_BENCH_CORPUS=${CORPORA_DIR}/${name} \\\\\n"
      if [[ -f "${SCRIPT_DIR}/patterns/${name}.txt" ]]; then
        printf "  QNDX_BENCH_PATTERNS=benchmarks/patterns/${name}.txt \\\\\n"
      fi
      printf "  cargo bench --bench real_corpus\n\n"
      printf "  Or run all standard corpora:\n"
      printf "  ./benchmarks/run_standard_benches.sh\n\n"
      break
    fi
  done
fi
