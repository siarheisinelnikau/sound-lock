use super::*;

fn plan(target: PathBuf) -> Plan {
    Plan {
        schema: 1,
        target,
        parent_pid: 0,
        parent_created: 0,
        old_hash: "ab".repeat(32),
        new_hash: "cd".repeat(32),
        helper_hash: "ef".repeat(32),
        version: "99.0.0".into(),
    }
}

#[test]
fn recovery_plan_rejects_paths_outside_its_application_folder() {
    let root = tempfile::tempdir().unwrap();
    let stage = tempfile::Builder::new()
        .prefix("sound-lock-update-")
        .tempdir_in(root.path())
        .unwrap();
    let path = stage.path().join("plan.json");
    let mut data = plan(root.path().join("app.exe"));
    fs::write(&data.target, b"old").unwrap();
    fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    load_plan(&path).unwrap();
    data.target = stage.path().join("app.exe");
    fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(load_plan(&path).is_err());
    data.target = root.path().join("app.exe");
    data.schema = 2;
    fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(load_plan(&path).is_err());
}
#[cfg(windows)]
#[test]
fn atomic_replacement_preserves_backup_and_rolls_back() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("app.exe");
    let staged = root.path().join("new.exe");
    let backup = root.path().join("old.exe");
    fs::write(&target, b"original").unwrap();
    fs::write(&staged, b"new version").unwrap();
    let mut data = plan(target.clone());
    data.old_hash = hash_file(&target).unwrap();
    platform::replace(&target, &staged, &backup).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"new version");
    assert_eq!(fs::read(&backup).unwrap(), b"original");
    restore_old(&data, root.path()).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"original");
    fs::write(&target, b"unrecognized").unwrap();
    assert!(restore_old(&data, root.path()).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"unrecognized");
}
#[cfg(windows)]
#[test]
fn parent_identity_rejects_reused_pid_or_wrong_executable() {
    let (created, path) = platform::identity(std::process::id()).unwrap();
    assert!(platform::Parent::open(std::process::id(), created + 1, &path).is_err());
    let other = tempfile::NamedTempFile::new().unwrap();
    assert!(platform::Parent::open(std::process::id(), created, other.path()).is_err());
}

#[test]
fn startup_receipt_cleanup_preserves_unrecognized_files_and_the_installed_exe() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("app.exe");
    fs::write(&target, b"installed app").unwrap();
    let directory = tempfile::Builder::new()
        .prefix("sound-lock-update-")
        .tempdir_in(root.path())
        .unwrap();
    let data = plan(target.clone());
    let path = directory.path().join("plan.json");
    fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    for name in ["updater.exe", "old.exe", "keep.txt"] {
        fs::write(directory.path().join(name), b"fixture").unwrap();
    }
    let mut startup = Startup {
        health: Some(path),
        failure: None,
    };
    startup.acknowledge().unwrap();
    assert_eq!(
        fs::read_to_string(directory.path().join("healthy")).unwrap(),
        data.new_hash
    );
    // Backup must remain before the helper confirms the startup receipt.
    assert!(directory.path().join("old.exe").exists());
    fs::write(directory.path().join("completed"), b"ok").unwrap();
    for _ in 0..30 {
        if !directory.path().join("old.exe").exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!directory.path().join("old.exe").exists());
    assert!(directory.path().join("keep.txt").exists());
    assert_eq!(fs::read(&target).unwrap(), b"installed app");
}

