<script lang="ts">
  /// Headless render harness for the transparency drawer. UI-AFFORDANCE
  /// verification ONLY (mocked IPC, no daemon), so the Cost + Reads feeds wired
  /// this session can be seen rendering their real shapes: Cost shows the folded
  /// token total, Reads shows a graph-access entry. Not shipped in any nav; dev
  /// only.
  import { onMount } from "svelte";
  import { tauriAvailable } from "$lib/tauri";
  import TransparencyDrawer from "$lib/components/chat/TransparencyDrawer.svelte";
  import { transparencyOpen } from "$lib/stores/transparency";

  let ready = $state(false);
  onMount(async () => {
    if (!tauriAvailable) {
      const { mockIPC } = await import("@tauri-apps/api/mocks");
      mockIPC((cmd) => {
        switch (cmd) {
          case "ai_capability":
            return { enabled: true, tier: "Project", actionMode: "Suggest", provider: "ollama-default", model: "qwen2.5:7b", executorLive: true };
          case "ai_access_grants":
            return [
              { id: "0192-grant", app_id: "ai-agent", declared_ceiling: "{\"read\":[\"system.File\"]}", required: false, identity_verified: false, live: true, revoked: false, superseded: false, issued_at: 0, reach: ["system.File"] },
            ];
          case "ai_working_set":
            // The agent's shape-only working set (no node contents).
            return { available: true, held: true, entityCounts: [{ type: "File", count: 3 }, { type: "Project", count: 1 }], activeBehaviour: "auto-tag-by-project", declaredReads: "the current project's files" };
          case "ai_activity_recent":
            return { entries: [
              { index: 289, timestampMicros: 1784764111663026, kind: "permission", actor: "ai-agent", subject: "agent.behaviour", outcome: "propose", nodeTypes: [], relations: [], callChainId: "run-9" },
            ], available: true, tampered: false, total: 289 };
          case "ai_reads_recent":
            // The fix this session: a graph.read now audits as graph-access, so the
            // anti-Recall feed sees it. This is the exact kind the ledger showed live.
            return { entries: [
              { index: 286, timestampMicros: 1784764111663026, kind: "graph-access", actor: "ai-agent", subject: "agent.behaviour", outcome: "tool-result", nodeTypes: ["File"], relations: [], callChainId: null },
            ], available: true, tampered: false, total: 289 };
          case "ai_usage":
            // The fix this session: folded from ai-proxy's live ledger.
            return JSON.stringify({ inputTokens: 1537, outputTokens: 140, totalTokens: 1677 });
          default:
            return null;
        }
      });
    }
    transparencyOpen.set(true);
    ready = true;
  });
</script>

{#if ready}
  <TransparencyDrawer />
{/if}
