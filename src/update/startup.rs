//! Startup receipts from the shared updater, plus compatibility with released helpers.
use super::*;
use anyhow::ensure;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Deserialize)]
struct LegacyReceipt {
    schema: u32,
    target: PathBuf,
    version: String,
    new_hash: String,
}
fn legacy_receipt(path: &Path) -> Result<(LegacyReceipt, PathBuf)> {
    let path = fs::canonicalize(path)?;
    ensure!(
        path.file_name().is_some_and(|n| n == "plan.json"),
        "Invalid update receipt"
    );
    let directory = path
        .parent()
        .context("Missing update directory")?
        .to_path_buf();
    ensure!(
        directory
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("sound-lock-update-")),
        "Invalid legacy update directory"
    );
    ensure!(
        fs::metadata(&path)?.len() <= 65536,
        "Update receipt too large"
    );
    let plan: LegacyReceipt = serde_json::from_slice(&fs::read(path)?)?;
    let actual = fs::canonicalize(std::env::current_exe()?)?;
    let target = fs::canonicalize(&plan.target)?;
    ensure!(
        plan.schema == 1
            && plan.version == VERSION
            && actual
                .to_string_lossy()
                .eq_ignore_ascii_case(&target.to_string_lossy()),
        "Update receipt does not match this application"
    );
    ensure!(
        directory
            .parent()
            .context("Missing staging parent")?
            .to_string_lossy()
            .eq_ignore_ascii_case(
                &target
                    .parent()
                    .context("Missing application folder")?
                    .to_string_lossy()
            ),
        "Update receipt is outside the application folder"
    );
    ensure!(
        rust_auto_updator::hash(&target).map_err(anyhow::Error::msg)? == plan.new_hash,
        "Update receipt checksum mismatch"
    );
    Ok((plan, directory))
}
#[derive(Default)]
pub struct Startup {
    legacy: Option<PathBuf>,
    cleanup: Option<PathBuf>,
    pub failure: Option<String>,
}
impl Startup {
    pub fn from_args() -> Result<Self> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let mut startup = Self::default();
        if args.first().is_some_and(|a| a == "--updated") {
            ensure!(args.len() == 2, "Missing update receipt");
            let path = PathBuf::from(&args[1]);
            legacy_receipt(&path)?;
            startup.legacy = Some(path);
        } else if let Some(index) = args.iter().position(|a| a == "--cleanup-update") {
            startup.cleanup = Some(
                args.get(index + 1)
                    .context("Missing update directory")?
                    .into(),
            );
        } else if args.first().is_some_and(|a| a == "--update-failed") {
            startup.failure = Some(
                args.get(1)
                    .map(|s| s.to_string_lossy().chars().take(1024).collect())
                    .unwrap_or_else(|| "Update failed; the previous version was restored.".into()),
            );
        }
        Ok(startup)
    }
    pub fn acknowledge(&mut self) -> Result<()> {
        if let Some(directory) = self.cleanup.take() {
            rust_auto_updator::cleanup_after_update(
                directory,
                config().map_err(anyhow::Error::msg)?.install,
            );
        }
        if let Some(path) = self.legacy.take() {
            let (plan, directory) = legacy_receipt(&path)?;
            fs::write(directory.join("healthy"), &plan.new_hash)?;
            // Released helpers own replacement and rollback. Acknowledge them
            // only after native presentation, then remove their known files.
            std::thread::spawn(move || {
                for _ in 0..300 {
                    if directory.join("completed").is_file() {
                        for _ in 0..50 {
                            if fs::remove_file(directory.join("updater.exe")).is_ok() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        for name in [
                            "old.exe",
                            "failed.exe",
                            "plan.json",
                            "update.log",
                            "healthy",
                            "completed",
                        ] {
                            let _ = fs::remove_file(directory.join(name));
                        }
                        let _ = fs::remove_dir(directory);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        }
        Ok(())
    }
}
pub fn helper_entry() -> Option<Result<()>> {
    if std::env::args_os()
        .nth(1)
        .is_none_or(|a| a != "--apply-update")
    {
        return None;
    }
    Some(
        config()
            .and_then(|config| rust_auto_updator::apply_update(&config))
            .map_err(anyhow::Error::msg),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_startup_receipt_is_bound_to_the_installed_binary() {
        let executable = std::env::current_exe().unwrap();
        let stage = tempfile::Builder::new()
            .prefix("sound-lock-update-")
            .tempdir_in(executable.parent().unwrap())
            .unwrap();
        let receipt = stage.path().join("plan.json");
        let mut plan = serde_json::json!({
            "schema": 1, "target": executable, "version": VERSION,
            "new_hash": rust_auto_updator::hash(&executable).unwrap(),
        });
        fs::write(&receipt, plan.to_string()).unwrap();
        assert!(legacy_receipt(&receipt).is_ok());
        plan["new_hash"] = "00".repeat(32).into();
        fs::write(&receipt, plan.to_string()).unwrap();
        assert!(legacy_receipt(&receipt).is_err());
    }
}
