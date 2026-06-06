/// Lunaris shared Svelte transitions.
///
/// Mirrors the CSS animation classes in motion.css so JS-driven
/// mount/unmount animations use the same timings and easings as
/// purely declarative ones.

import { cubicOut } from "svelte/easing";
import type { EasingFunction, TransitionConfig } from "svelte/transition";

const EASE_OUT: EasingFunction = cubicOut;

export interface FlyInOpts {
    delay?: number;
    duration?: number;
    /** Vertical offset in pixels. Defaults to -4 (mirrors shell-popover-anim). */
    y?: number;
    /** Opacity to start from (0 = invisible). */
    opacity?: number;
}

/// Fade-in with small downward translate. Use for popovers, cards,
/// dropdowns. Duration defaults to `--duration-medium` (250ms).
export function flyIn(_node: Element, opts: FlyInOpts = {}): TransitionConfig {
    const { delay = 0, duration = 250, y = -4, opacity = 0 } = opts;
    return {
        delay,
        duration,
        easing: EASE_OUT,
        css: (t) => {
            const o = opacity + (1 - opacity) * t;
            const ty = (1 - t) * y;
            return `opacity: ${o}; transform: translateY(${ty}px);`;
        },
    };
}

export interface FlyOutOpts extends FlyInOpts {}

/// Reverse of flyIn. Same shape but plays on unmount.
export function flyOut(_node: Element, opts: FlyOutOpts = {}): TransitionConfig {
    const { delay = 0, duration = 200, y = -4, opacity = 0 } = opts;
    return {
        delay,
        duration,
        easing: EASE_OUT,
        css: (t) => {
            const o = opacity + (1 - opacity) * t;
            const ty = (1 - t) * y;
            return `opacity: ${o}; transform: translateY(${ty}px);`;
        },
    };
}

export interface FadeScaleOpts {
    delay?: number;
    duration?: number;
    /** Starting scale. 1.0 = no scale, <1 = scale up. */
    start?: number;
}

/// Fade + gentle scale. Good for modals, menus, and overlays that
/// should feel like they pop into existence rather than slide.
export function fadeScale(_node: Element, opts: FadeScaleOpts = {}): TransitionConfig {
    const { delay = 0, duration = 200, start = 0.96 } = opts;
    return {
        delay,
        duration,
        easing: EASE_OUT,
        css: (t) => {
            const s = start + (1 - start) * t;
            return `opacity: ${t}; transform: scale(${s});`;
        },
    };
}

export interface SlideInOpts {
    delay?: number;
    duration?: number;
    /** Horizontal offset in pixels. Negative = from left, positive = from right. */
    x?: number;
}

/// Horizontal slide + fade. Use for side panels, sheets, or
/// navigation drawers. Duration defaults to `--duration-medium`.
export function slideIn(_node: Element, opts: SlideInOpts = {}): TransitionConfig {
    const { delay = 0, duration = 250, x = -16 } = opts;
    return {
        delay,
        duration,
        easing: EASE_OUT,
        css: (t) => {
            const tx = (1 - t) * x;
            return `opacity: ${t}; transform: translateX(${tx}px);`;
        },
    };
}
