#!/usr/bin/env python3
"""Fails on a user-facing string literal outside `t!` in the egui UI.

Every text the UI shows goes through the `t!` lookup (ext::i18n), so a
language is a translation file and not a code change (issue 24). This check
reads the Rust sources of `crates/ui-egui/src` (the code before the first
`#[cfg(test)]` of each file: tests may print what they like) and reports a
string literal with letters in it handed to a widget or a window title:

    ui.label("Folders first")          egui::Button::new("Apply")
    ui.checkbox(&mut v, "Hidden")      self.toast(level, "Done")
    .on_hover_text("Open the overview") egui::Window::new("Settings")

A literal inside `t!(...)` on the same line is the key, not a text, and a
line ending in `// i18n: ok` is exempt (an icon glyph, a format string of
numbers, a unit). The exit code is 1 when anything was found, so the check
is one step of the `fmt` job.

    .github/scripts/check-strings.py [--root DIR] [PATH ...]
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# The calls whose text the user reads. A method on `ui` or a response
# (`.label(`), a constructor (`egui::Button::new(`) or the shell's toast.
METHODS = (
    "label|heading|weak|small|strong|monospace|button|small_button|"
    "hyperlink_to|link|menu_button|collapsing|toggle_value|selectable_label|"
    "selectable_value|radio_value|checkbox|on_hover_text|on_disabled_hover_text|"
    "hint_text|text|title|toast|notify|show_tooltip_text"
)
CONSTRUCTORS = (
    "Button|Window|RichText|CollapsingHeader|Label|Hyperlink|SelectableLabel|"
    "Checkbox|Modal|TextEdit|ProgressBar|Slider"
)
CALL = re.compile(
    rf"(?:\.(?:{METHODS})\(|(?:egui::)?(?:{CONSTRUCTORS})::new\(|"
    rf"(?:egui::)?ComboBox::from_label\()"
)
# A literal with two letters in a row: text, not a glyph or a format of
# numbers.
LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')
LETTERS = re.compile(r"[A-Za-z]{2,}")
TEST_START = re.compile(r"^\s*#\[cfg\(test\)\]", re.M)
# `t!(` as a word: `format!(` must not count as one.
T_CALL = re.compile(r"(?<![A-Za-z0-9_])t!\(")
EXEMPT = "// i18n: ok"


def code_before_tests(text: str) -> str:
    match = TEST_START.search(text)
    return text[: match.start()] if match else text


def offending_lines(text: str):
    """The (line number, line) pairs with a user-facing literal."""
    for number, line in enumerate(code_before_tests(text).splitlines(), start=1):
        if EXEMPT in line or T_CALL.search(line):
            continue
        call = CALL.search(line)
        if not call:
            continue
        # The arguments after the call, up to the end of the line: the
        # first literal with letters is the text.
        rest = line[call.end():]
        for literal in LITERAL.finditer(rest):
            body = literal.group(1)
            if LETTERS.search(body) and not body.startswith("{"):
                yield number, line.strip()
                break


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--root", default=".", help="the repository root")
    parser.add_argument(
        "paths",
        nargs="*",
        default=["crates/ui-egui/src"],
        help="files or directories to check (default: the egui UI)",
    )
    args = parser.parse_args()
    root = Path(args.root)
    files: list[Path] = []
    for raw in args.paths:
        path = root / raw
        if path.is_dir():
            files.extend(sorted(path.rglob("*.rs")))
        elif path.is_file():
            files.append(path)
        else:
            print(f"check-strings: no such path {path}", file=sys.stderr)
            return 2
    found = 0
    for file in files:
        text = file.read_text(encoding="utf-8")
        for number, line in offending_lines(text):
            found += 1
            print(f"{file.relative_to(root)}:{number}: {line}")
    if found:
        print(
            f"check-strings: {found} user-facing string literal(s) outside t!; "
            "wrap each in t!(\"key\") with the text in locales/en.ftl, or mark a "
            f"glyph or number format with `{EXEMPT}`",
            file=sys.stderr,
        )
        return 1
    print(f"check-strings: {len(files)} files, no user-facing literal outside t!")
    return 0


if __name__ == "__main__":
    sys.exit(main())
