//! The running EXE is never overwritten. A copy of this version waits for its
//! parent to exit, atomically replaces the EXE, and waits for a startup receipt.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    io::BufRead,
    process::{Child, Command, Stdio},
    time::Instant,
};

#[derive(Serialize, Deserialize)]
struct Plan {
    schema: u32,
    target: PathBuf,
    parent_pid: u32,
    parent_created: u64,
    old_hash: String,
    new_hash: String,
    helper_hash: String,
    version: String,
}
fn same_path(a: &Path, b: &Path) -> Result<bool> {
    Ok(fs::canonicalize(a)?
        .to_string_lossy()
        .eq_ignore_ascii_case(&fs::canonicalize(b)?.to_string_lossy()))
}
fn load_plan(path: &Path) -> Result<(Plan, PathBuf)> {
    let path = fs::canonicalize(path)?;
    ensure!(
        path.file_name().is_some_and(|n| n == "plan.json"),
        "Invalid update plan name"
    );
    let directory = path
        .parent()
        .context("Missing update directory")?
        .to_owned();
    ensure!(
        directory
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("sound-lock-update-")),
        "Invalid update directory"
    );
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(65_537)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65_536, "Update plan is too large");
    let plan: Plan = serde_json::from_slice(&bytes)?;
    ensure!(
        plan.schema == 1 && plan.target.is_absolute(),
        "Unsupported update plan"
    );
    let parent = plan
        .target
        .parent()
        .context("Missing application directory")?;
    ensure!(
        same_path(
            directory.parent().context("Missing staging parent")?,
            parent
        )?,
        "Update staging directory is outside the application folder"
    );
    ensure!(
        plan.target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe")),
        "Update target is not an executable"
    );
    for hash in [&plan.old_hash, &plan.new_hash, &plan.helper_hash] {
        parse_hash(hash)?;
    }
    stable_version(&plan.version)?;
    Ok((plan, directory))
}

pub struct Prepared {
    directory: Option<tempfile::TempDir>,
    child: Option<Child>,
}
impl Prepared {
    /// Call only after saving settings and restoring audio. Dropping an
    /// uncommitted preparation cancels its helper without changing the EXE.
    pub fn handoff(mut self) {
        self.child.take();
        if let Some(directory) = self.directory.take() {
            let _ = directory.keep();
        }
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
pub fn prepare(download: &Path, hash: &str, version: &str) -> Result<Prepared> {
    let target = std::env::current_exe()?;
    prepare_inner(
        download,
        hash,
        version,
        &target,
        std::process::id(),
        &target,
    )
}
fn prepare_inner(
    download: &Path,
    hash: &str,
    version: &str,
    target: &Path,
    parent_pid: u32,
    helper: &Path,
) -> Result<Prepared> {
    verify_executable(download, hash)?;
    let target = fs::canonicalize(target)?;
    ensure!(
        !fs::metadata(&target)?.permissions().readonly(),
        "The installed executable is read-only. Move Sound Lock to a writable folder."
    );
    let parent = target.parent().context("Missing application directory")?;
    let directory = tempfile::Builder::new()
        .prefix("sound-lock-update-")
        .tempdir_in(parent)
        .context(
            "Cannot write beside Sound Lock. Move the portable app to a writable folder and retry.",
        )?;
    let staged = directory.path().join("new.exe");
    fs::copy(download, &staged)?;
    fs::OpenOptions::new()
        .write(true)
        .open(&staged)?
        .sync_all()?;
    verify_executable(&staged, hash)?;
    let copied_helper = directory.path().join("updater.exe");
    fs::copy(helper, &copied_helper)?;
    let plan = Plan {
        schema: 1,
        old_hash: hash_file(&target)?,
        new_hash: hash.to_owned(),
        helper_hash: hash_file(&copied_helper)?,
        target,
        parent_pid,
        parent_created: platform::identity(parent_pid)?.0,
        version: version.to_owned(),
    };
    let plan_path = directory.path().join("plan.json");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&plan_path)?;
    file.write_all(&serde_json::to_vec(&plan)?)?;
    file.sync_all()?;
    drop(file);
    let log = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.path().join("update.log"))?;
    let mut command = Command::new(&copied_helper);
    command
        .arg("--apply-update")
        .arg(&plan_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log));
    platform::hide(&mut command);
    let mut child = command
        .spawn()
        .context("Could not start the update helper")?;
    let output = child
        .stdout
        .take()
        .context("Missing update handshake pipe")?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = std::io::BufReader::new(output)
            .read_line(&mut line)
            .map(|_| line);
        let _ = sender.send(result);
    });
    let prepared = Prepared {
        directory: Some(directory),
        child: Some(child),
    };
    let ready = receiver
        .recv_timeout(Duration::from_secs(8))
        .context("Update helper did not respond; the application was not changed")??;
    if ready.trim() != "READY" {
        let mut detail = String::new();
        if let Ok(file) = fs::File::open(plan_path.with_file_name("update.log")) {
            let _ = file.take(4096).read_to_string(&mut detail);
        }
        anyhow::bail!(
            "Update helper could not prepare; the application was not changed. {}",
            detail.trim()
        );
    }
    Ok(prepared)
}

