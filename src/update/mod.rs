//! Public GitHub release discovery and verified, opt-in portable updates.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

mod install;
pub use install::{Startup, helper_entry};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES: &str = "https://github.com/siarheisinelnikau/sound-lock/releases";
const API: &str = "https://api.github.com/repos/siarheisinelnikau/sound-lock/releases/latest";
const MAX_EXE: u64 = 64 * 1024 * 1024;
const MAX_SUMS: u64 = 64 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    exe: Asset,
    sums: Asset,
}
impl Release {
    pub fn notes_url(&self) -> String {
        format!("{RELEASES}/tag/v{}", self.version)
    }
}
#[derive(Clone, Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}
#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

fn stable_version(value: &str) -> Result<semver::Version> {
    let version = semver::Version::parse(value)?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "Only stable releases are supported"
    );
    Ok(version)
}
fn parse_release(bytes: &[u8], current: &str) -> Result<Option<Release>> {
    let data: ApiRelease =
        serde_json::from_slice(bytes).context("GitHub returned invalid release metadata")?;
    ensure!(
        !data.draft && !data.prerelease,
        "GitHub did not return a stable published release"
    );
    let text = data
        .tag_name
        .strip_prefix('v')
        .context("Unsupported release tag")?;
    let version = stable_version(text)?;
    if version <= stable_version(current)? {
        return Ok(None);
    }
    let asset = |name: &str, maximum: u64| -> Result<Asset> {
        let matches: Vec<_> = data.assets.iter().filter(|a| a.name == name).collect();
        ensure!(
            matches.len() == 1,
            "The release must contain exactly one {name}"
        );
        let asset = matches[0];
        ensure!(
            asset.size > 0 && asset.size <= maximum,
            "Invalid {name} size"
        );
        ensure!(
            asset.browser_download_url == format!("{RELEASES}/download/v{version}/{name}"),
            "Update asset is outside the official release"
        );
        if let Some(digest) = &asset.digest {
            parse_hash(
                digest
                    .strip_prefix("sha256:")
                    .context("Unsupported GitHub asset digest")?,
            )?;
        }
        Ok(asset.clone())
    };
    // Prefer the canonical single-architecture name. Older published releases
    // remain readable; no legacy alias is generated for new releases.
    let names = [
        format!("SoundLock-v{version}.exe"),
        format!("SoundLock-v{version}-x64.exe"),
        format!("sound-lock-{version}-windows-x64.exe"),
    ];
    let exe_name = names
        .iter()
        .find(|name| data.assets.iter().any(|asset| asset.name == **name))
        .context("The release has no compatible Windows x64 executable")?;
    Ok(Some(Release {
        version: version.to_string(),
        exe: asset(exe_name, MAX_EXE)?,
        sums: asset("SHA256SUMS.txt", MAX_SUMS)?,
    }))
}
fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(5)
        .timeout_global(Some(Duration::from_secs(90)))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent()
}
fn get(agent: &ureq::Agent, url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    agent
        .get(url)
        .header(
            "User-Agent",
            concat!("Sound-Lock/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(403 | 429) => {
                anyhow::anyhow!("GitHub request limit reached. Try again later.")
            }
            other => anyhow::anyhow!(other)
                .context("Could not contact GitHub; check your connection and retry"),
        })
}
fn check(current: &str) -> Result<Option<Release>> {
    let bytes = get(&agent(), API)?
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_vec()?;
    parse_release(&bytes, current)
}
fn parse_hash(value: &str) -> Result<String> {
    ensure!(
        value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid SHA-256 checksum"
    );
    Ok(value.to_ascii_lowercase())
}
fn checksum(bytes: &[u8], name: &str) -> Result<String> {
    let text = std::str::from_utf8(bytes).context("Invalid checksum manifest")?;
    let mut found = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (hash, file) = line
            .split_once("  ")
            .context("Invalid checksum manifest line")?;
        let hash = parse_hash(hash)?;
        if file == name {
            ensure!(found.is_none(), "Duplicate executable checksum");
            found = Some(hash);
        }
    }
    found.context("The release checksum manifest does not contain the executable")
}
pub(super) fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        ensure!(total <= MAX_EXE, "Executable exceeds the update size limit");
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn verify_digest(bytes: &[u8], asset: &Asset) -> Result<()> {
    ensure!(
        bytes.len() as u64 == asset.size,
        "Download size does not match the release"
    );
    if let Some(digest) = &asset.digest {
        ensure!(
            format!("sha256:{:x}", Sha256::digest(bytes)) == digest.to_ascii_lowercase(),
            "GitHub asset checksum mismatch"
        );
    }
    Ok(())
}
pub(super) fn verify_executable(path: &Path, expected: &str) -> Result<()> {
    ensure!(
        hash_file(path)? == parse_hash(expected)?,
        "Update checksum mismatch; the current application was not changed"
    );
    let mut file = fs::File::open(path)?;
    let mut header = [0_u8; 64];
    file.read_exact(&mut header)?;
    ensure!(&header[..2] == b"MZ", "Update is not a Windows executable");
    let offset = u32::from_le_bytes(header[60..64].try_into().unwrap()) as u64;
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset))?;
    let mut signature = [0_u8; 6];
    file.read_exact(&mut signature)?;
    ensure!(
        signature == *b"PE\0\0\x64\x86",
        "Update is not a Windows x64 executable"
    );
    Ok(())
}
struct Download {
    _directory: tempfile::TempDir,
    path: PathBuf,
    hash: String,
}
fn download(release: &Release, progress: &AtomicU64) -> Result<Download> {
    let agent = agent();
    let sums = get(&agent, &release.sums.browser_download_url)?
        .body_mut()
        .with_config()
        .limit(MAX_SUMS)
        .read_to_vec()?;
    verify_digest(&sums, &release.sums)?;
    let expected = checksum(&sums, &release.exe.name)?;
    if let Some(digest) = &release.exe.digest {
        ensure!(
            *digest == format!("sha256:{expected}"),
            "The release contains inconsistent checksums"
        );
    }
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("SoundLock/updates");
    fs::create_dir_all(&cache)?;
    let directory = tempfile::Builder::new()
        .prefix("download-")
        .tempdir_in(cache)?;
    let path = directory.path().join("download.exe");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let mut response = get(&agent, &release.exe.browser_download_url)?;
    let mut reader = response.body_mut().with_config().limit(MAX_EXE).reader();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        ensure!(
            total <= release.exe.size,
            "Update download exceeds the declared size"
        );
        file.write_all(&buffer[..count])?;
        progress.store(total, Ordering::Relaxed);
    }
    ensure!(total == release.exe.size, "Update download was incomplete");
    file.sync_all()?;
    drop(file);
    verify_executable(&path, &expected)?;
    Ok(Download {
        _directory: directory,
        path,
        hash: expected,
    })
}

