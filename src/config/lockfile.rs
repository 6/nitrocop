use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cache::cache_root_dir;

#[derive(Debug, Serialize, Deserialize)]
pub struct NitroCopLock {
    pub version: u32,
    pub generated_at: String,
    /// SHA-256 of Gemfile.lock content (for staleness detection)
    pub gemfile_lock_sha256: Option<String>,
    /// Gem name → absolute path to gem root directory
    pub gems: HashMap<String, PathBuf>,
}

/// Compute the lockfile path for a project directory.
///
/// Returns `<cache_root>/lockfiles/<hash>.json` where `<hash>` is the
/// SHA-256 of the canonical project directory path.
pub fn lockfile_path(project_dir: &Path) -> PathBuf {
    let canonical = project_dir
        .canonicalize()
        .unwrap_or_else(|_| project_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    cache_root_dir()
        .join("lockfiles")
        .join(format!("{hash}.json"))
}

/// Write the lockfile for the given project directory.
///
/// The lockfile is stored at `~/.cache/nitrocop/lockfiles/<hash>.json`,
/// keyed by the canonical project directory path.
pub fn write_lock(gems: &HashMap<String, PathBuf>, project_dir: &Path) -> Result<()> {
    let gemfile_lock_sha256 = hash_file(&project_dir.join("Gemfile.lock"));

    let lock = NitroCopLock {
        version: 1,
        generated_at: chrono_now(),
        gemfile_lock_sha256,
        gems: gems.clone(),
    };

    let json = serde_json::to_string_pretty(&lock)?;
    let cache_path = lockfile_path(project_dir);
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create lockfile directory {}", parent.display()))?;
    }
    std::fs::write(&cache_path, json)
        .with_context(|| format!("Failed to write {}", cache_path.display()))?;
    Ok(())
}

/// Read and parse the lockfile for the given project directory.
pub fn read_lock(dir: &Path) -> Result<NitroCopLock> {
    let cache_path = lockfile_path(dir);
    if !cache_path.exists() {
        anyhow::bail!(
            "No lockfile found for {}. Run 'nitrocop --init' first, or use --no-cache to skip.",
            dir.display()
        );
    }

    let content = std::fs::read_to_string(&cache_path)
        .with_context(|| format!("Failed to read {}", cache_path.display()))?;
    let lock: NitroCopLock = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", cache_path.display()))?;
    if lock.version != 1 {
        anyhow::bail!(
            "Lockfile has version {} (expected 1). Run 'nitrocop --init' to regenerate.",
            lock.version
        );
    }
    Ok(lock)
}

/// Check that the lockfile is still fresh.
/// Detects: Gemfile.lock changes, Ruby version switches, gem reinstalls.
pub fn check_freshness(lock: &NitroCopLock, dir: &Path) -> Result<()> {
    let current_hash = hash_file(&dir.join("Gemfile.lock"));
    if lock.gemfile_lock_sha256 != current_hash {
        anyhow::bail!("Stale lockfile (Gemfile.lock changed). Run 'nitrocop --init' to refresh.");
    }
    // Verify cached gem paths still exist (catches Ruby version switches,
    // gem reinstalls, rbenv rehash, etc.)
    for (name, path) in &lock.gems {
        if !path.exists() {
            anyhow::bail!(
                "Stale lockfile (gem path for '{name}' no longer exists: {}). Run 'nitrocop --init' to refresh.",
                path.display()
            );
        }
    }
    Ok(())
}

/// SHA-256 hash of a file's content, or None if the file doesn't exist.
fn hash_file(path: &Path) -> Option<String> {
    let content = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&content);
    Some(format!("{:x}", hasher.finalize()))
}

