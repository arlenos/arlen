# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that the hand-kept build lists still describe the repo.

Four things have to agree and drift apart silently when they do not:

  1. `RUST_ALL` / `FRONT_ALL` in `.github/workflows/ci.yml` - what CI builds.
  2. `RUST_CRATES` / `FRONTENDS` in `dev/justfile` - what `just check`/`test`
     build.
  3. the crate roots on disk.
  4. the package.json files that declare a test script.

Nothing enforced (1) == (2), so the justfile fell nine crates behind CI and
`just test` reported a green that CI did not share. Nothing enforced (3) or (4)
either, so a new crate or app is built by CI only if whoever added it remembered
the matrix.

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
    # The same reason, for the two Tauri hosts that do not live under `apps/`.
    # Named rather than folded into a `*/src-tauri` glob: this file's patterns are
    # matched left-anchored by the integration test that reads them, and a glob
    # whose two readers anchor it differently is how the exclusion came to mean one
    # thing here and another there.
    "daemons/xdg-portal/picker-ui/src-tauri": "the portal's picker UI is a Tauri host; same webkit dependency",
    "sdk/ui-kit/src-tauri": "the kit's own Tauri host; same webkit dependency",
    "daemons/kernel-layer": "eBPF needs the bpf target and toolchain, not in the CI image",
    # Added to the matrix on 9 August to satisfy this gate, which produced a job
    # that failed in 17 seconds with no error line: too fast to have built a
    # WebKit-linked crate, so it was missing system libraries the runner has no
    # reason to carry. It is a PROBE - it drives a real webview to answer
    # questions about keyboard focus and transparency - so it needs a browser
    # engine and a display, and belongs here with a reason rather than in a job
    # that cannot succeed.
    "dev/ghost-webview": "a webview probe; needs webkit2gtk and a display, like the Tauri hosts",
}


def excluded(path: str) -> bool:
    p = pathlib.PurePath(path)
    return any(p.match(pat) or p.is_relative_to(pat) for pat in EXCLUDED)


def tracked_files(pattern: str) -> list[str]:
    return subprocess.run(
        ["git", "ls-files", pattern],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()


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
    front_ci_list = listed("FRONT_ALL", ci, r"FRONT_ALL='(\[.*?\])'")
    rust_just = listed("RUST_CRATES", jf, r'RUST_CRATES := "(.*?)"')
    front_just = listed("FRONTENDS", jf, r'FRONTENDS := "(.*?)"')

    # Four empty lists agree with each other perfectly, and with an empty tree, so
    # without this the check reports "0 rust, 0 frontend, tree fully covered" and
    # exits 0 the day a regex stops matching - a renamed variable, a reformatted
    # array, a moved justfile. That is the failure this gate is least able to
    # survive, because a matrix that lists nothing builds nothing and every
    # downstream job goes green by having no work.
    for label, got in (
        ("RUST_ALL", rust_ci),
        ("FRONT_ALL", front_ci_list),
        ("RUST_CRATES", rust_just),
        ("FRONTENDS", front_just),
    ):
        if not got:
            print(f"{label} came back empty; the list moved or its pattern stopped matching")
            return 2

    problems: list[str] = []

    for label, a, b in (
        ("RUST_ALL / RUST_CRATES", rust_ci, rust_just),
        ("FRONT_ALL / FRONTENDS", front_ci_list, front_just),
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
    tracked = tracked_files("*Cargo.toml")

    in_ci = set(rust_ci)
    front_ci = set(front_ci_list)
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

    # The frontend side of the same claim: the matrix comment says every
    # package.json declaring a test script is listed. The rust arm above checks
    # its half against the tree; without this the frontend half was only ever
    # checked against the justfile, so a new app with tests could be listed
    # nowhere and nothing would say so.
    for f in tracked_files("*package.json"):
        if "node_modules" in f:
            continue
        try:
            pkg = json.loads((ROOT / f).read_text())
        except json.JSONDecodeError:
            problems.append(f"{f} is not valid JSON")
            continue
        scripts = pkg.get("scripts", {})
        if not {"test", "check"} & set(scripts):
            continue
        d = str(pathlib.Path(f).parent)
        if d not in front_ci:
            problems.append(f"{d} declares a test script but is in no frontend matrix")

    if problems:
        print("crate list drift:\n")
        for p in problems:
            print(f"  - {p}")
        print("\nthe two lists and the tree have to agree; fix whichever is behind")
        return 1

    print(f"crate lists agree: {len(rust_ci)} rust, {len(front_ci_list)} frontend, tree fully covered")
    return 0


if __name__ == "__main__":
    sys.exit(main())
