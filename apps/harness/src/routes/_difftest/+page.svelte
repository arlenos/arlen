<script lang="ts">
  /// Headless render harness for the change-diff GateCard. UI-AFFORDANCE
  /// verification ONLY, NOT a behaviour claim. Renders the real GateCard with a
  /// unified-diff body across its states: a single-file proposed change (the diff
  /// IS the approve step), a multi-file change (collapsed, per-file), and the
  /// applied receipt with Undo. The diff text is a fixture; the real proposed
  /// diff + the approve/undo wiring are the coder's executor seams. Dev route.
  import GateCard from "$lib/components/chat/GateCard.svelte";

  const single = `diff --git a/ai/ai-agent/src/parser.rs b/ai/ai-agent/src/parser.rs
@@ -41,9 +41,11 @@ impl Tokenizer {
     fn next_token(&mut self) -> Option<Token> {
         self.skip_whitespace();
-        let c = self.peek()?;
-        if c.is_ascii_digit() {
-            return Some(self.read_number());
+        let c = self.peek()?;
+        if c.is_ascii_digit() || (c == '.' && self.peek_at(1).is_some_and(|n| n.is_ascii_digit())) {
+            return Some(self.read_number());
+        }
+        if c == '_' || c.is_alphabetic() {
+            return Some(self.read_ident());
         }
         None
     }`;

  const multi = `diff --git a/sdk/config/src/theme.rs b/sdk/config/src/theme.rs
@@ -12,6 +12,7 @@ pub struct Theme {
     pub name: String,
     pub accent: Color,
+    pub radius: f32,
 }
diff --git a/sdk/config/src/lib.rs b/sdk/config/src/lib.rs
@@ -3,5 +3,5 @@
 pub mod theme;
-pub use theme::Theme;
+pub use theme::{Theme, Color};
diff --git a/docs/themes.md b/docs/themes.md
new file mode 100644
@@ -0,0 +1,3 @@
+# Themes
+
+A theme carries a name, an accent colour and a corner radius.`;
</script>

<div class="harness">
  <section>
    <h2>Proposed change (single file): the diff is the approve step</h2>
    <div class="bubble">
      <GateCard
        title="Edit parser.rs"
        detail="Extend the tokenizer to read float literals and identifiers."
        diff={single}
        onapprove={() => {}}
        ondeny={() => {}}
        onalways={() => {}}
      />
    </div>
  </section>

  <section>
    <h2>Proposed change (multiple files): collapsed per file</h2>
    <div class="bubble">
      <GateCard
        title="Add a theme corner radius"
        detail="Thread a radius field through the theme and re-export Color."
        diff={multi}
        onapprove={() => {}}
        ondeny={() => {}}
        onalways={() => {}}
      />
    </div>
  </section>

  <section>
    <h2>Applied receipt (you approved): with per-change Undo</h2>
    <div class="bubble">
      <GateCard title="Edit parser.rs" diff={single} done onundo={() => {}} />
    </div>
  </section>

  <section>
    <h2>Auto-applied (no gate, under a standing grant): still a diff + Undo, never silent</h2>
    <div class="bubble">
      <GateCard
        title="Edit parser.rs"
        diff={single}
        done
        auto
        via="edits in this project"
        onundo={() => {}}
      />
    </div>
  </section>
</div>

<style>
  .harness {
    display: flex;
    flex-direction: column;
    gap: 28px;
    padding: 32px;
    min-height: 100vh;
    background: var(--background);
    color: var(--foreground);
  }
  h2 {
    margin: 0 0 12px;
    font-size: 0.75rem;
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .bubble {
    max-width: 40rem;
  }
</style>
