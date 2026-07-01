//! Transactional, per-browser render routing. The journal is written before any
//! Windows preference changes and retained until restoration succeeds.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::Duration,
};

#[cfg(windows)]
mod windows;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Browser {
    #[default]
    Chrome,
    Edge,
    Firefox,
    Brave,
    Opera,
    Vivaldi,
}
impl Browser {
    pub const ALL: [Self; 6] = [
        Self::Chrome,
        Self::Edge,
        Self::Firefox,
        Self::Brave,
        Self::Opera,
        Self::Vivaldi,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Firefox => "Firefox",
            Self::Brave => "Brave",
            Self::Opera => "Opera",
            Self::Vivaldi => "Vivaldi",
        }
    }
    pub fn exe(self) -> &'static str {
        match self {
            Self::Chrome => "chrome.exe",
            Self::Edge => "msedge.exe",
            Self::Firefox => "firefox.exe",
            Self::Brave => "brave.exe",
            Self::Opera => "opera.exe",
            Self::Vivaldi => "vivaldi.exe",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Process {
    pid: u32,
    created: u64,
    path: String,
    browser: Browser,
    has_audio: bool,
}

trait Backend {
    fn processes(&mut self) -> Result<Vec<Process>>;
    fn get(&mut self, process: &Process, role: usize) -> Result<Option<String>>;
    fn set(&mut self, process: &Process, role: usize, endpoint: Option<&str>) -> Result<()>;
    fn available(&mut self, endpoint: &str) -> Result<bool>;
    fn default_endpoint(&mut self, role: usize) -> Result<Option<String>>;
}

const PREFIX: &str = r"\\?\SWD#MMDEVAPI#";
const SUFFIX: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
fn endpoint_id(id: &str) -> String {
    let id = id
        .strip_prefix("wasapi:")
        .unwrap_or(id)
        .to_ascii_lowercase();
    let id = id.strip_prefix(&PREFIX.to_ascii_lowercase()).unwrap_or(&id);
    id.strip_suffix(SUFFIX).unwrap_or(id).to_owned()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Restore {
    browser: Browser,
    path: String,
    previous: [Option<String>; 3],
    target: String,
    fallback: String,
}
trait Store {
    fn load(&self) -> Result<Vec<Restore>>;
    fn save(&mut self, entries: &[Restore]) -> Result<()>;
}
#[derive(Serialize, Deserialize)]
struct Journal {
    version: u32,
    entries: Vec<Restore>,
}
struct FileStore {
    path: PathBuf,
    _lock: fs::File,
}
impl FileStore {
    fn open(path: PathBuf) -> Result<Self> {
        let parent = path.parent().context("Invalid routing journal path")?;
        fs::create_dir_all(parent)?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let lock = options.open(parent.join("browser-routing.lock")).context(
            "Another Sound Lock instance owns browser routing. Quit the earlier instance first.",
        )?;
        Ok(Self { path, _lock: lock })
    }
}
impl Store for FileStore {
    fn load(&self) -> Result<Vec<Restore>> {
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take(65_537).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 65_536,
            "Browser routing recovery file is too large"
        );
        let journal: Journal = serde_json::from_slice(&bytes)
            .context("Could not read browser routing recovery data")?;
        ensure!(
            journal.version == 1 && journal.entries.len() <= 64,
            "Unsupported browser routing recovery data"
        );
        Ok(journal.entries)
    }
    fn save(&mut self, entries: &[Restore]) -> Result<()> {
        if entries.is_empty() {
            match fs::remove_file(&self.path) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
        let data = serde_json::to_vec(&Journal {
            version: 1,
            entries: entries.to_vec(),
        })?;
        ensure!(
            data.len() <= 65_536,
            "Too many browser routes to save safely"
        );
        let mut temporary = tempfile::NamedTempFile::new_in(
            self.path.parent().context("Invalid routing directory")?,
        )?;
        temporary.write_all(&data)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|e| e.error)?;
        Ok(())
    }
}

struct Session {
    browser: Browser,
    target: String,
    fallback: String,
    seen: HashSet<(u32, u64)>,
}
struct Model<B, S> {
    backend: B,
    store: S,
    entries: Vec<Restore>,
    session: Option<Session>,
}
impl<B: Backend, S: Store> Model<B, S> {
    fn new(backend: B, store: S) -> Result<Self> {
        let entries = store.load()?;
        Ok(Self {
            backend,
            store,
            entries,
            session: None,
        })
    }
    fn start(&mut self, browser: Browser, target: &str, fallback: &str) -> Result<String> {
        self.stop()?;
        ensure!(
            self.entries.is_empty(),
            "Browser output recovery is pending. Open the browser, then try Start again."
        );
        let target = endpoint_id(target);
        let fallback = endpoint_id(fallback);
        ensure!(
            target != fallback
                && self.backend.available(&target)?
                && self.backend.available(&fallback)?,
            "Routing source or listening device is unavailable"
        );
        self.session = Some(Session {
            browser,
            target,
            fallback,
            seen: HashSet::new(),
        });
        self.tick()
    }
    fn route(&mut self, process: &Process, target: &str, fallback: &str) -> Result<()> {
        if !self
            .entries
            .iter()
            .any(|e| e.path.eq_ignore_ascii_case(&process.path))
        {
            let mut previous: [Option<String>; 3] = Default::default();
            for (role, value) in previous.iter_mut().enumerate() {
                *value = self.backend.get(process, role)?;
                // Migrate the previous manual-cable setup to an audible stop
                // destination instead of remembering the cable as the baseline.
                if value.as_deref() == Some(target)
                    || (value.is_none()
                        && self.backend.default_endpoint(role)?.as_deref() == Some(target))
                {
                    *value = Some(fallback.to_owned());
                }
            }
            let mut next = self.entries.clone();
            next.push(Restore {
                browser: process.browser,
                path: process.path.clone(),
                previous,
                target: target.into(),
                fallback: fallback.into(),
            });
            self.store
                .save(&next)
                .context("Could not save the browser's previous output; routing was not changed")?;
            self.entries = next;
        }
        for role in 0..3 {
            self.backend.set(process, role, Some(target))?;
            ensure!(
                self.backend.get(process, role)?.as_deref() == Some(target),
                "Windows did not confirm the browser output change"
            );
        }
        Ok(())
    }
    fn tick(&mut self) -> Result<String> {
        let Some(mut session) = self.session.take() else {
            self.restore()?;
            return Ok(if self.entries.is_empty() {
                String::new()
            } else {
                "Waiting to restore browser output".into()
            });
        };
        let processes = match self.backend.processes() {
            Ok(p) => p,
            Err(e) => {
                let _ = self.restore();
                return Err(e);
            }
        };
        let audio: Vec<_> = processes
            .into_iter()
            .filter(|p| p.browser == session.browser && p.has_audio)
            .collect();
        for process in &audio {
            if !session.seen.contains(&(process.pid, process.created)) {
                if let Err(error) = self.route(process, &session.target, &session.fallback) {
                    let rollback = self.restore();
                    return match rollback {
                        Ok(()) => Err(error.context(if self.entries.is_empty() {
                            "Browser routing failed; previous outputs were restored"
                        } else {
                            "Browser routing failed; reopen the browser to restore its output"
                        })),
                        Err(rollback) => Err(error.context(format!(
                            "Browser routing failed; restoration will retry: {rollback:#}"
                        ))),
                    };
                }
                session.seen.insert((process.pid, process.created));
            }
        }
        session
            .seen
            .retain(|key| audio.iter().any(|p| *key == (p.pid, p.created)));
        let status = if audio.is_empty() {
            format!("Waiting for {} audio", session.browser.label())
        } else {
            format!("{} routed to Source", session.browser.label())
        };
        self.session = Some(session);
        Ok(status)
    }
    fn restore(&mut self) -> Result<()> {
        if self.entries.is_empty() {
            return Ok(());
        }
        let processes = self.backend.processes()?;
        let mut remaining = Vec::new();
        let mut errors = Vec::new();
        for entry in &self.entries {
            let matching: Vec<_> = processes
                .iter()
                .filter(|p| {
                    p.has_audio
                        && p.browser == entry.browser
                        && p.path.eq_ignore_ascii_case(&entry.path)
                })
                .collect();
            if matching.is_empty() {
                remaining.push(entry.clone());
                continue;
            }
            let result: Result<()> = (|| {
                for role in 0..3 {
                    let mut previous = entry.previous[role].clone();
                    if let Some(id) = &previous
                        && !self.backend.available(id)?
                    {
                        previous = None;
                    }
                    if previous.is_none()
                        && self.backend.default_endpoint(role)?.as_deref() == Some(&entry.target)
                    {
                        ensure!(
                            self.backend.available(&entry.fallback)?,
                            "The previous listening device is unavailable. Select a physical Windows output."
                        );
                        previous = Some(entry.fallback.clone());
                    }
                    for process in &matching {
                        self.backend.set(process, role, previous.as_deref())?;
                        ensure!(
                            self.backend.get(process, role)? == previous,
                            "Windows did not confirm the restored browser output"
                        );
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                errors.push(format!("{e:#}"));
                remaining.push(entry.clone());
            }
        }
        self.store.save(&remaining)?;
        self.entries = remaining;
        ensure!(
            errors.is_empty(),
            "Could not restore browser output: {}",
            errors.join("; ")
        );
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        self.session = None;
        self.restore()
    }
}

enum Command {
    Start {
        browser: Browser,
        target: String,
        fallback: String,
        cancelled: Arc<AtomicBool>,
        reply: Sender<Result<()>>,
    },
    Stop(Sender<Result<()>>),
    Quit,
}
pub enum Event {
    Status(String),
    Error(String),
}
pub struct Service {
    commands: Option<Sender<Command>>,
    events: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
}
impl Service {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let (events, event_receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || run(receiver, events));
        Self {
            commands: Some(sender),
            events: event_receiver,
            worker: Some(worker),
        }
    }
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self {
            commands: None,
            events: mpsc::channel().1,
            worker: None,
        }
    }
    pub fn start(&self, browser: Browser, target: &str, fallback: &str) -> Result<()> {
        let (reply, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let commands = self
            .commands
            .as_ref()
            .context("Browser routing is unavailable")?;
        commands.send(Command::Start {
            browser,
            target: target.into(),
            fallback: fallback.into(),
            cancelled: cancelled.clone(),
            reply,
        })?;
        match receiver.recv_timeout(Duration::from_secs(8)) {
            Ok(result) => result,
            Err(error) => {
                cancelled.store(true, Ordering::Release);
                let _ = commands.send(Command::Stop(mpsc::channel().0));
                Err(error)
                    .context("Windows browser routing did not respond; restoration was queued")
            }
        }
    }
    pub fn stop(&self) -> Result<()> {
        let Some(commands) = &self.commands else {
            return Ok(());
        };
        let (reply, receiver) = mpsc::channel();
        commands.send(Command::Stop(reply))?;
        receiver
            .recv_timeout(Duration::from_secs(8))
            .context("Browser output restoration did not respond")?
    }
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::Quit);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn journal_path() -> PathBuf {
    crate::config::Config::path().with_file_name("browser-routing.json")
}
#[cfg(windows)]
fn run(receiver: Receiver<Command>, events: Sender<Event>) {
    // One service owns the per-user recovery journal. The Windows adapter is
    // created only when routing or recovery is actually needed.
    let mut model: Option<Model<windows::Platform, FileStore>> = None;
    let mut last = String::new();
    loop {
        let command = match receiver.recv_timeout(Duration::from_millis(500)) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(_) => Some(Command::Quit),
        };
        if matches!(&command, Some(Command::Quit)) {
            if let Some(m) = &mut model {
                let _ = m.stop();
            }
            break;
        }
        let needs_start = matches!(&command, Some(Command::Start { .. }));
        let needs_recovery = model.is_none() && journal_path().is_file();
        if model.is_none() && (needs_start || needs_recovery) {
            let result = (|| {
                let store = FileStore::open(journal_path())?;
                let backend = windows::Platform::new()?;
                Model::new(backend, store)
            })();
            match result {
                Ok(value) => model = Some(value),
                Err(error) => {
                    let message = format!("{error:#}");
                    if let Some(Command::Start { reply, .. }) = command {
                        let _ = reply.send(Err(anyhow::anyhow!(message.clone())));
                    }
                    if message != last {
                        let _ = events.send(Event::Error(message.clone()));
                        last = message;
                    }
                    continue;
                }
            }
        }
        match command {
            Some(Command::Start {
                browser,
                target,
                fallback,
                cancelled,
                reply,
            }) => {
                let result = model
                    .as_mut()
                    .context("Browser routing is unavailable")
                    .and_then(|m| {
                        ensure!(
                            !cancelled.load(Ordering::Acquire),
                            "Browser routing start was cancelled"
                        );
                        let result = m.start(browser, &target, &fallback);
                        if cancelled.load(Ordering::Acquire) {
                            m.stop()?;
                            anyhow::bail!("Browser routing start was cancelled");
                        }
                        result
                    });
                match result {
                    Ok(status) => {
                        if reply.send(Ok(())).is_err() {
                            if let Some(m) = &mut model {
                                let _ = m.stop();
                            }
                        } else {
                            let _ = events.send(Event::Status(status.clone()));
                            last = status;
                        }
                    }
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                }
            }
            Some(Command::Stop(reply)) => {
                let result = model.as_mut().map_or(Ok(()), |m| m.stop());
                let _ = reply.send(result);
                last.clear();
                let _ = events.send(Event::Status(String::new()));
            }
            Some(Command::Quit) => {
                if let Some(m) = &mut model {
                    let _ = m.stop();
                }
                break;
            }
            None => {
                if let Some(m) = &mut model {
                    let event = match m.tick() {
                        Ok(s) => Event::Status(s),
                        Err(e) => Event::Error(format!("{e:#}")),
                    };
                    let text = match &event {
                        Event::Status(s) | Event::Error(s) => s,
                    };
                    if *text != last {
                        last = text.clone();
                        let _ = events.send(event);
                    }
                }
            }
        }
    }
}
#[cfg(not(windows))]
fn run(receiver: Receiver<Command>, _: Sender<Event>) {
    for command in receiver {
        match command {
            Command::Start { reply, .. } => {
                let _ = reply.send(Err(anyhow::anyhow!(
                    "Automatic browser routing requires Windows"
                )));
            }
            Command::Stop(reply) => {
                let _ = reply.send(Ok(()));
            }
            Command::Quit => break,
        }
    }
}

pub fn probe() -> Result<()> {
    #[cfg(windows)]
    {
        std::thread::spawn(|| -> Result<()> {
            let mut backend = windows::Platform::new()?;
            let processes = backend.processes()?;
            println!(
                "Per-app audio policy available. Browser audio processes: {}",
                processes.iter().filter(|p| p.has_audio).count()
            );
            for process in processes.into_iter().filter(|p| p.has_audio) {
                let preferences: Result<Vec<_>> =
                    (0..3).map(|role| backend.get(&process, role)).collect();
                println!(
                    "{} audio PID {}: {:?}",
                    process.browser.label(),
                    process.pid,
                    preferences?
                );
            }
            Ok(())
        })
        .join()
        .map_err(|_| anyhow::anyhow!("Routing probe failed"))??;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("Windows only")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::HashMap, rc::Rc};

    #[derive(Default)]
    struct MemoryStore {
        entries: Vec<Restore>,
        saved: Rc<Cell<bool>>,
        fail: bool,
    }
    impl Store for MemoryStore {
        fn load(&self) -> Result<Vec<Restore>> {
            Ok(self.entries.clone())
        }
        fn save(&mut self, entries: &[Restore]) -> Result<()> {
            ensure!(!self.fail, "Disk full");
            self.entries = entries.to_vec();
            self.saved.set(true);
            Ok(())
        }
    }
    struct Fake {
        processes: Vec<Process>,
        values: HashMap<(String, usize), Option<String>>,
        available: HashSet<String>,
        defaults: [Option<String>; 3],
        fail_role_once: Option<usize>,
        saved: Rc<Cell<bool>>,
        writes: usize,
    }
    impl Backend for Fake {
        fn processes(&mut self) -> Result<Vec<Process>> {
            Ok(self.processes.clone())
        }
        fn get(&mut self, p: &Process, role: usize) -> Result<Option<String>> {
            ensure!(
                self.processes
                    .iter()
                    .any(|current| current.pid == p.pid && current.created == p.created),
                "Stale PID"
            );
            Ok(self
                .values
                .get(&(p.path.to_ascii_lowercase(), role))
                .cloned()
                .flatten())
        }
        fn set(&mut self, p: &Process, role: usize, endpoint: Option<&str>) -> Result<()> {
            assert!(self.saved.get(), "Journal must precede the first mutation");
            self.get(p, role)?;
            if self.fail_role_once == Some(role) {
                self.fail_role_once = None;
                anyhow::bail!("Injected Windows policy error");
            }
            self.writes += 1;
            self.values.insert(
                (p.path.to_ascii_lowercase(), role),
                endpoint.map(str::to_owned),
            );
            Ok(())
        }
        fn available(&mut self, endpoint: &str) -> Result<bool> {
            Ok(self.available.contains(endpoint))
        }
        fn default_endpoint(&mut self, role: usize) -> Result<Option<String>> {
            Ok(self.defaults[role].clone())
        }
    }
    fn process(pid: u32, created: u64, has_audio: bool) -> Process {
        Process {
            pid,
            created,
            path: r"C:\Browser\chrome.exe".into(),
            browser: Browser::Chrome,
            has_audio,
        }
    }
    fn fixture() -> Model<Fake, MemoryStore> {
        let store = MemoryStore::default();
        let fake = Fake {
            processes: vec![process(10, 1, true)],
            values: HashMap::new(),
            available: ["cable", "headphones", "speakers"]
                .map(str::to_owned)
                .into(),
            defaults: std::array::from_fn(|_| Some("headphones".into())),
            fail_role_once: None,
            saved: store.saved.clone(),
            writes: 0,
        };
        Model::new(fake, store).unwrap()
    }
    fn values(model: &mut Model<Fake, MemoryStore>) -> [Option<String>; 3] {
        let p = model.backend.processes[0].clone();
        std::array::from_fn(|role| model.backend.get(&p, role).unwrap())
    }
    fn start(model: &mut Model<Fake, MemoryStore>) {
        model.start(Browser::Chrome, "cable", "headphones").unwrap();
    }

    #[test]
    fn restores_each_role_and_follow_default_exactly() {
        let mut model = fixture();
        let p = model.backend.processes[0].clone();
        model
            .backend
            .values
            .insert((p.path.to_ascii_lowercase(), 1), Some("speakers".into()));
        let original = values(&mut model);
        start(&mut model);
        assert_eq!(
            values(&mut model),
            std::array::from_fn(|_| Some("cable".into()))
        );
        assert_eq!(model.store.entries.len(), 1);
        model.stop().unwrap();
        assert_eq!(values(&mut model), original);
        assert!(model.store.entries.is_empty());
        let writes = model.backend.writes;
        model.stop().unwrap();
        assert_eq!(model.backend.writes, writes, "Repeated Stop is harmless");
    }
    #[test]
    fn partial_policy_failure_rolls_back_all_roles() {
        let mut model = fixture();
        model.backend.fail_role_once = Some(1);
        assert!(model.start(Browser::Chrome, "cable", "headphones").is_err());
        assert_eq!(values(&mut model), [None, None, None]);
        assert!(model.entries.is_empty() && model.session.is_none());
    }
    #[test]
    fn journal_failure_prevents_any_route_change() {
        let mut model = fixture();
        model.store.fail = true;
        assert!(model.start(Browser::Chrome, "cable", "headphones").is_err());
        assert_eq!(model.backend.writes, 0);
        assert_eq!(values(&mut model), [None, None, None]);
    }
    #[test]
    fn waits_for_audio_and_tracks_new_children_without_losing_baseline() {
        let mut model = fixture();
        model.backend.processes[0].has_audio = false;
        assert!(
            model
                .start(Browser::Chrome, "cable", "headphones")
                .unwrap()
                .contains("Waiting")
        );
        assert_eq!(model.backend.writes, 0);
        model.backend.processes.push(process(20, 2, true));
        model.tick().unwrap();
        model.backend.processes.push(process(30, 3, true));
        model.tick().unwrap();
        assert_eq!(model.entries.len(), 1);
        assert_eq!(model.entries[0].previous, [None, None, None]);
        model.stop().unwrap();
        assert_eq!(values(&mut model), [None, None, None]);
    }
    #[test]
    fn closed_browser_retains_recovery_until_same_executable_returns() {
        let mut model = fixture();
        start(&mut model);
        model.backend.processes.clear();
        model.stop().unwrap();
        assert_eq!(model.store.entries.len(), 1);
        // Model reconstruction simulates a normal app restart or crash recovery.
        let mut model = Model::new(model.backend, model.store).unwrap();
        model.backend.processes.push(Process {
            path: r"D:\Other\chrome.exe".into(),
            ..process(10, 99, false)
        });
        model.tick().unwrap();
        assert_eq!(
            model.entries.len(),
            1,
            "A different installation or reused PID must not be restored"
        );
        model.backend.processes = vec![process(42, 100, false)];
        model.tick().unwrap();
        assert_eq!(
            model.entries.len(),
            1,
            "Windows policy needs a registered audio session"
        );
        model.backend.processes[0].has_audio = true;
        model.tick().unwrap();
        assert!(model.entries.is_empty());
        assert_eq!(values(&mut model), [None, None, None]);
    }
    #[test]
    fn previous_manual_cable_assignment_returns_to_listening_device() {
        let mut model = fixture();
        let p = model.backend.processes[0].clone();
        for role in 0..3 {
            model
                .backend
                .values
                .insert((p.path.to_ascii_lowercase(), role), Some("cable".into()));
        }
        start(&mut model);
        model.stop().unwrap();
        assert_eq!(
            values(&mut model),
            std::array::from_fn(|_| Some("headphones".into()))
        );
    }
    #[test]
    fn default_cable_and_disconnected_previous_device_use_safe_fallback() {
        let mut model = fixture();
        let p = model.backend.processes[0].clone();
        model
            .backend
            .values
            .insert((p.path.to_ascii_lowercase(), 0), Some("speakers".into()));
        start(&mut model);
        model.backend.available.remove("speakers");
        model.backend.defaults = std::array::from_fn(|_| Some("cable".into()));
        model.stop().unwrap();
        assert_eq!(
            values(&mut model),
            std::array::from_fn(|_| Some("headphones".into()))
        );
    }
    #[test]
    fn restore_error_retains_journal_and_retries() {
        let mut model = fixture();
        start(&mut model);
        model.backend.fail_role_once = Some(1);
        assert!(model.stop().is_err());
        assert_eq!(model.store.entries.len(), 1);
        model.tick().unwrap();
        assert!(model.store.entries.is_empty());
        assert_eq!(values(&mut model), [None, None, None]);
    }
    #[test]
    fn endpoint_ids_normalize_cpal_and_windows_policy_forms() {
        assert_eq!(
            endpoint_id("wasapi:{0.0.0.00000000}.{ABC}"),
            "{0.0.0.00000000}.{abc}"
        );
        assert_eq!(endpoint_id(&format!("{PREFIX}{{ABC}}{SUFFIX}")), "{abc}");
    }
    #[test]
    fn durable_journal_roundtrip_and_single_owner_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("browser-routing.json");
        let mut store = FileStore::open(path.clone()).unwrap();
        #[cfg(windows)]
        assert!(FileStore::open(path.clone()).is_err());
        assert!(store.load().unwrap().is_empty());
        let mut model = fixture();
        start(&mut model);
        store.save(&model.entries).unwrap();
        assert_eq!(store.load().unwrap()[0].previous, [None, None, None]);
        store.save(&[]).unwrap();
        assert!(!path.exists());
        fs::write(&path, b"{broken").unwrap();
        assert!(store.load().is_err());
        fs::write(&path, vec![0_u8; 65_537]).unwrap();
        assert!(store.load().is_err());
    }
}
