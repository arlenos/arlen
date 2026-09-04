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
# LC_ALL=C so the sentences asserted below are the English ones by decision
# rather than by whatever this machine is set to. `locale_get` skips C/POSIX and
# answers "en", which is the catalogue those greps are written against; without
# this the suite would go red on a German laptop for the right screen.
out=$(env LC_ALL=C XDG_STATE_HOME="$run/state" XDG_DATA_HOME="$run/data" XDG_RUNTIME_DIR="$run" HOME="$run" \
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
  "$(printf '%s' "$out" | grep -qE "alerts=\"[^\"]+\"" && echo 1 || echo 0)" "$out"

# And why. "greetd is not running" and "PAM refused you" are the same blank
# screen to a person and completely different things to do about it.
#
# THE CAUSE IS THE SENTENCE, not a parenthetical after it. This asked for a "("
# until 5 September, and that was asserting the presence of a defect: the old
# screen carried the command's OWN English prose - "login is not reachable
# (account list unavailable)" - onto the first screen of the system in every
# locale, and the parenthesis it matched was part of that leak. `55de08410`
# replaced it with a token the panel turns into one of five translated
# sentences, one per cause, so the check is now which sentence came back. With
# no greetd running that is the login-service one and not the generic
# not-connected one.
say "and the refusal names its cause" \
  "$(printf '%s' "$out" | grep -q "login service is not reachable" && echo 1 || echo 0)" "$out"

# The regression the above was written against, checked directly rather than
# implied: the backend's rejection token and its raw error text are for the
# panel, never for the person. Anything on screen that the catalogue does not
# define is the mapping being bypassed.
#
# THE CATALOGUE IS READ, NOT RESTATED. A list of forbidden words here would only
# ever catch the leaks somebody thought of - my first cut listed the five tokens
# and "Error", and the actual historical leak ("login is not reachable (account
# list unavailable)") walked straight through it, which is how I learnt that the
# check has to be membership rather than a blocklist. Asking whether the sentence
# is one the app SHIPS catches every shape of leak including the one that
# happened, and it follows a reworded sentence instead of going red at it.
known=$(grep -oE '"g\.[A-Za-z.]+": "[^"]+"' "$root/apps/greeter/src/lib/i18n/messages.ts" \
  | sed 's/^[^:]*: "//; s/"$//' | sort -u)
alert=$(printf '%s' "$out" | sed -n 's/.*alerts="\([^"]*\)".*/\1/p')
in_catalogue=0
[ -n "$alert" ] && while IFS= read -r line; do
  [ "$line" = "$alert" ] && in_catalogue=1 && break
done <<< "$known"
say "and it is a sentence the app ships, not the backend's own words" \
  "$in_catalogue" "on screen: [$alert] - not one of the $(printf '%s' "$known" | wc -l) catalogue strings"

# THE OTHER CORNER. A login screen's accessibility menu is not a nicety: a person
# who cannot read this contrast or this type size has no session to fix it from,
# which is why the greeter owns these toggles itself rather than borrowing the
# session's (greeter-onboarding-plan.md 2). Nothing had ever pressed them. The
# menu, four switches and the code that paints them were all built and none of it
# had been driven, so "it applies now" was a claim from the source.
#
# WHAT IS MEASURED IS THE PAINT, NOT THE FLAG. A check that reads
# `dataset.contrast` would go green on a page whose stylesheet never loaded - the
# attribute is set by the same line either way. So the probe reads the RESOLVED
# custom properties off the root before and after: a border that is white and a
# scale that is 1.25 mean the sheet reached the screen.
a11y=$(mktemp)
cat > "$a11y" <<'JS'
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const root = document.documentElement;
const read = () => {
  const cs = getComputedStyle(root);
  return {
    contrast: root.dataset.contrast || "",
    scale: cs.getPropertyValue("--greeter-scale").trim(),
    border: cs.getPropertyValue("--color-border").trim(),
  };
};
await wait(1500);
const before = read();
const trigger = document.querySelector("#greeter-a11y");
if (!trigger) return JSON.stringify({ opened: false, before });
trigger.click();
await wait(500);
const labelled = () => [...document.querySelectorAll("[aria-label]")];
const offered = labelled().map((e) => e.getAttribute("aria-label"));
const pick = (name) => labelled().find((e) => e.getAttribute("aria-label") === name);
const hc = pick("High contrast");
const lt = pick("Larger text");
if (!hc || !lt) return JSON.stringify({ opened: true, found: false, offered, before });
hc.click();
await wait(400);
const afterContrast = read();
lt.click();
await wait(400);
const afterBoth = read();
return JSON.stringify({ opened: true, found: true, offered, before, afterContrast, afterBoth });
JS

acc=$(SHOOT_INJECT="$a11y" SHOOT_INJECT_SETTLE=2 \
  "$here/shoot-app.sh" "$app" "$here/out/greeter-a11y.png" 2>&1 | sed -n 's/^inject result: //p')

say "the accessibility corner opens, and offers the four options" \
  "$(printf '%s' "$acc" | grep -q '"found":true' \
     && printf '%s' "$acc" | grep -q "On-screen keyboard" \
     && printf '%s' "$acc" | grep -q "Screen reader" && echo 1 || echo 0)" "$acc"

# High contrast against its own before-reading, so a page that was already white
# on black cannot pass it by standing still.
say "high contrast repaints the screen rather than only setting a flag" \
  "$(printf '%s' "$acc" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print(0); raise SystemExit
a, b = d.get("before", {}), d.get("afterContrast", {})
# A custom property comes back as the stylesheet wrote it - "#fff" here, not an
# rgb() triple - so whiteness is decided after normalising, not by looking for
# "255" in the string. The first cut did the latter and went red against a screen
# that had repainted correctly.
def white(v):
    v = v.strip().lower().lstrip("#")
    if len(v) == 3:
        v = "".join(c * 2 for c in v)
    return v == "ffffff" or v.replace(" ", "") in ("rgb(255,255,255)", "rgba(255,255,255,1)")
ok = b.get("contrast") == "high" and b.get("border") != a.get("border") and white(b.get("border", ""))
print(1 if ok else 0)
')" "$acc"

say "and larger text scales the screen it is on" \
  "$(printf '%s' "$acc" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print(0); raise SystemExit
a, b = d.get("before", {}), d.get("afterBoth", {})
print(1 if b.get("scale") == "1.25" and a.get("scale") != b.get("scale") else 0)
')" "$acc"

rm -rf "$run" "$probe" "$a11y" 2>/dev/null
[ "$fail" = 0 ] && echo "the login screen says when it cannot log you in, and why, and its accessibility corner works"
exit "$fail"