/// Simple ISO-8601 timestamp without pulling in chrono.
fn chrono_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Rough UTC timestamp (good enough for a lockfile)
    let days = secs / 86400;
    let remaining = secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    // Days since epoch to Y-M-D (simplified)
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Simplified Gregorian calendar conversion
    let mut year = 1970;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let month_days: [u64; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    /// Mutex to serialize tests that mutate NITROCOP_CACHE_DIR.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// Run a closure with NITROCOP_CACHE_DIR pointed at a temp directory.
    /// Serialized via ENV_MUTEX to prevent races with other tests.
    fn with_cache_dir(tmp: &Path, f: impl FnOnce()) {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev = std::env::var("NITROCOP_CACHE_DIR").ok();
        // SAFETY: holding ENV_MUTEX ensures no other test is concurrently
        // reading/writing this env var.
        unsafe { std::env::set_var("NITROCOP_CACHE_DIR", tmp) };
        f();
        match prev {
            Some(v) => unsafe { std::env::set_var("NITROCOP_CACHE_DIR", v) },
            None => unsafe { std::env::remove_var("NITROCOP_CACHE_DIR") },
        }
    }

    #[test]
    fn lockfile_path_lives_under_lockfiles_dir() {
        let tmp = tempfile::tempdir().unwrap();
        with_cache_dir(tmp.path(), || {
            let project = tempfile::tempdir().unwrap();
            let path = lockfile_path(project.path());
            // The path should contain a "lockfiles" component and end with .json
            let components: Vec<_> = path.components().collect();
            let has_lockfiles = components.iter().any(|c| c.as_os_str() == "lockfiles");
            assert!(
                has_lockfiles,
                "path should contain 'lockfiles' dir: {}",
                path.display()
            );
            assert!(
                path.to_string_lossy().ends_with(".json"),
                "path should end with .json: {}",
                path.display()
            );
        });
    }

    #[test]
    fn lockfile_path_is_stable_for_same_dir() {
        let tmp = tempfile::tempdir().unwrap();
        with_cache_dir(tmp.path(), || {
            let project = tempfile::tempdir().unwrap();
            let p1 = lockfile_path(project.path());
            let p2 = lockfile_path(project.path());
            assert_eq!(p1, p2);
        });
    }

    #[test]
    fn lockfile_path_differs_for_different_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        with_cache_dir(tmp.path(), || {
            let project_a = tempfile::tempdir().unwrap();
            let project_b = tempfile::tempdir().unwrap();
            let p1 = lockfile_path(project_a.path());
            let p2 = lockfile_path(project_b.path());
            assert_ne!(p1, p2);
        });
    }

    #[test]
    fn symlink_resolves_to_same_lockfile() {
        let tmp = tempfile::tempdir().unwrap();
        with_cache_dir(tmp.path(), || {
            let project = tempfile::tempdir().unwrap();
            let link = tmp.path().join("symlink_project");
            #[cfg(unix)]
            std::os::unix::fs::symlink(project.path(), &link).unwrap();
            #[cfg(not(unix))]
            return; // skip on non-unix
            let p1 = lockfile_path(project.path());
            let p2 = lockfile_path(&link);
            assert_eq!(p1, p2);
        });
    }

    #[test]
    fn write_read_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        // Create a Gemfile.lock so hash_file succeeds
        std::fs::write(project.path().join("Gemfile.lock"), b"GEM\n").unwrap();

        with_cache_dir(tmp.path(), || {
            let mut gems = HashMap::new();
            gems.insert("rubocop".to_string(), PathBuf::from("/usr/gems/rubocop"));

            write_lock(&gems, project.path()).unwrap();

            // Verify file was created at the expected location
            let expected = lockfile_path(project.path());
            assert!(
                expected.exists(),
                "lockfile not created at {}",
                expected.display()
            );

            // Verify no .nitrocop.cache was created in project dir
            assert!(!project.path().join(".nitrocop.cache").exists());

            // Read it back
            let lock = read_lock(project.path()).unwrap();
            assert_eq!(lock.version, 1);
            assert_eq!(lock.gems.len(), 1);
            assert_eq!(
                lock.gems.get("rubocop").unwrap(),
                &PathBuf::from("/usr/gems/rubocop")
            );
            assert!(lock.gemfile_lock_sha256.is_some());
        });
    }

    #[test]
    fn read_lock_missing_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();

        with_cache_dir(tmp.path(), || {
            let err = read_lock(project.path()).unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("No lockfile found"), "unexpected error: {msg}");
            assert!(msg.contains("nitrocop --init"), "unexpected error: {msg}");
        });
    }
}
