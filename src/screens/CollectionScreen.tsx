import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, List, Loader2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Histogram } from "@/components/Histogram";
import { api, errorText, type CollectionView, type InstalledCount, type LazerStatus, type PreviewRow } from "@/lib/api";
import { ago, count, diffs, num, setsIn } from "@/lib/format";
import { L } from "@/lib/i18n";
import { starColor, starTextColor } from "@/lib/stars";
import { useSettings } from "@/hooks/settings";
import { progressText, useCollectionLoader } from "./InputScreen";

interface Props {
  view: CollectionView;
  lazer: LazerStatus | null;
  onView: (v: CollectionView) => void;
  onStarted: () => void;
  onOpenSettings: () => void;
}

type Prompt = null | "replace" | "notRunning" | "notFound" | "launching";

export function CollectionScreen({ view, lazer, onView, onStarted, onOpenSettings }: Props) {
  const { settings, update } = useSettings();
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [showList, setShowList] = useState(false);
  const [prompt, setPrompt] = useState<Prompt>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const { load, loading, progress, error } = useCollectionLoader(onView);

  useEffect(() => setSelected(new Set()), [view.info.id]);

  const toggle = (star: number) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(star)) n.delete(star);
      else n.add(star);
      return n;
    });

  // what lazer already has: scanned once per opened collection, counted per selection
  const [scanned, setScanned] = useState(false);
  const [have, setHave] = useState<InstalledCount | null>(null);
  const checkInstalled = settings?.checkInstalled ?? false;
  useEffect(() => {
    setScanned(false);
    setHave(null);
    if (!checkInstalled) return;
    let alive = true;
    api
      .scanLazer()
      .then(() => alive && setScanned(true))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [view.info.id, checkInstalled]);
  const selKey = [...selected].sort((a, b) => a - b).join(",");
  useEffect(() => {
    if (!scanned || selected.size === 0) {
      setHave(null);
      return;
    }
    let alive = true;
    api.installedCount(view.info.id, [...selected]).then((c) => alive && setHave(c));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scanned, selKey, view.info.id]);

  const { selDiffs, selSets } = useMemo(() => {
    let mask = 0;
    let d = 0;
    for (const b of view.buckets) {
      if (selected.has(b.star)) {
        mask |= 1 << b.star;
        d += b.diffs;
      }
    }
    const s = mask ? view.setMasks.reduce((acc, m) => acc + (m & mask ? 1 : 0), 0) : 0;
    return { selDiffs: d, selSets: s };
  }, [selected, view]);

  const buckets = [...selected].sort((a, b) => a - b);

  const start = async (manualDir: string | null) => {
    setStartError(null);
    try {
      await api.startJob(view.info.id, buckets, manualDir);
      setPrompt(null);
      onStarted();
    } catch (e) {
      setStartError(errorText(e));
      setPrompt(null);
    }
  };

  /** Walks through the pre-flight checks: replace warning → lazer found → lazer running. */
  const begin = async (skipWarning = false) => {
    if (!settings) return;
    if (!skipWarning && !settings.replaceWarningSeen && !settings.noTrim && !settings.checkInstalled) {
      setPrompt("replace");
      return;
    }
    const status = await api.lazerStatus();
    if (!status.found) setPrompt("notFound");
    else if (!status.running) setPrompt("notRunning");
    else start(null);
  };

  const launchAndStart = async () => {
    setPrompt("launching");
    try {
      await api.launchLazer();
      await start(null);
    } catch (e) {
      setStartError(errorText(e));
      setPrompt(null);
    }
  };

  const pickManualDir = async () => {
    const dir = await openDialog({ directory: true, title: L("Куда сохранить урезанные .osz", "Where to save the trimmed .osz files") });
    if (typeof dir === "string") {
      update({ manualDir: dir });
      start(dir);
    }
  };

  return (
    <div className="flex h-full flex-col">
      {/* header: blurred beatmap cover like a beatmap page */}
      <header className="relative isolate shrink-0 overflow-hidden">
        {view.cover && (
          <div
            className="absolute inset-0 -z-10 scale-110 bg-cover bg-center opacity-35 blur-md dark:opacity-30"
            style={{ backgroundImage: `url("${view.cover}")` }}
          />
        )}
        <div className="absolute inset-0 -z-10 bg-gradient-to-b from-transparent to-background" />
        <div className="flex items-end justify-between gap-6 px-10 pt-8 pb-6">
          <div className="min-w-0">
            <h1 className="truncate font-display text-[28px] leading-tight font-semibold tracking-tight">
              {view.info.name || `${L("Коллекция", "Collection")} ${view.info.id}`}
            </h1>
            <p className="mt-1.5 text-sm text-muted-foreground">
              {view.fromDb
                ? L("Из collection.db. ", "From collection.db. ")
                : view.info.uploader
                  ? L(`Собрал ${view.info.uploader}. `, `By ${view.info.uploader}. `)
                  : ""}
              {L(
                `${diffs(view.stdDiffs)} osu!standard в ${setsIn(view.stdSets)}`,
                `${num(view.stdDiffs)} osu!standard ${view.stdDiffs === 1 ? "difficulty" : "difficulties"} in ${setsIn(view.stdSets)}`,
              )}
              {view.info.beatmapCount > view.stdDiffs && (
                <>
                  {" "}
                  ({L("ещё", "plus")} {num(view.info.beatmapCount - view.stdDiffs)} —{" "}
                  {view.fromDb
                    ? L("другие режимы или карты, которых нет на зеркалах", "other modes or maps missing from mirrors")
                    : L("другие режимы или удалённые", "other modes or deleted maps")}
                  )
                </>
              )}
            </p>
          </div>
          {!view.fromDb && (
          <div className="flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
            <span aria-live="polite">{loading ? progressText(progress) : L(`Данные ${ago(view.fetchedAt)}`, `Data from ${ago(view.fetchedAt)}`)}</span>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => load(String(view.info.id), true)}
              disabled={loading}
              aria-label={L("Обновить с osucollector", "Refresh from osucollector")}
              title={L("Обновить с osucollector", "Refresh from osucollector")}
            >
              {loading ? <Loader2 className="animate-spin" /> : <RefreshCw />}
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => openUrl(`https://osucollector.com/collections/${view.info.id}`)}
              aria-label={L("Открыть на osucollector.com", "Open on osucollector.com")}
              title={L("Открыть на osucollector.com", "Open on osucollector.com")}
            >
              <ExternalLink />
            </Button>
          </div>
          )}
        </div>
        {error && <p className="px-10 pb-2 text-sm text-destructive">{error}</p>}
      </header>

      <main className="flex min-h-0 flex-1 flex-col px-10">
        <p className="mb-3 text-sm text-muted-foreground">
          {L(
            "Отметьте звёзды, которые нужны. Корзина 7★ — это всё от 7,00 до 7,99★.",
            "Pick the star ratings you want. The 7★ bucket covers everything from 7.00 to 7.99★.",
          )}
        </p>
        <div className={showList ? "h-56 shrink-0" : "min-h-0 flex-1 pb-4"}>
          <Histogram buckets={view.buckets} selected={selected} onToggle={toggle} />
        </div>
        {showList && (
          <div className="mt-3 min-h-0 flex-1 pb-3">
            <PreviewList collectionId={view.info.id} buckets={buckets} />
          </div>
        )}
      </main>

      <footer className="flex shrink-0 items-center gap-4 border-t px-10 py-4">
        <div className="min-w-0 flex-1" aria-live="polite">
          {selDiffs > 0 ? (
            <>
              <div className="font-display text-[15px] font-semibold">
                {L("Выбрано", "Selected")}: {diffs(selDiffs)} {L("в", "in")} {setsIn(selSets)}
              </div>
              {checkInstalled && (
                <div className="mt-0.5 text-xs text-muted-foreground">
                  {!scanned
                    ? L("Проверяю, что уже есть в osu!lazer…", "Checking what osu!lazer already has…")
                    : have && have.diffs > 0
                      ? have.diffs === selDiffs
                        ? L(
                          "Всё это уже есть в osu!lazer — качать нечего, останется только добавить коллекцию",
                          "osu!lazer already has all of these — nothing to download, only the collection to add",
                        )
                        : L(
                          `Уже есть в osu!lazer: ${diffs(have.diffs)}. Скачаю остальные ${num(selDiffs - have.diffs)}`,
                          `Already in osu!lazer: ${diffs(have.diffs)}. The other ${num(selDiffs - have.diffs)} will be downloaded`,
                        )
                      : have
                        ? L("В osu!lazer пока ничего из этого нет", "osu!lazer has none of these yet")
                        : null}
                </div>
              )}
            </>
          ) : (
            <div className="text-sm text-muted-foreground">{L("Ничего не выбрано", "Nothing selected")}</div>
          )}
          {startError && <div className="mt-1 text-sm text-destructive">{startError}</div>}
          {lazer && !lazer.found && !startError && (
            <div className="mt-1 text-xs text-muted-foreground">
              {L("osu!lazer не найден: можно скачать урезанные .osz в папку или", "osu!lazer not found: you can save the trimmed .osz files to a folder or")}{" "}
              <button className="underline underline-offset-2 hover:text-foreground" onClick={onOpenSettings}>
                {L("указать osu!.exe", "pick osu!.exe")}
              </button>
              .
            </div>
          )}
        </div>
        <Button variant="ghost" onClick={() => setShowList((v) => !v)} disabled={selDiffs === 0} aria-pressed={showList}>
          <List />
          {showList ? L("Скрыть карты", "Hide maps") : L("Показать карты", "Show maps")}
        </Button>
        <Button size="lg" className="h-12 rounded-xl px-7 text-[15px]" disabled={selDiffs === 0} onClick={() => begin()}>
          {L("Скачать и установить в Lazer", "Download and install to Lazer")}
        </Button>
      </footer>

      {/* pre-flight dialogs */}
      <Dialog open={prompt === "replace"} onOpenChange={(o) => !o && setPrompt(null)}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle className="font-display">{L("Перед первым запуском", "Before the first run")}</DialogTitle>
            <DialogDescription asChild>
              <div className="space-y-3 text-sm leading-relaxed">
                <p>
                  {L(
                    "osu!lazer заменяет сет целиком, если такой уже установлен. Если у вас какой-то сет из коллекции стоит полностью, после установки в нём останутся только выбранные сложности.",
                    "osu!lazer replaces a set entirely if it's already installed. If you have a full set from this collection, only the selected difficulties will remain in it afterwards.",
                  )}
                </p>
                <p>
                  {L(
                    "Сеты, поставленные через это приложение, в безопасности: оно помнит, какие сложности уже ставило, и при повторной установке сохраняет их.",
                    "Sets installed through this app are safe: it remembers which difficulties it installed and keeps them on reinstall.",
                  )}
                </p>
              </div>
            </DialogDescription>
          </DialogHeader>
          <label className="flex cursor-pointer items-center justify-between gap-4 rounded-lg bg-raised px-3 py-3 text-sm">
            {L("Не вырезать сложности, ставить сеты целиком", "Don't trim, install whole sets")}
            <Switch checked={settings?.noTrim ?? false} onCheckedChange={(v) => update({ noTrim: v })} />
          </label>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setPrompt(null)}>
              {L("Отмена", "Cancel")}
            </Button>
            <Button
              onClick={async () => {
                await update({ replaceWarningSeen: true });
                begin(true);
              }}
            >
              {L("Понятно, продолжить", "Got it, continue")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={prompt === "notRunning" || prompt === "launching"} onOpenChange={(o) => !o && prompt !== "launching" && setPrompt(null)}>
        <DialogContent className="sm:max-w-md" showCloseButton={prompt !== "launching"}>
          <DialogHeader>
            <DialogTitle className="font-display">{L("osu!lazer не запущен", "osu!lazer is not running")}</DialogTitle>
            <DialogDescription>
              {L(
                "Карты передаются в запущенную игру. Запустить её сейчас? Можно начать и без этого: скачанные сеты подождут, пока lazer не откроется.",
                "Maps are handed to the running game. Start it now? You can also begin without it: downloaded sets will wait until lazer is open.",
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" disabled={prompt === "launching"} onClick={() => start(null)}>
              {L("Начать без запуска", "Start without it")}
            </Button>
            <Button disabled={prompt === "launching"} onClick={launchAndStart}>
              {prompt === "launching" && <Loader2 className="animate-spin" />}
              {prompt === "launching" ? L("Ждём запуска lazer…", "Waiting for lazer…") : L("Запустить lazer и начать", "Start lazer and begin")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={prompt === "notFound"} onOpenChange={(o) => !o && setPrompt(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="font-display">{L("osu!lazer не найден", "osu!lazer not found")}</DialogTitle>
            <DialogDescription>
              {L(
                "Укажите, где лежит osu!.exe, или скачайте урезанные .osz в папку: их можно перетащить в окно lazer вручную.",
                "Point to osu!.exe, or save the trimmed .osz files to a folder: you can drag them into lazer yourself.",
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="ghost"
              onClick={() => {
                setPrompt(null);
                onOpenSettings();
              }}
            >
              {L("Указать osu!.exe", "Pick osu!.exe")}
            </Button>
            <Button onClick={pickManualDir}>{L("Скачать в папку…", "Save to folder…")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function PreviewList({ collectionId, buckets }: { collectionId: number; buckets: number[] }) {
  const [rows, setRows] = useState<PreviewRow[] | null>(null);
  const parentRef = useRef<HTMLDivElement>(null);
  const key = buckets.join(",");

  useEffect(() => {
    let alive = true;
    setRows(null);
    api.selectionPreview(collectionId, buckets).then((r) => alive && setRows(r));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [collectionId, key]);

  const virtualizer = useVirtualizer({
    count: rows?.length ?? 0,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 36,
    overscan: 12,
  });

  if (!rows) return <div className="flex h-full items-center justify-center text-sm text-muted-foreground">{L("Загрузка…", "Loading…")}</div>;

  return (
    <div ref={parentRef} className="h-full overflow-y-auto rounded-xl bg-card ring-1 ring-border" aria-label={L("Выбранные карты", "Selected maps")}>
      <div style={{ height: virtualizer.getTotalSize() }} className="relative">
        {virtualizer.getVirtualItems().map((v) => {
          const r = rows[v.index];
          return (
            <div
              key={v.key}
              className="absolute inset-x-0 flex items-center gap-3 border-b border-border/60 px-4 text-sm"
              style={{ height: v.size, transform: `translateY(${v.start}px)` }}
            >
              <span
                className="w-12 shrink-0 rounded-full px-1.5 py-0.5 text-center font-display text-[11px] font-semibold"
                style={{ background: starColor(r.sr), color: starTextColor(r.sr) }}
              >
                {r.sr.toFixed(2)}
              </span>
              <span className="min-w-0 flex-1 truncate">
                {r.artist} – {r.title} <span className="text-muted-foreground">[{r.version}]</span>
              </span>
            </div>
          );
        })}
      </div>
      <div className="sr-only">
        {count(rows.length, ["карта", "карты", "карт"], ["map", "maps"])}
      </div>
    </div>
  );
}
