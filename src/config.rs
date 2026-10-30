use serde::{Deserialize, Serialize};
use soundlock_dsp::{Profile, Settings};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Night,
    Headphones,
    Speakers,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Night, Self::Headphones, Self::Speakers];
    pub fn label(self) -> &'static str {
        match self {
            Self::Night => "Night",
            Self::Headphones => "Headphones",
            Self::Speakers => "Speakers",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Night => "Softer peaks, gentle recovery.",
            Self::Headphones => "Balanced dynamics, smooth recovery.",
            Self::Speakers => "Faster recovery, more room for dynamics.",
        }
    }
    pub fn profile(self) -> Profile {
        match self {
            Self::Night => Profile::Night,
            Self::Headphones => Profile::Headphones,
            Self::Speakers => Profile::Speakers,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mode: Mode,
    pub volume_percent: f32,
    pub enabled: bool,
    pub input_id: Option<String>,
    pub output_id: Option<String>,
    pub source: Source,
    pub peak_limit: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[default]
    Windows,
    Bridge,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Night,
            volume_percent: 100.0,
            enabled: true,
            input_id: None,
            output_id: None,
            source: Source::Windows,
            peak_limit: 50.0,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("SoundLock")
            .join("settings.json")
    }
    pub fn load() -> (Self, Option<String>) {
        let path = Self::path();
        if !path.exists() {
            return (Self::default(), None);
        }
        match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).map_err(|e| e.to_string()))
        {
            Ok(mut config) => {
                config.volume_percent = if config.volume_percent.is_finite() {
                    config.volume_percent.clamp(100.0, 400.0)
                } else {
                    100.0
                };
                config.peak_limit = if config.peak_limit.is_finite() {
                    config.peak_limit.clamp(10.0, 100.0)
                } else {
                    50.0
                };
                (config, None)
            }
            Err(error) => (
                Self::default(),
                Some(format!("Could not read settings: {error}")),
            ),
        }
    }
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().expect("settings has a parent"))?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }
    pub fn settings(&self) -> Settings {
        Settings {
            profile: self.mode.profile(),
            boost: self.volume_percent / 100.0,
            enabled: self.enabled,
        }
    }
}
