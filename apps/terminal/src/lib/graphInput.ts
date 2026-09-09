/// What this window tells the knowledge graph about itself.
///
/// A DIRECTORY, AND DELIBERATELY NOT A COMMAND. This is the app in the tree with
/// the most tempting thing to publish and the strongest reason not to: a shell
/// history is the single most revealing record a desktop keeps, and a command
/// line carries arguments, hostnames, ticket numbers and sometimes a secret
/// somebody typed by mistake. The sensor sees an `execve` and this window sees
/// the whole line; publishing the difference would be the one place where saying
/// more makes the system worse.
///
/// So the subject is the active session's WORKING DIRECTORY, which is the same
/// class of fact every other app here publishes and which the sensor's file
/// events already cover. What is added is that somebody was AT A SHELL there,
/// rather than that a process happened to be running.
///
/// If a command-level record is ever wanted, it is a consent question and a
/// design decision - not something to arrive by widening this file.
///
/// NO TIMELINE RECORD. A command finishing is a moment, and it is exactly the
/// moment whose subject would have to be the command; the same reasoning that
/// keeps the command out of presence keeps this app off that surface entirely.

import { presence, type PresenceParams } from "@arlen/tauri-plugin-shell";

/// The presence for an open session, or null when there is none.
export function shellPresence(cwd: string | null, status: string): PresenceParams | null {
  if (!cwd) return null;
  return {
    activity: "shell",
    subject: cwd,
    // Whether the session is still alive. A terminal tab that exited an hour ago
    // is a different fact from one somebody is typing in, and the window is the
    // only thing that knows which of the two it is showing.
    ...(status ? { metadata: { status } } : {}),
    auto_clear: "on-blur",
  };
}

/// Publish the presence for the active session, or take the last one down.
///
/// Best-effort, like every publish in this tree: under vite there is no relay
/// and the call rejects, and a terminal that cannot reach the bus is still a
/// terminal.
export async function publishPresence(cwd: string | null, status: string): Promise<void> {
  const params = shellPresence(cwd, status);
  try {
    if (params) await presence.set(params);
    else await presence.clear();
  } catch {
    // No relay: the graph hears nothing, and nothing here depends on it.
  }
}
