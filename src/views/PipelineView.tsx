// PipelineView — inspect the real local agent server, task queue, audit
// namespace, and deterministic three-way merge engine.

import { useState } from "react";
import {
  agentServe,
  agentWorkers,
  auditCodes,
  revealLogs,
  runDoctor,
  taskStatus,
  versionMerge,
} from "../api";
import type { DoctorCheck, MergeSummary } from "../types";
import type { Lang } from "../i18n";

export function PipelineView({
  pid,
  embedded = false,
  lang = "en",
}: {
  pid: string | null;
  embedded?: boolean;
  lang?: Lang;
}) {
  const [port, setPort] = useState<number | null>(null);
  const [workers, setWorkers] = useState<unknown[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [checks, setChecks] = useState<DoctorCheck[] | null>(null);

  const startServer = async () => {
    setBusy(true);
    setErr(null);
    setInfo(null);
    try {
      const p = await agentServe(null);
      setPort(p);
      setWorkers(await agentWorkers());
      setInfo(`agent server bound on 127.0.0.1:${p}`);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const refreshRuntime = async () => {
    setBusy(true);
    setErr(null);
    try {
      setWorkers(await agentWorkers());
      const status = pid ? await taskStatus(pid) : null;
      setInfo(
        status
          ? `project ${pid}: pending=${status.pending} done=${status.done}`
          : "worker pool refreshed; select a project to inspect its task queue",
      );
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  // ---- deterministic 3-way merge inspector ----

  const [baseText, setBaseText] = useState("{}");
  const [oursText, setOursText] = useState("{}");
  const [theirsText, setTheirsText] = useState("{}");
  const [merge, setMerge] = useState<MergeSummary | null>(null);
  const [codes, setCodes] = useState<string[] | null>(null);

  const runMerge = async () => {
    setBusy(true);
    setErr(null);
    try {
      const out = await versionMerge(
        JSON.parse(baseText),
        JSON.parse(oursText),
        JSON.parse(theirsText),
      );
      setMerge(out);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const loadCodes = async () => {
    setBusy(true);
    try {
      setCodes(await auditCodes());
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const checkEnvironment = async () => {
    setBusy(true);
    setErr(null);
    try {
      setChecks(await runDoctor());
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const openLogs = async () => {
    setBusy(true);
    setErr(null);
    try {
      const path = await revealLogs();
      setInfo(lang === "zh" ? `日志目录：${path}` : `Log folder: ${path}`);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (embedded) {
    const copy = lang === "zh"
      ? {
          title: "环境检查",
          description: "Pipeline 和后台任务会在需要时自动启动，不需要手动启动服务器。",
          checking: "正在检查…",
          action: "运行环境检查",
          logs: "打开运行日志",
        }
      : {
          title: "Environment check",
          description: "Pipeline and background tasks start automatically when needed. You do not need to start a server.",
          checking: "Checking…",
          action: "Run environment check",
          logs: "Open runtime logs",
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
          {err && <pre className="out error">{err}</pre>}
          {info && <pre className="out">{info}</pre>}
        </div>
      </section>
    );
  }

  return (
    <section className={`view pipeline-view${embedded ? " embedded" : ""}`}>
      {!embedded && <h2>Pipeline</h2>}

      <div className="card">
        <h3>Agent server</h3>
        <p className="muted">
          spawns the local axum server on 127.0.0.1 with <code>/agent/next</code>,
          <code>/agent/submit</code>, <code>/agent/submit-next</code>,{" "}
          <code>/healthz</code>.
        </p>
        <div className="row">
          <button disabled={busy} onClick={startServer}>
            start server
          </button>
          {port && <code>127.0.0.1:{port}</code>}
          <button disabled={busy || !port} onClick={refreshRuntime}>
            refresh workers / queue
          </button>
        </div>
        {workers && <pre className="out">{JSON.stringify(workers, null, 2)}</pre>}
        {info && <pre className="out">{info}</pre>}
        {err && <pre className="out error">{err}</pre>}
      </div>

      <div className="card">
        <h3>3-way merge inspector</h3>
        <p className="muted">
          paste three JSON maps of <code>{`{cue_id: text}`}</code>. The
          algorithm picks winners deterministically.
        </p>
        <div className="grid">
          <label>base<textarea rows={6} value={baseText} onChange={(e) => setBaseText(e.target.value)} /></label>
          <label>ours<textarea rows={6} value={oursText} onChange={(e) => setOursText(e.target.value)} /></label>
          <label>theirs<textarea rows={6} value={theirsText} onChange={(e) => setTheirsText(e.target.value)} /></label>
        </div>
        <button disabled={busy} onClick={runMerge}>
          merge
        </button>
        {merge && (
          <pre className="out">
            merged: {JSON.stringify(merge.merged, null, 2)}
            {"\n"}conflicts: {merge.conflicts.length}
            {merge.conflicts.map((c) => (
              <div key={c.cue_id}>
                · {c.cue_id}: base={c.base} ours={c.ours} theirs={c.theirs}
              </div>
            ))}
          </pre>
        )}
      </div>

      <div className="card">
        <h3>Audit codes</h3>
        <p className="muted">
          the delivery checks used by the CLI and review workflow. The same checks run in{" "}
          <code>lumen-cut audit</code>.
        </p>
        <div className="row">
          <button disabled={busy} onClick={loadCodes}>
            load codes
          </button>
          {codes && (
            <ul className="code-list">
              {codes.map((c) => (
                <li key={c}><code>{c}</code></li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </section>
  );
}
