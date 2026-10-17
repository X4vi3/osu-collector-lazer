import { useEffect, useState, type ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getVersion } from "@tauri-apps/api/app";
import { ArrowDown, ArrowUp } from "lucide-react";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Slider } from "@/components/ui/slider";
import { api, errorText, type ExportInfo, type Mirror } from "@/lib/api";
import { useSettings } from "@/hooks/settings";
import { sets } from "@/lib/format";
import { L } from "@/lib/i18n";
import { cn } from "@/lib/utils";

const MIRROR_LABEL: Record<Mirror, string> = {
  osudirect: "osu.direct",
  catboy: "catboy.best",
  sayobot: "sayobot.cn",
  nerinyan: "nerinyan.moe",
};
const ALL_MIRRORS: Mirror[] = ["osudirect", "catboy", "sayobot", "nerinyan"];

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="space-y-3">
      <h3 className="font-display text-sm font-semibold">{title}</h3>
      {children}
    </section>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start justify-between gap-4 rounded-lg px-1 py-1">
      <span>
        <span className="block text-sm">{label}</span>
        {hint && <span className="mt-0.5 block text-xs text-muted-foreground">{hint}</span>}
      </span>
      <Switch checked={checked} onCheckedChange={onChange} className="mt-0.5" />
    </label>
  );
}

