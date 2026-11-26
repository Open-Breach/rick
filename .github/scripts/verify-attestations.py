#!/usr/bin/env python3
"""Checks the build attestations of a release's files, on the draft.

The release workflow's `verify` job runs it before anything is published
(specs/system-integration.md §9). It downloads one Linux package and the
macOS and Windows archives from the release of the tag and runs the two
documented lines on them, the way a user would:

    gh attestation verify <package> --owner <owner> --signer-repo <linux signer>
    gh attestation verify <archive> --repo <repository>

All three must verify. Then it flips one bit of the package and runs the
first line on that copy, which must be refused, and refused for the right
reason: `gh` has to say that there is no attestation for that checksum.

What a run of `gh` says is one of four things, told apart by what it
returns and prints and not by its exit status alone:

    verified         exit status 0.
    no attestation   `gh` reports HTTP 404 from GitHub's list of the
                     attestations of exactly this file's SHA-256, or says
                     in its own fixed words that it found none.
    not verified     `gh` found attestations and its verification of them
                     failed: its fixed words for that.
    no answer        everything else: no connection, a rate limit, a 5xx,
                     a token that is refused, words this script does not
                     know. Tried again a few times; if there is still no
                     answer the check could not be made.

So a failure that is not understood is never taken for a refusal, and
never for a verdict on a file either. What is matched, and why it holds:

  - The 404 is matched by its number and by the address it came from,
    which ends in the file's own SHA-256. `gh` prints an HTTP failure as
    `HTTP <status>: <message> (<address>)`, and has in every release that
    has `attestation verify` (2.49 to 2.105, read in its source). It does
    not try a 404 again and does not reword it.
  - That GitHub answers 404 for a checksum it has nothing for is what it
    does, seen in every dry run, and not what its documentation promises:
    that names only the answer 200. Were it to answer 200 with an empty
    list, `gh` says `no attestations found`, which is matched as well.
  - A 404 at that address could also mean an owner GitHub does not show
    to this token. Not here: the same line, with the same owner and the
    same token, has verified the unchanged package a moment before.
  - Should GitHub or `gh` say it another way one day, the answer is not
    known to this script, the check could not be made, and nothing is
    published. It never passes for a refusal.

The script ends in one of three ways, and says which in its last line and
in the job summary:

    exit 0   the three files verify and the changed one has no attestation
    exit 1   a verdict: a file does not verify, or a changed file verified
    exit 2   the check could not be made; it says nothing about the files

    verify-attestations.py <tag> --repo <owner>/<repository> --linux-signer <owner>/<repository>

`--gh <command> [<argument>...]`, last on the line, names another program
to run in place of `gh`: the tests of this script use it. The seconds to
wait before each further try are in RICK_VERIFY_DELAYS, separated by
commas; the default is three further tries, after 5, 15 and 30 seconds.
"""

import argparse
import hashlib
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

VERIFIED = "verified"
NO_ATTESTATION = "no attestation"
NOT_VERIFIED = "not verified"
NO_ANSWER = "no answer"

# `gh`'s own words, fixed strings of its source (cli/cli at 2.105,
# pkg/cmd/attestation), for the two verdicts it gives beside success.
# Compared in lower case.
#
# No attestation: the text of `ErrNoAttestationsFound`, what `gh` says
# where GitHub's list for the checksum comes back empty. The 404 GitHub
# answers today is matched in `classify`, by its status and its address.
NO_ATTESTATION_WORDS = ("no attestations found",)
# Not verified: attestations were found and the check of them failed. The
# signature or the signer does not fit (`verification/sigstore.go`), none
# of them passed (`verify.go`), or the certificate names another owner,
# repository or issuer than the line asked for (`verification/extensions.go`).
NOT_VERIFIED_WORDS = (
    'verifying with issuer "',
    "no attestations were verified",
    "expected sourcerepositoryowneruri to be",
    "expected sourcerepositoryuri to be",
    "expected issuer to be",
)


def classify(status, output, digest):
    """What one run of `gh attestation verify` said about the file with
    this SHA-256: its exit status and everything it printed."""
    if status == 0:
        return VERIFIED
    text = output.lower()
    lookup = f"/attestations/sha256:{digest}"
    for line in text.splitlines():
        if "http 404" in line and lookup in line:
            return NO_ATTESTATION
    if any(words in text for words in NO_ATTESTATION_WORDS):
        return NO_ATTESTATION
    if any(words in text for words in NOT_VERIFIED_WORDS):
        return NOT_VERIFIED
    return NO_ANSWER


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def last_words(output):
    """The end of what `gh` printed, on one line, for a message."""
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    return " | ".join(lines[-3:]) if lines else "nothing printed"


class Check:
    def __init__(self, gh, delays):
        self.gh = gh
        self.delays = delays
        self.summary = []

    def run(self, arguments, cwd):
        """One run of `gh`: its exit status, and stdout and stderr as one
        text. A `gh` that cannot be started is a failure like any other."""
        try:
            done = subprocess.run(
                [*self.gh, *arguments],
                cwd=cwd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                errors="replace",
                check=False,
            )
        except OSError as error:
            return 127, f"{self.gh[0]} could not be started: {error}"
        return done.returncode, done.stdout

    def answer(self, what, arguments, cwd, digest):
        """Runs one `gh attestation verify` until it gives an answer, a
        few times at most. Returns the answer and `gh`'s last output."""
        tries = 1 + len(self.delays)
        for attempt in range(1, tries + 1):
            status, output = self.run(arguments, cwd)
            said = classify(status, output, digest)
            if said != NO_ANSWER:
                return said, output
            print(f"no answer about {what}, try {attempt} of {tries}: {last_words(output)}", flush=True)
            if attempt < tries:
                time.sleep(self.delays[attempt - 1])
        return NO_ANSWER, output

    def download(self, tag, patterns, cwd):
        """The files of the release, with the same further tries. Whether
        a download failed for good or for a moment `gh` does not say in a
        way this script could rely on, so every failure is tried again."""
        arguments = ["release", "download", tag]
        for pattern in patterns:
            arguments += ["--pattern", pattern]
        tries = 1 + len(self.delays)
        for attempt in range(1, tries + 1):
            status, output = self.run(arguments, cwd)
            if status == 0:
                return
            print(f"the download failed, try {attempt} of {tries}: {last_words(output)}", flush=True)
            if attempt < tries:
                time.sleep(self.delays[attempt - 1])
        could_not_check(self, f"the files of {tag} could not be downloaded; gh's last words: {last_words(output)}")


