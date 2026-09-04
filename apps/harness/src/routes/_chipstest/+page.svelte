<script lang="ts">
  /// Dev-only preview of the composer's context chips (like _difftest for the
  /// gate): two attached files whose names collide, so the path reveal on
  /// hover, the one identity tooltip design-system.md 6.6 allows, can be seen
  /// without a host to attach a file through.
  import ContextChips from "$lib/components/chat/ContextChips.svelte";
  import type { MentionContent } from "$lib/stores/conversation";

  let attached = $state<MentionContent[]>([
    { path: "/home/tim/Documents/thesis/report.pdf", name: "report.pdf", content: "", truncated: false },
    { path: "/home/tim/Downloads/report.pdf", name: "report.pdf", content: "", truncated: true },
  ]);

  function remove(path: string) {
    attached = attached.filter((m) => m.path !== path);
  }
</script>

<div class="harness">
  <section>
    <h2>Two files with one name: the path tells them apart on hover</h2>
    <div class="composer">
      <ContextChips {attached} onremove={remove} />
    </div>
  </section>
</div>

<style>
  .harness {
    padding: 2rem;
    max-width: 40rem;
  }
  h2 {
    font-size: var(--text-sm);
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .composer {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    padding-bottom: 0.75rem;
  }
</style>
