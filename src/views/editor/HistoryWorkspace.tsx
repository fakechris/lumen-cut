import { useState } from "react";
import type { Lang } from "../../i18n";
import type { VersionHistory } from "../../types";

interface Props {
  busy: boolean;
  history: VersionHistory;
  lang: Lang;
  onCommit: (name: string, note: string) => Promise<void>;
  onCreateBranch: (name: string) => Promise<void>;
  onRestore: (id: string) => Promise<void>;
  onSwitchBranch: (id: string) => Promise<void>;
}

export function HistoryWorkspace({
  busy,
  history,
  lang,
  onCommit,
  onCreateBranch,
  onRestore,
  onSwitchBranch,
}: Props) {
  const [name, setName] = useState("");
  const [note, setNote] = useState("");
  const [branchName, setBranchName] = useState("");
  const [restoreId, setRestoreId] = useState<string | null>(null);
  const zh = lang === "zh";
  const versions = [...history.versions].reverse();
  const activeBranch = history.activeBranch || "main";

  const commit = async () => {
    if (!name.trim()) return;
    try {
      await onCommit(name.trim(), note.trim());
      setName("");
      setNote("");
    } catch {
      // The parent surfaces the error; keep the draft so the user can retry.
    }
  };

  const createBranch = async () => {
    if (!branchName.trim()) return;
    try {
      await onCreateBranch(branchName.trim());
      setBranchName("");
    } catch {
      // The parent surfaces the error; keep the draft so the user can retry.
    }
  };

  const restore = async (id: string) => {
    try {
      await onRestore(id);
      setRestoreId(null);
    } catch {
      // Keep the confirmation open after a failed restore so it can be retried.
    }
  };

  return (
    <div className="history-workspace">
      <section className="history-create" aria-labelledby="history-title">
        <p className="eyebrow">{zh ? "可恢复编辑" : "Recoverable editing"}</p>
        <h2 id="history-title">{zh ? "版本与分支" : "Versions & branches"}</h2>
        <p>
          {zh
            ? "在大改、重新转写或批量处理前保存一个版本。恢复只替换当前项目，不会删除历史。"
            : "Save a version before major edits, retranscription, or batch processing. Restore replaces the working project without deleting history."}
        </p>
        <label>
          <span>{zh ? "版本名称" : "Version name"}</span>
          <input
            placeholder={zh ? "例如：校对完成" : "e.g. Review complete"}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </label>
        <label>
          <span>{zh ? "备注（可选）" : "Note (optional)"}</span>
          <textarea
            placeholder={zh ? "记录这次修改的目的" : "What changed in this version?"}
            rows={3}
            value={note}
            onChange={(event) => setNote(event.target.value)}
          />
        </label>
        <button
          className="button-primary"
          disabled={busy || !name.trim()}
          onClick={() => void commit()}
        >
          {busy ? (zh ? "正在保存…" : "Saving…") : (zh ? "保存当前版本" : "Save current version")}
        </button>
      </section>

      <section className="history-list" aria-labelledby="version-list-title">
        <header>
          <div>
            <h2 id="version-list-title">{zh ? "版本历史" : "Version history"}</h2>
            <p>{zh ? `当前分支：${activeBranch}` : `Active branch: ${activeBranch}`}</p>
          </div>
          <span>{history.versions.length}</span>
        </header>
        {versions.length === 0 ? (
          <div className="history-empty">
            <strong>{zh ? "还没有保存版本" : "No saved versions yet"}</strong>
            <p>{zh ? "先保存当前状态，之后就可以安全恢复或创建分支。" : "Save the current state before restoring or creating a branch."}</p>
          </div>
        ) : (
          <div className="version-rows">
            {versions.map((version) => {
              const isHead = version.id === history.head;
              const confirming = restoreId === version.id;
              return (
                <article className={isHead ? "current" : ""} key={version.id}>
                  <div className="version-marker" aria-hidden="true" />
                  <div>
                    <strong>{version.name || version.id}</strong>
                    <small>
                      {new Intl.DateTimeFormat(zh ? "zh-CN" : "en-US", {
                        dateStyle: "medium",
                        timeStyle: "short",
                      }).format(new Date(version.at))}
                      {` · ${version.branch}`}
                    </small>
                    {version.note && <p>{version.note}</p>}
                  </div>
                  {isHead ? (
                    <span className="version-head">{zh ? "当前" : "Current"}</span>
                  ) : confirming ? (
                    <div className="restore-confirm">
                      <span>{zh ? "恢复到这里？" : "Restore this version?"}</span>
                      <button className="button-quiet" disabled={busy} onClick={() => setRestoreId(null)}>
                        {zh ? "取消" : "Cancel"}
                      </button>
                      <button
                        className="button-danger"
                        disabled={busy}
                        onClick={() => void restore(version.id)}
                      >
                        {zh ? "确认恢复" : "Restore"}
                      </button>
                    </div>
                  ) : (
                    <button className="button-quiet" disabled={busy} onClick={() => setRestoreId(version.id)}>
                      {zh ? "恢复" : "Restore"}
                    </button>
                  )}
                </article>
              );
            })}
          </div>
        )}

        <div className="branch-panel">
          <h3>{zh ? "分支" : "Branches"}</h3>
          <p>{zh ? "需要尝试另一套剪辑方案时，从当前版本创建分支。" : "Create a branch from the current version to try another edit."}</p>
          {history.branches.length > 0 && (
            <div className="branch-list">
              {history.branches.map((branch) => (
                <button
                  aria-pressed={branch.id === history.activeBranch}
                  className={branch.id === history.activeBranch ? "active" : ""}
                  disabled={busy || branch.id === history.activeBranch}
                  key={branch.id}
                  onClick={() => void onSwitchBranch(branch.id)}
                >
                  <strong>{branch.name}</strong>
                  <small>{branch.id === history.activeBranch ? (zh ? "当前" : "Current") : (zh ? "切换" : "Switch")}</small>
                </button>
              ))}
            </div>
          )}
          <div className="branch-create-row">
            <input
              aria-label={zh ? "新分支名称" : "New branch name"}
              disabled={history.versions.length === 0}
              placeholder={zh ? "新分支名称" : "New branch name"}
              value={branchName}
              onChange={(event) => setBranchName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void createBranch();
              }}
            />
            <button
              className="button-quiet"
              disabled={busy || history.versions.length === 0 || !branchName.trim()}
              onClick={() => void createBranch()}
            >
              {zh ? "创建分支" : "Create branch"}
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}
