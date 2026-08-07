// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// A GUI app: no console window on Windows in release. Kept for parity with the
// other apps even though Arlen targets Linux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    arlen_knowledge_lib::run()
}
