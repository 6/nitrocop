#!/usr/bin/env python3
"""Tests for verify_cop_locations.py — SKIP vs FIXED distinction."""
import importlib.util
import os
import sys
from pathlib import Path
from unittest.mock import patch

# verify_cop_locations.py imports from shared.corpus_artifacts which lives
# under scripts/.  Add scripts/ to sys.path so the import resolves.
_scripts_dir = str(Path(__file__).parents[2] / "scripts")
if _scripts_dir not in sys.path:
    sys.path.insert(0, _scripts_dir)

SCRIPT = Path(__file__).parents[2] / "scripts" / "verify_cop_locations.py"
SPEC = importlib.util.spec_from_file_location("verify_cop_locations", SCRIPT)
assert SPEC and SPEC.loader
vcl = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(vcl)


def test_run_nitrocop_on_repo_returns_none_when_repo_dir_missing(tmp_path):
    """When the corpus repo directory doesn't exist, return None (not empty sets)."""
    corpus_dir = tmp_path / "corpus"
    corpus_dir.mkdir()
    # repo dir does NOT exist
    result = vcl.run_nitrocop_on_repo(
        Path("/nonexistent/nitrocop"),
        corpus_dir,
        Path("/nonexistent/config.yml"),
        "some_repo__abc123",
        ["lib/foo.rb", "lib/bar.rb"],
        "Style/Foo",
    )
    assert result is None


def test_run_nitrocop_on_repo_returns_none_when_no_files_exist(tmp_path):
    """When the repo dir exists but none of the requested files are on disk,
    return None."""
    corpus_dir = tmp_path / "corpus"
    repo_dir = corpus_dir / "some_repo__abc123"
    repo_dir.mkdir(parents=True)
    result = vcl.run_nitrocop_on_repo(
        Path("/nonexistent/nitrocop"),
        corpus_dir,
        Path("/nonexistent/config.yml"),
        "some_repo__abc123",
        ["lib/foo.rb"],  # file doesn't exist in repo_dir
        "Style/Foo",
    )
    assert result is None


def test_parse_loc():
    assert vcl.parse_loc("repo__id__abc123: path/to/file.rb:42") == (
        "repo__id__abc123", "path/to/file.rb", 42,
    )
    assert vcl.parse_loc("r: a.rb:1") == ("r", "a.rb", 1)
    # Path with colon
    assert vcl.parse_loc("repo: dir/sub:file.rb:99") == (
        "repo", "dir/sub:file.rb", 99,
    )


def test_exit_code_nonzero_when_all_skipped(tmp_path, capsys):
    """When all locations are skipped (no repos cloned), exit code should be 1
    (not 0, which would falsely signal 'all fixed')."""
    # Create minimal corpus-results.json
    import json
    results_file = tmp_path / "corpus-results.json"
    results_file.write_text(json.dumps({
        "by_cop": [{
            "cop": "Style/Foo",
            "fp": 1, "fn": 0, "matches": 10,
            "fp_examples": [{"loc": "missing_repo__abc: lib/foo.rb:5", "msg": "bad"}],
            "fn_examples": [],
        }],
    }))

    # Patch ensure_fresh_release_binary to no-op, and set corpus_dir to empty
    with patch.object(vcl, "ensure_fresh_release_binary"):
        with patch("sys.exit") as mock_exit:
            # Patch sys.argv for argparse
            import sys
            old_argv = sys.argv
            sys.argv = ["verify_cop_locations.py", "Style/Foo", "--input", str(results_file)]
            # Patch corpus_dir / nitrocop_bin via find_project_root
            with patch.object(vcl, "find_project_root", return_value=tmp_path):
                # Create the vendor/corpus dir but no repos inside
                (tmp_path / "vendor" / "corpus").mkdir(parents=True)
                vcl.main()
            sys.argv = old_argv

    # Should exit with non-zero (1 = issues remain or nothing checked)
    mock_exit.assert_called()
    exit_code = mock_exit.call_args[0][0]
    assert exit_code != 0, "Exit code should be non-zero when all locations skipped"

    captured = capsys.readouterr()
    assert "SKIP" in captured.out
    assert "skipped" in captured.out.lower()


