import type { Lang } from "../i18n";

type Props = {
  state: string;
  phase: string;
  updatedAt?: number | null;
  lang: Lang;
};

export function PipelineFreshness({ state, phase, updatedAt, lang }: Props) {
  if (!["running", "cancelling"].includes(state) || phase === "waiting" || !updatedAt) {
    return null;
  }
  const age = Math.max(0, Date.now() / 1000 - updatedAt);
  if (age < 45) return null;
  const stale = age >= 120;
  const duration = stale
    ? (lang === "zh"
      ? `${Math.max(2, Math.round(age / 60))} 分钟`
      : `${Math.max(2, Math.round(age / 60))} min`)
    : (lang === "zh" ? `${Math.round(age)} 秒` : `${Math.round(age)} sec`);
  return (
    <small className={`pipeline-freshness${stale ? " stale" : ""}`} role="status">
      {lang === "zh"
        ? `${duration}没有收到新进度。任务仍在监测；如果资源占用也停止，可安全取消后重试。`
        : `No progress update for ${duration}. The task is still monitored; if resource use also stops, cancel safely and retry.`}
    </small>
  );
}
