<script lang="ts">
  /// Agent dashboard (ai-app.md §2.2) — the pull / observability
  /// surface: a read-only activity timeline (trigger → behaviour →
  /// predict → gate → act → audit), behaviour status, per-item Undo
  /// (the compensation trigger), and anomaly notices.
  ///
  /// A1 is the skeleton on the `Page` canon with honest empty states.
  /// The activity timeline reads the audit ledger via the shared read
  /// command (Settings S-U4, already built) and lands here in A4; Undo
  /// (A5) and behaviour status (A6) follow.
  import { Page } from "@lunaris/ui-kit/components/ui/page";
  import { SectionGrid } from "@lunaris/ui-kit/components/ui/section-grid";
  import { Group } from "@lunaris/ui-kit/components/ui/group";
  import { Activity, History, Bell } from "@lucide/svelte";
</script>

<Page
  title="Agent"
  description="What the assistant has done on your behalf. Read-only, from the tamper-evident audit ledger — review each curated action and undo it if you want."
>
  <SectionGrid>
    <Group label="Activity">
      <div class="placeholder">
        <History size={20} strokeWidth={1.5} />
        <p>The activity timeline (trigger → gate → act → audit, newest first)
          renders here in build step A4, reading the audit ledger through the
          shared read command.</p>
      </div>
    </Group>

    <Group label="Behaviours">
      <div class="placeholder">
        <Activity size={20} strokeWidth={1.5} />
        <p>Live behaviour status (enabled, kind, last runs, crash/retry)
          lands in A6. Enabling and disabling stays in Settings → AI.</p>
      </div>
    </Group>

    <Group label="Notices">
      <div class="placeholder">
        <Bell size={20} strokeWidth={1.5} />
        <p>Rare, important warnings from the Anomaly Detector surface here —
          the agent itself never pushes.</p>
      </div>
    </Group>
  </SectionGrid>
</Page>

<style>
  .placeholder {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: 0.85rem;
    line-height: 1.5;
  }
  .placeholder p {
    margin: 0;
  }
</style>
