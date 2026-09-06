/// When the chosen language is known, for the strings that are frozen when raised.
///
/// Most of the interface reads the catalog through `$t`, so a language that
/// arrives late re-renders it. A toast does not: it is worded once, at the moment
/// of the failure, and lives with those words until it is dismissed. That is the
/// right shape - a notice about something that happened should not silently
/// change wording under the reader - but it means a refusal raised BEFORE the
/// language is known is stuck in the source language for its whole life.
///
/// It is not hypothetical and it is not only a dev artefact. `initArlenLocale`
/// awaits two dynamic imports before it ever sets the store, and `initProjects`
/// can refuse in that window: photograph the shell at `?locale=de` and the
/// focus-restore refusal comes back reading "The focus mode you had set could not
/// be restored." with a German bar around it. The layout has carried a comment
/// since August saying the init ORDER is "what decides it in practice"; the
/// screenshot is what showed that it does not.
///
/// So the wording waits instead of guessing. A toast raised at boot appears a
/// beat later in the right language, which is what a reader wanted anyway.
///
/// Settles rather than resolves-on-success: a language that could not be read is
/// still an answer, and a notice nobody can see because a config read failed is a
/// worse failure than an English one.

let settle: () => void;

/// Resolves once the language has been read, or once reading it has failed.
export const localeReady: Promise<void> = new Promise<void>((resolve) => {
  settle = resolve;
});

/// Say the language question is answered. Idempotent; the layout calls it once
/// per window, and a second call is a no-op rather than an error.
export function markLocaleReady(): void {
  settle();
}
