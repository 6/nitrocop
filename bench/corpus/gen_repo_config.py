#!/usr/bin/env python3
"""Generate a per-repo RuboCop config overlay inside the repo directory.

Writes a `.rubocop_corpus.yml` overlay inside <repo_dir> that inherits from
the base config. The `.rubocop`-prefixed name ensures RuboCop treats it like
any project-local config, so `path_relative_to_config(file)` resolves to a
repo-relative path. That repo-relative path is what cop-level Include
patterns like `db/**/*.rb`, `spec/**/*.rb`, and `Rakefile` need to match.

Without this, those patterns silently fail to match any file, and the cops
are effectively disabled in the oracle run — even though a real user running
RuboCop from the repo root would see them fire.

If the repo has file exclusions (from repo_excludes.json), the overlay also
includes those Exclude patterns.

Usage:
    python3 gen_repo_config.py <repo_id> <base_config> <repo_dir>

Always prints the path to the generated overlay (inside repo_dir).
"""
import json
import sys
from pathlib import Path

EXCLUDES_PATH = Path(__file__).parent / "repo_excludes.json"

# Vendor-ish directories excluded with absolute paths. The baseline config
# can't use relative patterns for these because they resolve relative to
# the config file's parent (bench/corpus/), not the repo dir.
GLOBAL_EXCLUDE_PATTERNS = [
    ".*/**/*",
    "vendor/**/*",
    "vendor*/**/*",
    "_vendor/**/*",
    "cookbooks/**/*",
]


def generate_repo_config(repo_id: str, base_config: str, repo_dir: str) -> str:
    """Generate an in-repo config overlay with file exclusions.

    Returns path to `<repo_dir>/.rubocop_corpus.yml`, a YAML overlay that
    inherits from *base_config* and adds absolute-path exclusions for vendor
    directories and any repo-specific patterns from ``repo_excludes.json``.
    """
    if not EXCLUDES_PATH.exists():
        repo_patterns: list[str] = []
    else:
        with open(EXCLUDES_PATH) as f:
            excludes = json.load(f)
        entry = excludes.get(repo_id)
        repo_patterns = entry.get("exclude", []) if entry else []

    all_patterns = GLOBAL_EXCLUDE_PATTERNS + repo_patterns

    abs_base = str(Path(base_config).resolve())
    abs_repo = str(Path(repo_dir).resolve())

    # RuboCop merges AllCops/Exclude by default (union), so we only need
    # to list the additional excludes here.
    lines = [f"inherit_from: {abs_base}"]
    if all_patterns:
        lines += ["", "AllCops:", "  Exclude:"]
        for pattern in all_patterns:
            lines.append(f'    - "{abs_repo}/{pattern}"')

    overlay_path = Path(abs_repo) / ".rubocop_corpus.yml"
    overlay_path.write_text("\n".join(lines) + "\n")
    return str(overlay_path)


def main():
    if len(sys.argv) != 4:
        print(f"Usage: {sys.argv[0]} <repo_id> <base_config> <repo_dir>", file=sys.stderr)
        sys.exit(1)

    repo_id, base_config, repo_dir = sys.argv[1], sys.argv[2], sys.argv[3]
    print(generate_repo_config(repo_id, base_config, repo_dir))


if __name__ == "__main__":
    main()
