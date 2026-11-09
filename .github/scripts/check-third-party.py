#!/usr/bin/env python3
"""Checks third-party.json, the component list cargo-about generates for the
About dialog (specs/about-dialog.md), against Cargo.lock.

    check-third-party.py <third-party.json> [--features <spec>]

The file must exist and parse as a JSON array of objects with the keys
name, version, license, copyright, repository and text, every text
non-empty; every crate in it (name and version) must be in Cargo.lock; and
every third-party crate the release build links must be in it: each crate
that is a normal dependency of a workspace member on one of the release
targets of about.toml, as `cargo tree` resolves them with the release
build's features — build and dev dependencies and the crates of other
platforms left out, as about.toml tells cargo-about. A crate the release
links but the file lacks fails the check and is named, so a release never
ships a licence list with a crate missing; a crate the file has beyond
those (the workspace's own crates, which cargo-about lists since they do
not say `publish = false`, and a crate krates keeps for a target
expression it cannot evaluate) is reported and allowed (QUESTIONS.md 261).
The exit code is 1 on any failure.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

KEYS = ("name", "version", "license", "copyright", "repository", "text")


def run(args: list[str]) -> str:
    result = subprocess.run(args, check=False, capture_output=True, text=True)
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        raise SystemExit(f"{' '.join(args)} failed with {result.returncode}")
    return result.stdout


def workspace_members(root: Path) -> set[str]:
    """The names of the workspace's own crates."""
    meta = json.loads(
        run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", str(root / "Cargo.toml")])
    )
    return {package["name"] for package in meta["packages"]}


def expected(root: Path, targets: list[str], features: str | None, members: set[str]) -> set[tuple[str, str]]:
    """The (name, version) pairs of the third-party crates the release links."""
    crates: set[tuple[str, str]] = set()
    for target in targets:
        args = [
            "cargo", "tree", "--workspace", "--edges", "normal", "--target", target,
            "--prefix", "none", "--format", "{p}", "--manifest-path", str(root / "Cargo.toml"),
        ]
        if features:
            args += ["--features", features]
        for line in run(args).splitlines():
            match = re.match(r"^(\S+) v(\S+)", line.strip())
            if not match:
                continue
            name, version = match.group(1), match.group(2)
            if name in members:
                continue
            crates.add((name, version))
    return crates


def locked(root: Path) -> set[tuple[str, str]]:
    """Every (name, version) of Cargo.lock."""
    with open(root / "Cargo.lock", "rb") as handle:
        lock = tomllib.load(handle)
    return {(package["name"], package["version"]) for package in lock.get("package", [])}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("file", help="the generated third-party.json")
    parser.add_argument("--features", help="the features the release builds with, as given to cargo")
    parser.add_argument("--root", default=".", help="the repository root")
    args = parser.parse_args()
    root = Path(args.root).resolve()
    path = Path(args.file)
    if not path.is_file():
        print(f"{path}: missing", file=sys.stderr)
        return 1
    try:
        entries = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        print(f"{path}: not JSON: {error}", file=sys.stderr)
        return 1
    if not isinstance(entries, list) or not entries:
        print(f"{path}: not a non-empty array", file=sys.stderr)
        return 1
    failed = False
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict) or any(key not in entry for key in KEYS):
            print(f"{path}: entry {index} lacks a key of {KEYS}", file=sys.stderr)
            failed = True
            continue
        if not isinstance(entry["text"], str) or not entry["text"].strip():
            print(f"{path}: {entry['name']} {entry['version']} has no licence text", file=sys.stderr)
            failed = True
    if failed:
        return 1

    with open(root / "about.toml", "rb") as handle:
        targets = tomllib.load(handle).get("targets", [])
    if not targets:
        print("about.toml names no targets", file=sys.stderr)
        return 1
    members = workspace_members(root)
    want = expected(root, targets, args.features, members)
    have = {(entry["name"], entry["version"]) for entry in entries}
    lock = locked(root)

    for name, version in sorted(have - lock):
        print(f"{name} {version} is in {path} but not in Cargo.lock", file=sys.stderr)
        failed = True
    for name, version in sorted(want - have):
        print(f"{name} {version} is linked by the release but missing from {path}", file=sys.stderr)
        failed = True
    if failed:
        return 1
    extra = sorted(have - want)
    own = [name for name, _ in extra if name in members]
    if own:
        print(f"{len(own)} workspace crates listed (not third party; the dialog leaves them out): {', '.join(own)}")
    for name, version in extra:
        if name not in members:
            print(f"{name} {version} is listed although no release target links it (kept: over-attribution harms no one)")
    licenses = {entry["license"] for entry in entries}
    print(
        f"{path}: {len(have)} crates in {len(entries)} entries, {len(licenses)} licence types; "
        f"every crate of the {len(targets)} release targets is listed and every entry is in Cargo.lock"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
