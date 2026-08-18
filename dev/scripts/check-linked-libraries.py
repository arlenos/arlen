#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A crate that links a C library must have that library on BOTH sides.

On 18 August the image stopped building, and had been unbuildable since the
viewer's HEIC decoder landed:

    Package 'libheif', required by 'virtual:world', not found
    The system library `libheif` required by crate `libheif-sys` was not found.

`libheif` appeared in neither `01-install-deps.sh` (the build phase) nor
`mkosi.conf` (the image). The comment above the decoder build loop said it did -
"heic and jxl link C libraries (libheif, libjxl) that phases 1-2 installed" - and
was wrong twice over: libheif was never installed, and jxl links nothing at all
because `jxl-oxide` is pure Rust. Written from intent, not from the config.

Both sides matter and they fail differently, which is why this checks both:

    the DEV package missing   the build dies, loudly, at pkg-config. Annoying and
                              obvious - it is what happened here.
    the RUNTIME lib missing   the build passes and the binary cannot start, so a
                              format the desktop entry CLAIMS fails at the moment
                              a person opens one of those files. That is the one
                              worth a gate.

`check-runtime-deps.py` covers binaries a component SPAWNS. Nothing covered
libraries a component LINKS, which is how this reached the image with neither end
declared.

A NOTE ON PLUGINS, because naming the library is not always enough: libheif is a
container and its codecs are separate packages (`libheif-plugin-libde265` for
HEVC/heic, `libheif-plugin-dav1d` for AV1/avif). They arrive as Recommends, and
`mkosi.conf`'s own rule for `xdg-user-dirs` says a correctness-bearing package is
named rather than left to another package's suggestion list. So the table below
lists what the FEATURE needs, not what the linker needs.

Run: dev/scripts/check-linked-libraries.py [repo-root]
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
DEPS = ROOT / "dev/mkosi/mkosi.build.d/01-install-deps.sh"
CONF = ROOT / "dev/mkosi/mkosi.conf"

#: crate -> what it needs on each side. The crate key is the DIRECT dependency as
#: it appears in a Cargo.toml, not the `-sys` crate underneath it: `libheif-rs`
#: is what someone writes, `libheif-sys` is what fails.
LINKED: dict[str, dict] = {
    "libheif-rs": {
        "why": "the viewer's HEIC/AVIF decoder (apps/viewers/decode-heic)",
        "dev": ["libheif-dev"],
        # Not just libheif1: the codecs are plugins and BOTH claimed types need
        # one - image/heic is HEVC, image/avif is AV1, and core/src/lib.rs routes
        # both to the same worker.
        "runtime": ["libheif1", "libheif-plugin-libde265", "libheif-plugin-dav1d"],
    },
    "gtk": {
        "why": "the desktop shell's GTK layer-shell surfaces",
        "dev": ["libgtk-3-dev"],
        "runtime": ["libgtk-3-0"],
    },
    "gtk-layer-shell": {
        "why": "the shell's top bar is a layer-shell surface",
        "dev": ["libgtk-layer-shell-dev"],
        "runtime": ["libgtk-layer-shell0"],
    },
}

#: crate -> why it links no system library despite looking like it might. Kept
#: with reasons so the next person does not add a package nobody needs.
NO_SYSTEM_LIB: dict[str, str] = {
    "rusqlite": (
        'carries `features = ["bundled"]`, so it compiles SQLite into the binary '
        "rather than linking the system one"
    ),
    "jxl-oxide": (
        "pure Rust. The comment this check was written for claimed it linked "
        "libjxl; it does not, and adding libjxl would have been cargo-culting the "
        "same wrong sentence into the config"
    ),
}

#: Where a crate that reaches the image can live.
CRATE_ROOTS = ("apps", "daemons")


def _direct_deps(text: str) -> set[str]:
    """Dependency names from a Cargo.toml, without parsing TOML properly.

    Only lines that START a dependency entry count, so a `features = [...]` value
    mentioning a name is not read as a dependency.
    """
    out = set()
    in_deps = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("["):
            in_deps = s.startswith("[dependencies") or s.startswith("[target")
            in_deps = in_deps or "dependencies]" in s
            continue
        if not in_deps:
            continue
        m = re.match(r"^([A-Za-z0-9_-]+)\s*=", s)
        if m:
            out.add(m.group(1))
    return out


def main() -> int:
    if not DEPS.is_file() or not CONF.is_file():
        print(
            f"NOTHING WAS READ: expected {DEPS} and {CONF}",
            file=sys.stderr,
        )
        return 2

    deps_text = DEPS.read_text()
    conf_text = CONF.read_text()
    # The image's package list, so a name appearing in prose elsewhere in the file
    # does not count as declared.
    block = conf_text.split("Packages=", 1)
    if len(block) != 2:
        print(f"NOTHING WAS READ: no Packages= block in {CONF}", file=sys.stderr)
        return 2
    packages = block[1].split("\n[", 1)[0]
    declared = {
        line.strip()
        for line in packages.splitlines()
        if line.strip() and not line.strip().startswith("#")
    }

    manifests = [
        p
        for root in CRATE_ROOTS
        for p in (ROOT / root).rglob("Cargo.toml")
        if "target" not in p.parts and "node_modules" not in p.parts
    ]
    if not manifests:
        print(f"NOTHING WAS READ: no Cargo.toml under {CRATE_ROOTS}", file=sys.stderr)
        return 2

    problems: list[str] = []
    seen: dict[str, str] = {}

    for m in manifests:
        rel = m.relative_to(ROOT)
        for dep in _direct_deps(m.read_text()):
            if dep in NO_SYSTEM_LIB or dep in seen:
                continue
            if dep in LINKED:
                seen[dep] = str(rel)
                continue
            if dep.endswith("-sys"):
                problems.append(
                    f"{rel}: `{dep}` links a system library and is not in this "
                    f"check's table. Add it with its dev + runtime packages, or "
                    f"to NO_SYSTEM_LIB with the reason it needs none."
                )

    for crate, need in LINKED.items():
        if crate not in seen:
            continue
        for pkg in need["dev"]:
            if pkg not in deps_text:
                problems.append(
                    f"{crate} ({need['why']}) links a system library, but "
                    f"`{pkg}` is not installed in {DEPS.name}: the image build "
                    f"dies at pkg-config."
                )
        for pkg in need["runtime"]:
            if pkg not in declared:
                problems.append(
                    f"{crate} ({need['why']}) needs `{pkg}` on the image and "
                    f"{CONF.name} does not name it. The build passes and the "
                    f"feature fails when someone uses it."
                )

    if problems:
        print("a crate links a library the image does not carry:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1

    print(
        f"{len(seen)} crate(s) link a system library; each has its dev package in "
        f"the build phase and its runtime packages on the image "
        f"({len(NO_SYSTEM_LIB)} recorded as needing none)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
