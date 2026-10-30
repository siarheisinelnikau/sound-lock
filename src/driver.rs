//! Optional vendor driver setup. Download never launches software. Installation
//! opens the vendor's interactive installer only after a separate user click.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

pub const WEBSITE: &str = "https://vb-audio.com/Cable/";
const DOWNLOAD: &str = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip";
const PACKAGE: &str = "VBCABLE_Driver_Pack45.zip";
const INSTALLER: &str = "VBCABLE_Setup_x64.exe";
// Official Pack 45, checked against the vendor's signed setup on 2026-10-30.
const SHA256: &str = "b950e39f01af1d04ea623c8f6d8eb9b6ea5c477c637295fabf20631c85116bfb";
const MAX_DOWNLOAD: u64 = 16 * 1024 * 1024;
const MAX_EXTRACTED: u64 = 32 * 1024 * 1024;

enum Completed {
    Downloaded,
    InstallerClosed,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    Download,
    Install,
}

pub struct Setup {
    cache: PathBuf,
    receiver: Option<Receiver<Result<Completed, String>>>,
    work: Option<Work>,
    pub error: Option<String>,
    pub message: &'static str,
}

impl Default for Setup {
    fn default() -> Self {
        Self::new(cache_dir())
    }
}
impl Setup {
    fn new(cache: PathBuf) -> Self {
        Self {
            cache,
            receiver: None,
            work: None,
            error: None,
            message: "Download and installation are separate steps.",
        }
    }
    pub fn downloaded(&self) -> bool {
        self.cache.join(PACKAGE).is_file()
    }
    pub fn busy(&self) -> bool {
        self.work.is_some()
    }
    pub fn downloading(&self) -> bool {
        self.work == Some(Work::Download)
    }
    pub fn installing(&self) -> bool {
        self.work == Some(Work::Install)
    }
    pub fn download(&mut self, ctx: &eframe::egui::Context) {
        self.start(Work::Download, ctx);
    }
    pub fn install(&mut self, ctx: &eframe::egui::Context) {
        self.start(Work::Install, ctx);
    }
    fn start(&mut self, work: Work, ctx: &eframe::egui::Context) {
        if self.busy() {
            return;
        }
        let cache = self.cache.clone();
        let ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.work = Some(work);
        self.error = None;
        self.message = match work {
            Work::Download => "Downloading the official package…",
            Work::Install => "Opening the installer · approve Windows UAC to continue.",
        };
        std::thread::spawn(move || {
            let result = match work {
                Work::Download => download_to(&cache).map(|()| Completed::Downloaded),
                Work::Install => install_from(&cache).map(|()| Completed::InstallerClosed),
            };
            let _ = sender.send(result.map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
    }
    /// Returns true after the installer closes, so the UI can rescan endpoints.
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.receiver else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                Err("The driver setup task stopped unexpectedly. Please retry.".into())
            }
        };
        self.receiver = None;
        self.work = None;
        match result {
            Ok(Completed::Downloaded) => {
                self.message = "Download verified. Click Install driver when ready."
            }
            Ok(Completed::InstallerClosed) => {
                self.message = "Installer closed. Restart Windows, then refresh devices.";
                return true;
            }
            Err(error) => {
                self.message = "Setup could not finish. Retry or use the official website.";
                self.error = Some(error);
            }
        }
        false
    }
}

fn cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("SoundLock")
        .join("drivers")
        .join("vb-cable-45")
}

fn verify_package(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() as u64 <= MAX_DOWNLOAD,
        "Driver package is too large"
    );
    ensure!(
        format!("{:x}", Sha256::digest(bytes)) == SHA256,
        "Driver package checksum does not match Pack 45. Download from the official website instead."
    );
    Ok(())
}

fn download_to(cache: &Path) -> Result<()> {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    let agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(3)
        .timeout_global(Some(Duration::from_secs(60)))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent();
    let bytes = agent
        .get(DOWNLOAD)
        .call()
        .context("Could not download VB-CABLE")?
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()?;
    verify_package(&bytes)?;
    fs::create_dir_all(cache)?;
    let mut temporary = tempfile::NamedTempFile::new_in(cache)?;
    temporary.write_all(&bytes)?;
    temporary
        .persist(cache.join(PACKAGE))
        .map_err(|e| e.error)?;
    Ok(())
}

fn safe_path(name: &str) -> Result<&Path> {
    // Reject Windows separators, drive letters, alternate streams and device
    // names explicitly, even when ZIP tests run on a different platform.
    ensure!(
        !name.contains(['\\', ':', '\0']),
        "Unsafe path in driver archive"
    );
    let path = Path::new(name);
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        "Unsafe path in driver archive"
    );
    for part in name.trim_end_matches('/').split('/') {
        ensure!(
            !part.is_empty() && !part.ends_with(['.', ' ']),
            "Unsafe path in driver archive"
        );
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit()),
            "Unsafe device name in driver archive"
        );
    }
    Ok(path)
}

fn extract(bytes: &[u8], destination: &Path) -> Result<PathBuf> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(archive.len() <= 128, "Too many files in driver archive");
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let relative = safe_path(entry.name())?;
        ensure!(
            !entry.is_symlink(),
            "Links are not allowed in driver archive"
        );
        total = total
            .checked_add(entry.size())
            .context("Driver archive is too large")?;
        ensure!(total <= MAX_EXTRACTED, "Driver archive is too large");
        let path = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
            continue;
        }
        fs::create_dir_all(path.parent().context("Invalid package path")?)?;
        // Extraction always uses a fresh directory; never follow an existing file.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let declared = entry.size();
        let copied = std::io::copy(&mut (&mut entry).take(declared + 1), &mut file)?;
        ensure!(copied == declared, "Driver archive entry size mismatch");
    }
    let installer = destination.join(INSTALLER);
    ensure!(
        installer.is_file(),
        "The x64 installer is missing from the package"
    );
    Ok(installer)
}

