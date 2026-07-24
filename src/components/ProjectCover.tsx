import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { projectThumbnail as loadProjectThumbnail } from "../api";
import { FolderIcon } from "./Icons";

interface Props {
  compact?: boolean;
  mediaAvailable: boolean;
  pid: string;
  title: string;
  updatedAt?: string;
}

const successfulCovers = new Map<string, string>();
const pendingCovers = new Map<string, ReturnType<typeof loadProjectThumbnail>>();

function requestCover(cacheKey: string, pid: string) {
  const existing = pendingCovers.get(cacheKey);
  if (existing) return existing;
  const request = loadProjectThumbnail(pid).finally(() => {
    if (pendingCovers.get(cacheKey) === request) pendingCovers.delete(cacheKey);
  });
  pendingCovers.set(cacheKey, request);
  return request;
}

export function ProjectCover({
  compact = false,
  mediaAvailable,
  pid,
  title,
  updatedAt = "",
}: Props) {
  const cacheKey = `${pid}:${updatedAt}`;
  const hostRef = useRef<HTMLSpanElement | null>(null);
  const [visible, setVisible] = useState(false);
  const [source, setSource] = useState<string | null>(
    () => mediaAvailable ? successfulCovers.get(cacheKey) ?? null : null,
  );
  const [available, setAvailable] = useState(mediaAvailable);

  useEffect(() => {
    setAvailable(mediaAvailable);
    setSource(mediaAvailable ? successfulCovers.get(cacheKey) ?? null : null);
    setVisible(false);
  }, [cacheKey, mediaAvailable]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host || source) return;
    if (typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "160px" },
    );
    observer.observe(host);
    return () => observer.disconnect();
  }, [source]);

  useEffect(() => {
    if (!visible || source || !available) return;
    let disposed = false;
    let retry: number | null = null;
    const load = async () => {
      try {
        const result = await requestCover(cacheKey, pid);
        if (disposed) return;
        setAvailable(result.mediaAvailable);
        if (result.deferred) {
          retry = window.setTimeout(() => void load(), 2500);
          return;
        }
        if (result.path) {
          const url = convertFileSrc(result.path);
          successfulCovers.set(cacheKey, url);
          setSource(url);
        }
      } catch {
        // A cover is decorative. Keep the project fully usable when ffmpeg
        // cannot decode one particular frame.
      }
    };
    void load();
    return () => {
      disposed = true;
      if (retry !== null) window.clearTimeout(retry);
    };
  }, [available, cacheKey, pid, source, visible]);

  return (
    <span
      className={`project-cover${compact ? " compact" : ""}${available ? "" : " offline"}`}
      ref={hostRef}
      title={available ? title : `${title} · media offline`}
    >
      {source ? (
        <img alt="" src={source} />
      ) : (
        <span className="project-cover-placeholder" aria-hidden="true">
          {available ? <FolderIcon /> : "!"}
        </span>
      )}
    </span>
  );
}
