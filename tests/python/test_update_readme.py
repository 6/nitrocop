#!/usr/bin/env python3
"""Smoke tests for update_readme.py."""

import json
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPT = Path(__file__).parents[2] / "bench" / "corpus" / "update_readme.py"


SAMPLE_README = """\
# nitrocop

Features:
- **93.0% conformance** across a corpus of open-source repos
- Tested against [**500 open-source repos**](docs/corpus.md) (163k Ruby files)

## Cops

<!-- corpus-cops:start -->
Old generated cops section.
<!-- corpus-cops:end -->

Every cop reads its RuboCop YAML config options and has fixture-based test coverage.

More text here.
"""


SAMPLE_README_COMMA = SAMPLE_README.replace(
    "[**500 open-source repos**]",
    "[**1,000 open-source repos**]",
)


def sample_by_department() -> list[dict]:
    """Return full department coverage for the generated Cops section."""
    counts = {
        "Layout": (1, 1, 0, 1, 1, 0),
        "Lint": (1, 0, 0, 0, 0, 0),
        "Style": (1, 1, 1, 0, 0, 0),
        "Metrics": (1, 1, 0, 1, 1, 0),
        "Naming": (1, 1, 1, 0, 0, 0),
        "Security": (1, 0, 0, 0, 0, 0),
        "Bundler": (1, 1, 1, 0, 0, 0),
        "Gemspec": (1, 0, 0, 0, 0, 0),
        "Migration": (1, 1, 1, 0, 0, 0),
        "Rails": (1, 1, 0, 1, 1, 0),
        "Performance": (1, 1, 1, 0, 0, 0),
        "RSpec": (1, 1, 0, 1, 1, 0),
        "RSpecRails": (1, 0, 0, 0, 0, 0),
        "FactoryBot": (1, 1, 1, 0, 0, 0),
    }
    rows = []
    for department, (cops, seen, perfect, diverging, fp, fn) in counts.items():
        total = 100 if seen else 0
        match_rate = 1.0 if total > 0 and fp == 0 and fn == 0 else 0.99 if total > 0 else 1.0
        rows.append({
            "department": department,
            "cops": cops,
            "exercised_cops": seen,
            "perfect_cops": perfect,
            "diverging_cops": diverging,
            "inactive_cops": cops - seen,
            "matches": total,
            "fp": fp,
            "fn": fn,
            "match_rate": match_rate,
        })
    return rows


def make_corpus(tmp: Path, *, fp: int = 100000, fn: int = 100000,
                matches: int = 4900000) -> tuple[Path, Path, Path]:
    """Write minimal corpus-results.json, manifest.jsonl, and README.md."""
    by_department = sample_by_department()
    total = matches + fp + fn
    corpus = {
        "schema": 1,
        "baseline": {
            "rubocop": "1.84.2",
            "rubocop-rails": "2.34.3",
            "rubocop-performance": "1.26.1",
            "rubocop-rspec": "3.9.0",
            "rubocop-rspec_rails": "2.32.0",
            "rubocop-factory_bot": "2.28.0",
        },
        "summary": {
            "total_repos": 500,
            "repos_perfect": 100,
            "repos_error": 0,
            "total_offenses_compared": total,
            "matches": matches,
            "fp": fp,
            "fn": fn,
            "registered_cops": 14,
            "perfect_cops": 6,
            "diverging_cops": 4,
            "inactive_cops": 4,
            "overall_match_rate": matches / total if total > 0 else 0,
            "total_files_inspected": 167000,
        },
        "by_department": by_department,
        "by_repo": [
            {
                "repo": "rails__rails__abc123",
                "status": "ok",
                "match_rate": 0.96,
                "matches": 11520,
                "fp": 240,
                "fn": 240,
                "files_inspected": 3100,
            },
        ],
    }

    manifest_entry = {
        "id": "rails__rails__abc123",
        "repo_url": "https://github.com/rails/rails",
        "notes": "auto-discovered, 55000 stars",
    }

    input_path = tmp / "corpus-results.json"
    input_path.write_text(json.dumps(corpus))

    manifest_path = tmp / "manifest.jsonl"
    manifest_path.write_text(json.dumps(manifest_entry) + "\n")

    readme_path = tmp / "README.md"
    readme_path.write_text(SAMPLE_README)

    return input_path, manifest_path, readme_path


def test_dry_run():
    """Run update_readme.py --dry-run and verify it exits 0 without modifying README."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        input_path, manifest_path, readme_path = make_corpus(tmp)

        result = subprocess.run(
            [
                sys.executable, str(SCRIPT),
                "--input", str(input_path),
                "--manifest", str(manifest_path),
                "--readme", str(readme_path),
                "--dry-run",
            ],
            capture_output=True,
            text=True,
        )

        assert result.returncode == 0, f"Script failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        assert readme_path.read_text() == SAMPLE_README


def test_write():
    """Run update_readme.py and verify it updates README content."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        input_path, manifest_path, readme_path = make_corpus(tmp)

        result = subprocess.run(
            [
                sys.executable, str(SCRIPT),
                "--input", str(input_path),
                "--manifest", str(manifest_path),
                "--readme", str(readme_path),
            ],
            capture_output=True,
            text=True,
        )

        assert result.returncode == 0, f"Script failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"

        updated = readme_path.read_text()
        assert "6 of 14 cops match RuboCop exactly" in updated
        assert "500 open-source repos" in updated
        assert "Compared with RuboCop on [**500 open-source repos**](docs/corpus.md)" in updated
        assert "5.1M offenses compared" in updated
        assert "See [docs/corpus.md](docs/corpus.md) for the full corpus breakdown." in updated
        assert "| Department | Cops | Exact match (default) |" in updated
        assert "**[rubocop](https://github.com/rubocop/rubocop)** `1.84.2` (9 cops)" in updated
        assert "| Performance | 1 | **1 ✓** |" in updated
        assert "| **Total** | **1** |" not in updated
        assert "Old generated cops section." not in updated
        assert "## Conformance" not in updated
        assert "Per-repo results (top 15 by GitHub stars):" not in updated
        assert "| RuboCop offenses |" not in updated


def test_conformance_includes_fp():
    """Conformance rate should be matches/(matches+fp+fn), not matches/(matches+fn)."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        input_path, manifest_path, readme_path = make_corpus(
            tmp, matches=10_000_000, fp=120_000, fn=500_000,
        )

        result = subprocess.run(
            [
                sys.executable, str(SCRIPT),
                "--input", str(input_path),
                "--manifest", str(manifest_path),
                "--readme", str(readme_path),
            ],
            capture_output=True,
            text=True,
        )

        assert result.returncode == 0, f"Script failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"

        updated = readme_path.read_text()
        # Headline should show cop exact-match count, not percentage
        assert "6 of 14 cops match RuboCop exactly" in updated


def test_comma_repo_count():
    """Repo count regex should handle comma-formatted numbers (e.g. '1,000')."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        input_path, manifest_path, readme_path = make_corpus(tmp)
        readme_path.write_text(SAMPLE_README_COMMA)

        result = subprocess.run(
            [
                sys.executable, str(SCRIPT),
                "--input", str(input_path),
                "--manifest", str(manifest_path),
                "--readme", str(readme_path),
            ],
            capture_output=True,
            text=True,
        )

        assert result.returncode == 0, f"Script failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"

        updated = readme_path.read_text()
        assert "1,500" not in updated
        assert "1,1000" not in updated
        assert "500 open-source repos" in updated


if __name__ == "__main__":
    test_dry_run()
    test_write()
    test_conformance_includes_fp()
    test_comma_repo_count()
    print("OK: all tests passed")
