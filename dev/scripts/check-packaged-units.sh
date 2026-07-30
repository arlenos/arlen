#!/usr/bin/env bash
# Guard against silent drift between a daemon's canonical dist/*.service unit and
# the hand-maintained copy the image ships in dev/mkosi/mkosi.extra. The two are
# separate files today (mkosi.extra is copied verbatim into the image), so a
# hardening directive added to one but not the other deploys a unit that differs
# from the reviewed one - exactly the class that shipped an unaudited producer and
# a broken peer-auth sandbox before. This compares the DIRECTIVE lines only
# (stripping comments and blanks), so a comment reword never fails the gate but a
# real directive difference does. Units with no dist/ counterpart (arlen-ai-proxy,
# arlen-dogfood, arlen-config-broker, arlen-llama, arlen-graph, arlen-timeline)
# are mkosi-only and skipped.
#
# Exit 0 = every packaged unit's directives match its dist/ canonical (or has no
# canonical). Exit 1 = a drift a reviewer must reconcile.
set -euo pipefail

# Every gate below runs even when an earlier one fails, and the script exits
# non-zero at the end. Bailing on the first failure meant one long-standing
# drift silently disabled the gates after it - the netlink sandbox check was
# dead for exactly that reason, which is the kind of thing a guard is supposed
# to prevent, not demonstrate.
gate_failed=0

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

# Directive-only view of a unit: drop comments (# ...) and blank lines, so the
# comparison ignores prose and reflow.
directives() {
  grep -vE '^[[:space:]]*#|^[[:space:]]*$' "$1"
}

drift=0
checked=0
skipped=0

# Every packaged systemd unit under mkosi.extra (user + system trees), excluding
# the *.target.wants/ enablement symlinks (they point at the real unit files).
while IFS= read -r pkg; do
  base="$(basename "$pkg")"
  # The canonical unit is the dist/*.service with the same basename, if any.
  canonical="$(find daemons -path '*/dist/*.service' -name "$base" 2>/dev/null | head -1)"
  if [ -z "$canonical" ]; then
    skipped=$((skipped + 1))
    continue
  fi
  checked=$((checked + 1))
  if ! diff <(directives "$canonical") <(directives "$pkg") >/dev/null 2>&1; then
    # A KNOWN, reasoned exception does not fail the gate - but it is reprinted on
    # every run, because an exception nobody sees is just a silent failure with
    # extra steps. Anything NOT listed here still fails, which is the point:
    # leaving the whole gate red would mean new drift lands unnoticed behind an
    # old one.
    case "$base" in
      arlen-config-broker.service)
        echo "KNOWN DRIFT (not failing): $base"
        echo "  The image packages this as User=root with the state/runtime dirs and the"
        echo "  hardening stripped, because it creates no arlen-config user and ships no"
        echo "  /etc/arlen/config-broker.env - and the canonical EnvironmentFile= has no"
        echo "  '-' prefix, so the canonical unit would fail to start there outright."
        echo "  CONSEQUENCE: in the image, the daemon owning executor_live, access_level"
        echo "  and the provider settings runs as root with no separate-uid isolation."
        echo "  FIX: provision the user (sysusers.d) + write config-broker.env at image"
        echo "  build, then delete this exception. Not a reconcile."
        echo
        continue
        ;;
    esac
    drift=$((drift + 1))
    echo "DRIFT: $base"
    echo "  canonical: $canonical"
    echo "  packaged:  $pkg"
    echo "  --- directive diff (canonical vs packaged) ---"
    diff <(directives "$canonical") <(directives "$pkg") | sed 's/^/  /' || true
    echo
  fi
done < <(find dev/mkosi/mkosi.extra -name '*.service' -not -path '*.wants/*' | sort)

if [ "$drift" -ne 0 ]; then
  echo "FAIL: $drift packaged unit(s) drifted from their dist/ canonical (directives differ)."
  echo "Reconcile the packaged copy under dev/mkosi/mkosi.extra with the canonical daemons/*/dist unit."
  gate_failed=1
fi

echo "OK: $checked packaged unit(s) match their dist/ canonical; $skipped mkosi-only unit(s) skipped."

# --- Second check: a sandbox-spawning daemon must not be denied AF_NETLINK -----
#
# bwrap brings up loopback inside a new network namespace through a NETLINK_ROUTE
# socket. RestrictAddressFamilies is inherited by children, so a unit that lists
# only AF_UNIX makes that socket() fail EAFNOSUPPORT and bwrap dies BEFORE it execs
# the payload. This cost a silently-dead pi sidecar: the daemon started, owned its
# bus names and looked healthy while its confined child never ran once. Nothing on
# the host reproduces it (no such filter there), so it is invisible until a boot.
#
# So: any crate that spawns bwrap AND ships a unit that restricts address families
# must include AF_NETLINK. Crates are matched by their own dist/ dir, not by the
# ExecStart basename (arlen-accountsd lives in online-accounts, arlen-powerd in
# power-daemon - basename-to-crate does not hold).
netlink_fail=0
netlink_checked=0

