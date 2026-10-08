<script lang="ts">
  /// A single capability-scope chip: a reach label with an optional revoke
  /// control. Display-only by default; pass `onRevoke` to show the remove
  /// affordance. Unlike `ChipList` there is deliberately no add field - a
  /// capability browser can only ever take reach away, never grant it, so the
  /// chip carries a remove control and nothing else.
  import { Chip } from "../chip/index.js";

  let {
    label,
    onRevoke,
    id,
  }: {
    label: string;
    /// When set, renders a remove control that calls back. The caller owns the
    /// confirm step and the actual revoke; this chip only signals intent.
    onRevoke?: () => void;
    id?: string;
  } = $props();
</script>

<!-- The kit Chip, which this one used to be the only copy of; the remove control
     is now named in the reader's language instead of English. -->
<Chip {label} {id} onremove={onRevoke} />
