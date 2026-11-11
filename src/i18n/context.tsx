import { createContext, useContext, useCallback } from "react";
import { strings, type Locale } from "./strings";

const LocaleContext = createContext<Locale>("zh");

export const LocaleProvider = LocaleContext.Provider;

export function useLocale(): Locale {
  return useContext(LocaleContext);
}

export function translate(
  locale: Locale,
  key: string,
  vars?: Record<string, string | number>,
): string {
  let s = strings[locale]?.[key] ?? strings.zh[key] ?? key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      s = s.replace(`{${k}}`, String(v));
    }
  }
  return s;
}

export function useT() {
  const locale = useLocale();
  return useCallback(
    (key: string, vars?: Record<string, string | number>): string => translate(locale, key, vars),
    [locale],
  );
}
