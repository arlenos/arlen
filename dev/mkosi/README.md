# dev/mkosi - the minimal bootable Arlen image + QEMU verify VM

The system-under-test image: a Debian-Trixie UEFI disk image that boots the
compositor + shell + daemons + login, run in QEMU so the whole assembled stack
can be exercised and screenshot-verified (boot -> QMP-inject -> screendump ->
inspect). It is the first real Arlen boot and the path to autonomous verification
of the interactive tier the nested harness cannot drive. NOT the product image
(no installer / onboarding / stub apps). Grounded approach +the ranked hard parts:
`docs/architecture/minimal-image-vm-build-plan.md`.

## Host setup (one line, needs root)

mkosi is in Arch `extra`; install it once:

```
sudo pacman -S mkosi
```

Present already on this box: `qemu-system-x86_64` 11, `systemd-repart`, `tesseract`,
and OVMF at `/usr/share/edk2/x64/OVMF_CODE.4m.fd` + `OVMF_VARS.4m.fd` (the verify
harness points at these). `ToolsTree=default` in `mkosi.conf` carries the Debian
apt/dpkg/debootstrap, so no Debian tooling is needed on the host.

## Build + boot

```
dev/mkosi/build-image.sh                 # build everything + assemble arlen.raw
dev/vm/verify.py --require-bar           # boot headless, screendump, assert the full desktop
```

`build-image.sh` is the orchestrator: it zigbuilds the pure-Rust daemons (event-bus)
for the Debian target, then runs `mkosi build --force` whose `mkosi.build.d/` phases
build the C++/system-lib binaries Debian-native (the knowledge daemon, cosmic-comp,
the desktop shell) and stage them in. `verify.py` boots the image in QEMU with a
single virtio-gpu (software GL/llvmpipe), waits for the session, captures the
framebuffer over QMP `screendump`, and asserts the compositor rendered + (with
`--require-bar`) the shell's top bar is present. **Status: the image boots end-to-end
to a pixel-verified Arlen desktop** (greetd -> cosmic-comp -> arlen-desktop-shell bar)
with event-bus + the knowledge daemon running.

To run mkosi by hand: `PATH=/usr/sbin:/sbin:$PATH mkosi build --force` (then `mkosi
vm`). `--force` is required (mkosi refuses if `arlen.raw` exists and silently keeps
the stale image). The `/usr/sbin:/sbin` PATH prefix is mandatory on this Arch host.

