//! Sound Lock configuration and presentation adapter for the shared updater.
use anyhow::{Context, Result};
use rust_auto_updator::{Config, Event, Service};
mod startup;
pub use startup::{Startup, helper_entry};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES: &str = "https://github.com/siarheisinelnikau/sound-lock/releases";
pub fn config() -> Result<Config, String> {
    let mut config = Config::new("siarheisinelnikau", "sound-lock", VERSION, "SoundLock")?;
    config.asset_suffixes = vec!["-x64".into()];
    config.legacy_executable_names = vec!["sound-lock-{version}-windows-x64.exe".into()];
    config.install.stage_prefix = ".sound-lock-update-".into();
    config.install.helper_name = "SoundLock-updater.exe".into();
    config.install.restart_args = vec!["--settings".into()];
    config.install.rollback_args = vec![
        "--update-failed".into(),
        "Update failed; the previous version was restored.".into(),
    ];
    config.install.startup_health_timeout = Some(std::time::Duration::from_secs(30));
    config.expected_subsystem = Some(2);
    Ok(config)
}
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    shared: rust_auto_updator::Release,
}
impl Release {
    pub fn notes_url(&self) -> String {
        self.shared.page.clone()
    }
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
    service: Option<Service>,
    ready: Option<rust_auto_updator::Prepared>,
    work: Option<Work>,
    received: u64,
    total: u64,
    #[cfg(test)]
    fixture_ready: bool,
}
impl Default for Updater {
    fn default() -> Self {
        Self {
            release: None,
            error: None,
            message: "Check GitHub for a newer version.".into(),
            service: None,
            ready: None,
            work: None,
            received: 0,
            total: 0,
            #[cfg(test)]
            fixture_ready: false,
        }
    }
}
impl Updater {
    pub fn busy(&self) -> bool {
        self.work.is_some()
    }
    pub fn ready(&self) -> bool {
        #[cfg(test)]
        if self.fixture_ready {
            return true;
        }
        self.ready.is_some()
    }
    pub fn progress(&self) -> Option<f32> {
        (self.work == Some(Work::Download)).then(|| self.received as f32 / self.total.max(1) as f32)
    }
    fn service(&mut self, ctx: &egui::Context) -> Result<&Service, String> {
        if self.service.is_none() {
            let wake = ctx.clone();
            self.service = Some(Service::new(config()?, move || {
                wake.request_repaint_of(egui::ViewportId::ROOT);
            })?);
        }
        Ok(self.service.as_ref().unwrap())
    }
    pub fn check(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        match self.service(ctx).and_then(Service::check) {
            Ok(()) => {
                self.error = None;
                self.message = "Checking GitHub…".into();
                self.work = Some(Work::Check);
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn download(&mut self, ctx: &egui::Context) {
        if self.busy() || self.ready() {
            return;
        }
        let Some(release) = self.release.clone() else {
            return;
        };
        match self
            .service(ctx)
            .and_then(|service| service.download(Box::new(release.shared)))
        {
            Ok(()) => {
                self.error = None;
                self.message = "Downloading and verifying the update…".into();
                self.work = Some(Work::Download);
                self.received = 0;
                self.total = self.release.as_ref().unwrap().shared.size();
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn poll(&mut self) -> bool {
        let Some(service) = &self.service else {
            return false;
        };
        let mut changed = false;
        for event in service.events.try_iter() {
            changed = true;
            match event {
                Event::Progress { received, total } => {
                    self.received = received;
                    self.total = total;
                }
                Event::Checked(result) => {
                    self.work = None;
                    match result {
                        Ok(release) => {
                            self.ready = None;
                            self.release = release.map(|shared| Release {
                                version: shared.version.to_string(),
                                shared: *shared,
                            });
                            self.message = if self.release.is_some() {
                                "A newer version is available."
                            } else {
                                "You're up to date."
                            }
                            .into();
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::Ready(result) => {
                    self.work = None;
                    match result {
                        Ok(prepared) => {
                            self.ready = Some(prepared);
                            self.message = "Verified update ready. Restart when convenient.".into();
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
            }
        }
        changed
    }
    pub fn prepare_restart(&mut self) -> Result<Prepared> {
        Ok(Prepared(
            self.ready.take().context("Download an update first")?,
        ))
    }
}
pub struct Prepared(rust_auto_updator::Prepared);
impl Prepared {
    pub fn handoff(mut self) -> Result<()> {
        rust_auto_updator::handoff(&mut self.0).map_err(anyhow::Error::msg)
    }
}
#[cfg(any(test, feature = "ui-capture"))]
fn fixture_release() -> Release {
    let config = config().unwrap();
    let version = "0.1.5";
    let metadata = serde_json::json!({
        "tag_name": format!("v{version}"), "draft": false, "prerelease": false,
        "html_url": format!("{RELEASES}/tag/v{version}"),
        "assets": [
            {"name": format!("SoundLock-v{version}.exe"), "size": 1024,
             "browser_download_url": format!("{RELEASES}/download/v{version}/SoundLock-v{version}.exe")},
            {"name": "SHA256SUMS.txt", "size": 128,
             "browser_download_url": format!("{RELEASES}/download/v{version}/SHA256SUMS.txt")}
        ]
    });
    let shared = rust_auto_updator::parse_release(&metadata.to_string(), &config)
        .unwrap()
        .unwrap();
    Release {
        version: shared.version.to_string(),
        shared,
    }
}
#[cfg(feature = "ui-capture")]
pub fn fixture_updater_for_preview() -> Updater {
    Updater {
        release: Some(fixture_release()),
        message: "A newer version is available.".into(),
        ..Default::default()
    }
}
#[cfg(test)]
pub(crate) fn fixture_updater(ready: bool, downloading: bool) -> Updater {
    Updater {
        release: Some(fixture_release()),
        error: Some(
            "An example update error with details that must stay inside the fixed dialog.".into(),
        ),
        fixture_ready: ready,
        work: downloading.then_some(Work::Download),
        ..Default::default()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_adapter_creates_no_worker_and_cannot_install() {
        let mut updater = Updater::default();
        assert!(!updater.poll());
        assert!(updater.service.is_none());
        assert!(updater.prepare_restart().is_err());
    }
    #[test]
    fn canonical_product_asset_uses_the_shared_parser() {
        assert_eq!(
            fixture_release().shared.executable_name(),
            "SoundLock-v0.1.5.exe"
        );
    }
}
