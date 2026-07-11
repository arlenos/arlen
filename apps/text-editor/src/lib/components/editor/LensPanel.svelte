<script lang="ts">
  /// The KG-lens panel: the file's graph neighbourhood, surfaced automatically. This
  /// is the co-star, not a hidden sidebar - it is why the editor exists. Provenance
  /// (coarse lineage, honest fidelity), Related (inline contextual backlinks you act
  /// on), and Project membership. Read-only context; nothing hand-authored.
  import { lens, openRelated, type ProvenanceStep } from "$lib/stores/lens";
  import { t } from "$lib/i18n/messages";
  import { FileText } from "lucide-svelte";

  // Honest actor phrasing already lives in the data ("a process" at pid fidelity);
  // the origin only tints the small dot, never fabricates specificity.
  function originClass(step: ProvenanceStep): string {
    return step.origin;
  }
</script>

<aside class="lens">
  <section class="sec">
    <h2 class="sec-title">{$t("te.lens.provenance")}</h2>
    <div class="prov">
      {#each $lens.provenance as step (step.relation + step.actor)}
        <div class="prov-step">
          <span class="prov-dot" data-origin={originClass(step)} aria-hidden="true"></span>
          <div class="prov-body">
            <div class="prov-what">{step.relation} <span class="prov-actor">{step.actor}</span></div>
            <div class="prov-when">{step.when}</div>
          </div>
        </div>
      {/each}
    </div>
  </section>

  <section class="sec">
    <h2 class="sec-title">{$t("te.lens.related")}</h2>
    {#if $lens.related.length > 0}
      <div class="rel">
        {#each $lens.related as link (link.ref)}
          <button type="button" class="rel-item" onclick={() => openRelated(link.file)}>
            <span class="rel-file"><FileText size={13} strokeWidth={2} /> {link.file}</span>
            <span class="rel-snippet">{link.snippet}</span>
          </button>
        {/each}
      </div>
    {:else}
      <p class="empty">{$t("te.lens.related.empty")}</p>
    {/if}
  </section>

  {#if $lens.project}
    <section class="sec">
      <h2 class="sec-title">{$t("te.lens.project")}</h2>
      <p class="proj-name">{$t("te.lens.project.partOf", { name: $lens.project.name })}</p>
      <div class="proj-members">
        {#each $lens.project.members as m (m)}
          <button type="button" class="proj-chip" onclick={() => openRelated(m)}>{m}</button>
        {/each}
      </div>
    </section>
  {/if}
</aside>

<style>
  .lens {
    width: 19rem;
    flex-shrink: 0;
    padding: 1.5rem 1.25rem;
    border-inline-start: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    overflow-y: auto;
  }
  .sec {
    margin-bottom: 1.75rem;
  }
  .sec-title {
    margin: 0 0 0.7rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 42%, transparent);
  }

  .prov {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  .prov-step {
    display: flex;
    gap: 0.55rem;
  }
  .prov-dot {
    flex-shrink: 0;
    width: 0.45rem;
    height: 0.45rem;
    margin-top: 0.4rem;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-fg-primary) 35%, transparent);
  }
  .prov-dot[data-origin="agent"],
  .prov-dot[data-origin="model"] {
    background: color-mix(in srgb, var(--color-fg-primary) 65%, transparent);
  }
  .prov-what {
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .prov-actor {
    color: var(--color-fg-primary);
    font-weight: 500;
  }
  .prov-when {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }

  .rel {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .rel-item {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.55rem 0.6rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 9%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .rel-item:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .rel-file {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
  }
  .rel-snippet {
    font-size: var(--text-xs);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .empty {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }

  .proj-name {
    margin: 0 0 0.55rem;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 75%, transparent);
  }
  .proj-members {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }
  .proj-chip {
    padding: 0.15rem 0.45rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    cursor: pointer;
  }
  .proj-chip:hover {
    color: var(--color-fg-primary);
  }
</style>
