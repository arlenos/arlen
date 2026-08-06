#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Prove the accessible-name scan still answers both ways.
#
# The scan replaced a grep that produced 25 findings of which 25 were false, and
# what makes it right is that it asks the ENGINE rather than the markup. That is
# also what makes it unverifiable by reading: whether a `columnheader` takes a
# name from its contents is a WebKit fact, not an ARIA one. So the two pages
# beside this script are kept as inputs and driven through the real engine.
#
# Unlike the other checks' fixtures this cannot run in CI: it needs
# WebKitWebDriver and a Wayland compositor, and the render harness is a host
# dev-shell tool. Run it when touching the scan.
#
#   dev/screenshot/ax-fixtures/run.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
scan="$here/../ax-names.py"
port=4494

if ! curl -s -m 3 "http://127.0.0.1:$port/status" >/dev/null 2>&1; then
  echo "starting the headless WebKit driver on $port"
  setsid "$here/../webkit-headless.sh" "$port" >/tmp/arlen-ax-fixtures.log 2>&1 </dev/null &
  for _ in $(seq 20); do
    sleep 1
    curl -s -m 3 "http://127.0.0.1:$port/status" >/dev/null 2>&1 && break
  done
fi

fail=0

# The named page must come back clean. Without this half, a scan that reported
# every focusable element would pass the interesting case and still be useless.
if out=$(python3 "$scan" "file://$here/" named.html 2>&1); then
  echo "  ok   a page whose controls are all named reports nothing"
else
  echo "  FAIL named.html reported something:"
  echo "$out" | sed 's/^/      /'
  fail=1
fi

# The unnamed page must report the column header - and ONLY it. The bare button
# takes its name from its own text, so a scan reporting two has stopped modelling
# the engine and started matching markup.
out=$(python3 "$scan" "file://$here/" unnamed.html 2>&1 || true)
if grep -q "1 with no accessible name" <<<"$out" && grep -q "role=columnheader" <<<"$out"; then
  echo "  ok   an unnamed column header is reported, and the named button is not"
else
  echo "  FAIL unnamed.html did not report exactly the column header:"
  echo "$out" | sed 's/^/      /'
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo
  echo "the accessible-name scan no longer answers both ways"
  exit 1
fi
echo
echo "the accessible-name scan still answers both ways"
