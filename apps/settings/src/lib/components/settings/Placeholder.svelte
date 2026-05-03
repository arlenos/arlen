<script lang="ts">
  import SettingsPage from "./SettingsPage.svelte";

  let {
    title,
    description = "This panel is not yet implemented.",
  }: { title: string; description?: string } = $props();
</script>

<SettingsPage {title} {description}>
  <div
    class="flex h-64 items-center justify-center rounded-card border border-dashed border-border text-sm text-muted-foreground"
  >
    Coming soon
  </div>
</SettingsPage>