The `PATH=/usr/sbin:/sbin:$PATH` prefix is mandatory on this Arch host (a
cross-distro quirk, not optional). mkosi runs `depmod` for the kernel modules by
building the candidate exec paths from the *host* PATH, then `chroot`ing into the
Debian rootfs and `execve`ing those path strings. Arch ships the kmod tools in
`/usr/bin`, so the candidate is `/usr/bin/depmod` - but Debian puts depmod in
`/usr/sbin`, so post-chroot `/usr/bin/depmod` does not exist and the build dies
"depmod not found". Prepending `/usr/sbin:/sbin` makes the candidate `/usr/sbin/
depmod`, which resolves in the image (matches mkosi discussion #4059). The rootfs
also needs the `kmod` package named explicitly (it is not pulled implicitly);
both are in `mkosi.conf` / this prefix.

`mkosi boot` / `mkosi shell` are nspawn (no real kernel, no GPU/DRM) - rootfs
sanity only, NOT the compositor/GPU path. The verify pass drives `mkosi vm`'s
QEMU with QMP `screendump` + `input-send-event` (dev/vm/, Phase 4).

## Owned deploy-topology decisions (decided here, the plan's Phase 3)

The socket/runtime-dir layout - previously deferred as "Phase-10" - is decided:

- **System sockets live in `/run/arlen/`.** Every system daemon that uses it
  declares `RuntimeDirectory=arlen` + `RuntimeDirectoryMode=0755` +
  **`RuntimeDirectoryPreserve=yes`**. PID1 creates + bind-mounts the dir writable
  *before* `ProtectSystem=strict` remounts read-only, so a hardened daemon owns
  its writable socket dir with NO `ReadWritePaths=` (which would hard-fail on a
  missing path under strict). `Preserve=yes` is mandatory: systemd does not
  ref-count a shared `RuntimeDirectory`, so without it the first daemon to stop
  deletes `/run/arlen` out from under the others. This removes the start/stop
  ordering edges between the daemons that share it.
- **Per-user sockets live in `/run/user/%U/arlen/`** (the AI agent, anything
  session-scoped), created by a `systemd --user` service with
  `RuntimeDirectory=arlen` + `Preserve=yes` (resolves under `$XDG_RUNTIME_DIR`,
  per-user by construction). A *system* tmpfiles rule does not work (`%U` -> 0).
- **Daemon ordering:** with `Preserve=yes` the runtime dir is no longer an
  ordering constraint; the remaining edges are the real producer/consumer ones
  (the knowledge daemon `After=arlen-event-bus.service`, etc.), already declared
  on the units.

## Autostart (the plan's Phase 2)

greetd `[initial_session]` auto-logs-in the `arlen` user once per boot and starts
the session (it owns a PAM/logind session, so `XDG_RUNTIME_DIR`/`XDG_SEAT`/`XDG_VTNR`
exist). With logind present, libseat auto-detects logind - do NOT also run seatd.
The session must hand the compositor's env to the user units
(`dbus-update-activation-environment --systemd WAYLAND_DISPLAY ...` ->
`systemctl --user start graphical-session.target`) or the user services launch
blind. Wired in Phase 2 (`mkosi.extra/etc/greetd/config.toml` + the session glue).

## Status / phase roadmap

All four phases DONE: the image boots end-to-end to a pixel-verified Arlen desktop.

- **Phase 1 - the mkosi recipe (`mkosi.conf`):** Debian-Trixie UEFI disk image.
  First-build findings, resolved: `kmod` named explicitly (else "depmod not found"),
  and the `/usr/sbin:/sbin` host-PATH prefix (the cross-distro depmod-chroot quirk).
- **Phase 2 - the stack overlay + autostart (`mkosi.extra/`):** greetd
  `[initial_session]` + `[default_session]` -> `/usr/bin/arlen-session` (launches
  `cosmic-comp` via the udev/KMS backend, captures WAYLAND_DISPLAY, launches the
  shell), the PAM stack (pam_systemd), the logind `NAutoVTs=0` + greetd
  `Conflicts=getty@tty1` so the compositor owns vt1, the `/home/arlen` chown
  tmpfiles fix, and `mkosi.postinst` (the locked autologin user). The unit-enable
  symlinks ship via the overlay (`preset-all`-safe).
- **Phase 3 - runtime-dir topology:** system sockets in `/run/arlen`
  (`RuntimeDirectory=arlen` + `Preserve=yes`); the per-user-defaulting daemons run
  as SUT system services via drop-in env overrides.
- **Phase 4 - the QEMU verify channel (`dev/vm/verify.py`):** single `virtio-gpu`
  (software GL/llvmpipe, CPU-readable - no `gl=on`), OVMF pflash, `-qmp`
  `screendump` -> PNG, asserts the compositor rendered (non-black, not the
  kernel/login console via OCR) and `--require-bar` asserts the shell's top bar
  (a structural color check). The binary population: `event-bus` via cargo-zigbuild
  (pure-Rust, glibc-pinned), `arlen-graph-daemon`/`cosmic-comp`/`arlen-desktop-shell`
  Debian-native via `mkosi.build.d/`.

Both ranked hard parts are cleared with pixel proof: #2 cosmic-comp on virtio-gpu/
llvmpipe, #1 the Tauri shell's WebKitGTK under software GL. NEXT: the interactive
"click tier" (QMP `input-send-event` on the rendered desktop) and the KG-AI loop
dogfood (gated on the model toolchain).
