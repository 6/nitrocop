#!/usr/bin/env bash
#
# Clone all corpus repos into vendor/corpus/<repo_id>/ for local investigation.
#
# Usage:
#   bench/corpus/clone_repos.sh              # clone all corpus repos
#   bench/corpus/clone_repos.sh --jobs 8     # parallel clones (default: 4)
#   bench/corpus/clone_repos.sh --dry-run    # show what would be cloned
#
# Repos are cloned at depth 1 using the exact pinned SHA from the manifest,
# with no submodules. Already-cloned repos are skipped (safe to re-run).
#
# Estimated disk usage: ~12GB for all repos.

set -euo pipefail

# Safety: block on devcontainers and macOS where disk space is limited.
# Use check_cop.py --rerun --clone --sample N for targeted corpus checks instead.
if [[ "$(uname -s)" == "Darwin" ]]; then
    echo "ERROR: clone_repos.sh is not intended for macOS (limited disk)." >&2
    echo "Use: python3 scripts/check_cop.py Department/CopName --rerun --clone --sample 15" >&2
    exit 1
fi
if [[ "${USER:-}" == "vscode" ]] && [[ -f /.dockerenv ]]; then
    echo "ERROR: clone_repos.sh is not intended for devcontainers (limited disk)." >&2
    echo "Use: python3 scripts/check_cop.py Department/CopName --rerun --clone --sample 15" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MANIFEST="$REPO_ROOT/bench/corpus/manifest.jsonl"
DEST_DIR="$REPO_ROOT/vendor/corpus"
JOBS=4
DRY_RUN=false
CLONE_TIMEOUT=180
FETCH_TIMEOUT=120

while [[ $# -gt 0 ]]; do
    case "$1" in
        --jobs|-j) JOBS="$2"; shift 2 ;;
        --dry-run) DRY_RUN=true; shift ;;
        --clone-timeout) CLONE_TIMEOUT="$2"; shift 2 ;;
        --fetch-timeout) FETCH_TIMEOUT="$2"; shift 2 ;;
        --help|-h)
            echo "Usage: $0 [--jobs N] [--dry-run] [--clone-timeout SEC] [--fetch-timeout SEC]"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ ! -f "$MANIFEST" ]]; then
    echo "ERROR: manifest not found: $MANIFEST" >&2
    exit 1
fi

TOTAL=$(wc -l < "$MANIFEST" | tr -d ' ')
echo "Corpus: $TOTAL repos → $DEST_DIR"
echo "Parallelism: $JOBS"
echo "Clone timeout: ${CLONE_TIMEOUT}s, fetch timeout: ${FETCH_TIMEOUT}s"
echo ""

mkdir -p "$DEST_DIR"

# Parse manifest into arrays (bash 3 compatible; avoid mapfile/readarray)
TMP_IDS="$(mktemp)"
TMP_URLS="$(mktemp)"
TMP_SHAS="$(mktemp)"
FAILED_FILE="$(mktemp -t corpus-clone-failed.XXXXXX)"
trap 'rm -f "$TMP_IDS" "$TMP_URLS" "$TMP_SHAS" "$FAILED_FILE"' EXIT

python3 - "$MANIFEST" "$TMP_IDS" "$TMP_URLS" "$TMP_SHAS" <<'PY'
import json
import sys

manifest, ids_out, urls_out, shas_out = sys.argv[1:5]

with open(manifest, "r", encoding="utf-8") as f, \
        open(ids_out, "w", encoding="utf-8") as ids_f, \
        open(urls_out, "w", encoding="utf-8") as urls_f, \
        open(shas_out, "w", encoding="utf-8") as shas_f:
    for line in f:
        r = json.loads(line.strip())
        print(r["id"], file=ids_f)
        print(r["repo_url"], file=urls_f)
        print(r["sha"], file=shas_f)
PY

IDS=()
URLS=()
SHAS=()
while IFS= read -r line; do IDS+=("$line"); done < "$TMP_IDS"
while IFS= read -r line; do URLS+=("$line"); done < "$TMP_URLS"
while IFS= read -r line; do SHAS+=("$line"); done < "$TMP_SHAS"

RUNNING=0

# Avoid hanging on auth prompts for missing/private repos.
export GIT_TERMINAL_PROMPT=0
export GIT_ASKPASS=/bin/false

