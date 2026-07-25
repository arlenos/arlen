<script lang="ts">
  /// The Models hub, kept tiny (model-catalog-and-picker.md, split shape): pure
  /// configuration - which model answers each task and what lives on disk. The
  /// acquisition flow (picks for this machine, search, Hugging Face) is its own
  /// sub-page behind the one "Get a model" entry, so this page never turns into
  /// a catalog wall.
  import { onMount } from "svelte";
  import { HardDrive, Trash2, Upload, CirclePlus, ShieldOff } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { LinkCard } from "@arlen/ui-kit/components/ui/link-card";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { ProviderLogo } from "@arlen/ui-kit/components/ui/provider-logo";
  import { t } from "$lib/i18n/messages";
  import {
    models,
    hardware,
    modelsLoaded,
    modelsMocked,
    roles,
    installedModels,
    availableModels,
    roleMeta,
    modelById,
    loadModels,
    setRole,
    deleteModel,
    importModel,
    type Role,
    type Model,
  } from "$lib/stores/models";

  onMount(loadModels);

  const ROLES: Role[] = ["query", "agent", "title"];
  const roleOptions = $derived($availableModels.map((m) => ({ value: m.id, label: m.name })));

  function installedMeta(m: Model): string {
    const parts: string[] = [];
    if (m.baked) parts.push($t("s.mdl.builtIn"));
    if (m.imported) parts.push($t("s.mdl.imported"));
    if (m.sizeGb != null) parts.push(`${m.sizeGb.toFixed(1)} GB`);
    return parts.join(" · ");
  }
</script>

<Page
  title={$t("s.mdl.title")}
  description={$t("s.mdl.desc")}
>
  <SectionGrid>
    {#if $modelsMocked}
      <!-- Above the hardware line, because the summary below it is an invented
           claim about THIS machine and the list marks models installed that are
           not. Both drive real decisions. -->
      <p class="sample span-full">{$t("s.mdl.sample")}</p>
    {/if}
    {#if $hardware}
      <div class="hw span-full">
        <HardDrive size={15} strokeWidth={1.75} />
        <span>{$hardware.summary}</span>
      </div>
    {/if}

    <Group label={$t("s.mdl.active")} class="span-full">
      {#each ROLES as role (role)}
        {@const rm = roleMeta(role)}
        <Row label={rm.label} description={rm.description} id={`role-${role}`}>
          {#snippet control()}
            <PopoverSelect
              value={$roles[role]}
              options={roleOptions}
              ariaLabel={$t("s.mdl.roleModel", { role: rm.label })}
              width="15rem"
              onchange={(v) => setRole(role, v)}
              renderLabel={modelOption as never}
            />
          {/snippet}
        </Row>
      {/each}
    </Group>

    <div class="span-full">
      <LinkCard href="/ai/models/get" title={$t("s.mdl.get")} description={$t("s.mdl.get.hint")}>
        {#snippet icon()}<CirclePlus size={20} strokeWidth={1.75} />{/snippet}
      </LinkCard>
    </div>

    {#if $installedModels.length > 0}
      <Group label={$t("s.mdl.yourModels")} class="span-full">
        {#each $installedModels as m (m.id)}
          <Row label={m.name} description={installedMeta(m)} id={`installed-${m.id}`}>
            {#snippet control()}
              <span class="row-control">
                {#if m.uncensored}
                  <Badge variant="outline"><ShieldOff strokeWidth={2} />{$t("s.mdl.unc.badge")}</Badge>
                {/if}
                <IconAction
                  label={m.baked ? $t("s.mdl.bakedNoRemove") : $t("s.mdl.delete", { name: m.name })}
                  disabled={m.baked}
                  onclick={() => deleteModel(m.id)}
                >
                  <Trash2 size={15} strokeWidth={1.75} />
                </IconAction>
              </span>
            {/snippet}
          </Row>
        {/each}
        <Button
          variant="ghost"
          class="w-full justify-start gap-2 px-4 font-normal text-muted-foreground hover:text-foreground"
          onclick={() => importModel()}
        >
          <Upload size={15} strokeWidth={1.75} />
          {$t("s.mdl.import")}
        </Button>
      </Group>
    {/if}

    {#if $modelsLoaded && $models.length === 0}
      <Group label={$t("s.mdl.models")} class="span-full">
        <p class="quiet-note">{$t("s.mdl.noneAvailable")}</p>
      </Group>
    {/if}
  </SectionGrid>
</Page>

<!-- The picker label: a local model shows the on-device mark, a cloud model its
     provider logo, then the name; a no-guardrails model carries its badge. Cast
     to `never` (kit vs app resolve `svelte` to distinct Snippet types). -->
{#snippet modelOption(opt: { value: string; label: string })}
  {@const m = modelById($availableModels, opt.value)}
  <span class="opt">
    {#if m?.kind === "cloud"}
      <ProviderLogo id={m.provider} size={18} />
    {:else}
      <HardDrive size={16} strokeWidth={1.75} />
    {/if}
    <span class="opt-label">{opt.label}</span>
    {#if m?.uncensored}
      <Badge variant="outline" class="ms-auto shrink-0"><ShieldOff strokeWidth={2} />{$t("s.mdl.unc.badge")}</Badge>
    {/if}
  </span>
{/snippet}

<style>
  .sample {
    margin: 0;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .hw {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0 0.25rem;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .quiet-note {
    margin: 0;
    padding: 0.5rem 1rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .row-control {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
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
</style>
