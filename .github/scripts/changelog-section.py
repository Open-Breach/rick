#!/usr/bin/env python3
"""Print one version's section of CHANGELOG.md, for the release body.

    changelog-section.py <version> [CHANGELOG.md]

`version` is the tag without its `v`; a pre-release suffix (`0.1.0-rc1`) is
dropped, so a release candidate gets the section of the version it leads to.
When the section is missing the Unreleased section is printed with a note,
so a release is never created with an empty body.
"""

import re
import sys


def section(text: str, heading: str) -> str | None:
    """The body under `## [heading]`, up to the next `## ` heading."""
    lines = text.splitlines()
    start = None
    for index, line in enumerate(lines):
        if re.match(rf"^## \[{re.escape(heading)}\]", line):
            start = index + 1
            break
    if start is None:
        return None
    body = []
    for line in lines[start:]:
        if line.startswith("## "):
            break
        body.append(line)
    return "\n".join(body).strip() + "\n"


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    version = argv[1].removeprefix("v").split("-", 1)[0]
    path = argv[2] if len(argv) > 2 else "CHANGELOG.md"
    with open(path, encoding="utf-8") as handle:
        text = handle.read()
    body = section(text, version)
    if body is None:
        body = section(text, "Unreleased") or ""
        body = f"_No changelog section for {version} yet; the Unreleased section follows._\n\n{body}"
    sys.stdout.write(body)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
