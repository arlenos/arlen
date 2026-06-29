/// Test setup for the jsdom-rendered suites. Tears down the testing-library
/// DOM between tests and polyfills the browser APIs bits-ui / floating-ui touch
/// during render (jsdom ships none of them), so a component render never throws
/// on a missing observer instead of surfacing the real a11y result.
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/svelte";

afterEach(() => cleanup());

class NoopObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}

if (!("ResizeObserver" in globalThis)) {
  // @ts-expect-error test polyfill
  globalThis.ResizeObserver = NoopObserver;
}
if (!("IntersectionObserver" in globalThis)) {
  // @ts-expect-error test polyfill
  globalThis.IntersectionObserver = NoopObserver;
}
if (!window.matchMedia) {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as unknown as MediaQueryList;
}
// jsdom lacks these; bits-ui popover/tooltip call them during open.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}
if (!HTMLElement.prototype.hasPointerCapture) {
  HTMLElement.prototype.hasPointerCapture = () => false;
}
if (!HTMLElement.prototype.setPointerCapture) {
  HTMLElement.prototype.setPointerCapture = () => {};
}
if (!HTMLElement.prototype.releasePointerCapture) {
  HTMLElement.prototype.releasePointerCapture = () => {};
}
