import { readable } from "svelte/store";

/** Current time, updated every second. */
export const clock = readable(new Date(), (set) => {
  set(new Date());
  const interval = setInterval(() => set(new Date()), 1000);
  return () => clearInterval(interval);
});

/** Formatted time string: HH:MM */
export const timeString = readable("", (set) => {
  const format = () =>
    new Date().toLocaleTimeString("en-GB", {
      hour: "2-digit",
      minute: "2-digit",
    });
  set(format());
  const interval = setInterval(() => set(format()), 1000);
  return () => clearInterval(interval);
});

/** Formatted date string: Mon 22 Mar */
export const dateString = readable("", (set) => {
  const format = () =>
    new Date().toLocaleDateString("en-GB", {
      weekday: "short",
      day: "numeric",
      month: "short",
    });
  set(format());
  const interval = setInterval(() => set(format()), 60_000);
  return () => clearInterval(interval);
});
