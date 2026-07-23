import { useCallback, useEffect, useState } from "react";
import { greet, projectList, projectMarkOpened } from "./api";
import {
  FolderIcon,
  MoonIcon,
  SettingsIcon,
  SunIcon,
  TranscriptIcon,
} from "./components/Icons";
import { AppErrorBoundary } from "./components/AppErrorBoundary";
import { ProjectCover } from "./components/ProjectCover";
import lumenCutMark from "./assets/lumen-cut.svg";
import { t, type Lang } from "./i18n";
import { ProjectsView } from "./views/ProjectsView";
import { SettingsView } from "./views/SettingsView";
import { TranscriptView } from "./views/TranscriptView";
import type { TranscriptDraft } from "./views/editor/TranscriptEditor";
import type { TranslationDraft } from "./views/editor/TranslationWorkspace";
import type { ChapterDraft } from "./views/editor/ChapterWorkspace";
import type { ProjectSummary } from "./types";
import { TaskCenterView } from "./views/TaskCenterView";

type View = "projects" | "transcript" | "tasks" | "settings";
type Theme = "dark" | "light";

const LAST_PROJECT_KEY = "lumen-cut.lastProject";
const TRANSCRIPT_DRAFTS_KEY = "lumen-cut.transcriptDrafts";
const TRANSLATION_DRAFTS_KEY = "lumen-cut.translationDrafts";
const CHAPTER_DRAFTS_KEY = "lumen-cut.chapterDrafts";

type ProjectTranscriptDrafts = Record<string, Record<string, TranscriptDraft>>;
type ProjectTranslationDrafts = Record<
  string,
  Record<string, Record<string, TranslationDraft>>
>;
type ProjectChapterDrafts = Record<string, Record<string, ChapterDraft>>;

function initialTranscriptDrafts(): ProjectTranscriptDrafts {
  try {
    const parsed = JSON.parse(localStorage.getItem(TRANSCRIPT_DRAFTS_KEY) || "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    return Object.fromEntries(Object.entries(parsed).flatMap(([pid, value]) => {
      if (!value || typeof value !== "object" || Array.isArray(value)) return [];
      const drafts = Object.fromEntries(Object.entries(value).flatMap(([id, draft]) => {
        if (!draft || typeof draft !== "object" || Array.isArray(draft)) return [];
        const candidate = draft as Record<string, unknown>;
        return typeof candidate.text === "string" && typeof candidate.sourceText === "string"
          ? [[id, { text: candidate.text, sourceText: candidate.sourceText }]]
          : [];
      }));
      return Object.keys(drafts).length > 0 ? [[pid, drafts]] : [];
    }));
  } catch {
    return {};
  }
}

function initialTranslationDrafts(): ProjectTranslationDrafts {
  try {
    const parsed = JSON.parse(localStorage.getItem(TRANSLATION_DRAFTS_KEY) || "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    return Object.fromEntries(Object.entries(parsed).flatMap(([pid, value]) => {
      if (!value || typeof value !== "object" || Array.isArray(value)) return [];
      const languages = Object.fromEntries(Object.entries(value).flatMap(([language, rows]) => {
        if (!rows || typeof rows !== "object" || Array.isArray(rows)) return [];
        const drafts = Object.fromEntries(Object.entries(rows).flatMap(([id, draft]) => {
          if (!draft || typeof draft !== "object" || Array.isArray(draft)) return [];
          const candidate = draft as Record<string, unknown>;
          return typeof candidate.text === "string" && typeof candidate.savedText === "string"
            ? [[id, { text: candidate.text, savedText: candidate.savedText }]]
            : [];
        }));
        return Object.keys(drafts).length > 0 ? [[language, drafts]] : [];
      }));
      return Object.keys(languages).length > 0 ? [[pid, languages]] : [];
    }));
  } catch {
    return {};
  }
}

function initialChapterDrafts(): ProjectChapterDrafts {
  try {
    const parsed = JSON.parse(localStorage.getItem(CHAPTER_DRAFTS_KEY) || "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    return Object.fromEntries(Object.entries(parsed).flatMap(([pid, value]) => {
      if (!value || typeof value !== "object" || Array.isArray(value)) return [];
      const drafts = Object.fromEntries(Object.entries(value).flatMap(([id, draft]) => {
        if (!draft || typeof draft !== "object" || Array.isArray(draft)) return [];
        const candidate = draft as Record<string, unknown>;
        return typeof candidate.title === "string" && typeof candidate.sourceTitle === "string"
          ? [[id, { title: candidate.title, sourceTitle: candidate.sourceTitle }]]
          : [];
      }));
      return Object.keys(drafts).length > 0 ? [[pid, drafts]] : [];
    }));
  } catch {
    return {};
  }
}

