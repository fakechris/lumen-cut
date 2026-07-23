import { useState } from "react";
import { revealLogs, runDoctor } from "../api";
import type { Lang } from "../i18n";
import type { DoctorCheck } from "../types";

/// Read-only diagnostics for advanced troubleshooting. Runtime services are
/// intentionally absent: normal product features own their workers and start
/// them on demand.
export function PipelineView({ lang = "en" }: { lang?: Lang }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [checks, setChecks] = useState<DoctorCheck[] | null>(null);
  const copy = lang === "zh"
    ? {
        title: "环境检查",
        description: "后台任务会在功能需要时自动启动。这里仅用于检查依赖和查看日志，不需要手动启动 Pipeline 或服务器。",
        checking: "正在检查…",
        action: "运行环境检查",
        logs: "打开运行日志",
      }
    : {
        title: "Environment check",
        description: "Background tasks start when a feature needs them. This area only checks dependencies and opens logs; there is no Pipeline or server to start manually.",
        checking: "Checking…",
        action: "Run environment check",
        logs: "Open runtime logs",
      };

  const checkEnvironment = async () => {
    setBusy(true);
    setError(null);
    try {
      setChecks(await runDoctor());
    } catch (nextError) {
      setError(String(nextError));
    } finally {
      setBusy(false);
    }
  };

  const openLogs = async () => {
    setBusy(true);
    setError(null);
    try {
      const path = await revealLogs();
      setInfo(lang === "zh" ? `日志目录：${path}` : `Log folder: ${path}`);
    } catch (nextError) {
      setError(String(nextError));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="view pipeline-view embedded">
      <div className="card diagnostics-card">
        <h3>{copy.title}</h3>
        <p className="muted">{copy.description}</p>
        <div className="row">
          <button disabled={busy} onClick={checkEnvironment}>
            {busy ? copy.checking : copy.action}
          </button>
          <button className="button-quiet" disabled={busy} onClick={openLogs}>
            {copy.logs}
          </button>
        </div>
        {checks && (
          <ul className="diagnostic-list">
            {checks.map((check) => (
              <li key={check.name} className={check.ok ? "passed" : "failed"}>
                <span>{check.ok ? "✓" : "×"}</span>
                <strong>{check.name}</strong>
                <small>{check.detail}</small>
              </li>
            ))}
          </ul>
        )}
        {error && <pre className="out error">{error}</pre>}
        {info && <pre className="out">{info}</pre>}
      </div>
    </section>
  );
}
