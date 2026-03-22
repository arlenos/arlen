<script lang="ts">
  import { applyTokens, PANDA_TOKENS } from "$lib/theme";
  import "../app.css";

  // Apply tokens immediately, before render
  applyTokens(PANDA_TOKENS);
</script>

<slot />
