/// Types and IPC helpers for the greeter (greeter-onboarding-plan.md §2).
/// The greeter runs before any session and talks to nothing but greetd;
/// these commands are the seam the coder wires to greetd_ipc + the shared
/// `daemons/lock-auth` backend. The frontend renders against them and the
/// screenshot mock fills them.
///
/// Copy law (shared with the rest of the system): no em-dashes, no middot
/// separators, plain language a real person understands.
import { invoke } from "@tauri-apps/api/core";

/// What a profile is. Profiles are separate systemd-homed users
/// (profile-system-plan.md); the Guest profile is tmpfs, deleted on logout.
export interface Profile {
  /// The homed user id, opaque to the UI.
  id: string;
  /// The display name.
  name: string;
  /// An avatar image URL, or null for the initials fallback.
  avatar_url: string | null;
  /// Which kind of profile, for the small caption and ordering.
  kind: "standard" | "created" | "guest";
  /// The profile used last, pre-selected on a multi-user box.
  last_used: boolean;
  /// The strong factors enrolled for this profile beyond the password
  /// (FIDO2 / TPM2). Drives whether the hardware-key affordance shows.
  factors: Factor[];
}

/// A session the greeter can launch. Normally just the Arlen session; the
/// affordance exists for a fallback session.
export interface Session {
  id: string;
  name: string;
}

/// The strong factors that may release a systemd-homed/LUKS key at the
/// cold login (lockscreen-plan.md Decided §2/§5). Password is always one;
/// FIDO2 / TPM2 are the hardware factors. Phone-proximity and face are
/// convenience re-unlock of the lock screen only, never offered here.
export type Factor = "password" | "fido2" | "tpm2";

/// The result of an authentication attempt. `ok` releases the key and
/// launches the session; `error` is a lay-readable sentence for the shake.
export interface AuthResult {
  ok: boolean;
  error?: string;
}

/// A power action available from the greeter.
export type PowerAction = "suspend" | "reboot" | "power-off";

/// List the profiles greetd offers. `null` when the read fails, which the
/// surface renders as an honest "login is not reachable" state, never as
/// an empty profile list.
export async function listProfiles(): Promise<Profile[] | null> {
  try {
    return await invoke<Profile[]>("greeter_profiles");
  } catch {
    return null;
  }
}

/// List the launchable sessions. `null` on failure; the surface then
/// assumes the default session and hides the picker.
export async function listSessions(): Promise<Session[] | null> {
  try {
    return await invoke<Session[]>("greeter_sessions");
  } catch {
    return null;
  }
}

/// Attempt a password login for a profile. The backend runs PAM and, on
/// success, releases the homed key and starts the session.
export async function authenticate(profileId: string, secret: string): Promise<AuthResult> {
  try {
    return await invoke<AuthResult>("greeter_authenticate", { profileId, secret });
  } catch (e) {
    return { ok: false, error: String(e) };
  }
}

/// Begin a hardware-factor login (FIDO2 / TPM2). Resolves when the factor
/// completes or fails; the UI shows the waiting prompt meanwhile.
export async function beginFactor(profileId: string, factor: Factor): Promise<AuthResult> {
  try {
    return await invoke<AuthResult>("greeter_factor_begin", { profileId, factor });
  } catch (e) {
    return { ok: false, error: String(e) };
  }
}

/// The wallpaper URL to show behind the login, or `null` for the calm
/// gradient fallback. The coder wires the real source (the wallpaper
/// manifest, or a greeter default under /usr/share/arlen); the surface
/// always has a safe fallback.
export async function readWallpaper(): Promise<string | null> {
  try {
    return await invoke<string | null>("greeter_wallpaper");
  } catch {
    return null;
  }
}

/// Request a power action. Best-effort: a failure leaves the greeter up.
export async function power(action: PowerAction): Promise<void> {
  try {
    await invoke("greeter_power", { action });
  } catch {
    // Nothing to surface on the login screen; the machine stays up.
  }
}