#[cfg(windows)]
fn native_case(broken: bool) {
    let helper = PathBuf::from(
        std::env::var_os("SOUND_LOCK_TEST_EXE")
            .expect("Set SOUND_LOCK_TEST_EXE to the newly built sound-lock.exe"),
    );
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let local = workspace.join(".local");
    fs::create_dir_all(&local).unwrap();
    let root = tempfile::Builder::new()
        .prefix("update-test Ω & 'quotes' ")
        .tempdir_in(&local)
        .unwrap();
    assert!(
        fs::canonicalize(root.path())
            .unwrap()
            .starts_with(fs::canonicalize(&local).unwrap())
    );
    let source = root.path().join("fixture.rs");
    let build = |flavor: &str, name: &str| {
        fs::write(&source,r#"
use std::{env,fs,path::Path,time::Duration};
fn main() {
    let args:Vec<_>=env::args().collect();
    if args.get(1).map(String::as_str)==Some("--hold") { loop { std::thread::sleep(Duration::from_millis(100)); } }
    if args.get(1).map(String::as_str)==Some("--updated") {
        if "@FLAVOR@"=="broken" { std::process::exit(7); }
        let path=Path::new(&args[2]); let text=fs::read_to_string(path).unwrap();
        let hash=text.split("\"new_hash\":\"").nth(1).unwrap().split('"').next().unwrap();
        fs::write(path.parent().unwrap().join("healthy"),hash).unwrap();
    }
    if args.get(1).map(String::as_str)==Some("--update-failed") { fs::write(env::current_exe().unwrap().with_extension("rollback-ok"),b"ok").unwrap(); }
}
"#.replace("@FLAVOR@",flavor)).unwrap();
        let path = root.path().join(name);
        let mut command = Command::new("rustc");
        command
            .arg("--crate-name")
            .arg("update_fixture")
            .arg(&source)
            .arg("-o")
            .arg(&path);
        platform::hide(&mut command);
        assert!(command.status().unwrap().success());
        path
    };
    let target = build("old", "app.exe");
    let new = build(if broken { "broken" } else { "healthy" }, "download.exe");
    let old_hash = hash_file(&target).unwrap();
    let new_hash = hash_file(&new).unwrap();
    assert_ne!(old_hash, new_hash);
    let mut command = Command::new(&target);
    command.arg("--hold");
    platform::hide(&mut command);
    let mut parent = command.spawn().unwrap();
    let version = stable_version(VERSION).unwrap();
    let next = format!("{}.{}.{}", version.major, version.minor, version.patch + 1);
    let mut prepared = match prepare_inner(&new, &new_hash, &next, &target, parent.id(), &helper) {
        Ok(prepared) => prepared,
        Err(e) => {
            let _ = parent.kill();
            let _ = parent.wait();
            panic!("{e:#}");
        }
    };
    let directory = prepared.directory.as_ref().unwrap().path().to_owned();
    // The active parent image is still intact after READY; no replacement is
    // permitted until that exact process exits.
    assert_eq!(hash_file(&target).unwrap(), old_hash);
    let mut updater = prepared.child.take().unwrap();
    parent.kill().unwrap();
    parent.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = updater.try_wait().unwrap() {
            let log = fs::read_to_string(directory.join("update.log")).unwrap_or_default();
            assert!(status.success(), "Helper failed: {log}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = updater.kill();
            let _ = updater.wait();
            panic!("Helper timed out");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if broken {
        assert_eq!(hash_file(&target).unwrap(), old_hash);
        for _ in 0..30 {
            if target.with_extension("rollback-ok").is_file() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            target.with_extension("rollback-ok").is_file(),
            "Previous version must be relaunched"
        );
    } else {
        assert_eq!(hash_file(&target).unwrap(), new_hash);
        assert_eq!(hash_file(&directory.join("old.exe")).unwrap(), old_hash);
        assert!(directory.join("completed").is_file());
    }
}
#[cfg(windows)]
#[test]
#[ignore = "Requires SOUND_LOCK_TEST_EXE; replaces only a temporary fixture app, never the running Sound Lock or audio settings"]
fn native_helper_waits_replaces_and_relaunches_fixture() {
    native_case(false);
}
#[cfg(windows)]
#[test]
#[ignore = "Requires SOUND_LOCK_TEST_EXE; proves rollback using only temporary fixture apps"]
fn native_helper_restores_and_relaunches_after_failed_startup() {
    native_case(true);
}
