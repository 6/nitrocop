#!/usr/bin/env python3
"""Compute the list of include-gated cops from vendored plugin configs.

A cop is "include-gated" when its Include pattern (or a department-level
Include it inherits) does not start with `**/`. Such patterns can't match
absolute file paths when RuboCop is invoked with a config file outside the
target repo, which is how the corpus oracle's main pipeline runs — so these
cops get zero data there and need a parallel ig-pipeline run with a repo-
local config. See docs/investigations/investigation-target-dir-relativization.md
for the full story.

Parses:
- vendor/rubocop/config/default.yml          (core cops — mostly **/-prefixed)
- vendor/rubocop-rails/config/default.yml    (Rails/AddColumnIndex, etc.)
- vendor/rubocop-rake/config/default.yml     (Rake/* — inherit Rakefile)
- vendor/rubocop-rspec/config/default.yml
- vendor/rubocop-performance/config/default.yml
- vendor/rubocop-factory_bot/config/default.yml
- vendor/rubocop-rspec_rails/config/default.yml

Emits a single comma-separated line of cop names suitable for pasting into
the oracle workflow's `--only $IG_COPS` flag.

Usage:
    python3 bench/corpus/compute_ig_cops.py
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    print("PyYAML required: pip install pyyaml", file=sys.stderr)
    sys.exit(1)


class _RubyTagIgnoringLoader(yaml.SafeLoader):
    """SafeLoader that treats unknown Ruby-specific tags (e.g. `!ruby/regexp`
    in rubocop-core's default.yml) as opaque strings instead of failing."""


def _ignore_unknown_tag(loader, tag_suffix, node):
    return loader.construct_scalar(node) if hasattr(node, "value") else None


_RubyTagIgnoringLoader.add_multi_constructor("!ruby/", _ignore_unknown_tag)
_RubyTagIgnoringLoader.add_multi_constructor("!", _ignore_unknown_tag)


REPO_ROOT = Path(__file__).resolve().parents[2]
VENDOR_DIRS = [
    "vendor/rubocop",
    "vendor/rubocop-rails",
    "vendor/rubocop-rake",
    "vendor/rubocop-rspec",
    "vendor/rubocop-performance",
    "vendor/rubocop-factory_bot",
    "vendor/rubocop-rspec_rails",
]


def is_repo_relative_pattern(pattern: str) -> bool:
    """True if pattern needs repo-relative resolution to match corpus files.

    RuboCop resolves cop-level Include patterns against
    `config.path_relative_to_config(file)`. When the config lives outside
    the repo (corpus oracle main pipeline), that relativization fails and
    patterns without a `**/` prefix never match any file.
    """
    return not pattern.startswith("**/") and not pattern.startswith("/")


def collect_ig_cops(config_path: Path) -> set[str]:
    """Return cops in this config whose effective Include is repo-relative.

    Walks every cop section, tracks department-level Include as the
    inherited default for cops that don't override it, and yields the cop
    name when any applicable Include pattern is repo-relative. Department-
    only keys (no slash in the name) are expanded to every cop in that
    department that doesn't carry its own Include list.
    """
    try:
        with open(config_path) as f:
            data = yaml.load(f, Loader=_RubyTagIgnoringLoader) or {}
    except (OSError, yaml.YAMLError) as e:
        print(f"warning: failed to parse {config_path}: {e}", file=sys.stderr)
        return set()

    department_includes: dict[str, list[str]] = {}
    cop_includes: dict[str, list[str]] = {}
    for key, value in data.items():
        if not isinstance(value, dict):
            continue
        include = value.get("Include")
        if not isinstance(include, list):
            continue
        patterns = [str(p) for p in include]
        if "/" in key:
            cop_includes[key] = patterns
        else:
            department_includes[key] = patterns

    ig_cops: set[str] = set()

    for cop_name, patterns in cop_includes.items():
        if any(is_repo_relative_pattern(p) for p in patterns):
            ig_cops.add(cop_name)

    for dept, patterns in department_includes.items():
        if not any(is_repo_relative_pattern(p) for p in patterns):
            continue
        for key in data:
            if not isinstance(data.get(key), dict):
                continue
            if not key.startswith(f"{dept}/"):
                continue
            if key in cop_includes:
                continue
            ig_cops.add(key)

    return ig_cops


def main() -> None:
    ig_cops: set[str] = set()
    for vendor_dir in VENDOR_DIRS:
        config_path = REPO_ROOT / vendor_dir / "config" / "default.yml"
        if not config_path.exists():
            continue
        ig_cops |= collect_ig_cops(config_path)

    print(",".join(sorted(ig_cops)))


if __name__ == "__main__":
    main()
