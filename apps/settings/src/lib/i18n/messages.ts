/// The settings message catalog, authored in MessageFormat 2.0. English is the source
/// of truth; German proves the reactive locale switch. Follows the system-monitor +
/// meetings + text-editor + terminal + files + harness template (I18N-R4). The catalog
/// is split into two area files (System/Input/chrome + Personal/AI) and merged here,
/// so the big back-fill could run in parallel; consumers only import from here.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";
import { a } from "./messages.a";
import { b } from "./messages.b";

/// Re-exported so the app (and a future language switcher) drive the same shared store
/// instance the catalog is bound to.
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: { ...a.en, ...b.en },
  de: { ...a.de, ...b.de },
};

/// The bound translator: `$t("s.key", params?)`, reactive to `locale`.
export const t = createTranslator(messages);
