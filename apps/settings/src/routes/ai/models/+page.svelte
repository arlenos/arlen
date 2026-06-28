<script lang="ts">
  /// Default models (ai-providers-plan.md §Settings): which model answers each
  /// kind of task, kept separate from the provider list so one dropdown is not
  /// overloaded. Holds the default plus a ranked fallback (used when a choice
  /// is unavailable). The resolved endpoint is shown verbatim, never silently
  /// rewritten. The catalogue + the resolved endpoint come from the backend;
  /// this renders against it and is mocked until those land.
  import { ArrowUp, ArrowDown } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { ProviderLogo } from "@arlen/ui-kit/components/ui/provider-logo";

  interface ModelChoice {
    id: string;
    label: string;
    /// The provider endpoint this model resolves to, shown verbatim.
    endpoint: string;
  }

  // Illustrative: the enabled providers' models. Real list comes from the
  // catalogue command.
  const MODELS: ModelChoice[] = [
    { id: "ollama/llama3:8b", label: "Ollama · llama3:8b", endpoint: "http://localhost:11434" },
    { id: "mistral/mistral-large", label: "Mistral · mistral-large", endpoint: "https://api.mistral.ai/v1" },
    { id: "anthropic/claude-3.5-sonnet", label: "Anthropic · claude-3.5-sonnet", endpoint: "https://api.anthropic.com/v1" },
  ];
  const OPTIONS = MODELS.map((m) => ({ value: m.id, label: m.label }));

  let queryModel = $state("anthropic/claude-3.5-sonnet");
  let agentModel = $state("ollama/llama3:8b");
  let titleModel = $state("ollama/llama3:8b");

  // The fallback order: if the chosen model is unavailable, the next usable one
  // in this list answers. Reordered with the move controls.
  let fallback = $state<string[]>([
    "anthropic/claude-3.5-sonnet",
    "ollama/llama3:8b",
    "mistral/mistral-large",
  ]);

  // The provider segment of a model id ("anthropic/claude-3.5" -> "anthropic"),
  // used to look up the provider's brand mark.
  function providerOf(id: string): string {
    return id.split("/")[0] ?? id;
  }
  function labelFor(id: string): string {
    return MODELS.find((m) => m.id === id)?.label ?? id;
  }
  function endpointFor(id: string): string {
    return MODELS.find((m) => m.id === id)?.endpoint ?? "";
  }

  function move(i: number, dir: -1 | 1) {
    const j = i + dir;
    if (j < 0 || j >= fallback.length) return;
    const next = [...fallback];
    [next[i], next[j]] = [next[j], next[i]];
    fallback = next;
  }
</script>

<Page
  title="Default models"
  description="Which model answers each kind of task. If a choice is unavailable, the next model in the fallback order is used."
>
  <SectionGrid>
    <Group label="Models">
      <Row label="Query model" description="Answers your questions in chat." id="model-query">
        {#snippet control()}
          <PopoverSelect
            value={queryModel}
            options={OPTIONS}
            ariaLabel="Query model"
            width="16rem"
            onchange={(v) => (queryModel = v)}
            renderLabel={optionLabel as never}
          />
        {/snippet}
      </Row>
      <Row label="Agent model" description="Runs the background tasks you have turned on." id="model-agent">
        {#snippet control()}
          <PopoverSelect
            value={agentModel}
            options={OPTIONS}
            ariaLabel="Agent model"
            width="16rem"
            onchange={(v) => (agentModel = v)}
            renderLabel={optionLabel as never}
          />
        {/snippet}
      </Row>
      <Row label="Title model" description="Names new chats. A small local model is plenty." id="model-title">
        {#snippet control()}
          <PopoverSelect
            value={titleModel}
            options={OPTIONS}
            ariaLabel="Title model"
            width="16rem"
            onchange={(v) => (titleModel = v)}
            renderLabel={optionLabel as never}
          />
        {/snippet}
      </Row>
    </Group>

    <Group label="Fallback order">
      {#each fallback as id, i (id)}
        <Row label={labelFor(id)} description={endpointFor(id)}>
          {#snippet leading()}
            <span class="frank">{i + 1}</span>
            <ProviderLogo id={providerOf(id)} size={20} />
          {/snippet}
          {#snippet control()}
            <span class="fmove">
              <IconAction label="Move up" disabled={i === 0} onclick={() => move(i, -1)}>
                <ArrowUp size={14} strokeWidth={2} />
              </IconAction>
              <IconAction
                label="Move down"
                disabled={i === fallback.length - 1}
                onclick={() => move(i, 1)}
              >
                <ArrowDown size={14} strokeWidth={2} />
              </IconAction>
            </span>
          {/snippet}
        </Row>
      {/each}
    </Group>
  </SectionGrid>
</Page>

<!-- The select's label, with the provider logo tile, shared by the trigger and
     the option list (the same placeholder the in-chat picker uses). Passed to
     PopoverSelect's `renderLabel` cast to `never`: the kit and this app resolve
     `svelte` to different copies, so their `Snippet` types are nominally
     distinct though identical at runtime. -->
{#snippet optionLabel(opt: { value: string; label: string }, _selected: boolean)}
  <span class="opt">
    <ProviderLogo id={providerOf(opt.value)} size={18} />
    <span class="opt-label">{opt.label}</span>
  </span>
{/snippet}

<style>
  /* The rank number in the row's leading slot, beside the provider logo. */
  .frank {
    flex-shrink: 0;
    width: 1.25rem;
    text-align: center;
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  /* The select label lays the logo beside the provider-and-model text. */
  .opt {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }
  .opt-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fmove {
    display: inline-flex;
    gap: 0.25rem;
  }
</style>
