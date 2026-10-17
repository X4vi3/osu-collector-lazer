import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ArrowDownToLine, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, errorText } from "@/lib/api";
import { L } from "@/lib/i18n";

export const RELEASES_URL = "https://github.com/X4vi3/osu-collector-lazer/releases/latest";

/** Checks GitHub releases once on startup; offers a one-click update when a newer version exists. */
export function UpdateButton() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [open, setOpen] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const [installed, setInstalled] = useState(true);

  useEffect(() => {
    // dev builds have no release to compare against
    if (import.meta.env.DEV) return;
    api.isInstalled().then(setInstalled);
    check()
      .then((u) => u && setUpdate(u))
      .catch(() => {});
  }, []);

  if (!update) return null;

  const install = async () => {
    let total = 0;
    let done = 0;
    setProgress(0);
    try {
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") total = e.data.contentLength ?? 0;
        else if (e.event === "Progress") {
          done += e.data.chunkLength;
          setProgress(total ? done / total : 0);
        }
      });
      await relaunch();
    } catch (e) {
      setProgress(null);
      toast.error(errorText(e));
    }
  };

  return (
    <>
      <Button size="sm" className="h-7 rounded-full px-3 text-xs" onClick={() => setOpen(true)}>
        <ArrowDownToLine /> {L("Обновить до", "Update to")} {update.version}
      </Button>
      <Dialog open={open} onOpenChange={(o) => progress === null && setOpen(o)}>
        <DialogContent className="sm:max-w-md" showCloseButton={progress === null}>
          <DialogHeader>
            <DialogTitle className="font-display">
              {L("Вышла версия", "Version")} {update.version}
              {L("", " is out")}
            </DialogTitle>
            <DialogDescription asChild>
              <div className="max-h-60 overflow-y-auto text-sm leading-relaxed whitespace-pre-line">
                {update.body?.trim() || L("Без описания изменений.", "No release notes.")}
              </div>
            </DialogDescription>
          </DialogHeader>
          {progress !== null && (
            <div className="h-1.5 overflow-hidden rounded-full bg-raised">
              <div className="h-full bg-primary transition-[width] duration-150" style={{ width: `${progress * 100}%` }} />
            </div>
          )}
          <DialogFooter>
            <Button variant="ghost" disabled={progress !== null} onClick={() => setOpen(false)}>
              {L("Позже", "Later")}
            </Button>
            {installed ? (
              <Button disabled={progress !== null} onClick={install}>
                {progress !== null && <Loader2 className="animate-spin" />}
                {progress !== null ? L("Обновляю…", "Updating…") : L("Обновить и перезапустить", "Update and restart")}
              </Button>
            ) : (
              // portable exe: it can't replace itself, so point to the release page
              <Button onClick={() => openUrl(RELEASES_URL)}>{L("Скачать новую версию", "Download new version")}</Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
