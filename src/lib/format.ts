import { getLang, L } from "./i18n";

const nf = { ru: new Intl.NumberFormat("ru-RU"), en: new Intl.NumberFormat("en-US") };

export const num = (n: number) => nf[getLang()].format(n);

/** Russian plural: plural(5, ["дифф", "диффа", "диффов"]) */
export function plural(n: number, forms: [string, string, string]): string {
  const a = Math.abs(n) % 100;
  const b = a % 10;
  if (a > 10 && a < 20) return forms[2];
  if (b > 1 && b < 5) return forms[1];
  if (b === 1) return forms[0];
  return forms[2];
}

/** Counted noun in the current language. */
export function count(n: number, ru: [string, string, string], en: [string, string]): string {
  const word = getLang() === "en" ? (n === 1 ? en[0] : en[1]) : plural(n, ru);
  return `${num(n)} ${word}`;
}

export const diffs = (n: number) => count(n, ["дифф", "диффа", "диффов"], ["difficulty", "difficulties"]);
export const sets = (n: number) => count(n, ["сет", "сета", "сетов"], ["set", "sets"]);
/** "в N сетах" / "N sets" — the noun after a preposition */
export const setsIn = (n: number) => count(n, ["сете", "сетах", "сетах"], ["set", "sets"]);

export function bytesPerSec(bps: number): string {
  const [b, kb, mb] = getLang() === "en" ? ["B/s", "KB/s", "MB/s"] : ["Б/с", "КБ/с", "МБ/с"];
  if (bps < 1024) return `${Math.round(bps)} ${b}`;
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(0)} ${kb}`;
  return `${(bps / 1024 / 1024).toFixed(1)} ${mb}`;
}

export function bytes(v: number): string {
  const [kb, mb, gb] = getLang() === "en" ? ["KB", "MB", "GB"] : ["КБ", "МБ", "ГБ"];
  if (v < 1024 * 1024) return `${(v / 1024).toFixed(0)} ${kb}`;
  if (v < 1024 * 1024 * 1024) return `${(v / 1024 / 1024).toFixed(0)} ${mb}`;
  return `${(v / 1024 / 1024 / 1024).toFixed(2)} ${gb}`;
}

export function duration(secs: number): string {
  if (secs < 60) return L(`${secs} с`, `${secs} s`);
  const m = Math.floor(secs / 60);
  if (m < 60) return L(`${m} мин`, `${m} min`);
  const h = Math.floor(m / 60);
  return L(`${h} ч ${m % 60} мин`, `${h} h ${m % 60} min`);
}

export function ago(unixSecs: number): string {
  const d = Date.now() / 1000 - unixSecs;
  if (d < 90) return L("только что", "just now");
  if (d < 3600) return L(`${Math.round(d / 60)} мин назад`, `${Math.round(d / 60)} min ago`);
  if (d < 86400) return L(`${Math.round(d / 3600)} ч назад`, `${Math.round(d / 3600)} h ago`);
  const days = Math.round(d / 86400);
  return L(`${days} ${plural(days, ["день", "дня", "дней"])} назад`, `${days} ${days === 1 ? "day" : "days"} ago`);
}
