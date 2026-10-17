import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  AlertTriangle,
  Check,
  CircleDashed,
  Download,
  FolderOpen,
  Loader2,
  Pause,
  Play,
  RotateCcw,
  Scissors,
  SkipForward,
  Upload,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { api, errorText, type Progress, type SetRow, type SetStatus, type Summary } from "@/lib/api";
import { bytes, bytesPerSec, diffs, duration, num, sets, setsIn } from "@/lib/format";
import { cn } from "@/lib/utils";
import { L } from "@/lib/i18n";

interface Props {
  onBack: () => void;
  onNew: () => void;
}

/** Keeps the latest job state from `job-progress` events (rows arrive as diffs, ~10/s). */
function useJob() {
  const rows = useRef<SetRow[]>([]);
  const [summary, setSummary] = useState<Summary | null>(null);
  const [name, setName] = useState("");
  const [, setVersion] = useState(0);
  const jobId = useRef<number | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const apply = (p: Progress, full: boolean) => {
      if (full) {
        jobId.current = p.jobId;
        rows.current = p.rows;
      } else {
        if (p.jobId !== jobId.current) return;
        for (const r of p.rows) rows.current[r.idx] = r;
      }
      setSummary(p.summary);
      setName(p.collectionName);
      setVersion((v) => v + 1);
    };
    listen<Progress>("job-progress", (e) => {
      if (e.payload.jobId !== jobId.current) {
        api.jobSnapshot().then((s) => s && apply(s, true));
      } else apply(e.payload, false);
    }).then((u) => (unlisten = u));
    api.jobSnapshot().then((s) => s && apply(s, true));
    return () => unlisten?.();
  }, []);
  return { rows: rows.current, summary, name };
}

const STATUS: Record<SetStatus, { label: () => string; icon: typeof Check; tone: string }> = {
  pending: { label: () => L("Ожидание", "Waiting"), icon: CircleDashed, tone: "text-muted-foreground" },
  downloading: { label: () => L("Загрузка", "Downloading"), icon: Download, tone: "text-primary" },
  trimming: { label: () => L("Вырезание", "Trimming"), icon: Scissors, tone: "text-primary" },
  queued: { label: () => L("Ждёт импорта", "Waiting to import"), icon: Upload, tone: "text-muted-foreground" },
  importing: { label: () => L("Импорт", "Importing"), icon: Upload, tone: "text-primary" },
  done: { label: () => L("Готово", "Done"), icon: Check, tone: "text-success" },
  skipped: { label: () => L("Уже стоит", "Already installed"), icon: SkipForward, tone: "text-success" },
  failed: { label: () => L("Ошибка", "Failed"), icon: AlertTriangle, tone: "text-destructive" },
  cancelled: { label: () => L("Отменено", "Cancelled"), icon: X, tone: "text-muted-foreground" },
};

export function JobScreen({ onBack, onNew }: Props) {
  const { rows, summary, name: collectionName } = useJob();
  if (!summary) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        <Loader2 className="mr-2 animate-spin" /> {L("Запускаю…", "Starting…")}
      </div>
    );
  }
  return summary.finished ? (
    <DoneView rows={rows} s={summary} name={collectionName} onBack={onBack} onNew={onNew} />
  ) : (
    <ProgressView rows={rows} s={summary} name={collectionName} />
  );
}

function Stat({ label, value, tone }: { label: string; value: string; tone?: string }) {
  return (
    <div className="min-w-0">
      <div className={cn("font-display text-[22px] leading-none font-semibold tabular-nums", tone)}>{value}</div>
      <div className="mt-1.5 text-xs text-muted-foreground">{label}</div>
    </div>
  );
}