#[derive(Default)]
pub struct Startup {
    health: Option<PathBuf>,
    pub failure: Option<String>,
}
impl Startup {
    pub fn from_args() -> Result<Self> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args.first().is_some_and(|a| a == "--updated") {
            ensure!(args.len() == 2, "Missing update receipt");
            let path = PathBuf::from(&args[1]);
            let (plan, _) = load_plan(&path)?;
            ensure!(
                same_path(&std::env::current_exe()?, &plan.target)? && plan.version == VERSION,
                "Update receipt does not match this application"
            );
            verify_executable(&plan.target, &plan.new_hash)?;
            return Ok(Self {
                health: Some(path),
                failure: None,
            });
        }
        if args.first().is_some_and(|a| a == "--update-failed") {
            return Ok(Self {
                health: None,
                failure: Some(
                    args.get(1)
                        .map(|s| s.to_string_lossy().chars().take(1024).collect())
                        .unwrap_or_else(|| {
                            "Update failed; the previous version was restored.".into()
                        }),
                ),
            });
        }
        Ok(Self::default())
    }
    /// Called after the new application's first UI frame has initialized.
    pub fn acknowledge(&mut self) -> Result<()> {
        let Some(path) = self.health.take() else {
            return Ok(());
        };
        let (plan, directory) = load_plan(&path)?;
        fs::write(directory.join("healthy"), &plan.new_hash)?;
        std::thread::spawn(move || {
            // Never remove the backup until the helper confirms the receipt.
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
                    let _ = fs::remove_dir(&directory); // Only an empty, validated staging directory.
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        Ok(())
    }
}

pub fn helper_entry() -> Option<Result<()>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_none_or(|a| a != "--apply-update") {
        return None;
    }
    Some((|| {
        ensure!(args.len() == 2, "Missing update plan");
        apply_helper(Path::new(&args[1]))
    })())
}
fn restore_old(plan: &Plan, directory: &Path) -> Result<()> {
    if hash_file(&plan.target).is_ok_and(|hash| hash == plan.old_hash) {
        return Ok(());
    }
    let backup = directory.join("old.exe");
    ensure!(
        hash_file(&backup)? == plan.old_hash,
        "The previous executable could not be verified; recovery files were retained"
    );
    if plan.target.exists() {
        platform::replace(&plan.target, &backup, &directory.join("failed.exe"))?;
    } else {
        fs::rename(&backup, &plan.target)?;
    }
    ensure!(
        hash_file(&plan.target)? == plan.old_hash,
        "Restoration verification failed"
    );
    Ok(())
}
fn launch(target: &Path, flag: &str, value: &std::ffi::OsStr) -> Result<Child> {
    let mut command = Command::new(target);
    command
        .arg(flag)
        .arg(value)
        .current_dir(target.parent().context("Missing application directory")?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    platform::hide(&mut command);
    Ok(command.spawn()?)
}
fn apply_helper(path: &Path) -> Result<()> {
    let (plan, directory) = load_plan(path)?;
    ensure!(
        stable_version(&plan.version)? > stable_version(VERSION)?,
        "Refusing a downgrade or same-version update"
    );
    ensure!(
        same_path(&std::env::current_exe()?, &directory.join("updater.exe"))?,
        "Update helper must run from its own staging directory"
    );
    ensure!(
        hash_file(&directory.join("updater.exe"))? == plan.helper_hash,
        "Update helper changed"
    );
    ensure!(
        hash_file(&plan.target)? == plan.old_hash,
        "The installed application changed while preparing the update"
    );
    verify_executable(&directory.join("new.exe"), &plan.new_hash)?;
    let parent = platform::Parent::open(plan.parent_pid, plan.parent_created, &plan.target)?;
    std::io::stdout().write_all(b"READY\n")?;
    std::io::stdout().flush()?;
    parent.wait()?;
    drop(parent);
    let result: Result<Child> = (|| {
        ensure!(
            hash_file(&plan.target)? == plan.old_hash,
            "The application changed before replacement"
        );
        verify_executable(&directory.join("new.exe"), &plan.new_hash)?;
        platform::replace(
            &plan.target,
            &directory.join("new.exe"),
            &directory.join("old.exe"),
        )?;
        verify_executable(&plan.target, &plan.new_hash)?;
        launch(&plan.target, "--updated", path.as_os_str())
    })();
    let recover = |error: anyhow::Error| -> Result<()> {
        restore_old(&plan, &directory).with_context(|| format!("Update failed: {error:#}"))?;
        let message = format!("Update failed; the previous version was restored. {error:#}");
        let _ = launch(
            &plan.target,
            "--update-failed",
            std::ffi::OsStr::new(&message),
        )?;
        Ok(())
    };
    let mut child = match result {
        Ok(child) => child,
        Err(error) => return recover(error),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if fs::read_to_string(directory.join("healthy")).is_ok_and(|text| text == plan.new_hash) {
            fs::write(directory.join("completed"), b"ok")?;
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return recover(anyhow::anyhow!(
                "New version exited before startup completed ({status})"
            ));
        }
        // Keep both files when a live new process is merely slow or its state
        // cannot be queried. Never replace an EXE beneath a running process.
        if Instant::now() >= deadline {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::{ffi::OsStrExt, process::CommandExt};
    use windows::{
        Win32::{
            Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0},
            Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW},
            System::Threading::{
                GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW,
                WaitForSingleObject,
            },
        },
        core::{PCWSTR, PWSTR},
    };
    pub fn hide(command: &mut Command) {
        command.creation_flags(0x08000000);
    }
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    fn info(handle: HANDLE) -> Result<(u64, PathBuf)> {
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let mut path = vec![0_u16; 32_768];
        let mut size = path.len() as u32;
        unsafe {
            GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)?;
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut size,
            )?;
        }
        Ok((
            ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
            PathBuf::from(String::from_utf16(&path[..size as usize])?),
        ))
    }
    pub fn identity(pid: u32) -> Result<(u64, PathBuf)> {
        let handle = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)? });
        info(handle.0)
    }
    pub struct Parent(Handle);
    impl Parent {
        pub fn open(pid: u32, created: u64, target: &Path) -> Result<Self> {
            let handle = Handle(unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    pid,
                )?
            });
            let (actual, path) = info(handle.0)?;
            ensure!(
                actual == created && same_path(&path, target)?,
                "The application process changed; update cancelled"
            );
            Ok(Self(handle))
        }
        pub fn wait(&self) -> Result<()> {
            ensure!(
                unsafe { WaitForSingleObject(self.0.0, 120_000) } == WAIT_OBJECT_0,
                "Sound Lock did not exit; update cancelled"
            );
            Ok(())
        }
    }
    pub fn replace(target: &Path, staged: &Path, backup: &Path) -> Result<()> {
        let wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let (target, staged, backup) = (wide(target), wide(staged), wide(backup));
        unsafe {
            ReplaceFileW(
                PCWSTR(target.as_ptr()),
                PCWSTR(staged.as_ptr()),
                PCWSTR(backup.as_ptr()),
                REPLACE_FILE_FLAGS(0),
                None,
                None,
            )
        }
        .context("Windows could not replace the executable")?;
        Ok(())
    }
}
#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn hide(_: &mut Command) {}
    pub fn identity(_: u32) -> Result<(u64, PathBuf)> {
        anyhow::bail!("Self-update requires Windows")
    }
    pub struct Parent;
    impl Parent {
        pub fn open(_: u32, _: u64, _: &Path) -> Result<Self> {
            anyhow::bail!("Windows only")
        }
        pub fn wait(&self) -> Result<()> {
            anyhow::bail!("Windows only")
        }
    }
    pub fn replace(_: &Path, _: &Path, _: &Path) -> Result<()> {
        anyhow::bail!("Self-update requires Windows")
    }
}

#[cfg(test)]
mod tests;