export function SettingsDialog({ open: isOpen, onOpenChange }: { open: boolean; onOpenChange: (v: boolean) => void }) {
  const { settings, update } = useSettings();
  const [detected, setDetected] = useState<string[]>([]);
  const [historySize, setHistorySize] = useState(0);
  const [confirmHistory, setConfirmHistory] = useState(false);
  const [exportInfo, setExportInfo] = useState<ExportInfo | null>(null);
  const [version, setVersion] = useState("");

  useEffect(() => {
    if (!isOpen) return;
    api.detectedLazerPaths().then(setDetected);
    api.historySize().then(setHistorySize);
    api.exportInfo().then(setExportInfo);
    getVersion().then(setVersion);
    setConfirmHistory(false);
  }, [isOpen, settings?.exportDir]);

  if (!settings) return null;

  const enabled = settings.mirrors;
  const order = [...enabled, ...ALL_MIRRORS.filter((m) => !enabled.includes(m))];

  const move = (m: Mirror, dir: -1 | 1) => {
    const list = [...enabled];
    const i = list.indexOf(m);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
    update({ mirrors: list });
  };
  const toggleMirror = (m: Mirror, on: boolean) =>
    update({ mirrors: on ? [...enabled, m] : enabled.filter((x) => x !== m) });

  const pickExe = async () => {
    const path = await open({
      title: L("Где лежит osu!.exe (lazer)", "Where is osu!.exe (lazer)"),
      multiple: false,
      directory: false,
      filters: [{ name: "osu!.exe", extensions: ["exe"] }],
    });
    if (typeof path === "string") update({ lazerPath: path });
  };

  const effectivePath = settings.lazerPath ?? detected[0] ?? null;

  const pickExportDir = async () => {
    const dir = await open({ directory: true, title: L("Папка для коллекций osu!lazer", "Folder for osu!lazer collections") });
    if (typeof dir === "string") update({ exportDir: dir }).catch((e) => toast.error(errorText(e)));
  };

  return (
    <Dialog open={isOpen} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle className="font-display">{L("Настройки", "Settings")}</DialogTitle>
          <DialogDescription>
            {L("Сохраняются сразу.", "Changes are saved immediately.")} {L("Версия", "Version")} {version}.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-7">
          <Section title="osu!lazer">
            <div className="rounded-lg bg-raised px-3 py-2.5 text-sm">
              {effectivePath ? (
                <span className="selectable break-all">{effectivePath}</span>
              ) : (
                <span className="text-destructive">{L("Не найден. Укажите osu!.exe вручную.", "Not found. Pick osu!.exe manually.")}</span>
              )}
              <span className="mt-1 block text-xs text-muted-foreground">
                {settings.lazerPath ? L("Выбран вручную", "Picked manually") : effectivePath ? L("Найден автоматически", "Found automatically") : ""}
              </span>
            </div>
            <div className="flex gap-2">
              <Button variant="secondary" size="sm" onClick={pickExe}>
                {L("Выбрать osu!.exe…", "Pick osu!.exe…")}
              </Button>
              {settings.lazerPath && (
                <Button variant="ghost" size="sm" onClick={() => update({ lazerPath: null })}>
                  {L("Искать автоматически", "Detect automatically")}
                </Button>
              )}
            </div>
          </Section>

          <Section title={L("Зеркала", "Mirrors")}>
            <p className="text-xs text-muted-foreground">
              {L(
                "Качаем со всех включённых зеркал сразу. Порядок — какое зеркало занимать первым; если на одном карты нет, берём с другого.",
                "All enabled mirrors are used at once. The order decides which one is used first; if a map is missing on one, another is tried.",
              )}
            </p>
            <ul className="space-y-1.5">
              {order.map((m) => {
                const on = enabled.includes(m);
                const i = enabled.indexOf(m);
                return (
                  <li key={m} className={cn("flex items-center gap-3 rounded-lg bg-raised px-3 py-2", !on && "opacity-60")}>
                    <Switch checked={on} onCheckedChange={(v) => toggleMirror(m, v)} aria-label={MIRROR_LABEL[m]} />
                    <span className="flex-1 text-sm">{MIRROR_LABEL[m]}</span>
                    {on && (
                      <span className="flex gap-1">
                        <Button variant="ghost" size="icon-sm" disabled={i === 0} onClick={() => move(m, -1)} aria-label={L("Выше", "Move up")}>
                          <ArrowUp />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          disabled={i === enabled.length - 1}
                          onClick={() => move(m, 1)}
                          aria-label={L("Ниже", "Move down")}
                        >
                          <ArrowDown />
                        </Button>
                      </span>
                    )}
                  </li>
                );
              })}
            </ul>
          </Section>

          <Section title={L("Загрузка", "Downloads")}>
            <div className="flex items-center gap-4">
              <span className="w-44 text-sm">{L("Одновременно", "At once")}: {settings.concurrency}</span>
              <Slider
                min={1}
                max={16}
                step={1}
                value={[settings.concurrency]}
                onValueChange={([v]) => update({ concurrency: v })}
                className="flex-1"
                aria-label={L("Одновременных загрузок", "Simultaneous downloads")}
              />
            </div>
            <Toggle
              label={L("Хранить .osz", "Keep .osz files")}
              hint={L("Копия каждого урезанного сета остаётся в папке приложения.", "A copy of every trimmed set stays in the app folder.")}
              checked={settings.keepOsz}
              onChange={(v) => update({ keepOsz: v })}
            />
          </Section>

          <Section title={L("Сложности", "Difficulties")}>
            <Toggle
              label={L("Не качать то, что уже есть в osu!lazer", "Skip what osu!lazer already has")}
              hint={L("Приложение читает папку с картами lazer (только чтение, база игры не открывается). Уже стоящие сложности пропускаются, а у частично установленных сетов ничего не пропадёт.", "Reads lazer's beatmap files (read-only, the game database is never opened). Installed difficulties are skipped, and partially installed sets keep everything they have.")}
              checked={settings.checkInstalled}
              onChange={(v) => update({ checkInstalled: v })}
            />
            <Toggle
              label={L("Не вырезать сложности", "Don't trim difficulties")}
              hint={L("Ставить сеты целиком.", "Install whole sets.")}
              checked={settings.noTrim}
              onChange={(v) => update({ noTrim: v })}
            />
            <Toggle
              label={L("Версия на зеркале отличается — ставить сет целиком", "Mirror has a different version — install the whole set")}
              hint={L("Если на зеркале карта обновлена и нужных сложностей в ней нет. Иначе сет попадёт в ошибки.", "When the map was updated on the mirror and the wanted difficulties are gone. Otherwise the set is reported as failed.")}
              checked={settings.fullOnMismatch}
              onChange={(v) => update({ fullOnMismatch: v })}
            />
          </Section>

          <Section title={L("Коллекции для osu!lazer", "Collections for osu!lazer")}>
            <p className="text-xs text-muted-foreground">
              {L(
                "Сюда складываются все скачанные коллекции. Эту папку выбираете в мастере lazer — он добавит их разом.",
                "Every downloaded collection goes here. Pick this folder in lazer's setup wizard and it adds them all at once.",
              )}
            </p>
            <div className="rounded-lg bg-raised px-3 py-2.5 text-sm">
              <span className="selectable break-all">{exportInfo?.dir}</span>
              <span className="mt-1 block text-xs text-muted-foreground">
                {exportInfo && exportInfo.collections.length > 0
                  ? `${L("Ждут импорта", "Waiting for import")}: ${exportInfo.collections.map((c) => c.name).join(", ")}`
                  : L("Пока пусто", "Empty for now")}
              </span>
            </div>
            <div className="flex flex-wrap gap-2">
              <Button variant="secondary" size="sm" onClick={pickExportDir}>
                {L("Выбрать папку…", "Pick folder…")}
              </Button>
              {settings.exportDir && (
                <Button variant="ghost" size="sm" onClick={() => update({ exportDir: null })}>
                  {L("По умолчанию", "Default")}
                </Button>
              )}
              <Button
                variant="ghost"
                size="sm"
                disabled={!exportInfo?.collections.length}
                onClick={() =>
                  api.clearExport().then(() => {
                    api.exportInfo().then(setExportInfo);
                    toast.success(L("Список коллекций очищен", "Collection list cleared"));
                  })
                }
              >
                {L("Очистить список", "Clear list")}
              </Button>
            </div>
          </Section>

          <Section title={L("Данные", "Data")}>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="secondary"
                size="sm"
                onClick={() =>
                  api
                    .clearCache()
                    .then(() => toast.success(L("Кэш коллекций очищен", "Collection cache cleared")))
                    .catch((e) => toast.error(errorText(e)))
                }
              >
                {L("Очистить кэш коллекций", "Clear collection cache")}
              </Button>
              {confirmHistory ? (
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={() =>
                    api.clearHistory().then(() => {
                      setHistorySize(0);
                      setConfirmHistory(false);
                      toast.success(L("История импортов очищена", "Import history cleared"));
                    })
                  }
                >
                  {L("Точно очистить?", "Really clear?")}
                </Button>
              ) : (
                <Button variant="secondary" size="sm" disabled={historySize === 0} onClick={() => setConfirmHistory(true)}>
                  {L("Очистить историю импортов", "Clear import history")}
                </Button>
              )}
            </div>
            <p className="text-xs text-muted-foreground">
              {L("История помнит, какие сложности ставило приложение", "History remembers which difficulties this app installed")}:{" "}
              {sets(historySize)}.
            </p>
          </Section>
        </div>
      </DialogContent>
    </Dialog>
  );
}
