<script lang="ts">
  /// The greeter surface: a fullscreen, centered login over the wallpaper.
  /// The clock sits at the top, the auth zone in the middle (the profile
  /// chooser on a multi-user box, then the focused profile's password and
  /// hardware-key affordance), and the quiet controls on the bottom bar
  /// (accessibility left, session center, power right). This route owns the
  /// reads and the auth calls; rendering lives in $lib/components.
  import { onMount } from "svelte";
  import {
    listProfiles,
    listSessions,
    readWallpaper,
    authenticate,
    beginFactor,
    power,
    type Profile,
    type Session,
    type AuthResult,
  } from "$lib/greeter";
  import { a11y } from "$lib/a11y";
  import GreeterBackground from "$lib/components/GreeterBackground.svelte";
  import Clock from "$lib/components/Clock.svelte";
  import ProfileRow from "$lib/components/ProfileRow.svelte";
  import AuthPanel from "$lib/components/AuthPanel.svelte";
  import SessionSelect from "$lib/components/SessionSelect.svelte";
  import PowerMenu from "$lib/components/PowerMenu.svelte";
  import A11yMenu from "$lib/components/A11yMenu.svelte";
  import OnScreenKeyboard from "$lib/components/OnScreenKeyboard.svelte";

  let profiles = $state<Profile[] | null>(null);
  let sessions = $state<Session[]>([]);
  let loaded = $state(false);

  // The wallpaper URL; null until the coder wires the wallpaper source, so
  // the calm gradient fallback shows. The mock may pass one.
  let wallpaper = $state<string | null>(null);

  // `highlight` is the row's keyboard/visual highlight; `picked` is the
  // committed profile that reveals the password. A single-user box commits
  // immediately and never shows the row.
  let highlight = $state<string | null>(null);
  let picked = $state<string | null>(null);
  let sessionId = $state("");

  const multi = $derived((profiles?.length ?? 0) > 1);
  const pickedProfile = $derived(profiles?.find((p) => p.id === picked) ?? null);

  onMount(async () => {
    const [ps, ss, wp] = await Promise.all([listProfiles(), listSessions(), readWallpaper()]);
    profiles = ps;
    sessions = ss ?? [];
    wallpaper = wp;
    sessionId = sessions.find(() => true)?.id ?? "";
    if (ps && ps.length > 0) {
      const last = ps.find((p) => p.last_used) ?? ps[0];
      highlight = last.id;
      // Single user: skip the chooser, go straight to the password.
      if (ps.length === 1) picked = ps[0].id;
    }
    loaded = true;
  });

  function hasHardware(p: Profile): boolean {
    return p.factors.some((f) => f === "fido2" || f === "tpm2");
  }

  async function onsubmit(secret: string): Promise<AuthResult> {
    if (!pickedProfile) return { ok: false, error: "No profile selected." };
    return authenticate(pickedProfile.id, secret);
  }
  async function onfactor(): Promise<AuthResult> {
    if (!pickedProfile) return { ok: false, error: "No profile selected." };
    return beginFactor(pickedProfile.id, "fido2");
  }
</script>

<GreeterBackground image={wallpaper} highContrast={$a11y.highContrast} />

<main class="greeter" class:hc={$a11y.highContrast}>
  <div class="top">
    <Clock />
  </div>

  <div class="center">
    {#if !loaded}
      <p class="state">Starting</p>
    {:else if profiles === null}
      <p class="state">Login is not reachable right now.</p>
    {:else if profiles.length === 0}
      <p class="state">No profiles are set up on this device.</p>
    {:else if picked && pickedProfile}
      <AuthPanel
        profile={pickedProfile}
        hasHardwareFactor={hasHardware(pickedProfile)}
        canSwitch={multi}
        {onsubmit}
        {onfactor}
        onswitch={() => (picked = null)}
      />
    {:else}
      <ProfileRow
        profiles={profiles}
        selectedId={highlight}
        onselect={(id) => {
          highlight = id;
          picked = id;
        }}
      />
    {/if}
  </div>

  {#if loaded && $a11y.onScreenKeyboard && picked}
    <div class="osk-zone">
      <OnScreenKeyboard />
    </div>
  {/if}

  <div class="bar">
    <div class="bar-side left"><A11yMenu /></div>
    <div class="bar-center">
      <!-- Only a real choice shows: one session (the normal case) needs no
           chooser, so the bottom stays quiet. -->
      {#if loaded && sessions.length > 1}
        <SessionSelect {sessions} value={sessionId} onchange={(id) => (sessionId = id)} />
      {/if}
    </div>
    <div class="bar-side right"><PowerMenu onaction={(a) => power(a)} /></div>
  </div>
</main>

<style>
  .greeter {
    position: relative;
    z-index: 1;
    display: grid;
    grid-template-rows: 1fr auto 1fr;
    width: 100%;
    height: 100%;
    padding: 2.5rem;
  }
  /* Top zone: the clock sits high (near the upper fifth, the macOS login
     proportion), so there is generous air between it and the centered
     auth panel below. */
  .top {
    display: flex;
    align-items: center;
    justify-content: center;
  }
  /* Center zone: the auth, dead center. */
  .center {
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .state {
    font-size: calc(0.9375rem * var(--greeter-scale, 1));
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  /* The on-screen keyboard floats above the bottom bar without pushing the
     centered auth off-center. */
  .osk-zone {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 6rem;
    display: flex;
    justify-content: center;
  }
  /* Bottom bar: three zones, controls in the corners, session centered. */
  .bar {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: end;
  }
  .bar-side {
    display: flex;
    align-items: center;
  }
  .bar-side.right {
    justify-content: flex-end;
  }
  .bar-center {
    display: flex;
    align-items: center;
    justify-content: center;
    padding-bottom: 0.5rem;
  }
</style>
