#!/usr/bin/env python3
"""Tests for corpus_artifacts.py shared module.

Run with: python3 tests/python/shared/test_corpus_artifacts.py
"""

from __future__ import annotations

import io
import json
import os
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parents[3]
SCRIPTS_DIR = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS_DIR))

import shared.corpus_artifacts as cd


class TestDetectGithubRepo(unittest.TestCase):
    """Tests for _detect_github_repo()."""

    @patch("shared.corpus_artifacts.subprocess.run")
    def test_https_url(self, mock_run):
        mock_run.return_value = MagicMock(
            returncode=0, stdout="https://github.com/owner/repo.git\n"
        )
        self.assertEqual(cd._detect_github_repo(), "owner/repo")

    @patch("shared.corpus_artifacts.subprocess.run")
    def test_https_url_no_git_suffix(self, mock_run):
        mock_run.return_value = MagicMock(
            returncode=0, stdout="https://github.com/owner/repo\n"
        )
        self.assertEqual(cd._detect_github_repo(), "owner/repo")

    @patch("shared.corpus_artifacts.subprocess.run")
    def test_ssh_url(self, mock_run):
        mock_run.return_value = MagicMock(
            returncode=0, stdout="git@github.com:owner/repo.git\n"
        )
        self.assertEqual(cd._detect_github_repo(), "owner/repo")

    @patch("shared.corpus_artifacts.subprocess.run")
    def test_proxy_url(self, mock_run):
        mock_run.return_value = MagicMock(
            returncode=0,
            stdout="http://local_proxy@127.0.0.1:56237/git/myorg/myrepo\n",
        )
        self.assertEqual(cd._detect_github_repo(), "myorg/myrepo")

    @patch("shared.corpus_artifacts.subprocess.run")
    def test_no_remote(self, mock_run):
        mock_run.return_value = MagicMock(returncode=1, stdout="")
        self.assertIsNone(cd._detect_github_repo())

    @patch("shared.corpus_artifacts.subprocess.run")
    def test_unrecognized_url(self, mock_run):
        mock_run.return_value = MagicMock(
            returncode=0, stdout="https://gitlab.com/owner/repo.git\n"
        )
        self.assertIsNone(cd._detect_github_repo())


class TestCacheDir(unittest.TestCase):
    """Tests for _cache_dir()."""

    def test_creates_directory(self):
        d = cd._cache_dir()
        self.assertTrue(d.exists())
        self.assertTrue(d.is_dir())
        self.assertEqual(d.name, "nitrocop-corpus-cache")


