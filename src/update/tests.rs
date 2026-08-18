use super::*;

fn metadata(version: &str) -> serde_json::Value {
    let name = format!("sound-lock-{version}-windows-x64.exe");
    serde_json::json!({ "tag_name":format!("v{version}"), "draft":false, "prerelease":false,
        "assets":[{"name":name,"browser_download_url":format!("{RELEASES}/download/v{version}/{name}"),"size":128,"digest":null},
        {"name":"SHA256SUMS.txt","browser_download_url":format!("{RELEASES}/download/v{version}/SHA256SUMS.txt"),"size":198,"digest":null}] })
}
fn parse(data: &serde_json::Value, current: &str) -> Result<Option<Release>> {
    parse_release(&serde_json::to_vec(data)?, current)
}
fn fake_pe() -> Vec<u8> {
    let mut bytes = vec![0_u8; 128];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
    bytes[64..70].copy_from_slice(b"PE\0\0\x64\x86");
    bytes
}

#[test]
fn versions_compare_numerically_and_never_offer_downgrades() {
    assert!(parse(&metadata("0.1.10"), "0.1.9").unwrap().is_some());
    assert!(parse(&metadata("0.1.2"), "0.1.3").unwrap().is_none());
    assert!(parse(&metadata("0.1.3"), "0.1.3").unwrap().is_none());
    assert!(parse(&metadata("1.0.0"), "0.99.99").unwrap().is_some());
}

#[test]
fn canonical_asset_names_are_preferred_and_old_releases_remain_readable() {
    let mut data = metadata("0.1.4");
    let legacy = data["assets"][0].clone();
    let canonical = "SoundLock-v0.1.4.exe";
    data["assets"][0]["name"] = canonical.into();
    data["assets"][0]["browser_download_url"] =
        format!("{RELEASES}/download/v0.1.4/{canonical}").into();
    assert_eq!(parse(&data, "0.1.3").unwrap().unwrap().exe.name, canonical);
    data["assets"].as_array_mut().unwrap().push(legacy);
    assert_eq!(parse(&data, "0.1.3").unwrap().unwrap().exe.name, canonical);
    data["assets"][0]["browser_download_url"] = "https://example.com/other.exe".into();
    assert!(
        parse(&data, "0.1.3").is_err(),
        "An invalid canonical asset must not silently fall back"
    );
    assert!(parse(&metadata("0.1.3"), "0.1.2").unwrap().is_some());
    let mut multi = metadata("0.1.4");
    let x64 = "SoundLock-v0.1.4-x64.exe";
    multi["assets"][0]["name"] = x64.into();
    multi["assets"][0]["browser_download_url"] = format!("{RELEASES}/download/v0.1.4/{x64}").into();
    let mut arm = multi["assets"][0].clone();
    arm["name"] = "SoundLock-v0.1.4-arm64.exe".into();
    arm["browser_download_url"] =
        format!("{RELEASES}/download/v0.1.4/SoundLock-v0.1.4-arm64.exe").into();
    multi["assets"].as_array_mut().unwrap().push(arm);
    assert_eq!(parse(&multi, "0.1.3").unwrap().unwrap().exe.name, x64);
}
#[test]
fn only_published_stable_releases_and_complete_assets_are_offered() {
    for key in ["draft", "prerelease"] {
        let mut data = metadata("0.1.4");
        data[key] = true.into();
        assert!(parse(&data, "0.1.3").is_err());
    }
    for version in ["0.1.4-rc.1", "0.1.4+other", "01.2.3", "../update"] {
        assert!(parse(&metadata(version), "0.1.3").is_err());
    }
    let mut data = metadata("0.1.4");
    data["assets"].as_array_mut().unwrap().pop();
    assert!(parse(&data, "0.1.3").is_err());
    let mut data = metadata("0.1.4");
    let duplicate = data["assets"][0].clone();
    data["assets"].as_array_mut().unwrap().push(duplicate);
    assert!(parse(&data, "0.1.3").is_err());
}
#[test]
fn rejects_external_assets_and_unbounded_downloads() {
    for url in [
        "http://github.com/siarheisinelnikau/sound-lock/other.exe",
        "https://github.com/other/repo/update.exe",
        "https://github.com.evil.test/update.exe",
    ] {
        let mut data = metadata("0.1.4");
        data["assets"][0]["browser_download_url"] = url.into();
        assert!(parse(&data, "0.1.3").is_err());
    }
    for size in [0, MAX_EXE + 1] {
        let mut data = metadata("0.1.4");
        data["assets"][0]["size"] = size.into();
        assert!(parse(&data, "0.1.3").is_err());
    }
    let mut data = metadata("0.1.4");
    data["assets"][0]["digest"] = "sha256:bad".into();
    assert!(parse(&data, "0.1.3").is_err());
}
#[test]
fn checksum_manifest_requires_one_exact_executable_name() {
    let hash = "ab".repeat(32);
    assert_eq!(
        checksum(format!("{hash}  app.exe\r\n").as_bytes(), "app.exe").unwrap(),
        hash
    );
    assert!(checksum(format!("{hash}  ../app.exe\n").as_bytes(), "app.exe").is_err());
    assert!(
        checksum(
            format!("{hash}  app.exe\n{hash}  app.exe").as_bytes(),
            "app.exe"
        )
        .is_err()
    );
    assert!(checksum(b"broken  app.exe", "app.exe").is_err());
}
#[test]
fn corrupted_downloads_and_non_x64_executables_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.exe");
    let bytes = fake_pe();
    fs::write(&path, &bytes).unwrap();
    let hash = hash_file(&path).unwrap();
    verify_executable(&path, &hash).unwrap();
    let mut bad = bytes;
    bad[70] = 1;
    fs::write(&path, &bad).unwrap();
    assert!(verify_executable(&path, &hash).is_err());
    bad[68] = 0x4c;
    bad[69] = 0x01;
    fs::write(&path, &bad).unwrap();
    assert!(verify_executable(&path, &hash_file(&path).unwrap()).is_err());
    fs::write(&path, b"not executable").unwrap();
    assert!(verify_executable(&path, &hash_file(&path).unwrap()).is_err());
}
#[test]
fn github_asset_size_and_digest_must_match() {
    let bytes = b"manifest";
    let mut asset: Asset = serde_json::from_value(metadata("0.1.4")["assets"][0].clone()).unwrap();
    asset.size = bytes.len() as u64;
    asset.digest = Some(format!("sha256:{:x}", Sha256::digest(bytes)));
    verify_digest(bytes, &asset).unwrap();
    assert!(verify_digest(b"different", &asset).is_err());
    asset.size += 1;
    assert!(verify_digest(bytes, &asset).is_err());
}
#[test]
#[ignore = "Downloads the current public release, verifies it, and deletes the temporary download; never executes or installs it"]
fn official_github_release_download_verifies_without_installing() {
    let release = check("0.0.0").unwrap().expect("No published release");
    let progress = AtomicU64::new(0);
    let downloaded = download(&release, &progress).unwrap();
    assert_eq!(progress.load(Ordering::Relaxed), release.exe.size);
    verify_executable(&downloaded.path, &downloaded.hash).unwrap();
}

#[cfg(test)]
pub(super) fn available_fixture() -> Release {
    parse(&metadata("99.0.0"), VERSION).unwrap().unwrap()
}
