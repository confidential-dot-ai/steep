//! Kernel `.config` resolution and snapshot guard.
//!
//! The "configure phase" runs `make x86_64_defconfig`, applies fragments,
//! then `mod2yesconfig`, then `olddefconfig`. After that, the resolved
//! `.config` is compared against the committed snapshot via [`check_snapshot`].

use std::path::Path;

use anyhow::{anyhow, Context, Result};

/// Read two files and assert byte-equality. Returns Ok(()) on match.
/// On mismatch, error includes both paths so the caller can suggest a diff.
pub fn check_snapshot(resolved: &Path, snapshot: &Path) -> Result<()> {
    let a = fs_err::read(resolved)
        .with_context(|| format!("reading resolved config {}", resolved.display()))?;
    let b = fs_err::read(snapshot)
        .with_context(|| format!("reading snapshot {}", snapshot.display()))?;
    if a == b {
        Ok(())
    } else {
        Err(anyhow!(
            "kernel .config drift: {} differs from {}.\n\
             Review the diff and re-run with `steep kernel --update-snapshot` if intended.",
            resolved.display(),
            snapshot.display()
        ))
    }
}

/// Replace `snapshot` with the contents of `resolved`. Used by --update-snapshot.
pub fn update_snapshot(resolved: &Path, snapshot: &Path) -> Result<()> {
    let bytes = fs_err::read(resolved)
        .with_context(|| format!("reading resolved config {}", resolved.display()))?;
    fs_err::write(snapshot, bytes)
        .with_context(|| format!("writing snapshot {}", snapshot.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.path().join(name);
        fs_err::write(&p, content).unwrap();
        p
    }

    #[test]
    fn check_snapshot_passes_on_match() {
        let d = TempDir::new().unwrap();
        let a = write(&d, "a", "CONFIG_X=y\n");
        let b = write(&d, "b", "CONFIG_X=y\n");
        assert!(check_snapshot(&a, &b).is_ok());
    }

    #[test]
    fn check_snapshot_fails_on_diff_with_helpful_message() {
        let d = TempDir::new().unwrap();
        let a = write(&d, "a", "CONFIG_X=y\n");
        let b = write(&d, "b", "CONFIG_X=n\n");
        let err = check_snapshot(&a, &b).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(".config drift"));
        assert!(msg.contains("--update-snapshot"));
    }

    #[test]
    fn update_snapshot_overwrites_target() {
        let d = TempDir::new().unwrap();
        let a = write(&d, "a", "CONFIG_X=y\n");
        let b = write(&d, "b", "CONFIG_X=n\n");
        update_snapshot(&a, &b).unwrap();
        assert_eq!(fs_err::read_to_string(&b).unwrap(), "CONFIG_X=y\n");
    }

    #[test]
    fn check_snapshot_errors_on_missing_file() {
        let d = TempDir::new().unwrap();
        let a = write(&d, "a", "x");
        let b = d.path().join("does-not-exist");
        assert!(check_snapshot(&a, &b).is_err());
    }
}
