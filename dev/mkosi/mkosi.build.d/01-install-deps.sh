#!/bin/sh
# Phase 1 of the Debian-native build (NON-chroot: runs in mkosi's tools context,
# where apt is available and targets the image overlay - the minimal image itself
# ships no apt). Installs the build deps for the heavy binaries into the throwaway
# build overlay, so the next (chroot) script finds cargo + the C/C++ toolchain and
# none of this reaches the final image.
#
# Knowledge daemon: the C/C++ toolchain (cmake + g++ + make for lbug/Kuzu),
# prost's protoc, fuser's libfuse3, pkg-config, + curl/ca-certificates for the
# rustup bootstrap. NOT Debian's cargo/rustc: Trixie ships 1.85, but deps (time
# 0.3.47) need >= 1.88, so phase 2 installs a current stable toolchain via rustup.
set -eu

[ "${WITH_NETWORK:-0}" = "1" ] || { echo "needs WithNetwork=yes (apt fetch)" >&2; exit 1; }

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
    ca-certificates curl cmake g++ make protobuf-compiler pkg-config libfuse3-dev \
    libudev-dev libgbm-dev libxkbcommon-dev libegl1-mesa-dev libwayland-dev \
    libinput-dev libdbus-1-dev libsystemd-dev libseat-dev libdisplay-info-dev \
    libpixman-1-dev
