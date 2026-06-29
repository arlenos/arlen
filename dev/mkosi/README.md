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
mkosi --directory dev/mkosi build      # assemble the Debian rootfs + disk image
mkosi --directory dev/mkosi vm         # boot it in QEMU (a real kernel + DRM path)
```

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

- **Phase 1 - the mkosi recipe: DONE** (`mkosi.conf`). Verify on the first real
  build (per the plan, don't lock from docs): the implicit minbase set, the exact
  Debian package names for the Mesa/WebKitGTK runtime, `mkosi vm` vs `qemu` for
  the installed mkosi version.
- **Phase 2 - the stack overlay + autostart:** `mkosi.extra/` (the arlen binaries,
  the daemon + user units, greetd config, `/etc/arlen/*`), `mkosi.postinst`
  (`systemctl --root enable`), the session env-handoff glue. NB the event-bus has
  no systemd unit yet (create it with the topology above); the compositor binary
  is `cosmic-comp` (separate repo).
- **Phase 3 - runtime-dir topology:** the `RuntimeDirectory=`/`Preserve=yes` decided
  above, applied across the units.
- **Phase 4 - the QEMU verify channel (`dev/vm/`):** plain `virtio-gpu` (software
  GL/llvmpipe, CPU-readable framebuffer - NOT `gl=on`, which captures black), OVMF
  pflash, `-qmp` + `-device usb-tablet`, the WebKitGTK software-GL env vars in the
  guest session, `screendump` PNG + tesseract OCR (the NixOS-test-driver pattern).

The hard parts (WebKitGTK under software GL #1, cosmic-comp on virtio-gpu llvmpipe
#2) are worked through in-build, not escalated - see the plan.
