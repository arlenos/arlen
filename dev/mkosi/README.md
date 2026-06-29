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
cd dev/mkosi
PATH=/usr/sbin:/sbin:$PATH mkosi build --force   # assemble the Debian rootfs + disk image
mkosi vm                                         # boot it in QEMU (a real kernel + DRM path)
```

`--force` is required to re-run a build: mkosi refuses if `arlen.raw` already
exists ("Output path ... exists already. Use --force to rebuild.") and silently
keeps the stale image otherwise.

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

- **Phase 1 - the mkosi recipe: DONE** (`mkosi.conf`). First real build surfaced
  two findings, both now resolved: the rootfs needs `kmod` named explicitly (else
  "depmod not found" at the module step), and the build must run with
  `/usr/sbin:/sbin` on the host PATH (the cross-distro depmod-chroot quirk above).
- **Phase 2 - the stack overlay + autostart: session glue DONE** (`mkosi.extra/`):
  greetd `[initial_session]` -> `/usr/bin/arlen-session` (launches `cosmic-comp`,
  waits for its Wayland socket, imports the Arlen socket env to the --user manager,
  starts `graphical-session.target`), the shell `systemd --user` unit, and
  `mkosi.postinst` (creates the locked autologin `arlen` user, enables greetd +
  the shell user unit). REMAINING: the binary population - the arlen binaries
  (`cosmic-comp` from the compositor repo, `arlen-desktop-shell`, `event-bus`,
  the daemons) built for the Debian target + dropped into `mkosi.extra/usr/bin/`,
  and the system-daemon units copied into `mkosi.extra/usr/lib/systemd/system/`.
  Until then the image boots to greetd autologin -> arlen-session, which fails
  gracefully on the missing compositor (the autostart plumbing is in + testable).
- **Phase 3 - runtime-dir topology:** the `RuntimeDirectory=`/`Preserve=yes` decided
  above, applied across the units.
- **Phase 4 - the QEMU verify channel (`dev/vm/`):** plain `virtio-gpu` (software
  GL/llvmpipe, CPU-readable framebuffer - NOT `gl=on`, which captures black), OVMF
  pflash, `-qmp` + `-device usb-tablet`, the WebKitGTK software-GL env vars in the
  guest session, `screendump` PNG + tesseract OCR (the NixOS-test-driver pattern).

The hard parts (WebKitGTK under software GL #1, cosmic-comp on virtio-gpu llvmpipe
#2) are worked through in-build, not escalated - see the plan.
