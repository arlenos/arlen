<script lang="ts">
  /// Privacy settings page — placeholder until Phase 8.
  ///
  /// The Permission system is fully designed and partially
  /// implemented (token-store, profile-parser, permission-helper
  /// daemon — see `sdk/permissions/` and `installd/permission-helper`).
  /// What's missing is the per-app UI surface that lets the user
  /// see what each app can read/write, change scopes, and revoke
  /// access. That UX lives in this page once first-party apps
  /// register their permission profiles in Phase 8.

  import { invoke } from "@tauri-apps/api/core";
  import { Lock, ExternalLink } from "lucide-svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { Group } from "$lib/components/ui/group";
  import { Row } from "$lib/components/ui/row";
  import { Button } from "$lib/components/ui/button";

  /// xdg-open via the `open_url` Tauri command (Codex Sprint D
  /// review MEDIUM 2 — the prior `window.__TAURI__.opener` path
  /// silently no-oped without the opener plugin installed).
  async function openUrl(url: string) {
    try {
      await invoke("open_url", { url });
    } catch (e) {
      console.warn(`open_url(${url}) failed:`, e);
    }
  }
</script>

<SettingsPage
  title="Privacy"
  description="Per-app permissions and data access controls."
>
  <Group label="Coming with Phase 8">
    <Row
      label="Per-app permissions"
      description="Granular control over Knowledge Graph, Event Bus, filesystem, network, clipboard, camera, microphone, and location access. Arrives with the first-party apps in Phase 8 — by then app-files / app-knowledge / app-terminal will register their permission profiles for review here."
      id="privacy-permissions"
    >
      {#snippet control()}
        <Lock size={16} strokeWidth={1.5} class="opacity-50" />
      {/snippet}
    </Row>

    <Row
      label="Permission system architecture"
      description="The token-store, profile-parser, and permission-helper daemon already exist. What's missing is the per-app UI surface."
      id="privacy-arch-link"
    >
      {#snippet control()}
        <Button
          variant="outline"
          size="sm"
          onclick={() =>
            openUrl(
              "https://github.com/lunaris-sys/docs/blob/main/architecture/permission-system.md",
            )}
        >
          <ExternalLink size={14} />
          Read the spec
        </Button>
      {/snippet}
    </Row>
  </Group>
</SettingsPage>
