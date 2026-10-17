import { useEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { DbImport } from "@/components/DbImport";
import { api, errorText, type CollectionView, type LoadProgress, type RecentEntry } from "@/lib/api";
import { ago, diffs, num } from "@/lib/format";
import { L } from "@/lib/i18n";

interface Props {
  onOpen: (view: CollectionView) => void;
}

export function useCollectionLoader(onOpen: (v: CollectionView) => void) {
  const [loading, setLoading] = useState(false);
  const [progress, setProgress] = useState<LoadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = async (input: string, refresh = false) => {
    setLoading(true);
    setError(null);
    setProgress(null);
    try {
      onOpen(await api.loadCollection(input, refresh, setProgress));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoading(false);
    }
  };
  return { load, loading, progress, error };
}

export function progressText(p: LoadProgress | null): string {
  if (!p || p.stage === "info") return L("Загружаю коллекцию…", "Loading collection…");
  if (p.loaded > 0 && p.loaded < p.total) return L(`Загружаю список карт: ${num(p.loaded)} из ${num(p.total)}`, `Loading maps: ${num(p.loaded)} of ${num(p.total)}`);
  return p.total ? L(`Загружаю ${diffs(p.total)}…`, `Loading ${diffs(p.total)}…`) : L("Загружаю список карт…", "Loading maps…");
}

export function InputScreen({ onOpen }: Props) {
  const [value, setValue] = useState("");
  const [recent, setRecent] = useState<RecentEntry[]>([]);
  const { load, loading, progress, error } = useCollectionLoader(onOpen);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.recentCollections().then(setRecent);
    inputRef.current?.focus();
  }, []);

  const submit = () => {
    if (value.trim() && !loading) load(value);
  };

  return (
    <div className="mx-auto flex h-full w-full max-w-4xl flex-col px-10 pt-14 pb-8">
      <h1 className="font-display text-[34px] leading-[1.15] font-semibold tracking-tight">
        {L("Какую коллекцию ставим?", "Which collection are we installing?")}
      </h1>
      <p className="mt-3 max-w-xl text-[15px] leading-relaxed text-muted-foreground">
        {L(
          "Вставьте ссылку с osucollector.com. Покажу, сколько в ней карт на каждую звезду, а в osu!lazer поставлю только нужные сложности.",
          "Paste an osucollector.com link. You'll see how many maps it has per star rating, and only the difficulties you pick go into osu!lazer.",
        )}
      </p>

      <form
        className="mt-8"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <label htmlFor="collection" className="mb-2 block text-sm font-medium">
          {L("Ссылка или ID коллекции", "Collection link or ID")}
        </label>
        <div className="flex gap-3">
          <input
            id="collection"
            ref={inputRef}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            className="h-14 min-w-0 flex-1 rounded-xl border border-input bg-card px-5 text-lg outline-none transition-[border-color,box-shadow] duration-150 focus:border-primary focus:ring-4 focus:ring-primary/20"
            aria-invalid={!!error}
            aria-describedby={error ? "collection-error" : undefined}
          />
          <Button type="submit" className="h-14 rounded-xl px-8 text-base" disabled={!value.trim() || loading}>
            {loading ? <Loader2 className="animate-spin" /> : null}
            {L("Открыть", "Open")}
          </Button>
        </div>
        <div className="mt-3 min-h-6 text-sm" aria-live="polite">
          {loading ? (
            <span className="text-muted-foreground">{progressText(progress)}</span>
          ) : error ? (
            <span id="collection-error" className="text-destructive">
              {error}
            </span>
          ) : (
            <span className="text-muted-foreground">
              {L("Например", "For example")}: osucollector.com/collections/44/speed-practice {L("или просто", "or just")} 44
            </span>
          )}
        </div>
      </form>

      <DbImport onOpen={onOpen} disabled={loading} />

      {recent.length > 0 && (
        <section className="mt-8 min-h-0 flex-1 overflow-y-auto pr-1">
          <h2 className="mb-3 font-display text-sm font-semibold text-muted-foreground">{L("Недавние", "Recent")}</h2>
          <ul className="grid grid-cols-3 gap-3">
            {recent.map((r) => (
              <li key={r.id}>
                <button
                  type="button"
                  disabled={loading}
                  onClick={() => load(String(r.id))}
                  className="group block w-full overflow-hidden rounded-xl bg-card text-left ring-1 ring-border transition-[box-shadow,transform] duration-150 hover:ring-primary/60 focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.99]"
                >
                  <div
                    className="h-20 bg-raised bg-cover bg-center"
                    style={r.cover ? { backgroundImage: `url("${r.cover}")` } : undefined}
                  />
                  <div className="px-3.5 py-3">
                    <div className="truncate font-medium">{r.name || `${L("Коллекция", "Collection")} ${r.id}`}</div>
                    <div className="mt-0.5 truncate text-xs text-muted-foreground">
                      {r.uploader} · {diffs(r.diffs)}
                    </div>
                    <div className="mt-1 text-[11px] text-muted-foreground/80">{L("Загружена", "Loaded")} {ago(r.fetchedAt)}</div>
                  </div>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