while IFS= read -r unit; do
  crate="${unit%/dist/*}"
  # Does this crate spawn a sandbox?
  if ! grep -rqE 'arlen-confiner|arlen_confiner|"bwrap"' "$crate" 2>/dev/null; then
    continue
  fi
  raf="$(grep -m1 '^RestrictAddressFamilies=' "$unit" 2>/dev/null || true)"
  # No restriction at all is fine - nothing is being denied.
  [ -z "$raf" ] && continue
  netlink_checked=$((netlink_checked + 1))
  case "$raf" in
    *AF_NETLINK*) ;;
    *)
      netlink_fail=$((netlink_fail + 1))
      echo "MISSING AF_NETLINK: $unit"
      echo "  crate $crate spawns bwrap, but the unit restricts families without AF_NETLINK:"
      echo "  $raf"
      echo "  bwrap will die at loopback setup and the confined payload will never exec."
      ;;
  esac
done < <(find daemons -path '*/dist/*.service' | sort)

if [ "$netlink_fail" -ne 0 ]; then
  echo "FAIL: $netlink_fail sandbox-spawning unit(s) deny AF_NETLINK."
  gate_failed=1
fi

echo "OK: $netlink_checked sandbox-spawning unit(s) allow the netlink socket bwrap needs."

# ---------------------------------------------------------------------------
# Third gate: a daemon the image BUILDS must have its unit packaged.
#
# The drift check above only compares units present in both places, so it is
# blind to the opposite mistake: adding a daemon to the image build and never
# packaging its unit. The binary lands in /usr/lib/arlen/libexec and nothing
# ever starts it - a daemon that is present, looks installed, and is simply
# never running. That reads as a runtime bug, not a packaging one, which is why
# it is worth catching here.
#
# Only daemons the image actually builds are checked. Most canonical units are
# for daemons deliberately outside this image's scope (the install stack, the
# accounts and connection daemons, the settings broker - the image builds no
# apps, so nothing would call them); flagging those would be noise, and the
# absence of a unit for them is the scope boundary, not an omission.
missing_unit=0
built_checked=0

while read -r crate; do
  [ -d "$crate" ] || continue
  # Every SYSTEMD unit the crate ships, whichever convention it uses (dist/ or
  # systemd/). A `org.*.service` is a D-BUS ACTIVATION file, not a unit: it says
  # how to start a daemon on demand, and a daemon whose unit is WantedBy a target
  # is already running without it. Different artifact, different question, so it
  # is not checked here.
  while read -r canonical; do
    base="$(basename "$canonical")"
    case "$base" in
      org.*.service) continue ;;
    esac
    built_checked=$((built_checked + 1))
    # Not the *.target.wants/ enablement symlinks. They are named after the unit
    # and satisfied this check on their own, so a packaged unit could be deleted
    # while its dangling symlink kept the gate green - which is worse than no
    # gate, because the symlink is precisely what makes systemd try to start a
    # unit that is not there. The drift check above already excludes them.
    if ! find dev/mkosi/mkosi.extra -name "$base" -not -path '*.wants/*' | grep -q .; then
      missing_unit=$((missing_unit + 1))
      echo "UNPACKAGED UNIT: $base"
      echo "  the image builds $crate but ships no copy of its unit under mkosi.extra,"
      echo "  so the binary installs and nothing ever starts it."
    fi
  done < <(find "$crate" -name '*.service' -not -path '*/target/*' 2>/dev/null | sort)
done < <({ # Two mechanisms stage a daemon into the image, and reading only the
           # first is a real blind spot: the mkosi build scripts compile most of
           # them in the chroot, while build-image.sh zigbuilds a few on the host
           # and installs them straight into mkosi.extra. event-bus goes the
           # second way, so it was outside this gate entirely. It ships a unit
           # today, but nothing was checking that it did.
           grep -h "manifest-path" dev/mkosi/mkosi.build.d/*.chroot 2>/dev/null
           grep -hoE '^(daemons|ai)/[a-z0-9-]+:' dev/mkosi/build-image.sh 2>/dev/null
         } | grep -oE "(daemons|ai)/[a-z0-9-]+" | sort -u)

if [ "$missing_unit" -ne 0 ]; then
  echo "FAIL: $missing_unit image-built daemon(s) ship no unit."
  gate_failed=1
else
  echo "OK: all $built_checked image-built daemon(s) ship their unit."
fi

if [ "$gate_failed" -ne 0 ]; then
  exit 1
fi
