#!/usr/bin/env python3
"""Check that a Windows bundle carries the Visual C++ runtime it imports.

    check-vcruntime.py <dir>

`<dir>` holds rick.exe. Every .exe and .dll under it is read for the
libraries it imports, at load and delay-loaded, and each import that is a
Visual C++ runtime library (vcruntime140.dll, msvcp140.dll and their
kin) must be a file beside rick.exe: Windows looks there first for every
library the process loads, so the archive starts on a machine that has no
Visual C++ Redistributable installed. The Universal C Runtime
(api-ms-win-crt-*, ucrtbase.dll) is part of Windows 10 and later and is
not asked for.

Nothing is executed and no tool is needed: the import tables are read
from the files, so the check says the same on a runner that has Visual
Studio installed, where starting rick.exe proves nothing. Used by the
release workflow.
"""

import os
import re
import struct
import sys

# The redistributable libraries of the Visual C++ toolset: the C and C++
# runtimes, the concurrency runtime, OpenMP and C++/CX.
RUNTIME = re.compile(r"^(vcruntime|msvcp|concrt|vcomp|vccorlib)\d+.*\.dll$")

IMPORT_DIRECTORY = 1
DELAY_IMPORT_DIRECTORY = 13


class NotPortableExecutable(Exception):
    """The file is not a PE image."""


def imports(path: str) -> set[str]:
    """The lower-cased names of the libraries a PE image imports."""
    with open(path, "rb") as handle:
        data = handle.read()
    if len(data) < 0x40 or data[:2] != b"MZ":
        raise NotPortableExecutable(path)
    (pe,) = struct.unpack_from("<I", data, 0x3C)
    if data[pe : pe + 4] != b"PE\0\0":
        raise NotPortableExecutable(path)
    sections_count, = struct.unpack_from("<H", data, pe + 6)
    optional_size, = struct.unpack_from("<H", data, pe + 20)
    optional = pe + 24
    (magic,) = struct.unpack_from("<H", data, optional)
    # PE32 and PE32+ differ in where the data directories start.
    directories = optional + {0x10B: 96, 0x20B: 112}[magic]
    sections = []
    table = optional + optional_size
    for index in range(sections_count):
        virtual_size, virtual_address, raw_size, raw_offset = struct.unpack_from("<IIII", data, table + 40 * index + 8)
        sections.append((virtual_address, max(virtual_size, raw_size), raw_offset))

    def offset(rva: int) -> int | None:
        for address, size, raw in sections:
            if address <= rva < address + size:
                return raw + rva - address
        return None

    def name_at(rva: int) -> str | None:
        start = offset(rva)
        if start is None:
            return None
        end = data.index(b"\0", start)
        return data[start:end].decode("ascii", "replace").lower()

    def directory(index: int) -> int | None:
        if directories + 8 * index + 8 > optional + optional_size:
            return None
        (rva,) = struct.unpack_from("<I", data, directories + 8 * index)
        return offset(rva) if rva else None

    names: set[str] = set()
    # Import descriptors: 20 bytes each, the name's address fourth; the
    # delay-load descriptors: 32 bytes each, the name's address second.
    # Both lists end with a descriptor of zeros.
    for index, size, name_field in ((IMPORT_DIRECTORY, 20, 12), (DELAY_IMPORT_DIRECTORY, 32, 4)):
        at = directory(index)
        while at is not None and at + size <= len(data) and any(data[at : at + size]):
            (rva,) = struct.unpack_from("<I", data, at + name_field)
            name = name_at(rva)
            if name:
                names.add(name)
            at += size
    return names


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    root = argv[1]
    if not os.path.isfile(os.path.join(root, "rick.exe")):
        print(f"no rick.exe in {root}", file=sys.stderr)
        return 2
    beside = {name.lower() for name in os.listdir(root)}
    wanted: dict[str, list[str]] = {}
    modules = 0
    for folder, _, files in os.walk(root):
        for file in sorted(files):
            if not file.lower().endswith((".exe", ".dll")):
                continue
            path = os.path.join(folder, file)
            try:
                found = imports(path)
            except NotPortableExecutable:
                print(f"not a PE image, skipped: {os.path.relpath(path, root)}")
                continue
            modules += 1
            for name in found:
                if RUNTIME.match(name):
                    wanted.setdefault(name, []).append(os.path.relpath(path, root))
    if not modules:
        print(f"no modules under {root}", file=sys.stderr)
        return 2
    missing = sorted(name for name in wanted if name not in beside)
    for name in sorted(wanted):
        users = wanted[name]
        state = "MISSING" if name in missing else "beside rick.exe"
        print(f"{name}: {state}; imported by {len(users)} of {modules} modules, e.g. {users[0]}")
    if missing:
        print(f"the bundle imports {', '.join(missing)} and does not carry {'it' if len(missing) == 1 else 'them'}")
        return 1
    print(f"every Visual C++ runtime library the {modules} modules import is beside rick.exe")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
