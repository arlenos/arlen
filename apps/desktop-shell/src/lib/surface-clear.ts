/**
 * Clearing the vacated region of a transparent shell surface.
 *
 * Every shell surface is a transparent layer-shell webview, and on all of them an
 * overlay that shrinks or closes can leave its last frame behind: the element is
 * gone from the DOM and its pixels are still presented. Measured on the image on
 * 10 August: the waypointer, which does the clear below, returns to a frame that
 * is byte-identical to the desktop before it opened (max channel delta 0). The
 * consent surface, which did not, still shows its card after the request has been
 * resolved - the same build, the same session, the same session-wide rendering
 * settings.
 *
 * This CLEARS rather than covers. Painting an opaque fill over the region would
 * leave the stale pixels exactly where they are and merely stop us seeing them,
 * and it would spend the see-through look that is part of the shell's visual
 * language on a rendering defect. What is here forces the engine to present the
 * whole surface again, so the vacated region is written empty.
 *
 * The ladder behind the two steps, each rung measured rather than reasoned:
 * swapping the backdrop's background cleared the band OUTSIDE the card and
 * nothing else; adding the card's own background changed nothing, because the
 * stale pixels inside the old box are painted by ROWS that no longer exist and an
 * ancestor's background does not cover them; detaching and reattaching the CARD
 * cleared its oversized box and faded the rest. So both steps are applied to the
 * element that SPANS THE WHOLE SURFACE - a backdrop or root - because that is the
 * only one that covers every region an overlay can ever have occupied.
 *
 * It is a workaround and is labelled as one. The right fix is in the engine; this
 * keeps the shipped surface correct until that arrives.
 */

/**
 * Force `el` and everything under it to be presented again, clearing whatever was
 * left behind in the region it spans.
 *
 * Pass the surface-spanning element (the backdrop, or the root when there is no
 * backdrop). Both writes happen inside one animation frame, so no intermediate
 * state is ever presented.
 */
export function clearSurface(el: HTMLElement | undefined | null): void {
  if (!el) return;
  requestAnimationFrame(() => {
    // Step one reaches the region outside the overlay's own box.
    el.style.backgroundImage = "linear-gradient(transparent, transparent)";
    void el.offsetHeight;
    el.style.backgroundImage = "";
    // Step two is the only thing measured to reach pixels whose element is gone.
    el.style.display = "none";
    void el.offsetHeight;
    el.style.display = "";
  });
}
