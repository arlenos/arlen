/// The mark for each capability family, shared between the privacy browser
/// (both pivots) and the per-app settings page. Keep these in step with the
/// settings nav icons where the system already has one for the same thing:
/// Knowledge Graph -> Brain, Notifications -> Bell, System (Actions) -> Zap,
/// the assistant -> Sparkles.
import { Sparkles, Brain, Globe, Folder, Camera, Clipboard, Bell, Zap, Workflow } from "lucide-svelte";
import type { Family } from "$lib/stores/grants";

const FAMILY_ICONS: Record<Family, typeof Sparkles> = {
  data: Brain,
  network: Globe,
  files: Folder,
  devices: Camera,
  clipboard: Clipboard,
  notifications: Bell,
  system: Zap,
  automation: Workflow,
};

/// The icon component for a family key, with a safe fallback.
export function familyIcon(key: string) {
  return FAMILY_ICONS[key as Family] ?? Brain;
}
