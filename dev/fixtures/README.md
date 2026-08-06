<!--
SPDX-FileCopyrightText: 2026 Tim Kicker

SPDX-License-Identifier: AGPL-3.0-only
-->
# Cross-component fixtures

Files that two components must agree about, held once so neither side's tests can
drift without the other noticing.

`sensing-off.toml` is the sensing master switch in its off position. Settings
writes this file and the xdg portal reads it, and they hold separate four-line
copies of the predicate that decides what "off" means - deliberately, rather than
coupling an app to a daemon for one function. What keeps those copies honest is
that both sides' tests use this file: Settings asserts it renders exactly these
bytes, the portal asserts it parses them as off. A change on either side that the
other does not follow turns one of those red.
