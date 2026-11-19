#!/usr/bin/env python3
"""Writes the part of THIRD-PARTY-LICENSES.md that cargo-about cannot: the
components Rick ships outside Cargo, from about/bundled.toml.

    bundled-licenses.py <about/bundled.toml> >> THIRD-PARTY-LICENSES.md

cargo-about lists the crates of Cargo.lock (about.hbs). What ships beside
them — pdfium, 7-Zip and GStreamer, the Rust standard library, the fonts and
the icon font compiled into the binary — is listed by hand in
about/bundled.toml for the About dialog (specs/about-dialog.md). This script
prints the same list as Markdown, each component with its licence text, so
that the file a release carries names everything the release ships and the
notices are in a file as well as in the dialog. It ends with the note on the
project's own artwork (assets/README.md).

The release workflow appends the output to the file cargo-about generated.
A component without a name, a version, a licence id, a copyright holder, a
URL or a text fails the run and is named. The exit code is 1 on any failure.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

KEYS = ("name", "version", "license", "copyright", "url", "text")

INTRO = """\
## Bundled components

Shipped with Rick outside Cargo: the native libraries beside the binary and
its plugins, the Rust standard library the binary and the plugins share, and
the fonts and the icon font compiled into the binary. Not every build
carries every one of them: GStreamer comes with the audio and video plugin
on macOS and Windows and is the system's on Linux.

Two of them are built from further libraries whose notices ship as files, in
the `licenses` folder that comes with Rick:

- `licenses/pdfium/`: pdfium's own licence and one text for each library
  built into it, copied from the pdfium build the release ships.
- `licenses/rust/COPYRIGHT-library.html`: the notices of the Rust standard
  library and of the crates it is built from, as the toolchain that built
  the release carries them.
"""

ARTWORK = """\
## Artwork

The app icons and the About image are original artwork of the Rick project,
made by Serge with AI assistance. They are not third-party material.
"""


def fence(text: str) -> str:
    """A code fence longer than any run of backticks in `text`."""
    longest = run = 0
    for char in text:
        run = run + 1 if char == "`" else 0
        longest = max(longest, run)
    return "`" * max(3, longest + 1)


def cell(value: str) -> str:
    """`value` as the content of a table cell."""
    return value.replace("|", "\\|")


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write(__doc__)
        return 1
    path = Path(sys.argv[1])
    try:
        components = tomllib.loads(path.read_text(encoding="utf-8")).get("component", [])
    except (OSError, tomllib.TOMLDecodeError) as error:
        sys.stderr.write(f"{path}: {error}\n")
        return 1
    failures = []
    if not components:
        failures.append("no [[component]] in the file")
    for index, component in enumerate(components):
        name = component.get("name") or f"component {index + 1}"
        for key in KEYS:
            value = component.get(key)
            if not isinstance(value, str) or not value.strip():
                failures.append(f"{name}: no {key}")
    if failures:
        for failure in failures:
            sys.stderr.write(f"{path}: {failure}\n")
        return 1

    out = ["", INTRO, "| Component | Version | License |", "| --- | --- | --- |"]
    for component in components:
        out.append(
            f"| {cell(component['name'])} | {cell(component['version'])} | {cell(component['license'])} |"
        )
    out.append("")
    for component in components:
        text = component["text"].strip("\n")
        marks = fence(text)
        out += [
            f"### {component['name']} {component['version']} ({component['license']})",
            "",
            f"Copyright {component['copyright']} · <{component['url']}>",
            "",
            f"{marks}text",
            text,
            marks,
            "",
        ]
    out.append(ARTWORK)
    sys.stdout.write("\n".join(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
