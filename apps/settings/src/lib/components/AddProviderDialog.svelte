<script lang="ts">
  /// The add-provider dialog (ai-providers-plan.md §Settings): the escape hatch
  /// for a provider not in the catalogue. Four fields (name, base URL, API key,
  /// models) plus the wire-format selector, a real connection test, and the
  /// resolved endpoint shown verbatim so the base URL is never silently
  /// rewritten. The key never lives here; on save it goes to the TPM-sealed
  /// broker. Backend (add, test, fetch-models) is the coder's; this renders
  /// against it and is mocked until those land.
  import { Dialog } from "@arlen/ui-kit/components/ui/dialog";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { ChipList } from "@arlen/ui-kit/components/ui/chip-list";

  let {
    open,
    onClose,
  }: {
    open: boolean;
    onClose: () => void;
  } = $props();

  type WireFormat = "openai" | "anthropic" | "gemini";
  type TestState =
    | { kind: "idle" }
    | { kind: "testing" }
    | { kind: "ok" }
    | { kind: "http"; status: number }
    | { kind: "network" };

  let name = $state("");
  let baseUrl = $state("");
  let apiKey = $state("");
  let wireFormat = $state<WireFormat>("openai");
  let models = $state<string[]>([]);
  let test = $state<TestState>({ kind: "idle" });

  const WIRE_OPTIONS = [
    { value: "openai", label: "OpenAI" },
    { value: "anthropic", label: "Anthropic" },
    { value: "gemini", label: "Gemini" },
  ];

  // The chat path each wire format appends. Shown resolved so the user sees
  // exactly where requests go; the base URL they typed is never altered.
  const WIRE_PATH: Record<WireFormat, string> = {
    openai: "/chat/completions",
    anthropic: "/messages",
    gemini: "/models/{model}:generateContent",
  };

  const resolved = $derived(
    baseUrl.trim() ? baseUrl.trim().replace(/\/+$/, "") + WIRE_PATH[wireFormat] : "",
  );
  const canSave = $derived(name.trim().length > 0 && baseUrl.trim().length > 0);

  // Mocked until `ai_provider_fetch_models` lands: pretend the endpoint
  // answered with a couple of model ids.
  function fetchModels() {
    models = ["model-large", "model-small"];
  }

  // Mocked until `ai_provider_test` lands. The real command returns
  // ok / an HTTP status / a network failure; this cycles so the states are
  // designable.
  function runTest() {
    test = { kind: "testing" };
    test = { kind: "ok" };
  }

  function testLabel(t: TestState): string {
    switch (t.kind) {
      case "idle":
        return "";
      case "testing":
        return "Testing…";
      case "ok":
        return "Connection works";
      case "network":
        return "Could not reach the server";
      case "http":
        if (t.status === 401) return "Key rejected (401)";
        if (t.status === 403) return "Not allowed (403)";
        if (t.status === 429) return "Rate limited (429)";
        return `Failed (${t.status})`;
    }
  }

  function reset() {
    name = "";
    baseUrl = "";
    apiKey = "";
    wireFormat = "openai";
    models = [];
    test = { kind: "idle" };
  }
  function close() {
    reset();
    onClose();
  }
</script>

<Dialog {open} onClose={close} labelledby="add-provider-title" size="md">
  <div class="ap">
    <h2 id="add-provider-title" class="ap-title">Add provider</h2>

    <div class="field">
      <label class="flabel" for="ap-name">Name</label>
      <Input id="ap-name" bind:value={name} placeholder="My provider" />
    </div>

    <div class="field">
      <label class="flabel" for="ap-url">Base URL</label>
      <Input id="ap-url" bind:value={baseUrl} placeholder="https://api.example.com/v1" />
    </div>

    <div class="field">
      <span class="flabel">Wire format</span>
      <SegmentedControl
        options={WIRE_OPTIONS}
        value={wireFormat}
        ariaLabel="Wire format"
        onchange={(v) => (wireFormat = v as WireFormat)}
      />
    </div>

    <div class="field">
      <label class="flabel" for="ap-key">API key</label>
      <Input id="ap-key" type="password" bind:value={apiKey} placeholder="Stored in the system keystore" />
    </div>

    <div class="field">
      <div class="flabel-row">
        <span class="flabel">Models</span>
        <Button variant="ghost" size="sm" disabled={!baseUrl.trim()} onclick={fetchModels}>
          Fetch from server
        </Button>
      </div>
      <ChipList bind:items={models} placeholder="Add a model id" />
    </div>

    {#if resolved}
      <div class="resolved">
        <span class="rlabel">Requests go to</span>
        <code class="rurl">{resolved}</code>
      </div>
    {/if}

    <div class="ap-foot">
      <div class="test">
        <Button variant="outline" size="sm" disabled={!baseUrl.trim()} onclick={runTest}>
          Test connection
        </Button>
        {#if test.kind !== "idle"}
          <span class="test-result" data-ok={test.kind === "ok"}>
            {#if test.kind === "ok"}<span class="dot ok"></span>{:else if test.kind !== "testing"}<span class="dot err"></span>{/if}
            {testLabel(test)}
          </span>
        {/if}
      </div>
      <div class="actions">
        <Button variant="ghost" onclick={close}>Cancel</Button>
        <Button variant="default" disabled={!canSave} onclick={close}>Save</Button>
      </div>
    </div>
  </div>
</Dialog>

<style>
  .ap {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    padding: 1.5rem;
  }
  .ap-title {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }
  .flabel {
    font-size: 0.75rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .flabel-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .resolved {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.625rem 0.75rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .rlabel {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .rurl {
    font-family: var(--font-mono, monospace);
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
    word-break: break-all;
  }
  .ap-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    margin-top: 0.25rem;
  }
  .test {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    min-width: 0;
  }
  .test-result {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    flex-shrink: 0;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-chip, 4px);
    flex-shrink: 0;
  }
  .dot.ok {
    background: var(--color-success);
  }
  .dot.err {
    background: var(--color-error);
  }
</style>
