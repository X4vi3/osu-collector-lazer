import { motion } from "motion/react";
import { Check } from "lucide-react";
import type { Bucket } from "@/lib/api";
import { bucketColor, bucketLabel, starTextColor } from "@/lib/stars";
import { diffs, num, sets } from "@/lib/format";
import { L } from "@/lib/i18n";
import { cn } from "@/lib/utils";

interface Props {
  buckets: Bucket[];
  selected: Set<number>;
  onToggle: (star: number) => void;
}

/** One bar per whole star, coloured with lazer's difficulty spectrum. Click toggles a bucket. */
export function Histogram({ buckets, selected, onToggle }: Props) {
  const max = Math.max(1, ...buckets.map((b) => b.diffs));
  return (
    <div className="flex h-full min-h-0 items-stretch gap-2" role="group" aria-label={L("Диффы по звёздам", "Difficulties by star rating")}>
      {buckets.map((b, i) => {
        const empty = b.diffs === 0;
        const on = selected.has(b.star);
        const color = bucketColor(b.star);
        const h = empty ? 0 : Math.max(2.5, (b.diffs / max) * 100);
        return (
          <button
            key={b.star}
            type="button"
            disabled={empty}
            aria-pressed={on}
            aria-label={`${bucketLabel(b.star)}: ${diffs(b.diffs)}`}
            onClick={() => onToggle(b.star)}
            className={cn(
              "group relative flex min-w-0 flex-1 flex-col items-center rounded-lg px-1 pt-1 pb-2 outline-none transition-colors duration-150",
              "focus-visible:ring-2 focus-visible:ring-ring",
              empty ? "cursor-default" : "cursor-pointer hover:bg-raised",
              on && "bg-raised",
            )}
          >
            <div className="flex w-full flex-1 flex-col items-center justify-end">
              <span
                className={cn(
                  "font-display text-[13px] font-semibold leading-none tabular-nums transition-colors",
                  empty ? "text-muted-foreground/40" : on ? "text-foreground" : "text-muted-foreground",
                )}
              >
                {empty ? "—" : num(b.diffs)}
              </span>
              {!empty && (
                <span className="mt-1 text-[11px] leading-none text-muted-foreground">
                  {sets(b.sets)}
                </span>
              )}
              <div className="relative mt-2 flex w-full flex-1 items-end justify-center">
                <motion.div
                  initial={{ height: 0 }}
                  animate={{ height: `${h}%` }}
                  transition={{ duration: 0.45, delay: i * 0.025, ease: [0.2, 0.8, 0.2, 1] }}
                  className="relative w-full max-w-14 rounded-t-md"
                  style={{
                    background: on ? color : `color-mix(in oklab, ${color} 28%, transparent)`,
                    boxShadow: on
                      ? `inset 0 0 0 1px color-mix(in oklab, ${color} 70%, white 30%)`
                      : `inset 0 0 0 1px color-mix(in oklab, ${color} 55%, var(--border))`,
                  }}
                />
              </div>
            </div>
            <span className="mt-2 flex items-center gap-1">
              <span
                className={cn(
                  "rounded-full px-2 py-0.5 font-display text-[11px] font-semibold leading-tight transition-opacity",
                  empty && "opacity-35",
                )}
                style={{ background: color, color: starTextColor(b.star + 0.5) }}
              >
                {bucketLabel(b.star)}
              </span>
              {on && (
                <span className="flex size-[18px] items-center justify-center rounded-full bg-primary text-primary-foreground">
                  <Check className="size-3" strokeWidth={3.5} />
                </span>
              )}
            </span>
          </button>
        );
      })}
    </div>
  );
}
