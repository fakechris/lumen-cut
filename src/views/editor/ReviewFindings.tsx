import { useMemo, useState } from "react";
import { VirtualList } from "../../components/VirtualList";
import type { Lang } from "../../i18n";
import type { FindingSummary } from "../../types";

interface Props {
  findings: FindingSummary[];
  lang: Lang;
}

interface FindingRow {
  key: string;
  finding: FindingSummary;
}

function severityRank(value: string): number {
  switch (value.toLowerCase()) {
    case "fail":
    case "error":
      return 0;
    case "warning":
    case "warn":
      return 1;
    default:
      return 2;
  }
}

export function ReviewFindings({ findings, lang }: Props) {
  const [query, setQuery] = useState("");
  const [severity, setSeverity] = useState("all");
  const [code, setCode] = useState("all");
  const severities = useMemo(
    () => [...new Set(findings.map((finding) => finding.severity.toLowerCase()))]
      .sort((left, right) => severityRank(left) - severityRank(right) || left.localeCompare(right)),
    [findings],
  );
  const codes = useMemo(
    () => [...new Set(findings.map((finding) => finding.code))].sort(),
    [findings],
  );
  const counts = useMemo(
    () => findings.reduce<Record<string, number>>((result, finding) => {
      const key = finding.severity.toLowerCase();
      result[key] = (result[key] ?? 0) + 1;
      return result;
    }, {}),
    [findings],
  );
  const rows = useMemo<FindingRow[]>(() => {
    const normalizedQuery = query.trim().toLowerCase();
    return findings
      .map((finding, index) => ({
        key: `${finding.code}:${finding.location}:${index}`,
        finding,
      }))
      .filter(({ finding }) => {
        const findingSeverity = finding.severity.toLowerCase();
        return (severity === "all" || findingSeverity === severity)
          && (code === "all" || finding.code === code)
          && (!normalizedQuery || [
            finding.message,
            finding.location,
            finding.code,
          ].some((value) => value.toLowerCase().includes(normalizedQuery)));
      })
      .sort(({ finding: left }, { finding: right }) =>
        severityRank(left.severity) - severityRank(right.severity));
  }, [code, findings, query, severity]);

  return (
    <>
      <div className="finding-summary" aria-label={lang === "zh" ? "问题统计" : "Finding summary"}>
        <strong>
          {lang === "zh"
            ? `${findings.length} 个问题，当前显示 ${rows.length} 个`
            : `${findings.length} findings, ${rows.length} shown`}
        </strong>
        <div>
          {severities.map((value) => (
            <span className={`severity ${value}`} key={value}>
              {value} {counts[value]}
            </span>
          ))}
        </div>
      </div>
      <div className="finding-filters">
        <input
          aria-label={lang === "zh" ? "搜索审查问题" : "Search review findings"}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={lang === "zh" ? "搜索消息、位置或代码…" : "Search message, location, or code…"}
          type="search"
          value={query}
        />
        <select
          aria-label={lang === "zh" ? "按严重程度筛选" : "Filter by severity"}
          onChange={(event) => setSeverity(event.target.value)}
          value={severity}
        >
          <option value="all">{lang === "zh" ? "全部严重程度" : "All severities"}</option>
          {severities.map((value) => (
            <option key={value} value={value}>{value} ({counts[value]})</option>
          ))}
        </select>
        <select
          aria-label={lang === "zh" ? "按问题代码筛选" : "Filter by finding code"}
          onChange={(event) => setCode(event.target.value)}
          value={code}
        >
          <option value="all">{lang === "zh" ? "全部问题类型" : "All finding types"}</option>
          {codes.map((value) => <option key={value} value={value}>{value}</option>)}
        </select>
      </div>
      {rows.length === 0 ? (
        <p className="finding-empty">
          {lang === "zh" ? "没有符合当前筛选条件的问题。" : "No findings match these filters."}
        </p>
      ) : (
        <VirtualList
          ariaLabel={lang === "zh" ? "审查问题列表" : "Review findings"}
          className="finding-list review-finding-list"
          estimateHeight={64}
          itemKey={(row) => row.key}
          items={rows}
          role="list"
          renderItem={({ finding }) => (
            <div className="finding-row" role="listitem">
              <span className={`severity ${finding.severity.toLowerCase()}`}>
                {finding.severity}
              </span>
              <div>
                <strong>{finding.message}</strong>
                <small>{finding.location} · {finding.code}</small>
              </div>
            </div>
          )}
        />
      )}
    </>
  );
}