class TestCleanStaleLocal(unittest.TestCase):
    """Tests for _clean_stale_local()."""

    def test_removes_existing_file(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            p = Path(tmpdir) / "corpus-results.json"
            p.write_text("{}")
            cd._clean_stale_local(Path(tmpdir))
            self.assertFalse(p.exists())

    def test_no_error_when_missing(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            cd._clean_stale_local(Path(tmpdir))


class TestTryGh(unittest.TestCase):
    """Tests for _try_gh()."""

    @patch("shared.corpus_artifacts.shutil.which", return_value=None)
    def test_gh_not_installed(self, _mock_which):
        self.assertIsNone(cd._try_gh("owner/repo"))

    @patch("shared.corpus_artifacts.subprocess.run")
    @patch("shared.corpus_artifacts.shutil.which", return_value="/usr/bin/gh")
    def test_gh_not_authenticated(self, _mock_which, mock_run):
        mock_run.return_value = MagicMock(returncode=1, stderr="not logged in")
        self.assertIsNone(cd._try_gh("owner/repo"))

    @patch("shared.corpus_artifacts.shutil.copy2")
    @patch("shared.corpus_artifacts.subprocess.run")
    @patch("shared.corpus_artifacts.shutil.which", return_value="/usr/bin/gh")
    def test_gh_success_with_cache(self, _mock_which, mock_run, _mock_copy):
        auth_result = MagicMock(returncode=0)
        list_result = MagicMock(
            returncode=0,
            stdout=json.dumps([{"databaseId": 12345, "headSha": "abc123"}]),
        )
        mock_run.side_effect = [auth_result, list_result]

        cache_dir = cd._cache_dir()
        cache_file = cache_dir / "corpus-results-12345.json"
        cache_file.write_text('{"summary": {}}')
        try:
            result = cd._try_gh("owner/repo")
            self.assertIsNotNone(result)
            path, run_id, sha = result
            self.assertEqual(run_id, 12345)
            self.assertEqual(sha, "abc123")
            self.assertTrue(path.exists())
        finally:
            cache_file.unlink(missing_ok=True)

    @patch("shared.corpus_artifacts.subprocess.run")
    @patch("shared.corpus_artifacts.shutil.which", return_value="/usr/bin/gh")
    def test_gh_no_runs(self, _mock_which, mock_run):
        auth_result = MagicMock(returncode=0)
        list_result = MagicMock(returncode=0, stdout="[]")
        mock_run.side_effect = [auth_result, list_result]
        self.assertIsNone(cd._try_gh("owner/repo"))


class TestTryCurlApi(unittest.TestCase):
    """Tests for _try_curl_api()."""

    def test_no_repo(self):
        self.assertIsNone(cd._try_curl_api(None))

    @patch("shared.corpus_artifacts._github_api_get")
    def test_api_error(self, mock_get):
        from urllib.error import URLError

        mock_get.side_effect = URLError("connection failed")
        self.assertIsNone(cd._try_curl_api("owner/repo"))

    @patch("shared.corpus_artifacts._github_api_get")
    def test_no_runs(self, mock_get):
        mock_get.return_value = {"workflow_runs": []}
        self.assertIsNone(cd._try_curl_api("owner/repo"))

    @patch.dict(os.environ, {"GH_TOKEN": "", "GITHUB_TOKEN": ""}, clear=False)
    @patch("shared.corpus_artifacts._github_api_get")
    def test_no_token_prompts(self, mock_get):
        mock_get.return_value = {
            "workflow_runs": [{"id": 99, "head_sha": "def456"}]
        }
        cache_file = cd._cache_dir() / "corpus-results-99.json"
        cache_file.unlink(missing_ok=True)
        result = cd._try_curl_api("owner/repo")
        self.assertIsNone(result)

    @patch("shared.corpus_artifacts._github_api_download")
    @patch("shared.corpus_artifacts._github_api_get")
    @patch.dict(os.environ, {"GH_TOKEN": "fake-token"}, clear=False)
    def test_full_download_success(self, mock_get, mock_download):
        run_id = 77777
        cache_file = cd._cache_dir() / f"corpus-results-{run_id}.json"
        cache_file.unlink(missing_ok=True)

        mock_get.side_effect = [
            {"workflow_runs": [{"id": run_id, "head_sha": "sha1"}]},
            {"artifacts": [{"name": "corpus-report", "id": 555}]},
        ]

        corpus_data = json.dumps({"summary": {}, "by_cop": []}).encode()
        zip_buf = io.BytesIO()
        with zipfile.ZipFile(zip_buf, "w") as zf:
            zf.writestr("corpus-results.json", corpus_data)
        mock_download.return_value = zip_buf.getvalue()

        try:
            result = cd._try_curl_api("owner/repo")
            self.assertIsNotNone(result)
            path, rid, sha = result
            self.assertEqual(rid, run_id)
            self.assertEqual(sha, "sha1")
            self.assertTrue(path.exists())
            loaded = json.loads(path.read_text())
            self.assertIn("summary", loaded)
        finally:
            cache_file.unlink(missing_ok=True)

    @patch("shared.corpus_artifacts._github_api_download")
    @patch("shared.corpus_artifacts._github_api_get")
    @patch.dict(os.environ, {"GH_TOKEN": "fake-token"}, clear=False)
    def test_bad_zip(self, mock_get, mock_download):
        run_id = 88888
        cache_file = cd._cache_dir() / f"corpus-results-{run_id}.json"
        cache_file.unlink(missing_ok=True)

        mock_get.side_effect = [
            {"workflow_runs": [{"id": run_id, "head_sha": "sha2"}]},
            {"artifacts": [{"name": "corpus-report", "id": 666}]},
        ]
        mock_download.return_value = b"not a zip file"

        result = cd._try_curl_api("owner/repo")
        self.assertIsNone(result)

    @patch("shared.corpus_artifacts._github_api_get")
    @patch.dict(os.environ, {"GH_TOKEN": "fake-token"}, clear=False)
    def test_no_corpus_report_artifact(self, mock_get):
        run_id = 99999
        cache_file = cd._cache_dir() / f"corpus-results-{run_id}.json"
        cache_file.unlink(missing_ok=True)

        mock_get.side_effect = [
            {"workflow_runs": [{"id": run_id, "head_sha": "sha3"}]},
            {"artifacts": [{"name": "other-artifact", "id": 777}]},
        ]
        result = cd._try_curl_api("owner/repo")
        self.assertIsNone(result)


class TestTryCorpusMd(unittest.TestCase):
    """Tests for _try_corpus_md()."""

    SAMPLE_MD = """\
# Corpus Oracle Results

> Last updated: 2026-03-04

## Summary

| Metric | Value |
|--------|------:|
| Repos | 1000 |
| Offenses compared | 500,000 |
| Matches (both agree) | 490,000 |
| FP (nitrocop extra) | 5,000 |
| FN (nitrocop missing) | 5,000 |

## Diverging Cops

| Cop | Matches | FP | FN | Match % |
|-----|--------:|---:|---:|--------:|
| Style/Foo | 100 | 10 | 5 | 87.0% |
| RSpec/Bar | 200 | 0 | 20 | 90.9% |

<details>
<summary>Perfect cops</summary>

| Cop | Matches |
|-----|--------:|
| Lint/Baz | 500 |
| Style/Qux | 300 |

</details>
"""

    def _write_corpus_md(self, tmpdir: str, content: str) -> None:
        docs = Path(tmpdir) / "docs"
        docs.mkdir()
        (docs / "corpus.md").write_text(content)

    @patch("shared.corpus_artifacts._find_project_root")
    @patch("shared.corpus_artifacts.subprocess.run")
    def test_parses_diverging_cops(self, mock_run, mock_root):
        with tempfile.TemporaryDirectory() as tmpdir:
            self._write_corpus_md(tmpdir, self.SAMPLE_MD)
            mock_root.return_value = Path(tmpdir)
            mock_run.return_value = MagicMock(returncode=1, stdout="")

            result = cd._try_corpus_md()
            self.assertIsNotNone(result)
            path, run_id, sha = result
            data = json.loads(path.read_text())

            self.assertEqual(data["run_date"], "2026-03-04")
            self.assertEqual(data["summary"]["total_repos"], 1000)
            self.assertEqual(data["summary"]["total_offenses_compared"], 500000)

            cops = {c["cop"]: c for c in data["by_cop"]}
            self.assertIn("Style/Foo", cops)
            self.assertEqual(cops["Style/Foo"]["fp"], 10)
            self.assertEqual(cops["Style/Foo"]["fn"], 5)

    @patch("shared.corpus_artifacts._find_project_root")
    @patch("shared.corpus_artifacts.subprocess.run")
    def test_parses_perfect_cops(self, mock_run, mock_root):
        with tempfile.TemporaryDirectory() as tmpdir:
            self._write_corpus_md(tmpdir, self.SAMPLE_MD)
            mock_root.return_value = Path(tmpdir)
            mock_run.return_value = MagicMock(returncode=1, stdout="")

            result = cd._try_corpus_md()
            data = json.loads(result[0].read_text())

            cops = {c["cop"]: c for c in data["by_cop"]}
            self.assertIn("Lint/Baz", cops)
            self.assertEqual(cops["Lint/Baz"]["fp"], 0)
            self.assertEqual(cops["Lint/Baz"]["fn"], 0)
            self.assertEqual(cops["Lint/Baz"]["matches"], 500)
            self.assertEqual(cops["Lint/Baz"]["match_rate"], 1.0)

    @patch("shared.corpus_artifacts._find_project_root")
    @patch("shared.corpus_artifacts.subprocess.run")
    def test_total_cop_count(self, mock_run, mock_root):
        with tempfile.TemporaryDirectory() as tmpdir:
            self._write_corpus_md(tmpdir, self.SAMPLE_MD)
            mock_root.return_value = Path(tmpdir)
            mock_run.return_value = MagicMock(returncode=1, stdout="")

            result = cd._try_corpus_md()
            data = json.loads(result[0].read_text())
            self.assertEqual(len(data["by_cop"]), 4)

    @patch("shared.corpus_artifacts._find_project_root")
    def test_no_corpus_md_returns_none(self, mock_root):
        with tempfile.TemporaryDirectory() as tmpdir:
            mock_root.return_value = Path(tmpdir)
            self.assertIsNone(cd._try_corpus_md())

    @patch("shared.corpus_artifacts._find_project_root")
    @patch("shared.corpus_artifacts.subprocess.run")
    def test_source_marker(self, mock_run, mock_root):
        with tempfile.TemporaryDirectory() as tmpdir:
            self._write_corpus_md(tmpdir, self.SAMPLE_MD)
            mock_root.return_value = Path(tmpdir)
            mock_run.return_value = MagicMock(returncode=1, stdout="")

            result = cd._try_corpus_md()
            data = json.loads(result[0].read_text())
            self.assertIn("docs/corpus.md", data.get("_source", ""))

class TestDownloadCorpusResults(unittest.TestCase):
    """Tests for the main download_corpus_results() entry point."""

    @patch("shared.corpus_artifacts._try_corpus_md", return_value=None)
    @patch("shared.corpus_artifacts._try_curl_api", return_value=None)
    @patch("shared.corpus_artifacts._try_gh", return_value=None)
    @patch("shared.corpus_artifacts._detect_github_repo", return_value="owner/repo")
    def test_all_methods_fail_exits(self, _repo, _gh, _curl, _md):
        with self.assertRaises(SystemExit) as ctx:
            cd.download_corpus_results()
        self.assertEqual(ctx.exception.code, 1)

    @patch("shared.corpus_artifacts._clean_stale_local")
    @patch("shared.corpus_artifacts._try_gh")
    @patch("shared.corpus_artifacts._detect_github_repo", return_value="owner/repo")
    def test_gh_success(self, _repo, mock_gh, mock_clean):
        fake_path = Path("/tmp/fake.json")
        mock_gh.return_value = (fake_path, 123, "abc")
        path, run_id, sha = cd.download_corpus_results()
        self.assertEqual(path, fake_path)
        self.assertEqual(run_id, 123)
        self.assertEqual(sha, "abc")
        mock_clean.assert_called_once()

    @patch("shared.corpus_artifacts._clean_stale_local")
    @patch("shared.corpus_artifacts._try_curl_api")
    @patch("shared.corpus_artifacts._try_gh", return_value=None)
    @patch("shared.corpus_artifacts._detect_github_repo", return_value="owner/repo")
    def test_falls_back_to_curl(self, _repo, _gh, mock_curl, mock_clean):
        fake_path = Path("/tmp/fake2.json")
        mock_curl.return_value = (fake_path, 456, "def")
        path, run_id, sha = cd.download_corpus_results()
        self.assertEqual(path, fake_path)
        self.assertEqual(run_id, 456)
        mock_clean.assert_called_once()

    @patch("shared.corpus_artifacts._clean_stale_local")
    @patch("shared.corpus_artifacts._try_corpus_md")
    @patch("shared.corpus_artifacts._try_curl_api", return_value=None)
    @patch("shared.corpus_artifacts._try_gh", return_value=None)
    @patch("shared.corpus_artifacts._detect_github_repo", return_value="owner/repo")
    def test_falls_back_to_corpus_md(self, _repo, _gh, _curl, mock_md, mock_clean):
        fake_path = Path("/tmp/fake3.json")
        mock_md.return_value = (fake_path, 0, "")
        path, run_id, sha = cd.download_corpus_results()
        self.assertEqual(path, fake_path)
        self.assertEqual(run_id, 0)
        mock_clean.assert_called_once()

class TestGithubApiGet(unittest.TestCase):
    """Tests for _github_api_get()."""

    @patch("shared.corpus_artifacts.urlopen")
    def test_without_token(self, mock_urlopen):
        mock_resp = MagicMock()
        mock_resp.read.return_value = b'{"key": "value"}'
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp

        result = cd._github_api_get("https://api.github.com/test")
        self.assertEqual(result, {"key": "value"})

        req = mock_urlopen.call_args[0][0]
        self.assertNotIn("Authorization", req.headers)

    @patch("shared.corpus_artifacts.urlopen")
    def test_with_token(self, mock_urlopen):
        mock_resp = MagicMock()
        mock_resp.read.return_value = b'{"authed": true}'
        mock_resp.__enter__ = lambda s: s
        mock_resp.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_resp

        result = cd._github_api_get("https://api.github.com/test", token="mytoken")
        self.assertEqual(result, {"authed": True})

        req = mock_urlopen.call_args[0][0]
        self.assertEqual(req.headers["Authorization"], "Bearer mytoken")


class TestCallerScriptImports(unittest.TestCase):
    """Verify that all caller scripts can import from corpus_artifacts."""

    def test_check_cop_import(self):
        source = (SCRIPTS_DIR / "check_cop.py").read_text()
        self.assertIn("from shared.corpus_artifacts import", source)

    def test_triage_import(self):
        source = (
            ROOT / ".claude" / "skills" / "triage" / "scripts" / "triage.py"
        ).read_text()
        self.assertIn("from shared.corpus_artifacts import", source)

    def test_gem_progress_import(self):
        source = (
            ROOT / ".claude" / "skills" / "fix-department" / "scripts" / "gem_progress.py"
        ).read_text()
        self.assertIn("from shared.corpus_artifacts import", source)


if __name__ == "__main__":
    unittest.main()
