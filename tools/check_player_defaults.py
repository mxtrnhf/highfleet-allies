#!/usr/bin/env python3
"""Run the real config/default and pure-math Rust tests on the host, never Wine."""
import argparse
from pathlib import Path
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[1]
AI_MODULES = (
    "tactics_math", "tactics_config", "twr_math", "flares_config",
    "plane_group_math", "panic_math", "curtain_math",
)
CONFIG_TESTS = {
    "ally": ("defaults_match_the_checked_in_player_preset",),
    "ai": (
        "defaults_match_the_checked_in_player_preset",
        "plane_group_settings_default_and_accept_overrides",
        "twr_cutoff_is_optional_and_configurable",
        "missing_flare_block_uses_player_defaults_and_explicit_off_is_respected",
        "missing_tactics_block_uses_player_defaults_and_explicit_off_is_respected",
    ),
}


def item(text, marker):
    start = text.index(marker)
    return text[start:text.index("\n}", start) + 2]


def config_module(crate, names):
    text = (REPO / crate / "src/lib.rs").read_text()
    position = text.index("struct Config {")
    start = text.rfind("#[derive", 0, position)
    struct = text[start:text.index("\n}", position) + 2]
    code = f"\nmod {crate}_defaults {{\nuse serde::{{Serialize, Deserialize}};\n"
    if crate == "ai":
        code += "use crate::{flares_config::AiFlaresConfig, tactics_config::TacticsConfig, plane_group_math};\n"
    code += struct + "\n" + item(text, "impl Default for Config {") + "\n"
    if crate == "ally":
        code += item(text, "fn deserialize_hex_color<'de, D>") + "\n"
    code += "#[cfg(test)]\nmod tests {\nuse super::*;\n"
    for name in names:
        start = text.index("    #[test]\n    fn " + name + "(")
        end = text.index("\n    }", start) + 6
        code += text[start:end].replace(
            "../config/", str(REPO / crate / "config") + "/"
        ) + "\n"
    return code + "}\n}\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    version = subprocess.check_output(["rustc", "-vV"], text=True)
    host = next(line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: "))
    code = "#![allow(dead_code, unused_imports)]\n"
    for name in AI_MODULES:
        code += f'#[path = "{REPO}/ai/src/{name}.rs"]\nmod {name};\n'
    for crate, names in CONFIG_TESTS.items():
        code += config_module(crate, names)
    with tempfile.TemporaryDirectory(prefix="hf-player-defaults-") as directory:
        root = Path(directory)
        (root / "src").mkdir()
        (root / "src/lib.rs").write_text(code)
        (root / "Cargo.toml").write_text(
            '[package]\nname = "hf-player-defaults-host"\nversion = "0.0.0"\nedition = "2021"\n'
            '[dependencies]\nserde = { version = "1", features = ["derive"] }\nserde_json = "1"\n'
        )
        command = ["cargo", "test", "--manifest-path", str(root / "Cargo.toml"), "--target", host, "--quiet"]
        if args.offline:
            command.append("--offline")
        subprocess.run(command, cwd=root, check=True)


if __name__ == "__main__":
    main()
