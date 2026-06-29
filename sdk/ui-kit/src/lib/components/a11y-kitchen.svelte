<script lang="ts">
  /// A11y test fixture: every primitive the A11Y-R1 gate covers, composed the
  /// way a consumer should (labels supplied, realistic props). The gate renders
  /// this once and runs axe over the whole tree, so a regression in any covered
  /// primitive trips the build. NOT shipped - imported only by `a11y.test.ts`.
  import { Button } from "$lib/components/ui/button";
  import { Badge } from "$lib/components/ui/badge";
  import { Input } from "$lib/components/ui/input";
  import { NumberInput } from "$lib/components/ui/number-input";
  import { TimeInput } from "$lib/components/ui/time-input";
  import { ValueSlider } from "$lib/components/ui/value-slider";
  import { FillSlider } from "$lib/components/ui/fill-slider";
  import { SegmentedControl } from "$lib/components/ui/segmented-control";
  import { ChoiceList } from "$lib/components/ui/choice-list";
  import { ChipList } from "$lib/components/ui/chip-list";
  import { Toolbar } from "$lib/components/ui/toolbar";
  import { ColorPicker } from "$lib/components/ui/color-picker";
  import { Switch } from "$lib/components/ui/switch";
  import { Toggle } from "$lib/components/ui/toggle";
  import { PopoverSelect } from "$lib/components/ui/popover-select";

  let chips = $state(["alpha", "beta"]);
  let toggled = $state(false);
  let switched = $state(false);
  let view = $state("list");
  let density = $state("cozy");
  let format = $state("png");
  let volume = $state(40);
  const noop = () => {};
</script>

<main>
  <h1>Kit a11y fixture</h1>

  <section aria-label="Buttons and badges">
    <Button variant="default">Save</Button>
    <Button variant="outline">Cancel</Button>
    <Badge>New</Badge>
    <Toggle aria-label="Bold">B</Toggle>
  </section>

  <section aria-label="Text fields">
    <label for="a11y-name">Name</label>
    <Input id="a11y-name" placeholder="Your name" value="" />
    <NumberInput value={3} min={0} max={10} ariaLabel="Quantity" onchange={noop} />
    <TimeInput value="08:30" ariaLabel="Reminder time" onchange={noop} />
  </section>

  <section aria-label="Choices">
    <SegmentedControl
      ariaLabel="View"
      value={view}
      options={[
        { value: "list", label: "List" },
        { value: "grid", label: "Grid" },
      ]}
      onchange={(v) => (view = v)}
    />
    <ChoiceList
      ariaLabel="Density"
      value={density}
      options={[
        { value: "compact", label: "Compact" },
        { value: "cozy", label: "Cozy" },
      ]}
      onchange={(v) => (density = v)}
    />
    <PopoverSelect
      value={format}
      options={[
        { value: "png", label: "PNG" },
        { value: "jpg", label: "JPG" },
      ]}
      onchange={(v) => (format = v)}
    />
    <ChipList bind:items={chips} placeholder="Add a tag" />
  </section>

  <section aria-label="Sliders and switches">
    <ValueSlider value={volume} ariaLabel="Volume" unit="%" onchange={(v) => (volume = v)} />
    <FillSlider value={60} ariaLabel="Brightness" oninput={noop} />
    <Switch bind:value={switched} ariaLabel="Wi-Fi" />
    <ColorPicker value="#3366cc" />
  </section>

  <section aria-label="Toolbar">
    <Toolbar>
      <Button variant="ghost" aria-label="Undo">U</Button>
      <Button variant="ghost" aria-label="Redo">R</Button>
    </Toolbar>
  </section>

  <p>toggled: {toggled}</p>
</main>
