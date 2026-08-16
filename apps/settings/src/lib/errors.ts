/// Whether a failure message is machinery talking rather than something for a
/// person.
///
/// A backend's own words are worth showing - "no such printer", "permission
/// denied on /etc/foo" tells someone what is wrong and sometimes what to do. A
/// JavaScript runtime error names an internal and offers nothing: the reader
/// learns that something called `window.__TAURI_INTERNALS__` is undefined, which
/// is true, unactionable and alarming.
///
/// This is the THIRD copy of this predicate. ui-kit's `FileBrowser` learned it
/// first, by greeting a user with "TypeError: undefined is not an object
/// (evaluating 'window.__TAURI_INTERNALS__.invoke')" in the middle of the pane;
/// the viewers app copied it and wrote "if a third app needs it, that is the
/// moment it moves somewhere both can reach". That moment is here - measured on
/// 16 August, Settings/Notifications with no backend renders exactly that
/// sentence under its heading - but the shared home is `@arlen/ui-kit`, which is
/// another lane's file, so the move is theirs to make. Until then this copy at
/// least stops the app repeating the mistake its neighbours already fixed.
export function readsAsInternal(message: string): boolean {
  return /\b(TypeError|ReferenceError|SyntaxError)\b|undefined is not|is not a function|window\.__/.test(
    message,
  );
}
