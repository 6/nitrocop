#!/usr/bin/env python3
"""Validate that a rubocop JSON result only contains files under a repo directory.

Exits 0 if valid, 1 if any file path is outside the expected repo directory.
Used by corpus-oracle.yml to prevent cache poisoning from misconfigured runs.
"""

import json
import sys


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <result.json> <repo_dir>", file=sys.stderr)
        sys.exit(2)

    result_path, repo_dir = sys.argv[1], sys.argv[2]
    prefix = repo_dir.rstrip("/") + "/"

    try:
        with open(result_path) as f:
            data = json.load(f)
    except (json.JSONDecodeError, ValueError) as e:
        # TODO(corpus): This fires every run for jruby__jruby__0303464 because
        # rubocop produces empty JSON (suspected OOM on 7,445 .rb files).
        # The empty file causes the caller's "out-of-repo paths" warning,
        # which is misleading. Once the rubocop failure is diagnosed (see
        # corpus-oracle.yml TODO), this can be simplified.
        print(f"INVALID JSON in {result_path}: {e}", file=sys.stderr)
        sys.exit(1)

    for fobj in data.get("files", []):
        path = fobj.get("path", "")
        if not path.startswith(prefix):
            print(f"POISONED: {path} not under {prefix}", file=sys.stderr)
            sys.exit(1)


if __name__ == "__main__":
    main()
