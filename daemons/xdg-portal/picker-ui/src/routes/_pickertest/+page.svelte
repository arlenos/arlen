<script module lang="ts">
  /// Headless look-mock for the picker. Seeds the Tauri IPC with a
  /// mocked daemon so `PickerView` renders against a fake listing +
  /// thumbnails for the screenshot loop. `?mode=` picks the request
  /// shape (open / openmulti / folder / save / savefiles); `?noart`
  /// drops thumbnails to show the icon fallback. Not shipped in any
  /// nav; a dev route to audit the look with Tim. The live data is the
  /// daemon's (the coder's MPRIS-equivalent picker commands).
  import { mockIPC } from "@tauri-apps/api/mocks";

  type Mode = "open" | "openmulti" | "folder" | "save" | "savefiles";

  const DIRS = ["Holidays", "Screenshots", "Work", "Wallpapers"];
  const FILES = [
    "beach.jpg",
    "sunset.png",
    "city.webp",
    "cat.gif",
    "diagram.bmp",
    "notes.txt",
    "report.pdf",
    "archive.zip",
  ];

  const art = (h: number) =>
    `data:image/svg+xml;base64,${btoa(
      `<svg xmlns='http://www.w3.org/2000/svg' width='120' height='120'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='hsl(${h},58%,58%)'/><stop offset='1' stop-color='hsl(${(h + 60) % 360},55%,34%)'/></linearGradient></defs><rect width='120' height='120' fill='url(#g)'/><circle cx='80' cy='42' r='16' fill='hsl(${h},72%,86%)'/></svg>`,
    )}`;

  function listing(path: string) {
    const base = path.replace(/\/$/, "");
    return [
      ...DIRS.map((name) => ({
        name,
        path: `${base}/${name}`,
        isDirectory: true,
        isHidden: false,
      })),
      ...FILES.map((name) => ({
        name,
        path: `${base}/${name}`,
        isDirectory: false,
        isHidden: false,
      })),
      { name: ".hidden", path: `${base}/.hidden`, isDirectory: false, isHidden: true },
    ];
  }

  const IMAGES = ["Images", [
    { kind: "glob", pattern: "*.png" },
    { kind: "glob", pattern: "*.jpg" },
    { kind: "glob", pattern: "*.jpeg" },
    { kind: "glob", pattern: "*.webp" },
    { kind: "glob", pattern: "*.gif" },
    { kind: "glob", pattern: "*.bmp" },
  ]] as const;

  function request(mode: Mode) {
    const handle = "mock-1";
    const filters = [{ name: IMAGES[0], patterns: IMAGES[1] }];
    const currentFolder = "/home/tim/Pictures";
    if (mode === "openmulti")
      return { type: "openFile", handle, appId: "org.gimp.GIMP", title: "", filters, currentFilter: null, multiple: true, modal: true, directory: false, currentFolder, parentWindow: null };
    if (mode === "folder")
      return { type: "openFile", handle, appId: "org.kde.kdenlive", title: "Choose a project folder", filters: [], currentFilter: null, multiple: false, modal: true, directory: true, currentFolder, parentWindow: null };
    if (mode === "save")
      return { type: "saveFile", handle, appId: "md.obsidian.Obsidian", title: "", filters, currentFilter: null, currentName: "diagram-export.png", currentFolder, currentFile: null, parentWindow: null };
    if (mode === "savefiles")
      return { type: "saveFiles", handle, appId: "org.gnome.Shotwell", title: "Export photos", files: ["/tmp/a.jpg", "/tmp/b.jpg", "/tmp/c.jpg"], currentFolder, parentWindow: null };
    return { type: "openFile", handle, appId: "org.mozilla.firefox", title: "", filters, currentFilter: null, multiple: false, modal: true, directory: false, currentFolder, parentWindow: null };
  }

  if (typeof window !== "undefined") {
    const params = new URLSearchParams(window.location.search);
    const mode = (params.get("mode") as Mode) || "open";
    const noart = params.get("noart") !== null;

    mockIPC((cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "picker_take_pending":
          return request(mode);
        case "resolve_start_dir":
          // A provided folder resolves to itself; the null probe (home
          // resolution) returns the real home, distinct from the start.
          return (args.provided as string) || "/home/tim";
        case "list_directory":
          return listing((args.path as string) || "/home/tim/Pictures");
        case "picker_thumbnail": {
          if (noart) return null;
          const p = (args.path as string) || "";
          if (/\.(png|jpe?g|gif|bmp|webp)$/i.test(p)) {
            let h = 0;
            for (const c of p) h = (h + c.charCodeAt(0) * 7) % 360;
            return art(h);
          }
          return null;
        }
        case "picker_recent":
          return [
            { label: "report.pdf", icon: "recent", path: "/home/tim/Documents/report.pdf" },
            { label: "Holidays", icon: "recent", path: "/home/tim/Pictures/Holidays" },
          ];
        case "file_exists":
          return false;
        case "get_theme":
          throw new Error("no theme in mock");
        default:
          return null;
      }
    });
  }
</script>

<script lang="ts">
  import PickerView from "$lib/components/PickerView.svelte";
</script>

<PickerView />
