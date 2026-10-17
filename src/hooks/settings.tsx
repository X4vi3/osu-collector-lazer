import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { api, type Settings } from "@/lib/api";
import { detectLang, setLang, type Lang } from "@/lib/i18n";

interface Ctx {
  settings: Settings | null;
  update: (patch: Partial<Settings>) => Promise<void>;
}

const SettingsContext = createContext<Ctx>({ settings: null, update: async () => {} });

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  useEffect(() => {
    api.getSettings().then(async (s) => {
      if (!s.language) s = await api.saveSettings({ ...s, language: detectLang() });
      setLang((s.language as Lang) ?? "en");
      setSettings(s);
    });
  }, []);
  useEffect(() => {
    if (!settings) return;
    document.documentElement.classList.toggle("dark", settings.theme !== "light");
  }, [settings?.theme]);
  const update = useCallback(
    async (patch: Partial<Settings>) => {
      if (!settings) return;
      const next = { ...settings, ...patch };
      if (patch.language) setLang(patch.language as Lang);
      setSettings(next);
      setSettings(await api.saveSettings(next));
    },
    [settings],
  );
  // render nothing until the language is known, so the first paint is already translated
  if (!settings) return null;
  return <SettingsContext.Provider value={{ settings, update }}>{children}</SettingsContext.Provider>;
}

export const useSettings = () => useContext(SettingsContext);
