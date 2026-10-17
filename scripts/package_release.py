"""Package a checked Windows artifact. Prints only its Cargo version for CI."""

import hashlib
from pathlib import Path
import re
import shutil
import struct
import sys
import tomllib
import zipfile


def package(artifact: Path, output: Path) -> str:
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("Release packaging requires a plain semantic version")
    executable = artifact / "target/release/sound-lock.exe"
    data = executable.read_bytes()
    if len(data) < 64 or data[:2] != b"MZ":
        raise ValueError("The artifact is not a Windows executable")
    pe_offset = struct.unpack_from("<I", data, 60)[0]
    if data[pe_offset:pe_offset + 6] != b"PE\0\0\x64\x86":
        raise ValueError("The release must contain a Windows x64 executable")
    output.mkdir(parents=True, exist_ok=True)
    stem = f"sound-lock-{version}-windows-x64"
    standalone = output / f"SoundLock-v{version}.exe"
    shutil.copyfile(executable, standalone)
    archive = output / f"{stem}.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
        files = [(executable, "sound-lock.exe"), (artifact / "README.md", "README.md"), (artifact / "LICENSE", "LICENSE")]
        files.extend((source, source.relative_to(artifact).as_posix()) for source in sorted((artifact / "docs/screenshots").glob("*.png")))
        files.extend((source, source.relative_to(artifact).as_posix()) for source in sorted((artifact / "vendor/egui-software").glob("LICENSE-*")))
        files.extend((source, source.relative_to(artifact).as_posix()) for source in sorted((artifact / "licenses").glob("*.txt")))
        for source, name in files:
            # Stable archive metadata permits retrying an interrupted draft upload.
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            bundle.writestr(info, source.read_bytes())
    checksums = "".join(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in (standalone, archive))
    (output / "SHA256SUMS.txt").write_text(checksums, encoding="utf-8", newline="\n")
    return version


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("Usage: package_release.py ARTIFACT_DIRECTORY OUTPUT_DIRECTORY")
    print(package(Path(sys.argv[1]), Path(sys.argv[2])))