fn prepare(cache: &Path) -> Result<(tempfile::TempDir, PathBuf)> {
    let mut bytes = Vec::new();
    fs::File::open(cache.join(PACKAGE))
        .context("Download the driver package first")?
        .take(MAX_DOWNLOAD + 1)
        .read_to_end(&mut bytes)?;
    verify_package(&bytes)?;
    let directory = tempfile::Builder::new()
        .prefix("setup-")
        .tempdir_in(cache)?;
    let installer = extract(&bytes, directory.path())?;
    platform::verify_signature(&installer)?;
    Ok((directory, installer))
}

fn install_from(cache: &Path) -> Result<()> {
    let (directory, installer) = prepare(cache)?;
    // Keep the extracted companions if Sound Lock exits while setup is running.
    // The vendor installer needs the INF/SYS/CAT files beside its executable.
    let directory = directory.keep();
    platform::run_installer(&installer, &directory)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Foundation::{CloseHandle, HWND, WAIT_OBJECT_0},
            Security::WinTrust::*,
            System::{
                Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
                Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
            },
            UI::{
                Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
                WindowsAndMessaging::SW_SHOWNORMAL,
            },
        },
        core::{PCWSTR, w},
    };
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    pub fn verify_signature(path: &Path) -> Result<()> {
        let path = wide(path);
        let mut file = WINTRUST_FILE_INFO {
            cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(path.as_ptr()),
            ..Default::default()
        };
        let mut data = WINTRUST_DATA {
            cbStruct: size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        // SAFETY: both structures and all path buffers outlive both API calls.
        let status = unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                (&mut data as *mut WINTRUST_DATA).cast(),
            )
        };
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                (&mut data as *mut WINTRUST_DATA).cast(),
            );
        }
        ensure!(
            status == 0,
            "Windows could not verify the installer signature (0x{:08X})",
            status as u32
        );
        Ok(())
    }
    pub fn run_installer(installer: &Path, directory: &Path) -> Result<()> {
        let executable = wide(installer);
        let directory = wide(directory);
        // SAFETY: a dedicated setup worker owns COM and all UTF-16 buffers.
        // 'runas' shows Windows UAC; there are no silent-install arguments.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            struct Com;
            impl Drop for Com {
                fn drop(&mut self) {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
            let _com = Com;
            let mut info = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS,
                lpVerb: w!("runas"),
                lpFile: PCWSTR(executable.as_ptr()),
                lpDirectory: PCWSTR(directory.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            ShellExecuteExW(&mut info)
                .context("Installer was not started; Windows elevation may have been cancelled")?;
            ensure!(
                !info.hProcess.is_invalid(),
                "Windows did not return an installer process"
            );
            let waited = WaitForSingleObject(info.hProcess, INFINITE);
            let mut exit_code = 0;
            let result = GetExitCodeProcess(info.hProcess, &mut exit_code);
            let _ = CloseHandle(info.hProcess);
            ensure!(
                waited == WAIT_OBJECT_0,
                "Could not wait for the driver installer"
            );
            result?;
            ensure!(
                matches!(exit_code, 0 | 1641 | 3010),
                "Driver installer exited with code {exit_code}"
            );
        }
        Ok(())
    }
}
#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn verify_signature(_: &Path) -> Result<()> {
        anyhow::bail!("Driver setup requires Windows")
    }
    pub fn run_installer(_: &Path, _: &Path) -> Result<()> {
        anyhow::bail!("Driver setup requires Windows")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    #[test]
    fn rejects_paths_that_can_escape_or_alias_windows_files() {
        for name in [
            "../outside",
            "/absolute",
            "C:/driver",
            "nested/../../outside",
            "a\\b",
            "file:stream",
            "NUL.txt",
            "COM1",
            "trailing. ",
        ] {
            assert!(safe_path(name).is_err(), "{name}");
        }
        assert!(safe_path("drivers/driver.sys").is_ok());
    }
    #[test]
    fn extracts_all_installer_companions() {
        let root = tempfile::tempdir().unwrap();
        let bytes = archive(&[
            (INSTALLER, b"setup"),
            ("driver.inf", b"inf"),
            ("nested/driver.sys", b"sys"),
        ]);
        let setup = extract(&bytes, root.path()).unwrap();
        assert_eq!(fs::read(setup).unwrap(), b"setup");
        assert_eq!(
            fs::read(root.path().join("nested/driver.sys")).unwrap(),
            b"sys"
        );
    }
    #[test]
    fn extraction_rejects_traversal_and_missing_installer() {
        let root = tempfile::tempdir().unwrap();
        assert!(extract(&archive(&[("../outside", b"bad")]), root.path()).is_err());
        assert!(extract(&archive(&[("readme.txt", b"nothing")]), root.path()).is_err());
    }
    #[test]
    fn modified_package_is_not_accepted() {
        assert!(verify_package(b"unexpected package").is_err());
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "Downloads the official package and checks its signature; never runs the installer"]
    fn official_package_prepares_without_installation() {
        fs::create_dir_all(".local").unwrap();
        let cache = tempfile::tempdir_in(".local").unwrap();
        download_to(cache.path()).unwrap();
        let (_folder, installer) = prepare(cache.path()).unwrap();
        assert_eq!(installer.file_name().unwrap(), INSTALLER);
    }
}
