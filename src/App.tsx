import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { ArrowLeft, Moon, Settings as SettingsIcon, Sun } from "lucide-react";
import { Toaster } from "@/components/ui/sonner";
import { Button } from "@/components/ui/button";
import { LazerPill, useLazerStatus } from "@/components/LazerPill";
import { SettingsDialog } from "@/components/SettingsDialog";
import { UpdateButton } from "@/components/UpdateButton";
import { SettingsProvider, useSettings } from "@/hooks/settings";
import { api, type CollectionView } from "@/lib/api";
import { getLang, L } from "@/lib/i18n";
import { InputScreen } from "@/screens/InputScreen";
import { CollectionScreen } from "@/screens/CollectionScreen";
import { JobScreen } from "@/screens/JobScreen";

type Screen = "input" | "collection" | "job";

function Shell() {
  const { settings, update } = useSettings();
  const lazer = useLazerStatus();
  const [screen, setScreen] = useState<Screen>("input");
  const [view, setView] = useState<CollectionView | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // a job may still be running after a webview reload
  useEffect(() => {
    api.jobSnapshot().then((s) => s && !s.summary.finished && setScreen("job"));
  }, []);

  const back = () => setScreen(screen === "job" ? (view ? "collection" : "input") : "input");
  const dark = settings?.theme !== "light";

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-12 shrink-0 items-center gap-2 border-b px-4">
        {screen !== "input" ? (
          <Button variant="ghost" size="icon-sm" onClick={back} aria-label={L("Назад", "Back")}>
            <ArrowLeft />
          </Button>
        ) : (
          <span className="w-8" />
        )}
        <span className="font-display text-[13px] font-semibold tracking-tight">
          Collector <span className="text-primary">→</span> Lazer
        </span>
        <div className="ml-auto flex items-center gap-1.5">
          <UpdateButton />
          <LazerPill status={lazer} />
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => update({ theme: dark ? "light" : "dark" })}
            aria-label={dark ? L("Светлая тема", "Light theme") : L("Тёмная тема", "Dark theme")}
            title={dark ? L("Светлая тема", "Light theme") : L("Тёмная тема", "Dark theme")}
          >
            {dark ? <Sun /> : <Moon />}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="w-9 px-0 font-display text-[11px]"
            onClick={() => update({ language: getLang() === "ru" ? "en" : "ru" })}
            aria-label={L("Switch to English", "Переключить на русский")}
            title={L("Switch to English", "Переключить на русский")}
          >
            {getLang() === "ru" ? "EN" : "RU"}
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => setSettingsOpen(true)}
            aria-label={L("Настройки", "Settings")}
            title={L("Настройки", "Settings")}
          >
            <SettingsIcon />
          </Button>
        </div>
      </div>

      <div className="relative min-h-0 flex-1">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={screen}
            className="absolute inset-0"
            initial={{ opacity: 0, x: 8 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: -8 }}
            transition={{ duration: 0.14 }}
          >
            {screen === "input" && (
              <InputScreen
                onOpen={(v) => {
                  setView(v);
                  setScreen("collection");
                }}
              />
            )}
            {screen === "collection" && view && (
              <CollectionScreen
                view={view}
                lazer={lazer}
                onView={setView}
                onStarted={() => setScreen("job")}
                onOpenSettings={() => setSettingsOpen(true)}
              />
            )}
            {screen === "job" && (
              <JobScreen
                onBack={() => setScreen(view ? "collection" : "input")}
                onNew={() => setScreen("input")}
              />
            )}
          </motion.div>
        </AnimatePresence>
      </div>

      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
      <Toaster theme={dark ? "dark" : "light"} position="bottom-right" />
    </div>
  );
}

export default function App() {
  return (
    <SettingsProvider>
      <Shell />
    </SettingsProvider>
  );
}
