/// Pick a placeholder name nothing in the folder is using yet.
///
/// WHY THIS IS NOT THE BACKEND'S JOB. `core::ops::new_folder` is "create exactly
/// this name" and refuses a collision with the kernel's own EEXIST - atomically,
/// not with a racy pre-check - and that rule is right: somebody who TYPED a name
/// must never silently get a different one.
///
/// But the name here was not typed. "New folder" and "Link to x" are
/// PLACEHOLDERS this app chose, and the person is about to replace the first one
/// anyway - the row opens in rename mode. So when the placeholder is taken, the
/// app picks the next free one rather than handing somebody a refusal about a
/// name they never chose. Seen in a screenshot on 9 September: two New folder
/// presses in a row produced "Something with that name is already there, so
/// nothing was changed", which is a true sentence about the wrong thing.
///
/// Every file manager does this, and they all do it the same way, so the shape
/// is a convention rather than a decision: `base`, then `base 2`, `base 3`.

/// The first free name in the `base`, `base 2`, `base 3` sequence.
///
/// `taken` is compared exactly: a Linux directory is case-sensitive, and
/// treating `Notes` as taken because `notes` exists would refuse a name the
/// filesystem would have accepted.
///
/// Bounded, and the bound is not defensive dressing: a folder holding a thousand
/// of these is one where the person is better served by the backend's own
/// refusal than by this counting for ever. Falling back to `base` hands them
/// exactly that.
export function freeName(base: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  if (!used.has(base)) return base;
  for (let n = 2; n <= 1000; n++) {
    const candidate = `${base} ${n}`;
    if (!used.has(candidate)) return candidate;
  }
  return base;
}