enum Completed {
    Checked(Option<Release>),
    Downloaded(Download),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    Check,
    Download,
}
pub struct Updater {
    pub release: Option<Release>,
    pub error: Option<String>,
    pub message: String,
    ready: Option<Download>,
    receiver: Option<Receiver<Result<Completed, String>>>,
    work: Option<Work>,
    progress: Arc<AtomicU64>,
}
impl Default for Updater {
    fn default() -> Self {
        Self {
            release: None,
            error: None,
            message: "Check GitHub for a newer version.".into(),
            ready: None,
            receiver: None,
            work: None,
            progress: Arc::new(AtomicU64::new(0)),
        }
    }
}
impl Updater {
    pub fn busy(&self) -> bool {
        self.work.is_some()
    }
    pub fn ready(&self) -> bool {
        self.ready.is_some()
    }
    pub fn progress(&self) -> Option<f32> {
        (self.work == Some(Work::Download)).then(|| {
            self.progress.load(Ordering::Relaxed) as f32
                / self.release.as_ref().map_or(1, |r| r.exe.size) as f32
        })
    }
    pub fn check(&mut self, ctx: &eframe::egui::Context) {
        if self.busy() {
            return;
        }
        self.error = None;
        self.message = "Checking GitHub…".into();
        self.work = Some(Work::Check);
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(
                check(VERSION)
                    .map(Completed::Checked)
                    .map_err(|e| format!("{e:#}")),
            );
            ctx.request_repaint();
        });
    }
    pub fn download(&mut self, ctx: &eframe::egui::Context) {
        if self.busy() || self.ready() {
            return;
        }
        let Some(release) = self.release.clone() else {
            return;
        };
        self.error = None;
        self.message = "Downloading and verifying the update…".into();
        self.work = Some(Work::Download);
        self.progress.store(0, Ordering::Relaxed);
        let progress = self.progress.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(
                download(&release, &progress)
                    .map(Completed::Downloaded)
                    .map_err(|e| format!("{e:#}")),
            );
            ctx.request_repaint();
        });
    }
    pub fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(_) => Err("Update task stopped unexpectedly. Please retry.".into()),
        };
        self.receiver = None;
        self.work = None;
        match result {
            Ok(Completed::Checked(release)) => {
                self.ready = None;
                self.message = if release.is_some() {
                    "A newer version is available."
                } else {
                    "You're up to date."
                }
                .into();
                self.release = release;
            }
            Ok(Completed::Downloaded(download)) => {
                self.ready = Some(download);
                self.message = "Verified update ready. Restart when convenient.".into();
            }
            Err(e) => {
                self.error = Some(e);
                self.message = "Update could not finish. Retry or open Releases.".into();
            }
        }
    }
    pub fn prepare_restart(&self) -> Result<install::Prepared> {
        let download = self.ready.as_ref().context("Download an update first")?;
        let release = self.release.as_ref().context("No newer release selected")?;
        ensure!(
            stable_version(&release.version)? > stable_version(VERSION)?,
            "Refusing to install an older version"
        );
        install::prepare(&download.path, &download.hash, &release.version)
    }
}

#[cfg(test)]
mod tests;

#[cfg(feature = "ui-capture")]
pub fn fixture_updater_for_preview() -> Updater {
    let version = "0.1.5".to_owned();
    let asset = |name: &str| Asset {
        name: name.into(),
        browser_download_url: format!("{RELEASES}/download/v{version}/{name}"),
        size: 1024,
        digest: None,
    };
    Updater {
        release: Some(Release {
            exe: asset("SoundLock-v0.1.5.exe"),
            sums: asset("SHA256SUMS.txt"),
            version,
        }),
        message: "A newer version is available.".into(),
        ..Updater::default()
    }
}

#[cfg(test)]
pub(crate) fn fixture_updater(ready: bool, downloading: bool) -> Updater {
    let mut updater = Updater {
        release: Some(tests::available_fixture()),
        error: Some(
            "An example update error with details that must stay inside the fixed dialog.".into(),
        ),
        ..Updater::default()
    };
    if ready {
        let directory = tempfile::tempdir().unwrap();
        updater.ready = Some(Download {
            path: directory.path().join("new.exe"),
            hash: "ab".repeat(32),
            _directory: directory,
        });
    }
    if downloading {
        updater.work = Some(Work::Download);
    }
    updater
}
