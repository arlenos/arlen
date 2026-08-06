import { readable } from "svelte/store";

/** Current time, updated every second. */
export const clock = readable(new Date(), (set) => {
  set(new Date());
  const interval = setInterval(() => set(new Date()), 1000);
  return () => clearInterval(interval);
});
