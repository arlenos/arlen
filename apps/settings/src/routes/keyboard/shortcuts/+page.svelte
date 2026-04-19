<script lang="ts">
  import { onMount } from "svelte";
  import { AlertTriangle, ChevronRight, Plus, Search } from "lucide-svelte";

  import { Input } from "$lib/components/ui/input";
  import { Button } from "$lib/components/ui/button";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import KeybindingRow from "$lib/components/settings/KeybindingRow.svelte";
  import KeyCapture from "$lib/components/settings/KeyCapture.svelte";
  import AddCustomBinding from "$lib/components/settings/AddCustomBinding.svelte";
  import { ConfirmDialog } from "$lib/components/ui/confirm-dialog";

  import {
    keybindings,
    entriesByCategory,
    CATEGORIES,
    load,
    setBinding,
    addCustom,
    remove,
    resetOne,
    resetAll,
    resetToBuiltinDefaults,
    resetModuleFragments,
    type KeybindingEntry,
  } from "$lib/stores/keybindings";

  onMount(async () => {
    await load();
    // If the user arrived via `/keyboard/shortcuts#cat-foo`, expand
    // that category and scroll to it after the DOM has the section
    // mounted. Svelte renders the category only if it has entries,
    // hence the `requestAnimationFrame` delay — the load above has
    // just populated the store.
    const hash = typeof window !== "undefined" ? window.location.hash : "";
    const match = hash.match(/^#cat-([\w_]+)$/);
    if (match) {
      const catId = match[1];
      expandedCategories = new Set([...expandedCategories, catId]);
      requestAnimationFrame(() => {
        const el = document.getElementById(`cat-${catId}`);
        if (el) {
          el.scrollIntoView({ behavior: "smooth", block: "start" });
        }
      });
    }
  });

  let query = $state("");
  let editingEntry = $state<KeybindingEntry | null>(null);
  let capturing = $state(false);
  let addOpen = $state(false);
  let lastError = $state<string | null>(null);

  /// Categories that are currently open. Starts with window + workspace
  /// expanded because those are the most common first-click targets.
  /// When the user is searching we auto-expand everything so filtered
  /// results are visible; see effect below.
  let expandedCategories = $state<Set<string>>(
    new Set(["window", "workspace"])
  );

  function toggleCategory(id: string): void {
    const next = new Set(expandedCategories);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    expandedCategories = next;
  }

  // Active search: expand every visible category so filtered entries
  // are discoverable without an extra click. When the search clears we
  // reset to the default set.
  let previousExpanded: Set<string> | null = null;
  $effect(() => {
    const q = query.trim();
    if (q.length > 0 && previousExpanded === null) {
      previousExpanded = expandedCategories;
      expandedCategories = new Set(CATEGORIES.map((c) => c.id));
    } else if (q.length === 0 && previousExpanded !== null) {
      expandedCategories = previousExpanded;
      previousExpanded = null;
    }
  });

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const result: Record<string, KeybindingEntry[]> = {};
    const by = $entriesByCategory;
    for (const cat of CATEGORIES) {
      const entries = by[cat.id] ?? [];
      const pass = q
        ? entries.filter(
            (e) =>
              e.label.toLowerCase().includes(q) ||
              (e.binding?.toLowerCase().includes(q) ?? false) ||
              e.action.toLowerCase().includes(q)
          )
        : entries;
      if (pass.length > 0) result[cat.id] = pass;
    }
    return result;
  });

  const conflictBindings = $derived(
    new Set($keybindings.conflicts.map((c) => c.binding))
  );

  function entryHasConflict(e: KeybindingEntry): boolean {
    return e.binding !== null && conflictBindings.has(e.binding);
  }

  function startRebind(entry: KeybindingEntry): void {
    editingEntry = entry;
    capturing = true;
  }

  // KeyCapture now shows its own live-conflict warning and lets the
  // user "Use anyway" — so when onCapture fires we trust the user has
  // seen the conflict (if any) and deliberately chose to proceed.
  // No extra findConflict() gate here.
  async function onCapture(combo: string): Promise<void> {
    capturing = false;
    if (!editingEntry) return;
    const entry = editingEntry;
    editingEntry = null;
    try {
      lastError = null;
      await setBinding(entry.action, combo);
    } catch (e) {
      lastError = String(e);
    }
  }

  function onCaptureCancel(): void {
    capturing = false;
    editingEntry = null;
  }

  async function onReset(entry: KeybindingEntry): Promise<void> {
    try {
      lastError = null;
      await resetOne(entry.action);
    } catch (e) {
      lastError = String(e);
    }
  }

  async function onRemove(entry: KeybindingEntry): Promise<void> {
    try {
      lastError = null;
      if (entry.category === "custom" && entry.binding) {
        await remove(entry.binding);
      } else {
        await setBinding(entry.action, null);
      }
    } catch (e) {
      lastError = String(e);
    }
  }

  // Same Use-Anyway principle for custom-bindings: KeyCapture inside
  // AddCustomBinding already warned the user; this handler just writes.
  async function onAddCustom(
    binding: string,
    action: string
  ): Promise<void> {
    addOpen = false;
    try {
      lastError = null;
      await addCustom(binding, action);
    } catch (e) {
      lastError = String(e);
    }
  }

  // --- Reset buttons + confirmation dialog --------------------------

  type ResetKind = "all" | "builtin" | "modules";

  const RESET_COPY: Record<
    ResetKind,
    { title: string; message: string; confirmLabel: string }
  > = {
    all: {
      title: "Reset all keybindings?",
      message:
        "Every user-defined binding is replaced with the compositor defaults.",
      confirmLabel: "Reset all",
    },
    builtin: {
      title: "Use compositor defaults?",
      message:
        "The whole [keybindings] section is removed from compositor.toml so the compositor falls back to its built-in defaults. Custom bindings will be lost.",
      confirmLabel: "Use defaults",
    },
    modules: {
      title: "Reset module shortcuts?",
      message:
        "Every installed module's keybinding fragment is deleted. Reinstall the owning module to get its shortcuts back.",
      confirmLabel: "Remove fragments",
    },
  };

  let pendingReset = $state<ResetKind | null>(null);

  async function runReset(kind: ResetKind): Promise<void> {
    try {
      lastError = null;
      if (kind === "all") {
        await resetAll();
      } else if (kind === "builtin") {
        await resetToBuiltinDefaults();
      } else if (kind === "modules") {
        const removed = await resetModuleFragments();
        if (removed === 0) {
          lastError = "No module fragments were present.";
        }
      }
    } catch (e) {
      lastError = String(e);
    } finally {
      pendingReset = null;
    }
  }
