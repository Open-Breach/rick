#!/usr/bin/env python3
"""Checks the plugin packages of one platform against their manifest.

The release workflow's smoke test runs it in the directory that holds
`plugins-<platform>.json` and the packages it lists (issue 250,
specs/plugin-distribution.md §4): the manifest has the shape the plugin
registry's publisher reads (Open-Breach/trebuchet, docs/spec.md 7.4), it
lists nine packages, each is a file that is there with the size and the
SHA-256 the manifest gives, named `<id>-<version>-<platform>.rkp`, and each
holds the plugin's folder `<id>/` with a `plugin.toml` built for the Rick
the manifest names.

    check-packages.py <manifest> <platform>
"""

import hashlib
import json
import re
import sys
import zipfile
from pathlib import Path

# The separate plugins of a platform: nine each. ZFS is Linux only, and the
# media previewer is separate where the GStreamer runtime travels with it.
EVERYWHERE = {"git", "svn", "ssh", "vfs-s3", "vfs-azure", "vfs-gcs", "preview-office", "preview-3d"}
TOP = ["app", "app_version", "platform", "plugins"]
ENTRY = ["id", "name", "description", "version", "api_version", "meta", "file", "size", "sha256"]


def fail(message):
    print(message)
    sys.exit(1)


def main():
    if len(sys.argv) != 3:
        fail("usage: check-packages.py <manifest> <platform>")
    path, platform = Path(sys.argv[1]), sys.argv[2]
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if list(manifest) != TOP:
        fail(f"{path}: the keys are {list(manifest)}, not {TOP}")
    if manifest["app"] != "rick" or manifest["platform"] != platform:
        fail(f"{path}: for {manifest['app']} on {manifest['platform']}, not rick on {platform}")
    rick = manifest["app_version"]
    expected = EVERYWHERE | ({"zfs"} if platform.startswith("linux") else {"preview-media"})
    listed = [entry["id"] for entry in manifest["plugins"]]
    if sorted(listed) != sorted(expected) or listed != sorted(listed):
        fail(f"{path}: lists {listed}, not {sorted(expected)} in that order")
    for entry in manifest["plugins"]:
        ident = entry["id"]
        if list(entry) != ENTRY:
            fail(f"{ident}: the keys are {list(entry)}, not {ENTRY}")
        if list(entry["meta"]) != ["points", "triggers"] or not entry["meta"]["points"]:
            fail(f"{ident}: meta is {entry['meta']}")
        if not isinstance(entry["api_version"], int) or not entry["name"]:
            fail(f"{ident}: no name or no API version")
        name = entry["file"]
        if name != f"{ident}-{entry['version']}-{platform}.rkp":
            fail(f"{ident}: the file is named {name}")
        package = path.parent / name
        if not package.is_file():
            fail(f"{ident}: {name} is not here")
        data = package.read_bytes()
        if len(data) != entry["size"]:
            fail(f"{ident}: {name} has {len(data)} bytes, the manifest says {entry['size']}")
        digest = hashlib.sha256(data).hexdigest()
        if digest != entry["sha256"]:
            fail(f"{ident}: {name} hashes to {digest}, the manifest says {entry['sha256']}")
        with zipfile.ZipFile(package) as archive:
            names = archive.namelist()
            if names != sorted(names) or not all(n.startswith(f"{ident}/") for n in names):
                fail(f"{ident}: {name} is not the folder {ident}/ in the order of its names")
            toml = archive.read(f"{ident}/plugin.toml").decode("utf-8")
        built_for = re.search(r'^rick = "(.*)"$', toml, re.M)
        if not built_for or built_for.group(1) != rick:
            fail(f"{ident}: plugin.toml is built for {built_for and built_for.group(1)}, not {rick}")
        print(f"{name}: {len(data)} bytes, {len(names)} files, for Rick {rick}")
    print(f"{path.name}: {len(listed)} packages, each as its manifest says")


if __name__ == "__main__":
    main()
