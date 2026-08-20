#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open the login screen and try to log in on a machine with no greetd.
#
# WHY THIS EXISTS. The greeter is the first screen anyone sees and the last one
# that had never been opened by this loop. It also has form: its whole German
# catalogue was unreachable for a while because nothing called `initArlenLocale`,
# and that is exactly the class of defect no test and no English render can show.
#
# THE CASE THAT MATTERS IS A LOGIN THAT CANNOT HAPPEN. A login screen that
# swallows the failure is worse than any other app that does, because the person
# in front of it concludes they typed their own password wrong. They try again,
# slower. Then they try their old password. There is no notification area here
# and no second window to explain it in, so the sentence has to be on this
# screen or it does not exist.
#
# The second case is about where the accounts came from. `greeter_profiles` reads
# `/etc/passwd`, so the name on the card must be a real login account on THIS
# machine - checked against `getent passwd` rather than against a string I typed,
# because a hardcoded "tim" would pass on a fixture just as happily.
#
# Run: dev/screenshot/drive-greeter.sh [path-to-arlen-greeter]
#
# Build with `tauri build --no-bundle`, not a plain `cargo build`: the latter
# leaves the binary pointing at devUrl.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-greeter}"
fail=0

[ -x "$app" ] || { echo "no greeter binary at $app"; exit 2; }

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

echo "greeter:"

# What this machine actually offers as a login, by the same rule the backend
# uses (login.defs UID_MIN/UID_MAX, 1000-60000). The check below asks whether the
# name on screen is in THIS set, so it cannot be satisfied by an invented one.
accounts=$(getent passwd | awk -F: '$3 >= 1000 && $3 <= 60000 { print $1 }')

probe=$(mktemp)
cat > "$probe" <<'JS'
// Land on the resting screen first, then attempt a login, so one run covers
// both. The password is deliberately wrong; on a machine with no greetd the
// call cannot get far enough for that to matter, which is the point.
const before = document.body.innerText.replace(/\s+/g, " ").trim();
const fields = [...document.querySelectorAll("input")].map((i) => i.type).join("|");
// The account names THEMSELVES, off the elements that carry them, not scraped
// out of the page text. A whole-output grep for the username would also be
// satisfied by `/home/tim/...` inside some future error string - true, and for
// a reason that has nothing to do with whether a profile was offered.
const names = [...document.querySelectorAll(".name")].map((e) => e.innerText.trim()).filter(Boolean).join(",");
const inp = document.querySelector('input[type=password]');
if (inp) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
  setter.call(inp, "definitely-not-the-password");
  inp.dispatchEvent(new Event("input", { bubbles: true }));
  inp.focus();
  const form = inp.closest("form");
  if (form) form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  else inp.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 13, bubbles: true, cancelable: true }));
  // Poll rather than sleep: the answer arrives when the backend gives up, and a
  // fixed wait either flakes or wastes the difference.
  for (let i = 0; i < 60; i++) {
    await new Promise((r) => setTimeout(r, 100));
    if ([...document.querySelectorAll("[role=alert]")].some((e) => e.innerText.trim())) break;
  }
}
const alerts = [...document.querySelectorAll("[role=alert]")].map((e) => e.innerText.replace(/\s+/g, " ").trim()).filter(Boolean).join(" // ");
return `fields=${JSON.stringify(fields)} names=${JSON.stringify(names)} before=${JSON.stringify(before.slice(0, 200))} alerts=${JSON.stringify(alerts.slice(0, 200))}`;
JS

run="$(mktemp -d)"
out=$(env XDG_STATE_HOME="$run/state" XDG_DATA_HOME="$run/data" XDG_RUNTIME_DIR="$run" HOME="$run" \
  SHOOT_INJECT="$probe" "$here/shoot-app.sh" "$app" "$here/out/greeter.png" 2>&1 \
  | sed -n 's/^inject result: //p')

say "it comes up as a login screen with somewhere to type" \
  "$(printf '%s' "$out" | grep -q "fields=\"password" && echo 1 || echo 0)" "$out"

# The accounts are this machine's, not a sample. Any one of them on screen is
# enough; the list is short and which one leads is the app's business.
found=0
while read -r acct; do
  [ -n "$acct" ] || continue
  printf '%s' "$out" | grep -qE "names=\"[^\"]*\\b${acct}\\b" && found=1
done <<< "$accounts"
say "the account it offers is a real login on this machine" "$found" \
  "none of [$(printf '%s' "$accounts" | tr '\n' ' ')] appeared in: $out"

# THE case. A login screen that cannot reach the login must say so, on itself.
say "a login it cannot perform is refused out loud" \
  "$(printf '%s' "$out" | grep -q "login is not reachable" && echo 1 || echo 0)" "$out"

# And why. "greetd is not running" and "PAM refused you" are the same blank
# screen to a person and completely different things to do about it.
say "and the refusal names its cause" \
  "$(printf '%s' "$out" | grep -qE "alerts=\"[^\"]*\(" && echo 1 || echo 0)" "$out"

rm -rf "$run" "$probe" 2>/dev/null
[ "$fail" = 0 ] && echo "the login screen says when it cannot log you in, and why"
exit "$fail"
