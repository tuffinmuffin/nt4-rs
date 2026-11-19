#!/usr/bin/env python3
"""Prepare a release: set the version and date the changelog.

    python3 scripts/prepare_release.py 0.3.0 [--date 2026-11-19]

Written by Claude. Does every edit a release needs, so nobody has to find
them by hand:

- the workspace version and the published crates' versions for each other,
  all in the root Cargo.toml ([workspace.package] and
  [workspace.dependencies]);
- CHANGELOG.md: the "## Unreleased" section (any text after the word, such
  as "(0.3.0)", is dropped) becomes "## <version> (<date>)";
- crates/*/README.md: install snippets such as `nt4 = "0.2"` name the new
  major.minor (or the full version for a pre-release).

It refuses, changing nothing, when the version isn't newer than the current
one, or the changelog has no Unreleased section or an empty one. The
"Prepare release" workflow runs it; it works the same locally. Cargo.lock is
updated by the caller (`cargo update --workspace`).
"""

import argparse
import datetime
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
PUBLISHED = ["nt4-proto", "nt4-core", "nt4-derive", "nt4", "nt4-wpimath"]
VERSION = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$")


def parse(v):
    m = VERSION.match(v)
    if not m:
        sys.exit(f"error: {v!r} isn't a version like 0.3.0 or 0.3.0-alpha.1")
    major, minor, patch, pre = m.groups()
    # A pre-release sorts before its release (0.3.0-alpha.1 < 0.3.0).
    return (int(major), int(minor), int(patch), pre is None, pre or "")


def current_version(cargo):
    m = re.search(r'^\[workspace\.package\]\n(?:.*\n)*?version = "([^"]+)"', cargo, re.M)
    if not m:
        sys.exit("error: no version in [workspace.package] in Cargo.toml")
    return m.group(1)


def bump_cargo(cargo, old, new):
    # The workspace version.
    cargo, n = re.subn(
        r'(^\[workspace\.package\]\n(?:.*\n)*?version = ")' + re.escape(old) + '"',
        lambda m: m.group(1) + new + '"',
        cargo,
        count=1,
        flags=re.M,
    )
    assert n == 1
    # Each published crate in [workspace.dependencies].
    for crate in PUBLISHED:
        pattern = (
            r"(^" + re.escape(crate) + r' = \{ path = "crates/' + re.escape(crate)
            + r'", version = ")[^"]+"'
        )
        cargo, n = re.subn(pattern, lambda m: m.group(1) + new + '"', cargo, flags=re.M)
        if n != 1:
            sys.exit(f"error: {crate} isn't in [workspace.dependencies] as expected")
    return cargo


def date_changelog(changelog, new, date):
    m = re.search(r"^## Unreleased.*$", changelog, re.M)
    if not m:
        sys.exit("error: CHANGELOG.md has no '## Unreleased' section to release")
    rest = changelog[m.end():]
    nxt = re.search(r"^## ", rest, re.M)
    body = rest[: nxt.start()] if nxt else rest
    if not body.strip():
        sys.exit("error: CHANGELOG.md's Unreleased section is empty")
    return changelog[: m.start()] + f"## {new} ({date})" + changelog[m.end():]


def readme_snippets(text, new):
    v = parse(new)
    # What users write in Cargo.toml: "0.3" for 0.3.x, the full version for
    # a pre-release (Cargo doesn't pick pre-releases otherwise).
    want = new if not v[3] else f"{v[0]}.{v[1]}"
    for crate in PUBLISHED:
        text = re.sub(
            r'(^' + re.escape(crate) + r' = (?:\{ version = )?")[^"]+(")',
            lambda m: m.group(1) + want + m.group(2),
            text,
            flags=re.M,
        )
    return text


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("version")
    ap.add_argument("--date", default=datetime.date.today().isoformat())
    args = ap.parse_args()
    new = args.version.removeprefix("v")

    cargo_path = ROOT / "Cargo.toml"
    changelog_path = ROOT / "CHANGELOG.md"
    cargo = cargo_path.read_text()
    old = current_version(cargo)
    if parse(new) <= parse(old):
        sys.exit(f"error: {new} isn't newer than the current version {old}")

    # Work everything out first, so a refusal changes nothing.
    edits = {
        cargo_path: bump_cargo(cargo, old, new),
        changelog_path: date_changelog(changelog_path.read_text(), new, args.date),
    }
    for readme in sorted(ROOT.glob("crates/*/README.md")):
        text = readme.read_text()
        bumped = readme_snippets(text, new)
        if bumped != text:
            edits[readme] = bumped
    for path, text in edits.items():
        path.write_text(text)
        print(f"updated {path.relative_to(ROOT)}")
    print(f"{old} -> {new}; now run: cargo update --workspace")


if __name__ == "__main__":
    main()
