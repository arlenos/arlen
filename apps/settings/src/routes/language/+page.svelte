<script lang="ts">
  /// The UI language.
  ///
  /// Its own page rather than a row on Appearance: language is not how the
  /// desktop looks. The catalogs, the fallback chain and the reactive `locale`
  /// store have all been in place for a while; what was missing was any way for
  /// a person to choose, so every translation was unreachable.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { t, locale, CATALOGS, SOURCE_LOCALE } from "$lib/i18n/messages";

  /// The languages there are catalogs for. Derived, not listed: a catalog that
  /// ships is a language that can be chosen, and a list beside it would be a
  /// second thing to keep in step.
  /// A language's name in itself, which is what someone scanning a list looks
  /// for - "Deutsch", not "German". `Intl.DisplayNames` asked in that language
  /// gives exactly that, so there is no hand-written table of endonyms to keep.
  function endonym(tag: string): string {
    try {
      return new Intl.DisplayNames([tag], { type: "language" }).of(tag) ?? tag;
    } catch {
      return tag;
    }
  }

  const options = Object.keys(CATALOGS).map((tag) => ({ value: tag, label: endonym(tag) }));

  let chosen = $state(SOURCE_LOCALE);

  onMount(async () => {
    // The layout already adopted the choice at startup; this page only needs to
    // show which one is current.
    try {
      const ui = await invoke<string>("config_get", { file: "locale", key: "locale.ui" });
      if (typeof ui === "string" && ui) chosen = ui;
    } catch {
      // No file yet: the default stands, and choosing writes one.
    }
  });

  async function choose(tag: string): Promise<void> {
    chosen = tag;
    // Set first, write second. The UI answers immediately and a failed write
    // leaves the choice visible rather than silently reverting to the old file.
    locale.set(tag);
    try {
      await invoke("config_set", { file: "locale", key: "locale.ui", value: tag });
    } catch (e) {
      console.error("[language] could not save:", e);
    }
  }
</script>

<Page title={$t("s.lang.title")} description={$t("s.lang.desc")}>
  <SectionGrid>
    <Section label={$t("s.lang.section")}>
      <Row label={$t("s.lang.ui")} description={$t("s.lang.uiDesc")} id="language-ui">
        {#snippet control()}
          <PopoverSelect
            value={chosen}
            {options}
            ariaLabel={$t("s.lang.ui")}
            onchange={choose}
          />
        {/snippet}
      </Row>
    </Section>
  </SectionGrid>
</Page>
