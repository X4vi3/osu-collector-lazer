export type Lang = "ru" | "en";

let current: Lang = "en";

export function setLang(l: Lang) {
  current = l;
  document.documentElement.lang = l;
}

export const getLang = () => current;

/** Picks the Russian or English variant. The tree remounts on language change, so plain calls are enough. */
export const L = (ru: string, en: string) => (current === "en" ? en : ru);

export function detectLang(): Lang {
  return (navigator.language || "").toLowerCase().startsWith("ru") ? "ru" : "en";
}