function ProgressView({ rows, s, name }: { rows: SetRow[]; s: Summary; name: string }) {
  const [filter, setFilter] = useState<"all" | "active" | "failed">("all");
  const [busy, setBusy] = useState(false);

  const downloaded = s.trimming + s.queued + s.importing + s.done;
  const trimmed = s.queued + s.importing + s.done;
  const finished = s.done + s.skipped + s.failed + s.cancelled;
  const pct = (n: number) => `${(n / Math.max(1, s.total)) * 100}%`;
  const manual = s.lazer === "manual";

  const launch = async () => {
    setBusy(true);
    try {
      await api.launchLazer();
    } catch (e) {
      toast.error(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const shown = useMemo(() => {
    if (filter === "all") return rows;
    if (filter === "failed") return rows.filter((r) => r.status === "failed");
    return rows.filter((r) => ["downloading", "trimming", "queued", "importing"].includes(r.status));
  }, [rows, filter, s]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="flex h-full flex-col">
      <header className="shrink-0 px-10 pt-7 pb-5">
        <div className="flex items-start justify-between gap-6">
          <div className="min-w-0">
            <h1 className="truncate font-display text-2xl font-semibold tracking-tight">
              {s.cancelling
                ? L("Останавливаю…", "Stopping…")
                : s.paused
                  ? L("На паузе", "Paused")
                  : manual
                    ? L("Скачиваю в папку", "Saving to folder")
                    : L("Ставлю в osu!lazer", "Installing to osu!lazer")}
            </h1>
            <p className="mt-1 truncate text-sm text-muted-foreground">
              {name}: {L(`${num(finished)} из ${num(s.total)} сетов обработано`, `${num(finished)} of ${num(s.total)} sets processed`)}
            </p>
          </div>
          <div className="flex shrink-0 gap-2">
            {s.paused ? (
              <Button variant="secondary" onClick={() => api.resumeJob()} disabled={s.cancelling}>
                <Play /> {L("Продолжить", "Resume")}
              </Button>
            ) : (
              <Button variant="secondary" onClick={() => api.pauseJob()} disabled={s.cancelling}>
                <Pause /> {L("Пауза", "Pause")}
              </Button>
            )}
            <Button variant="ghost" onClick={() => api.cancelJob()} disabled={s.cancelling}>
              <X /> {L("Отменить", "Cancel")}
            </Button>
          </div>
        </div>

        {/* segmented bar: done | in progress | failed */}
        <div className="mt-5 flex h-2.5 overflow-hidden rounded-full bg-raised" role="progressbar" aria-valuenow={finished} aria-valuemax={s.total}>
          <div className="bg-success transition-[width] duration-200" style={{ width: pct(s.done + s.skipped) }} />
          <div className="bg-primary transition-[width] duration-200" style={{ width: pct(s.downloading + s.trimming + s.queued + s.importing) }} />
          <div className="bg-destructive transition-[width] duration-200" style={{ width: pct(s.failed) }} />
        </div>

        <div className="mt-5 grid grid-cols-6 gap-4">
          <Stat label={L("скачано", "downloaded")} value={num(downloaded)} />
          <Stat label={L("вырезано", "trimmed")} value={num(trimmed)} />
          <Stat label={manual ? L("сохранено", "saved") : L("импортировано", "imported")} value={num(s.done)} tone="text-success" />
          <Stat label={L("ошибок", "failed")} value={num(s.failed)} tone={s.failed ? "text-destructive" : undefined} />
          <Stat label={L(`скорость, всего ${bytes(s.bytes)}`, `speed, ${bytes(s.bytes)} total`)} value={s.paused ? "—" : bytesPerSec(s.speedBps)} />
          <Stat label={L("осталось", "remaining")} value={s.etaSecs != null ? `~${duration(s.etaSecs)}` : "—"} />
        </div>

        {s.pauseReason && (
          <div className="mt-4 flex items-center gap-3 rounded-lg bg-warning/15 px-4 py-2.5 text-sm">
            <AlertTriangle className="size-4 shrink-0 text-warning" />
            <span className="flex-1">{s.pauseReason}</span>
          </div>
        )}
        {s.lazer === "notRunning" && (
          <div className="mt-4 flex items-center gap-3 rounded-lg bg-raised px-4 py-2.5 text-sm">
            <span className="size-2 rounded-full bg-muted-foreground/60" />
            <span className="flex-1">
              {L(
                `osu!lazer не запущен. Скачанные сеты (${num(s.queued)}) ждут, импорт продолжится, когда игра откроется.`,
                `osu!lazer is not running. Downloaded sets (${num(s.queued)}) are waiting; import resumes once the game is open.`,
              )}
            </span>
            <Button size="sm" onClick={launch} disabled={busy}>
              {busy && <Loader2 className="animate-spin" />} {L("Запустить lazer", "Start lazer")}
            </Button>
          </div>
        )}
        {s.lazer === "ready" && (
          <div className="mt-4 flex items-center gap-2 text-xs text-muted-foreground">
            <span className="size-2 rounded-full bg-success" /> {L("osu!lazer запущен, импорт работает", "osu!lazer is running, import is working")}
          </div>
        )}
      </header>

      <div className="flex shrink-0 gap-1 px-10 pb-2" role="tablist">
        {(
          [
            ["all", `${L("Все", "All")} ${num(s.total)}`],
            ["active", `${L("В работе", "Active")} ${num(s.downloading + s.trimming + s.queued + s.importing)}`],
            ["failed", `${L("Ошибки", "Failed")} ${num(s.failed)}`],
          ] as const
        ).map(([k, label]) => (
          <button
            key={k}
            role="tab"
            aria-selected={filter === k}
            onClick={() => setFilter(k)}
            className={cn(
              "rounded-full px-3 py-1 text-xs transition-colors",
              filter === k ? "bg-foreground text-background" : "text-muted-foreground hover:bg-raised",
            )}
          >
            {label}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 px-10 pb-6">
        <SetList rows={shown} />
      </div>
    </div>
  );
}

function SetList({ rows }: { rows: SetRow[] }) {
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 52,
    overscan: 10,
  });
  if (rows.length === 0) {
    return (
      <div className="flex h-full items-center justify-center rounded-xl bg-card text-sm text-muted-foreground ring-1 ring-border">
        {L("Здесь пусто", "Nothing here")}
      </div>
    );
  }
  return (
    <div ref={parentRef} className="h-full overflow-y-auto rounded-xl bg-card ring-1 ring-border">
      <div style={{ height: virtualizer.getTotalSize() }} className="relative">
        {virtualizer.getVirtualItems().map((v) => {
          const r = rows[v.index];
          const st = STATUS[r.status];
          const Icon = st.icon;
          const spinning = r.status === "trimming" || r.status === "importing";
          return (
            <div
              key={v.key}
              className="absolute inset-x-0 flex items-center gap-3 border-b border-border/60 px-4"
              style={{ height: v.size, transform: `translateY(${v.start}px)` }}
            >
              <Icon className={cn("size-4 shrink-0", st.tone, spinning && "animate-pulse")} />
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm">
                  {r.artist} – {r.title}
                </div>
                <div className={cn("truncate text-xs", r.error ? "text-destructive" : "text-muted-foreground")}>
                  {r.error
                    ? `${r.error}`
                    : r.note
                      ? `${diffs(r.diffs)} · ${r.note}`
                      : `${diffs(r.diffs)} · #${r.setId}${r.mirror && r.status === "downloading" ? ` · ${r.mirror}` : ""}`}
                </div>
              </div>
              <div className="flex w-36 shrink-0 items-center justify-end gap-2">
                {r.status === "downloading" ? (
                  <>
                    <div className="h-1.5 w-20 overflow-hidden rounded-full bg-raised">
                      <div className="h-full bg-primary transition-[width] duration-150" style={{ width: `${Math.round(r.progress * 100)}%` }} />
                    </div>
                    <span className="w-9 text-right text-xs tabular-nums text-muted-foreground">
                      {Math.round(r.progress * 100)}%
                    </span>
                  </>
                ) : (
                  <span className={cn("text-xs", st.tone)}>{st.label()}</span>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function DoneView({
  rows,
  s,
  name,
  onBack,
  onNew,
}: {
  rows: SetRow[];
  s: Summary;
  name: string;
  onBack: () => void;
  onNew: () => void;
}) {
  const failed = rows.filter((r) => r.status === "failed");
  const mismatched = rows.filter((r) => r.status === "done" && r.mismatch);
  const [showFailed, setShowFailed] = useState(failed.length > 0 && failed.length <= 8);
  const manual = s.lazer === "manual";
  const installedSets = s.done + s.skipped;
  const stopped = s.cancelled > 0;
  const off = manual && s.manualDir ? 1 : 0;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-3xl px-10 pt-10 pb-10">
        <h1 className="font-display text-[30px] leading-tight font-semibold tracking-tight">
          {stopped ? L("Остановлено", "Stopped") : failed.length ? L("Готово, но не всё", "Done, with some failures") : L("Готово", "Done")}
        </h1>
        <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
          {manual ? L("Сохранено", "Saved") : L("В osu!lazer установлено", "Installed to osu!lazer:")} {diffs(s.doneDiffs)} {L("в", "in")}{" "}
          {setsIn(installedSets)}.
          {s.skipped > 0 && <> {L("Уже были установлены раньше", "Already installed before")}: {sets(s.skipped)}.</>}
          {failed.length > 0 && <> {L("Не удалось", "Failed")}: {num(failed.length)}.</>}
          {s.cancelled > 0 && <> {L("Отменено", "Cancelled")}: {num(s.cancelled)}.</>}
        </p>

        {(failed.length > 0 || mismatched.length > 0) && (
          <div className="mt-6 rounded-xl bg-card ring-1 ring-border">
            {failed.length > 0 && (
              <div className="flex items-center gap-3 px-4 py-3">
                <AlertTriangle className="size-4 text-destructive" />
                <span className="flex-1 text-sm">
                  {L(`${num(failed.length)} с ошибками`, `${num(failed.length)} failed`)}
                </span>
                <Button variant="ghost" size="sm" onClick={() => setShowFailed((v) => !v)}>
                  {showFailed ? L("Скрыть", "Hide") : L("Показать", "Show")}
                </Button>
                <Button size="sm" variant="secondary" onClick={() => api.retryFailed()}>
                  <RotateCcw /> {L("Повторить неудачные", "Retry failed")}
                </Button>
              </div>
            )}
            {showFailed && failed.length > 0 && (
              <ul className="max-h-60 overflow-y-auto border-t px-4 py-2 text-sm">
                {failed.map((r) => (
                  <li key={r.idx} className="py-1.5">
                    <span className="selectable">
                      {r.artist} – {r.title} <span className="text-muted-foreground">#{r.setId}</span>
                    </span>
                    <div className="text-xs text-destructive">{r.error}</div>
                  </li>
                ))}
              </ul>
            )}
            {mismatched.length > 0 && (
              <div className="border-t px-4 py-3 text-sm text-muted-foreground">
                {L(
                  `Сетов поставлено целиком: ${num(mismatched.length)}. Версия на зеркале отличалась от коллекции или от вашей.`,
                  `Sets installed in full: ${num(mismatched.length)}. The mirror's version differed from the collection or from yours.`,
                )}
              </div>
            )}
          </div>
        )}

        {s.cancelled > 0 && failed.length === 0 && (
          <Button className="mt-4" size="sm" variant="secondary" onClick={() => api.retryFailed()}>
            <RotateCcw /> {L("Докачать отменённые", "Resume cancelled")}
          </Button>
        )}

        {s.exportPath ? (
          <section className="mt-10">
            <h2 className="font-display text-lg font-semibold">{L("Осталось добавить коллекцию", "One step left: add the collection")}</h2>
            <p className="mt-1.5 text-sm text-muted-foreground">
              {L("Коллекции в osu!lazer добавляет мастер настройки.", "Collections are added to osu!lazer by its setup wizard.")}{" "}
              {s.exportCollections.length > 1
                ? L(
                    `В папке ждут импорта ${num(s.exportCollections.length)} коллекции — мастер добавит все разом.`,
                    `${num(s.exportCollections.length)} collections are waiting in the folder — the wizard adds them all at once.`,
                  )
                : L(
                    "Можно сначала скачать ещё коллекции и пройти мастер один раз в конце: он добавит все.",
                    "You can download more collections first and run the wizard once at the end: it adds them all.",
                  )}
            </p>
            {s.exportCollections.length > 0 && (
              <ul className="mt-3 flex flex-wrap gap-1.5">
                {s.exportCollections.map((c) => (
                  <li
                    key={c.name}
                    className={cn(
                      "rounded-full px-3 py-1 text-xs",
                      c.name === name ? "bg-primary/15 text-foreground" : "bg-raised text-muted-foreground",
                    )}
                  >
                    {c.name} · {num(c.count)}
                  </li>
                ))}
              </ul>
            )}
            <ol className="mt-6 space-y-6">
              {manual && s.manualDir && (
                <Step n={1} title={L("Добавьте карты в osu!lazer", "Add the maps to osu!lazer")}>
                  <p>{L("Откройте папку и перетащите все .osz в окно игры.", "Open the folder and drag all .osz files into the game window.")}</p>
                  <div className="mt-3">
                    <Button variant="secondary" size="sm" onClick={() => api.openFolder(s.manualDir!)}>
                      <FolderOpen /> {L("Открыть папку с .osz", "Open .osz folder")}
                    </Button>
                  </div>
                </Step>
              )}
              <Step n={off + 1} title={L("Откройте мастер в osu!lazer", "Open the wizard in osu!lazer")}>
                <p>
                  {L("Настройки → Общие →", "Settings → General →")} <b>Run setup wizard</b>{" "}
                  {L("(мастер первоначальной настройки) → шаг", "→ the")} <b>{L("Импорт", "Import")}</b>
                  {L(".", " step.")}
                </p>
              </Step>
              <Step n={off + 2} title={L("Выберите папку с коллекциями", "Pick the collections folder")}>
                <p>
                  {L(
                    "Нажмите на поле «предыдущая директория osu!» и пройдите в окне выбора по папкам:",
                    "Click \"previous osu! install\" and go through these folders in the picker:",
                  )}
                </p>
                <PathSteps path={s.exportPath} />
                <div className="mt-3">
                  <Button size="sm" variant="secondary" onClick={() => api.openFolder(s.exportPath!)}>
                    <FolderOpen /> {L("Показать папку в проводнике", "Show in Explorer")}
                  </Button>
                </div>
              </Step>
              <Step n={off + 3} title={L("Оставьте только «Коллекции»", "Keep only \"Collections\"")}>
                <p>
                  {L("Выключите карты, рекорды и скины, оставьте", "Turn off beatmaps, scores and skins, keep")}{" "}
                  <b>{L("Коллекции", "Collections")}</b> {L("и нажмите", "and press")}{" "}
                  <b>{L("Импортировать из предыдущей версии", "Import content from previous version")}</b>.{" "}
                  {L(
                    "Коллекции с такими же именами дополнятся новыми картами: старые не пропадут и не задвоятся.",
                    "Collections with the same name get the new maps added: nothing is lost or duplicated.",
                  )}
                </p>
              </Step>
            </ol>
          </section>
        ) : (
          <p className="mt-8 text-sm text-muted-foreground">
            {L("Коллекцию создавать не из чего: ни одна карта не установлена.", "No collection to create: no maps were installed.")}
          </p>
        )}

        <div className="mt-10 flex gap-2">
          <Button variant="secondary" onClick={onBack}>
            {L("К коллекции", "Back to collection")}
          </Button>
          <Button variant="ghost" onClick={onNew}>
            {L("Другая коллекция", "Another collection")}
          </Button>
        </div>
      </div>
    </div>
  );
}

/** A Windows path as a sequence of folders to click through in lazer's folder browser. */
function PathSteps({ path }: { path: string }) {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return (
    <div className="selectable mt-2 flex flex-wrap items-center gap-1 text-[13px]">
      {parts.map((p, i) => (
        <span key={i} className="flex items-center gap-1">
          {i > 0 && <span className="text-muted-foreground/60">›</span>}
          <span className={cn("rounded-md px-2 py-0.5", i === parts.length - 1 ? "bg-primary/15 font-medium text-foreground" : "bg-raised")}>
            {p}
          </span>
        </span>
      ))}
    </div>
  );
}

function Step({ n, title, children }: { n: number; title: string; children: React.ReactNode }) {
  return (
    <li className="flex gap-4">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-primary font-display text-sm font-semibold text-primary-foreground">
        {n}
      </span>
      <div className="min-w-0 flex-1 pt-1 text-sm leading-relaxed">
        <div className="mb-1 font-medium text-foreground">{title}</div>
        <div className="text-muted-foreground">{children}</div>
      </div>
    </li>
  );
}
