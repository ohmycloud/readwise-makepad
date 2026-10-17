#!/usr/bin/env python3
"""Install cargo-makepad from Cargo.lock, adding a device-only build command."""
import pathlib
import os
import subprocess
import tomllib

root = pathlib.Path(__file__).resolve().parents[1]
lock = tomllib.loads((root / "Cargo.lock").read_text())
revisions = {p["source"].split("#")[1] for p in lock["package"]
             if p.get("source", "").startswith("git+https://github.com/makepad/makepad?")}
if len(revisions) != 1:
    raise SystemExit("Expected exactly one locked Makepad revision")
revision = revisions.pop()
checkout = root / "target" / "release-tools" / revision
if not checkout.exists():
    source_repo = os.environ.get("READWISE_MAKEPAD_SOURCE", "https://github.com/makepad/makepad")
    subprocess.run(["git", "clone", source_repo, str(checkout)], check=True)
subprocess.run(["git", "-C", str(checkout), "checkout", "--detach", revision], check=True)
source = checkout / "tools/cargo_makepad/src/apple/mod.rs"
text = source.read_text()
if '"build" | "build-device" =>' not in text:
    start = text.index('        "build" => {')
    end = text.index('        "run-device" => {', start)
    block = text[start:end]
    old = "let apple_target = AppleTarget::sim_target(apple_os);"
    if block.count(old) != 1:
        raise SystemExit("Makepad build implementation changed; review device-build patch")
    block = block.replace('"build" =>', '"build" | "build-device" =>').replace(old,
        'let apple_target = if args[0] == "build-device" { AppleTarget::device_target(apple_os) } '
        'else { AppleTarget::sim_target(apple_os) };')
    source.write_text(text[:start] + block + text[end:])
tool_root = root / "target" / "release-tools" / "installed"
subprocess.run(["cargo", "install", "--path", str(checkout / "tools/cargo_makepad"),
                "--root", str(tool_root), "--locked", "--force"], check=True)
print(f"Add to PATH: {tool_root / 'bin'}")
