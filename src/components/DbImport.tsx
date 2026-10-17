import { useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { FileUp, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api, errorText, type CollectionView, type DbCollection, type LookupProgress } from "@/lib/api";
import { diffs, duration, num } from "@/lib/format";
import { cn } from "@/lib/utils";
import { L } from "@/lib/i18n";

/** Alternative input: a collection.db dropped on the window or picked from disk. */
export function DbImport({ onOpen, disabled }: { onOpen: (v: CollectionView) => void; disabled?: boolean }) {
  const [hover, setHover] = useState(false);
  const [file, setFile] = useState<{ path: string; cols: DbCollection[] } | null>(null);
  const [progress, setProgress] = useState<LookupProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const openFile = async (path: string) => {
    setError(null);
    if (!path.toLowerCase().endsWith(".db")) {
      setError(L("Нужен файл collection.db", "A collection.db file is needed"));
      return;
    }
    try {
      const cols = (await api.readCollectionDb(path)).filter((c) => c.count > 0);
      if (cols.length === 0) setError(L("В файле нет коллекций с картами", "The file has no collections with maps"));
      else if (cols.length === 1) resolve(path, 0, cols);
      else setFile({ path, cols });
    } catch (e) {
      setError(errorText(e));
    }
  };

  const resolve = async (path: string, displayIndex: number, cols: DbCollection[]) => {
    setFile(null);
    setBusy(true);
    setProgress(null);
    setError(null);
    try {
      // indexes in the file include empty collections; map back by name/count order
      const all = await api.readCollectionDb(path);
      const target = cols[displayIndex];
      const index = all.findIndex((c) => c.name === target.name && c.count === target.count);
      onOpen(await api.loadDbCollection(path, index, setProgress));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "enter") setHover(p.paths.some((x) => x.toLowerCase().endsWith(".db")));
        else if (p.type === "leave") setHover(false);
        else if (p.type === "drop") {
          setHover(false);
          const db = p.paths.find((x) => x.toLowerCase().endsWith(".db"));
          if (db && !busy && !disabled) openFile(db);
        }
      })
      .then((u) => (unlisten = u));
    return () => unlisten?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [busy, disabled]);

  const pick = async () => {
    const path = await open({ title: "collection.db", filters: [{ name: "collection.db", extensions: ["db"] }] });
    if (typeof path === "string") openFile(path);
  };

  const slow = progress && progress.total - progress.done > 1500;

  return (
    <>
      <div
        className={cn(
          "mt-6 flex items-center gap-4 rounded-xl border border-dashed px-5 py-4 transition-colors duration-150",
          hover ? "border-primary bg-primary/10" : "border-input",
        )}
      >
        <FileUp className={cn("size-5 shrink-0", hover ? "text-primary" : "text-muted-foreground")} />
        <div className="min-w-0 flex-1 text-sm">
          {busy ? (
            <div aria-live="polite">
              <div>
                {progress
                  ? L(
                      `Ищу карты на зеркалах: ${num(progress.done)} из ${num(progress.total)}, найдено ${num(progress.found)}`,
                      `Looking up maps on mirrors: ${num(progress.done)} of ${num(progress.total)}, ${num(progress.found)} found`,
                    )
                  : L("Читаю collection.db…", "Reading collection.db…")}
                {progress?.etaSecs != null && progress.etaSecs > 5 && <>, {L("осталось", "about")} ~{duration(progress.etaSecs)}{L("", " left")}</>}
              </div>
              {slow && (
                <div className="mt-0.5 text-xs text-muted-foreground">
                  {L(
                    "Звёзды для каждой карты ищутся по одной. Если коллекция есть на osucollector, по ссылке будет быстрее.",
                    "Star ratings are looked up one map at a time. If the collection is on osucollector, opening it by link is faster.",
                  )}
                </div>
              )}
              {progress && (
                <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-raised">
                  <div
                    className="h-full bg-primary transition-[width] duration-200"
                    style={{ width: `${(progress.done / Math.max(1, progress.total)) * 100}%` }}
                  />
                </div>
              )}
            </div>
          ) : (
            <>
              <div>{hover ? L("Отпустите, чтобы открыть", "Drop to open") : L("Или перетащите сюда collection.db", "Or drop a collection.db here")}</div>
              <div className={cn("mt-0.5 text-xs", error ? "text-destructive" : "text-muted-foreground")}>
                {error ?? L("Например, скачанный с osucollector или из папки osu!stable", "For example, one downloaded from osucollector or from an osu!stable folder")}
              </div>
            </>
          )}
        </div>
        {busy ? (
          <Button variant="ghost" size="sm" onClick={() => api.cancelDbLookup()}>
            {L("Остановить", "Stop")}
          </Button>
        ) : (
          <Button variant="secondary" size="sm" onClick={pick} disabled={disabled}>
            {L("Выбрать файл…", "Pick file…")}
          </Button>
        )}
        {busy && !progress && <Loader2 className="size-4 animate-spin text-muted-foreground" />}
      </div>

      <Dialog open={!!file} onOpenChange={(o) => !o && setFile(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="font-display">{L("Какую коллекцию открыть?", "Which collection to open?")}</DialogTitle>
            <DialogDescription>{L("В файле их несколько.", "The file has several.")}</DialogDescription>
          </DialogHeader>
          <ul className="max-h-80 space-y-1 overflow-y-auto">
            {file?.cols.map((c, i) => (
              <li key={i}>
                <button
                  className="flex w-full items-center justify-between gap-3 rounded-lg px-3 py-2.5 text-left text-sm hover:bg-raised focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => resolve(file.path, i, file.cols)}
                >
                  <span className="truncate">{c.name || L("Без названия", "Untitled")}</span>
                  <span className="shrink-0 text-xs text-muted-foreground">{diffs(c.count)}</span>
                </button>
              </li>
            ))}
          </ul>
        </DialogContent>
      </Dialog>
    </>
  );
}
