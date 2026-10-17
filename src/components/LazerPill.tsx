import { useEffect, useState } from "react";
import { api, type LazerStatus } from "@/lib/api";
import { cn } from "@/lib/utils";
import { L } from "@/lib/i18n";

export function useLazerStatus(intervalMs = 3000) {
  const [status, setStatus] = useState<LazerStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const tick = () =>
      api
        .lazerStatus()
        .then((s) => alive && setStatus(s))
        .catch(() => {});
    tick();
    const t = setInterval(tick, intervalMs);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [intervalMs]);
  return status;
}

export function LazerPill({ status }: { status: LazerStatus | null }) {
  const [label, tone] = !status
    ? ["osu!lazer…", "idle"]
    : !status.found
      ? [L("osu!lazer не найден", "osu!lazer not found"), "bad"]
      : status.running
        ? [L("osu!lazer запущен", "osu!lazer is running"), "ok"]
        : [L("osu!lazer не запущен", "osu!lazer is not running"), "idle"];
  return (
    <span
      className="inline-flex h-7 items-center gap-2 rounded-full bg-raised px-3 text-xs text-muted-foreground"
      title={status?.path ?? undefined}
    >
      <span
        className={cn(
          "size-2 rounded-full",
          tone === "ok" && "bg-success",
          tone === "bad" && "bg-destructive",
          tone === "idle" && "bg-muted-foreground/50",
        )}
      />
      {label}
    </span>
  );
}
