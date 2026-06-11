/// The drag-ghost: a clone of the dragged card (or a stacked-cards
/// presentation for multi-drags) that trails the pointer.
///
/// The ghost lives on `document.body`, outside the component's
/// scoped subtree, and single-drag clones carry the full scoped
/// `.window-card` ruleset via the copied hash class — inline
/// properties are the one layer that outranks those declarations
/// without a specificity fight. JS owns the ghost's whole lifecycle
/// (build, position, tilt, remove), so its float look lives here
/// too. One controller per indicator instance: ghost state never
/// crosses outputs when more than one topbar is mounted.

// Dynamic tilt. Maps horizontal pointer velocity (delta-X between
// consecutive pointermove events) to a rotation that feels like the
// card is swinging while carried. Smoothing is exponential so the
// ghost doesn't jitter on tiny jumps and doesn't flip instantly on
// direction changes. `lastX` is reset to the current pointer X when
// the ghost is created (see `build`) so the first frame doesn't
// compute a huge delta against 0.
const TILT_GAIN = 0.5; // degrees per pixel of delta-X
const TILT_CLAMP = 10; // max absolute rotation
const TILT_LERP = 0.25; // 0 = frozen, 1 = no smoothing

/// Applies the float-effect to a ghost element as inline styles.
function applyGhostFloatStyle(el: HTMLElement): void {
  Object.assign(el.style, {
    position: "fixed",
    top: "0",
    left: "0",
    // Above everything in the shell z-scale (see the table in
    // app.css) — the ghost trails the pointer across all chrome.
    zIndex: "10001",
    pointerEvents: "none",
    opacity: "0.95",
    // Initial value before the first position call lands, so
    // rotation=0 looks clean during the single frame between
    // appendChild and the first pointermove.
    transform: "translate3d(0, 0, 0) rotate(0deg) scale(1.05)",
    boxShadow:
      "0 12px 32px rgba(0, 0, 0, 0.35), 0 4px 8px rgba(0, 0, 0, 0.2)",
    transition: "none",
    cursor: "grabbing",
    outline: "none",
    willChange: "transform",
  });
}

/// Owns one drag-ghost DOM element across a gesture.
export class GhostController {
  private el: HTMLElement | null = null;
  private offsetX = 0;
  private offsetY = 0;
  private lastX = 0;
  private rotation = 0;

  /// Whether a ghost element is currently mounted.
  get active(): boolean {
    return this.el !== null;
  }

  /// Stashes where inside the card the pointer grabbed it, so the
  /// ghost keeps that grab point under the cursor while carried.
  setGrabOffset(offsetX: number, offsetY: number): void {
    this.offsetX = offsetX;
    this.offsetY = offsetY;
  }

  /// Builds the ghost element and mounts it on `document.body`.
  ///
  /// Single-window drag: clones the source card directly.
  ///
  /// Multi-window drag (targets.length > 1): builds a stacked-cards
  /// presentation. Up to 3 card clones are layered with a small
  /// x/y offset so the user sees a physical "stack" under the
  /// cursor. If more than 3 targets exist, a "+N" badge appears in
  /// the bottom-right corner indicating the overflow.
  ///
  /// `pointerX` seeds the tilt tracker so the first frame doesn't
  /// compute a huge delta against 0.
  build(sourceCard: HTMLElement, targets: string[], pointerX: number): void {
    this.lastX = pointerX;
    this.rotation = 0;
    const rect = sourceCard.getBoundingClientRect();

    if (targets.length <= 1) {
      const clone = sourceCard.cloneNode(true) as HTMLElement;
      clone.removeAttribute("draggable");
      clone.classList.add("drag-ghost");
      applyGhostFloatStyle(clone);
      clone.style.width = `${rect.width}px`;
      clone.style.height = `${rect.height}px`;
      document.body.appendChild(clone);
      this.el = clone;
      return;
    }

    // Multi: stack container. Gets a fixed size equal to the source
    // card plus the total stack offset so the whole stack is one
    // positional unit for translate3d.
    const STACK_VISIBLE = 3;
    const STACK_OFFSET_PX = 4;
    const visible = Math.min(targets.length, STACK_VISIBLE);
    const container = document.createElement("div");
    container.classList.add("drag-ghost", "drag-ghost-stack");
    applyGhostFloatStyle(container);
    // The container is a bare positioning frame — the inner clones
    // carry their own card look and per-card shadow instead.
    container.style.boxShadow = "none";
    container.style.width = `${rect.width + (visible - 1) * STACK_OFFSET_PX}px`;
    container.style.height = `${rect.height + (visible - 1) * STACK_OFFSET_PX}px`;

    // Paint back-to-front so the clicked card (index 0) sits on top.
    for (let i = visible - 1; i >= 0; i--) {
      // Pick the card DOM for each target via its `data-window-id`.
      // If another selected card isn't currently in the DOM
      // (off-screen workspace column), fall back to cloning the
      // source card — the visual still reads as "N cards".
      const targetId = targets[i];
      const targetEl =
        targetId === targets[0]
          ? sourceCard
          : (document.querySelector<HTMLElement>(
              `[data-window-id="${CSS.escape(targetId)}"]`,
            ) ?? sourceCard);
      const clone = targetEl.cloneNode(true) as HTMLElement;
      clone.removeAttribute("draggable");
      clone.classList.add("drag-ghost-card");
      clone.style.position = "absolute";
      clone.style.top = `${i * STACK_OFFSET_PX}px`;
      clone.style.left = `${i * STACK_OFFSET_PX}px`;
      clone.style.width = `${rect.width}px`;
      clone.style.height = `${rect.height}px`;
      // Per-card shadow so the layering reads even though the
      // container itself casts none.
      clone.style.boxShadow = "0 4px 12px rgba(0, 0, 0, 0.3)";
      container.appendChild(clone);
    }

    if (targets.length > STACK_VISIBLE) {
      const badge = document.createElement("span");
      badge.classList.add("drag-ghost-badge");
      badge.textContent = `+${targets.length - STACK_VISIBLE}`;
      container.appendChild(badge);
    }

    document.body.appendChild(container);
    this.el = container;
  }

  /// Moves the ghost so the grab point stays under the cursor, and
  /// swings it by the smoothed horizontal velocity.
  position(clientX: number, clientY: number): void {
    if (!this.el) return;
    const x = clientX - this.offsetX;
    const y = clientY - this.offsetY;
    const deltaX = clientX - this.lastX;
    this.lastX = clientX;
    // Target rotation from velocity. Clamp before smoothing so
    // `rotation` itself never exceeds the clamp, even if a
    // pathological single-frame delta is huge.
    const target = Math.max(
      -TILT_CLAMP,
      Math.min(TILT_CLAMP, deltaX * TILT_GAIN),
    );
    this.rotation = this.rotation + (target - this.rotation) * TILT_LERP;
    this.el.style.transform = `translate3d(${x}px, ${y}px, 0) rotate(${this.rotation.toFixed(2)}deg) scale(1.05)`;
  }

  /// Unmounts the ghost and resets the tilt so the next drag starts
  /// neutral instead of inheriting the last drag's final angle.
  remove(): void {
    if (!this.el) return;
    const el = this.el;
    this.el = null;
    this.lastX = 0;
    this.rotation = 0;
    requestAnimationFrame(() => {
      try {
        el.remove();
      } catch {
        /* already detached */
      }
    });
  }
}
