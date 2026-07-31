# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that the two hand-kept crate lists still describe the repo.

Three lists have to agree and drift apart silently when they do not:

  1. `RUST_ALL` in `.github/workflows/ci.yml` - what CI actually builds.
  2. `RUST_CRATES` in `dev/justfile` - what `just check`/`test`/`lint` build.
  3. the crate roots on disk.

Nothing enforced (1) == (2), so the justfile fell nine crates behind CI and
`just test` reported a green that CI did not share. Nothing enforced (3) either,
so a new crate is built by CI only if whoever added it remembered the matrix.

The exclusions below are the two documented ones. They are listed as prefixes
rather than inferred, so dropping a crate out of CI is a visible edit here.
"""

import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

# Crate roots CI deliberately does not build, and why. Anything else missing
# from the matrix is a mistake, not a decision.
EXCLUDED = {
    "apps/*/src-tauri": "Tauri hosts need system webkit2gtk; the frontend job covers the app",
    "daemons/kernel-layer": "eBPF needs the bpf target and toolchain, not in the CI image",
}


def excluded(path: str) -> bool:
    p = pathlib.PurePath(path)
    return any(p.match(pat) or p.is_relative_to(pat) for pat in EXCLUDED)


def listed(name: str, text: str, pattern: str) -> list[str]:
    m = re.search(pattern, text, re.S)
    if not m:
        sys.exit(f"could not find {name}; the check needs updating")
    raw = m.group(1)
    return json.loads(raw) if raw.lstrip().startswith("[") else raw.split()


def main() -> int:
    ci = (ROOT / ".github/workflows/ci.yml").read_text()
    jf = (ROOT / "dev/justfile").read_text()

    rust_ci = listed("RUST_ALL", ci, r"RUST_ALL='(\[.*?\])'")
    front_ci = listed("FRONT_ALL", ci, r"FRONT_ALL='(\[.*?\])'")
    rust_just = listed("RUST_CRATES", jf, r'RUST_CRATES := "(.*?)"')
    front_just = listed("FRONTENDS", jf, r'FRONTENDS := "(.*?)"')

    problems: list[str] = []

    for label, a, b in (
        ("RUST_ALL / RUST_CRATES", rust_ci, rust_just),
        ("FRONT_ALL / FRONTENDS", front_ci, front_just),
    ):
        only_ci = sorted(set(a) - set(b))
        only_just = sorted(set(b) - set(a))
        if only_ci:
            problems.append(f"{label}: CI builds these, the justfile does not: {only_ci}")
        if only_just:
            problems.append(f"{label}: the justfile builds these, CI does not: {only_just}")

    # Every path in the matrix has to exist, or the job fails on a missing manifest.
    for c in rust_ci:
        if not (ROOT / c / "Cargo.toml").exists():
            problems.append(f"RUST_ALL lists {c}, which has no Cargo.toml")

    # Every crate root on disk is built by CI, covered by a workspace root that is,
    # or explicitly excluded above.
    tracked = subprocess.run(
        ["git", "ls-files", "*Cargo.toml"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()

    in_ci = set(rust_ci)
    workspaces = {c for c in in_ci if "[workspace]" in (ROOT / c / "Cargo.toml").read_text()}

    for f in tracked:
        d = str(pathlib.Path(f).parent)
        text = (ROOT / f).read_text()
        if "[package]" not in text and "[workspace]" not in text:
            continue
        if d in in_ci or excluded(d):
            continue
        if any(str(anc) in workspaces for anc in pathlib.PurePath(d).parents):
            continue
        problems.append(f"{d} is a crate nothing builds: add it to RUST_ALL + RUST_CRATES")

    if problems:
        print("crate list drift:\n")
        for p in problems:
            print(f"  - {p}")
        print("\nthe two lists and the tree have to agree; fix whichever is behind")
        return 1

    print(f"crate lists agree: {len(rust_ci)} rust, {len(front_ci)} frontend, tree fully covered")
    return 0


if __name__ == "__main__":
    sys.exit(main())