</script>

<SettingsPage
  title="Shortcuts"
  description="Bindings for window management, workspaces, apps, and shell actions."
>
  <div class="flex flex-wrap items-center gap-2" data-anchor="search">
    <div class="relative min-w-56 flex-1">
      <Search
        class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
      />
      <Input
        bind:value={query}
        placeholder="Search bindings…"
        class="pl-9"
      />
    </div>
    <Button variant="outline" onclick={() => (addOpen = true)}>
      <Plus class="mr-1 h-4 w-4" /> Add custom
    </Button>
    <Button variant="ghost" onclick={() => (pendingReset = "all")}>
      Reset all
    </Button>
    <Button
      variant="ghost"
      onclick={() => (pendingReset = "builtin")}
      title="Remove the [keybindings] section entirely so compositor defaults apply"
    >
      Use compositor defaults
    </Button>
    <Button
      variant="ghost"
      onclick={() => (pendingReset = "modules")}
      title="Delete every installed module's keybinding fragment"
    >
      Reset module shortcuts
    </Button>
  </div>

  {#if $keybindings.conflicts.length > 0}
    <div
      class="flex items-start gap-2 rounded-[var(--radius-sm)] border border-destructive/40 bg-destructive/10 p-3 text-sm"
    >
      <AlertTriangle class="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
      <div>
        <div class="font-medium text-destructive">
          {$keybindings.conflicts.length} conflict{$keybindings.conflicts
            .length === 1
            ? ""
            : "s"} detected
        </div>
        <div class="mt-0.5 text-muted-foreground">
          {#each $keybindings.conflicts as c (c.binding)}
            <div>
              <span class="font-mono">{c.binding}</span>
              bound to: {c.actions.join(", ")}
            </div>
          {/each}
        </div>
      </div>
    </div>
  {/if}

  {#if lastError}
    <div
      class="rounded-[var(--radius-sm)] border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive"
    >
      {lastError}
    </div>
  {/if}

  <div class="flex flex-col gap-2">
    {#each CATEGORIES as category (category.id)}
      {@const entries = filtered[category.id] ?? []}
      {#if entries.length > 0}
        {@const expanded = expandedCategories.has(category.id)}
        <section
          class="overflow-hidden rounded-[var(--radius)] border border-border bg-card"
          id={`cat-${category.id}`}
        >
          <button
            type="button"
            class="flex w-full items-center gap-2 px-4 py-2.5 text-left text-xs font-semibold uppercase tracking-wider text-muted-foreground transition-colors hover:bg-muted/40"
            onclick={() => toggleCategory(category.id)}
            aria-expanded={expanded}
          >
            <ChevronRight
              class="h-3.5 w-3.5 transition-transform"
              style={expanded ? "transform: rotate(90deg);" : ""}
            />
            <span class="flex-1">{category.label}</span>
            <span class="text-xs font-normal normal-case text-muted-foreground">
              {entries.length}
            </span>
          </button>
          {#if expanded}
            <div class="divide-y divide-border border-t border-border">
              {#each entries as entry (entry.id)}
                <KeybindingRow
                  {entry}
                  hasConflict={entryHasConflict(entry)}
                  onRebind={startRebind}
                  onReset={onReset}
                  onRemove={onRemove}
                />
              {/each}
            </div>
          {/if}
        </section>
      {/if}
    {/each}
  </div>

  {#if $keybindings.loading}
    <div class="text-sm text-muted-foreground">Loading…</div>
  {:else if Object.keys(filtered).length === 0}
    <div class="text-sm text-muted-foreground">
      No bindings match "{query}".
    </div>
  {/if}
</SettingsPage>

<KeyCapture
  open={capturing}
  onCapture={onCapture}
  onCancel={onCaptureCancel}
/>

<AddCustomBinding
  open={addOpen}
  onClose={() => (addOpen = false)}
  onAdd={onAddCustom}
/>

{#if pendingReset}
  {@const copy = RESET_COPY[pendingReset]}
  <ConfirmDialog
    open={true}
    title={copy.title}
    message={copy.message}
    confirmLabel={copy.confirmLabel}
    variant="destructive"
    onConfirm={() => runReset(pendingReset!)}
    onCancel={() => (pendingReset = null)}
  />
{/if}