def write_summary(title, lines):
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not path:
        return
    with open(path, "a", encoding="utf-8") as summary:
        summary.write(f"### {title}\n\n")
        for line in lines:
            summary.write(f"- {line}\n")
        summary.write("\n")


def verdict(check, message):
    """A file does not verify, or a changed one did: the release must not
    be published."""
    print(f"::error::NOT VERIFIED: {message}", flush=True)
    write_summary("Not verified", [*check.summary, f"**{message}**"])
    print(f"NOT VERIFIED: {message}")
    sys.exit(1)


def could_not_check(check, message):
    """No verdict: the check itself could not be made."""
    print(f"::error::COULD NOT CHECK: {message}", flush=True)
    write_summary(
        "Could not check",
        [
            *check.summary,
            f"**{message}**",
            "This says nothing about the files: the check could not be made. Run the job again.",
        ],
    )
    print(f"COULD NOT CHECK: {message}")
    sys.exit(2)


def main():
    parser = argparse.ArgumentParser(description="Checks the build attestations of a release's files.")
    parser.add_argument("tag")
    parser.add_argument("--repo", required=True, help="the repository of the release, <owner>/<repository>")
    parser.add_argument("--linux-signer", required=True, help="the repository whose workflow builds the Linux files")
    parser.add_argument("--gh", nargs=argparse.REMAINDER, default=None, help="a program to run in place of gh")
    options = parser.parse_args()
    if "/" not in options.repo:
        parser.error("--repo is <owner>/<repository>")
    owner = options.repo.split("/", 1)[0]
    version = options.tag.removeprefix("v")
    delays = [float(part) for part in os.environ.get("RICK_VERIFY_DELAYS", "5,15,30").split(",") if part.strip()]
    check = Check(options.gh or ["gh"], delays)

    with tempfile.TemporaryDirectory() as folder:
        folder = Path(folder)
        macos = f"rick-{version}-macos-arm64.zip"
        windows = f"rick-{version}-windows-x64.zip"
        check.download(options.tag, ["rick_*_amd64.deb", macos, windows], folder)
        packages = sorted(folder.glob("rick_*_amd64.deb"))
        if not packages:
            could_not_check(check, f"the release of {options.tag} has no rick_*_amd64.deb")
        package = packages[0]
        for archive in (macos, windows):
            if not (folder / archive).is_file():
                could_not_check(check, f"the release of {options.tag} has no {archive}")

        linux_line = ["--owner", owner, "--signer-repo", options.linux_signer]
        own_line = ["--repo", options.repo]
        files = [
            (package.name, linux_line, f"built by the workflow of {options.linux_signer}"),
            (macos, own_line, "built by this workflow"),
            (windows, own_line, "built by this workflow"),
        ]
        for name, line, built in files:
            print(f"::group::{name}, {built}", flush=True)
            said, output = check.answer(name, ["attestation", "verify", name, *line], folder, sha256(folder / name))
            print("::endgroup::", flush=True)
            if said == VERIFIED:
                print(f"verified: {name}", flush=True)
                check.summary.append(f"`{name}` verifies.")
            elif said == NO_ATTESTATION:
                verdict(check, f"{name} has no attestation. gh's last words: {last_words(output)}")
            elif said == NOT_VERIFIED:
                verdict(check, f"{name} has an attestation that does not verify. gh's last words: {last_words(output)}")
            else:
                could_not_check(
                    check,
                    f"gh gave no answer about {name} in {1 + len(delays)} tries; its last words: {last_words(output)}",
                )

        # One bit of the package flipped, in the middle of the file.
        changed = folder / "changed.deb"
        data = bytearray(package.read_bytes())
        data[len(data) // 2] ^= 1
        changed.write_bytes(data)
        digest = sha256(changed)
        print("::group::the same package with one byte changed", flush=True)
        said, output = check.answer(
            "the changed package", ["attestation", "verify", changed.name, *linux_line], folder, digest
        )
        print("::endgroup::", flush=True)
        if said == NO_ATTESTATION:
            print(f"refused, as it must be: no attestation for sha256:{digest}", flush=True)
            check.summary.append("A copy of the package with one byte changed is refused: there is no attestation for its checksum.")
        elif said == VERIFIED:
            verdict(check, f"A CHANGED FILE VERIFIED: a copy of {package.name} with one byte changed passed the check.")
        elif said == NOT_VERIFIED:
            could_not_check(
                check,
                "the changed package was refused, but not because it has no attestation; "
                f"gh's last words: {last_words(output)}",
            )
        else:
            could_not_check(
                check,
                f"gh gave no answer about the changed package in {1 + len(delays)} tries; "
                f"its last words: {last_words(output)}",
            )

    write_summary(f"Verified, the draft of {options.tag}", check.summary)
    print("the package and both archives verify, and the changed package has no attestation")


if __name__ == "__main__":
    main()