run_git_with_timeout() {
    local timeout_secs="$1"
    shift
    python3 - "$timeout_secs" "$@" <<'PY'
import subprocess
import sys

timeout = int(sys.argv[1])
cmd = sys.argv[2:]
try:
    proc = subprocess.run(
        cmd,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=timeout,
    )
    sys.exit(proc.returncode)
except subprocess.TimeoutExpired:
    sys.exit(124)
PY
}

clone_one() {
    local id="$1" repo_url="$2" sha="$3"
    local dest="$DEST_DIR/$id"

    # Skip if already cloned at the right SHA
    if [[ -d "$dest/.git" ]]; then
        local current_sha
        current_sha=$(git -C "$dest" rev-parse HEAD 2>/dev/null || echo "")
        if [[ "$current_sha" == "$sha"* ]]; then
            echo "SKIP  $id (already at ${sha:0:7})"
            return 0
        else
            echo "STALE $id (at ${current_sha:0:7}, want ${sha:0:7}) — removing"
            rm -rf "$dest"
        fi
    fi

    if $DRY_RUN; then
        echo "WOULD $id ← $repo_url @ ${sha:0:7}"
        return 0
    fi

    # Fetch exactly the pinned SHA at depth 1 (minimal clone)
    if ! run_git_with_timeout "$CLONE_TIMEOUT" git init -q "$dest"; then
        echo "FAIL  $id (init failed)" >&2
        echo "$id" >> "$FAILED_FILE"
        return 1
    fi

    if ! run_git_with_timeout "$CLONE_TIMEOUT" git -C "$dest" fetch --depth 1 -q "$repo_url" "$sha"; then
        # Fallback: some hosts don't allow fetching arbitrary SHAs
        if ! run_git_with_timeout "$CLONE_TIMEOUT" git -C "$dest" fetch --depth 1 -q "$repo_url"; then
            echo "FAIL  $id (fetch failed)" >&2
            echo "$id" >> "$FAILED_FILE"
            rm -rf "$dest"
            return 1
        fi
    fi

    if ! run_git_with_timeout "$FETCH_TIMEOUT" git -C "$dest" checkout -q FETCH_HEAD; then
        echo "WARN  $id (checkout failed, using fetched HEAD)" >&2
    fi

    echo "OK    $id (${sha:0:7})"
}

wait_one_pid() {
    local pid="$1"
    wait "$pid" || true
}

PIDS=()
for i in "${!IDS[@]}"; do
    clone_one "${IDS[$i]}" "${URLS[$i]}" "${SHAS[$i]}" &
    PIDS+=("$!")

    RUNNING=$(( RUNNING + 1 ))
    if (( RUNNING >= JOBS )); then
        wait_one_pid "${PIDS[0]}"
        PIDS=("${PIDS[@]:1}")
        RUNNING=$(( RUNNING - 1 ))
    fi
done

# Wait for remaining jobs
for pid in "${PIDS[@]}"; do
    wait_one_pid "$pid"
done

echo ""

# Remove stale repos not in the manifest (e.g., denylisted or removed repos)
STALE_COUNT=0
for dir in "$DEST_DIR"/*/; do
    dir_name="$(basename "$dir")"
    if ! grep -q "\"$dir_name\"" "$MANIFEST"; then
        echo "STALE $dir_name (not in manifest) — removing"
        rm -rf "$dir"
        STALE_COUNT=$(( STALE_COUNT + 1 ))
    fi
done
if [[ "$STALE_COUNT" -gt 0 ]]; then
    echo "Removed $STALE_COUNT stale repos"
fi

TOTAL_CLONED=$(find "$DEST_DIR" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
FAILED_COUNT=$(wc -l < "$FAILED_FILE" | tr -d ' ')
echo "Done. $TOTAL_CLONED/$TOTAL repos in $DEST_DIR"
echo "Failed in this run: $FAILED_COUNT"
if [[ "$FAILED_COUNT" != "0" ]]; then
    RETRY_LIST="$REPO_ROOT/bench/corpus/failed_repos.txt"
    cp "$FAILED_FILE" "$RETRY_LIST"
    echo "Failed repo IDs saved to: $RETRY_LIST"
fi
du -sh "$DEST_DIR" 2>/dev/null || true