def test_exit_code_2_in_ci_when_all_skipped(tmp_path, capsys):
    """In CI (CI env var set), exit with code 2 when all repos are skipped."""
    import json
    results_file = tmp_path / "corpus-results.json"
    results_file.write_text(json.dumps({
        "by_cop": [{
            "cop": "Style/Foo",
            "fp": 1, "fn": 0, "matches": 10,
            "fp_examples": [{"loc": "missing_repo__abc: lib/foo.rb:5", "msg": "bad"}],
            "fn_examples": [],
        }],
    }))

    with patch.object(vcl, "ensure_fresh_release_binary"):
        with patch("sys.exit") as mock_exit:
            import sys
            old_argv = sys.argv
            sys.argv = ["verify_cop_locations.py", "Style/Foo", "--input", str(results_file)]
            with patch.object(vcl, "find_project_root", return_value=tmp_path):
                (tmp_path / "vendor" / "corpus").mkdir(parents=True)
                # Set CI env var
                with patch.dict(os.environ, {"CI": "true"}):
                    vcl.main()
            sys.argv = old_argv

    mock_exit.assert_called()
    exit_code = mock_exit.call_args[0][0]
    assert exit_code == 2, f"Expected exit code 2 in CI, got {exit_code}"


def test_run_nitrocop_on_repo_passes_config_override(tmp_path):
    """When config_override is set, it is used instead of per-repo resolved config."""
    corpus_dir = tmp_path / "corpus"
    repo_dir = corpus_dir / "test_repo"
    repo_dir.mkdir(parents=True)
    (repo_dir / "app.rb").write_text("puts 'hello'\n")

    # Create a fake binary that outputs no offenses
    fake_bin = tmp_path / "fake_nitrocop"
    fake_bin.write_text('#!/bin/sh\necho \'{"offenses":[]}\'\n')
    fake_bin.chmod(0o755)

    override_config = str(tmp_path / "override.yml")
    Path(override_config).write_text("# override\n")

    # Capture the command used by patching subprocess.run
    import subprocess
    cmds_seen = []
    original_run = subprocess.run

    def capture_run(cmd, **kwargs):
        cmds_seen.append(cmd)
        return original_run(cmd, **kwargs)

    with patch("subprocess.run", side_effect=capture_run):
        vcl.run_nitrocop_on_repo(
            Path(fake_bin), corpus_dir, Path("/unused/config.yml"),
            "test_repo", ["app.rb"], "Style/Foo",
            config_override=override_config,
        )

    # The nitrocop command should use the override config
    assert any(override_config in " ".join(str(a) for a in cmd) for cmd in cmds_seen), (
        f"Expected override config '{override_config}' in command, got: {cmds_seen}"
    )


def test_run_nitrocop_on_repo_uses_resolved_config_without_override(tmp_path):
    """Without config_override, the per-repo resolved config is used."""
    corpus_dir = tmp_path / "corpus"
    repo_dir = corpus_dir / "test_repo"
    repo_dir.mkdir(parents=True)
    (repo_dir / "app.rb").write_text("puts 'hello'\n")

    fake_bin = tmp_path / "fake_nitrocop"
    fake_bin.write_text('#!/bin/sh\necho \'{"offenses":[]}\'\n')
    fake_bin.chmod(0o755)

    import subprocess
    cmds_seen = []
    original_run = subprocess.run

    def capture_run(cmd, **kwargs):
        cmds_seen.append(cmd)
        return original_run(cmd, **kwargs)

    # Mock resolve_repo_config to return a known path
    resolved_path = "/resolved/config.yml"
    with patch("subprocess.run", side_effect=capture_run):
        with patch("run_nitrocop.resolve_repo_config", return_value=resolved_path):
            vcl.run_nitrocop_on_repo(
                Path(fake_bin), corpus_dir, Path("/unused/config.yml"),
                "test_repo", ["app.rb"], "Style/Foo",
            )

    assert any(resolved_path in " ".join(str(a) for a in cmd) for cmd in cmds_seen), (
        f"Expected resolved config '{resolved_path}' in command, got: {cmds_seen}"
    )