function initialTheme(): Theme {
  const stored = localStorage.getItem("lumen-cut.theme");
  if (stored === "dark" || stored === "light") return stored;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function App() {
  const [view, setView] = useState<View>("projects");
  const [pid, setPid] = useState<string | null>(null);
  const [editorMounted, setEditorMounted] = useState(false);
  const [version, setVersion] = useState<string>("—");
  const [recentProjects, setRecentProjects] = useState<ProjectSummary[]>([]);
  const [transcriptDrafts, setTranscriptDrafts] =
    useState<ProjectTranscriptDrafts>(initialTranscriptDrafts);
  const [translationDrafts, setTranslationDrafts] =
    useState<ProjectTranslationDrafts>(initialTranslationDrafts);
  const [chapterDrafts, setChapterDrafts] =
    useState<ProjectChapterDrafts>(initialChapterDrafts);
  const [theme, setTheme] = useState<Theme>(initialTheme);
  const [lang, setLang] = useState<Lang>(
    () => (localStorage.getItem("lumen-cut.lang") as Lang) || "zh",
  );

  useEffect(() => {
    void greet()
      .then((result) => setVersion(result.version))
      .catch(() => setVersion("—"));

    void projectList()
      .then((projects) => {
        setRecentProjects(projects);
        const previous = localStorage.getItem(LAST_PROJECT_KEY);
        if (!previous) return;
        const project = projects.find((candidate) => candidate.pid === previous);
        if (project) {
          setPid(previous);
        }
      })
      .catch(() => {
        // The Projects view owns user-facing load errors.
      });
  }, []);

  useEffect(() => {
    localStorage.setItem("lumen-cut.theme", theme);
  }, [theme]);

  useEffect(() => {
    localStorage.setItem("lumen-cut.lang", lang);
  }, [lang]);

  useEffect(() => {
    try {
      if (Object.keys(transcriptDrafts).length === 0) {
        localStorage.removeItem(TRANSCRIPT_DRAFTS_KEY);
      } else {
        localStorage.setItem(TRANSCRIPT_DRAFTS_KEY, JSON.stringify(transcriptDrafts));
      }
    } catch {
      // Drafts remain available for this session if browser storage is full.
    }
  }, [transcriptDrafts]);

  useEffect(() => {
    try {
      if (Object.keys(translationDrafts).length === 0) {
        localStorage.removeItem(TRANSLATION_DRAFTS_KEY);
      } else {
        localStorage.setItem(TRANSLATION_DRAFTS_KEY, JSON.stringify(translationDrafts));
      }
    } catch {
      // Drafts remain available for this session if browser storage is full.
    }
  }, [translationDrafts]);

  useEffect(() => {
    try {
      if (Object.keys(chapterDrafts).length === 0) {
        localStorage.removeItem(CHAPTER_DRAFTS_KEY);
      } else {
        localStorage.setItem(CHAPTER_DRAFTS_KEY, JSON.stringify(chapterDrafts));
      }
    } catch {
      // Drafts remain available for this session if browser storage is full.
    }
  }, [chapterDrafts]);

  useEffect(() => {
    const hasTranscriptDrafts = Object.values(transcriptDrafts)
      .some((drafts) => Object.keys(drafts).length > 0);
    const hasTranslationDrafts = Object.values(translationDrafts)
      .some((languages) => Object.values(languages)
        .some((drafts) => Object.keys(drafts).length > 0));
    const hasChapterDrafts = Object.values(chapterDrafts)
      .some((drafts) => Object.keys(drafts).length > 0);
    if (!hasTranscriptDrafts && !hasTranslationDrafts && !hasChapterDrafts) {
      return;
    }
    const warnOnClose = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warnOnClose);
    return () => window.removeEventListener("beforeunload", warnOnClose);
  }, [chapterDrafts, transcriptDrafts, translationDrafts]);

  const updateTranscriptDrafts = useCallback((update: (
    current: Record<string, TranscriptDraft>,
  ) => Record<string, TranscriptDraft>) => {
    if (!pid) return;
    setTranscriptDrafts((current) => {
      const nextProject = update(current[pid] || {});
      if (Object.keys(nextProject).length === 0) {
        const { [pid]: _removed, ...rest } = current;
        return rest;
      }
      return { ...current, [pid]: nextProject };
    });
  }, [pid]);

  const updateTranslationDrafts = useCallback((
    language: string,
    update: (
      current: Record<string, TranslationDraft>,
    ) => Record<string, TranslationDraft>,
  ) => {
    if (!pid) return;
    setTranslationDrafts((current) => {
      const nextLanguage = update(current[pid]?.[language] || {});
      const nextProject = { ...(current[pid] || {}) };
      if (Object.keys(nextLanguage).length === 0) {
        delete nextProject[language];
      } else {
        nextProject[language] = nextLanguage;
      }
      if (Object.keys(nextProject).length === 0) {
        const { [pid]: _removed, ...rest } = current;
        return rest;
      }
      return { ...current, [pid]: nextProject };
    });
  }, [pid]);

  const updateChapterDrafts = useCallback((update: (
    current: Record<string, ChapterDraft>,
  ) => Record<string, ChapterDraft>) => {
    if (!pid) return;
    setChapterDrafts((current) => {
      const nextProject = update(current[pid] || {});
      if (Object.keys(nextProject).length === 0) {
        const { [pid]: _removed, ...rest } = current;
        return rest;
      }
      return { ...current, [pid]: nextProject };
    });
  }, [pid]);

  const openProject = (id: string) => {
    setPid(id);
    setEditorMounted(true);
    localStorage.setItem(LAST_PROJECT_KEY, id);
    setView("transcript");
    const openedAt = new Date().toISOString();
    setRecentProjects((projects) => projects
      .map((project) => project.pid === id
        ? { ...project, last_opened_at: openedAt }
        : project)
      .sort((left, right) =>
        Date.parse(right.last_opened_at || right.updated_at)
        - Date.parse(left.last_opened_at || left.updated_at)));
    void projectMarkOpened(id)
      .then(() => projectList())
      .then(setRecentProjects)
      .catch(() => undefined);
  };

  const projectDeleted = (id: string) => {
    setRecentProjects((projects) => projects.filter((project) => project.pid !== id));
    localStorage.removeItem(`lumen-cut.brollDrafts.${id}`);
    localStorage.removeItem(`lumen-cut.styleDrafts.${id}`);
    localStorage.removeItem(`lumen-cut.timelineDrafts.${id}`);
    setTranscriptDrafts((current) => {
      const { [id]: _removed, ...rest } = current;
      return rest;
    });
    setTranslationDrafts((current) => {
      const { [id]: _removed, ...rest } = current;
      return rest;
    });
    setChapterDrafts((current) => {
      const { [id]: _removed, ...rest } = current;
      return rest;
    });
    if (pid !== id) return;
    setPid(null);
    setEditorMounted(false);
    localStorage.removeItem(LAST_PROJECT_KEY);
  };

  const projectTitleChanged = (title: string) => {
    if (!pid) return;
    setRecentProjects((projects) =>
      projects.map((project) => project.pid === pid ? { ...project, title } : project),
    );
  };

  const navigation = [
    { id: "projects" as const, label: t("projects", lang), icon: FolderIcon },
    { id: "transcript" as const, label: t("editor", lang), icon: TranscriptIcon },
  ];

  return (
    <div className={`app-shell app-view-${view}`} data-theme={theme}>
      <aside className="sidebar">
        <div className="brand-block">
          <img className="brand-mark" src={lumenCutMark} alt="" aria-hidden="true" />
          <div>
            <strong>lumen-cut</strong>
            <span>{t("tagline", lang)}</span>
          </div>
        </div>

        <button className="sidebar-new-project" onClick={() => setView("projects")}>
          <span aria-hidden="true">＋</span>
          {lang === "zh" ? "新建转写" : "New transcription"}
        </button>

        <nav className="primary-nav" aria-label={t("navigation", lang)}>
          {navigation.map((item) => {
            const Icon = item.icon;
            const disabled = item.id === "transcript" && !pid;
            return (
              <button
                aria-current={view === item.id ? "page" : undefined}
                className={view === item.id ? "active" : ""}
                disabled={disabled}
                key={item.id}
                title={disabled ? t("chooseProjectFirst", lang) : undefined}
                onClick={() => {
                  if (item.id === "transcript") setEditorMounted(true);
                  setView(item.id);
                }}
              >
                <Icon />
                <span>{item.label}</span>
              </button>
            );
          })}
        </nav>

        <div className="sidebar-utilities">
          <button
            className={view === "tasks" ? "active" : ""}
            onClick={() => setView("tasks")}
          >
            <span className="sidebar-utility-icon" aria-hidden="true">◷</span>
            {lang === "zh" ? "后台任务" : "Background tasks"}
          </button>
          <button
            className={view === "settings" ? "active" : ""}
            onClick={() => setView("settings")}
          >
            <SettingsIcon />
            {lang === "zh" ? "设置" : "Settings"}
          </button>
        </div>

        <section className="sidebar-recents" aria-label={lang === "zh" ? "最近项目" : "Recent projects"}>
          <header>
            <span>{lang === "zh" ? "最近项目" : "Recent"}</span>
            <button onClick={() => setView("projects")}>{lang === "zh" ? "全部" : "All"}</button>
          </header>
          <div>
            {recentProjects.slice(0, 12).map((project) => (
              <button
                aria-current={pid === project.pid && view === "transcript" ? "page" : undefined}
                aria-label={`${lang === "zh" ? "切换到" : "Switch to"} ${project.title}`}
                className={pid === project.pid ? "active" : ""}
                key={project.pid}
                onClick={() => openProject(project.pid)}
              >
                <ProjectCover
                  compact
                  mediaAvailable={project.media_available !== false}
                  pid={project.pid}
                  title={project.title}
                  updatedAt={project.updated_at}
                />
                <span className="recent-project-copy">
                  <strong>{project.title}</strong>
                  <small>
                    {project.word_count > 0
                      ? `${project.word_count} ${lang === "zh" ? "字" : "words"}`
                      : (lang === "zh" ? "等待转写" : "Ready")}
                  </small>
                </span>
              </button>
            ))}
            {recentProjects.length === 0 && (
              <p>{lang === "zh" ? "导入媒体后会出现在这里。" : "Imported media will appear here."}</p>
            )}
          </div>
        </section>

        <div className="sidebar-footer">
          <button
            aria-label={t("toggleTheme", lang)}
            className="icon-button"
            onClick={() => setTheme((value) => (value === "dark" ? "light" : "dark"))}
          >
            {theme === "dark" ? <SunIcon /> : <MoonIcon />}
          </button>
          <button
            aria-label={t("toggleLanguage", lang)}
            className="language-button"
            onClick={() => setLang((value) => (value === "zh" ? "en" : "zh"))}
          >
            {lang === "zh" ? "EN" : "中文"}
          </button>
          <span className="app-version">v{version}</span>
        </div>
      </aside>

      <main className={`workspace workspace-${view}`}>
        {view === "projects" && (
          <ProjectsView
            currentPid={pid}
            lang={lang}
            onDeleteProject={projectDeleted}
            onOpenProject={openProject}
          />
        )}
        {editorMounted && pid && (
          <div className="persistent-editor-host" hidden={view !== "transcript"}>
            <TranscriptView
              active={view === "transcript"}
              chapterDrafts={chapterDrafts[pid] || {}}
              key={pid}
              lang={lang}
              onChapterDraftsChange={updateChapterDrafts}
              onTranscriptDraftsChange={updateTranscriptDrafts}
              onTranslationDraftsChange={updateTranslationDrafts}
              pid={pid}
              onOpenProjects={() => setView("projects")}
              onOpenSettings={() => setView("settings")}
              onProjectTitleChange={projectTitleChanged}
              transcriptDrafts={transcriptDrafts[pid] || {}}
              translationDrafts={translationDrafts[pid] || {}}
            />
          </div>
        )}
        {view === "settings" && (
          <SettingsView lang={lang} />
        )}
        {view === "tasks" && (
          <TaskCenterView
            lang={lang}
            projects={recentProjects}
            onOpenProject={openProject}
          />
        )}
      </main>
    </div>
  );
}

export default function AppWithErrorBoundary() {
  return (
    <AppErrorBoundary>
      <App />
    </AppErrorBoundary>
  );
}
