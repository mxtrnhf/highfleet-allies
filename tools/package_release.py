#!/usr/bin/env python3
"""Package existing Windows builds with the tracked player presets; no publishing."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib
import zipfile

REPO = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"
GUIDES = (
    "ally-config.md", "hf-ai-config-reference.md", "hf-ai-tuning-guide.md",
    "qol-guide.md", "player-defaults.md",
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def component_versions():
    return {
        crate: tomllib.loads((REPO / crate / "Cargo.toml").read_text())["package"]["version"]
        for crate in ("ally", "ai", "qol")
    }


def populate(stage, version, versions, commit):
    shutil.copytree(REPO / "release", stage, dirs_exist_ok=True)
    shutil.copy2(REPO / "LICENSE", stage / "LICENSE.txt")

    readme = stage / "README.txt"
    text = readme.read_text()
    for key, value in {"PACK_VERSION": version, "ALLY_VERSION": versions["ally"], "AI_VERSION": versions["ai"]}.items():
        text = text.replace("@" + key + "@", value)
    readme.write_text(text)
    (stage / "docs").mkdir()
    for name in GUIDES:
        destination = stage / "docs" / name
        text = (REPO / "docs" / name).read_text()
        text = text.replace("../ally/config/ally.json", "../Modloader/config/ally.json")
        text = text.replace("../ai/config/hf_ai.json", "../Modloader/config/hf_ai.json")
        destination.write_text(text)
    mods = stage / "Modloader/mods"
    mods.mkdir()
    for crate, library, destination in (
        ("ally", "hf_ally.dll", f'hf_ally_v{versions["ally"]}.dll'),
        ("ai", "hf_ai.dll", f'hf_ai_v{versions["ai"]}.dll'),
        ("qol", "highfleet_qol.dll", "highfleet_qol.dll"),
    ):
        source = REPO / crate / "target" / TARGET / "release" / library
        if source.read_bytes()[:2] != b"MZ":
            raise RuntimeError(f"Not a Windows DLL: {source}")
        shutil.copy2(source, mods / destination)
    for crate, name in (("ally", "ally.json"), ("ai", "hf_ai.json")):
        preset = REPO / crate / "config" / name
        json.loads(preset.read_bytes())
        destination = stage / "Modloader/config" / name
        shutil.copy2(preset, destination)
        assert destination.read_bytes() == preset.read_bytes()
    manifest = {
        "pack_version": version, "source_commit": commit, "components": versions,
        "target": TARGET, "cargo_features": {"ally": [], "ai": [], "qol": []},
        "gameplay_validation": "Not run by the agent; player testing required.",
        "files": {
            str(path.relative_to(stage)): digest(path)
            for path in sorted(stage.rglob("*")) if path.is_file()
        },
    }
    (stage / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, default=REPO.parent / "release_packs")
    args = parser.parse_args()
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        parser.error("--version must be a three-part numeric version")
    dirty = subprocess.check_output(
        ["git", "status", "--porcelain", "--untracked-files=no"], cwd=REPO, text=True
    )
    if dirty:
        parser.error("Commit tracked source/preset changes before packaging")
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    name = "hf_release_pack_v" + args.version
    pack, archive = output / name, output / (name + ".zip")
    if pack.exists() or archive.exists():
        parser.error("Release output already exists; choose a new version or output directory")
    with tempfile.TemporaryDirectory(prefix="hf-pack-", dir=output) as temporary:
        stage = Path(temporary) / "pack"
        stage.mkdir()
        populate(stage, args.version, component_versions(), commit)
        pending = Path(temporary) / "pack.zip"
        with zipfile.ZipFile(pending, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as handle:
            for path in sorted(stage.rglob("*")):
                if path.is_file():
                    handle.write(path, str(path.relative_to(stage)))
        with zipfile.ZipFile(pending) as handle:
            assert handle.testzip() is None
        stage.replace(pack)
        pending.replace(archive)
    checksum = digest(archive)
    archive.with_suffix(".zip.sha256").write_text(f"{checksum}  {archive.name}\n")
    print(archive)
    print("SHA256:", checksum)


if __name__ == "__main__":
    main()
