<script lang="ts">
  /// The block right-click menu (terminal.md §4.11): the home for everything you
  /// do *to* a block. arlen-ui owns the look + grouping + the right-click anchor;
  /// the coder wires each action to the hovered block's validated engine record
  /// and the two AI entry points. Flat system-app menu on the @arlen/ui-kit
  /// ContextMenu, matching the file-manager folder right-click (text rows, no
  /// icons, separators between groups) - never a web/native context menu.
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu";
  import type { Snippet } from "svelte";

  /// The block actions, each bound to the block under the pointer. All optional:
  /// an action without a handler renders its item disabled (greyed), the desktop
  /// convention for an unavailable action, never a silent no-op.
  type BlockActions = {
    runAgain?: () => void;
    copyCommand?: () => void;
    copyOutput?: () => void;
    copyBoth?: () => void;
    copyMarkdown?: () => void;
    editRerun?: () => void;
    selectBlock?: () => void;
    saveOutput?: () => void;
    explain?: () => void;
    ask?: () => void;
  };

  type Props = {
    actions?: BlockActions;
    /// The right-click target: the grid / block area.
    children: Snippet;
  };

  let { actions = {}, children }: Props = $props();
</script>

<ContextMenu.Root>
  <ContextMenu.Trigger>{@render children()}</ContextMenu.Trigger>
  <ContextMenu.Content class="w-60">
    <ContextMenu.Item onclick={actions.runAgain} disabled={!actions.runAgain}>
      Run again
    </ContextMenu.Item>

    <ContextMenu.Separator />
    <ContextMenu.Sub>
      <ContextMenu.SubTrigger>Copy</ContextMenu.SubTrigger>
      <ContextMenu.SubContent class="w-48">
        <ContextMenu.Item onclick={actions.copyCommand} disabled={!actions.copyCommand}>
          Command
        </ContextMenu.Item>
        <ContextMenu.Item onclick={actions.copyOutput} disabled={!actions.copyOutput}>
          Output
        </ContextMenu.Item>
        <ContextMenu.Item onclick={actions.copyBoth} disabled={!actions.copyBoth}>
          Command + output
        </ContextMenu.Item>
        <ContextMenu.Item onclick={actions.copyMarkdown} disabled={!actions.copyMarkdown}>
          As Markdown
        </ContextMenu.Item>
      </ContextMenu.SubContent>
    </ContextMenu.Sub>

    <ContextMenu.Separator />
    <ContextMenu.Item onclick={actions.editRerun} disabled={!actions.editRerun}>
      Edit &amp; re-run
    </ContextMenu.Item>
    <ContextMenu.Item onclick={actions.selectBlock} disabled={!actions.selectBlock}>
      Select block
    </ContextMenu.Item>
    <ContextMenu.Item onclick={actions.saveOutput} disabled={!actions.saveOutput}>
      Save output to file&hellip;
    </ContextMenu.Item>

    <ContextMenu.Separator />
    <ContextMenu.Group>
      <ContextMenu.GroupHeading
        class="text-muted-foreground px-2 pt-1 pb-0.5 text-[0.625rem] font-semibold tracking-wide uppercase"
      >
        Arlen
      </ContextMenu.GroupHeading>
      <ContextMenu.Item onclick={actions.explain} disabled={!actions.explain}>
        Explain this
      </ContextMenu.Item>
      <ContextMenu.Item onclick={actions.ask} disabled={!actions.ask}>
        Ask Arlen about this block
      </ContextMenu.Item>
    </ContextMenu.Group>
  </ContextMenu.Content>
</ContextMenu.Root>
