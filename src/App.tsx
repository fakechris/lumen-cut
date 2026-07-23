import { useEffect, useState } from "react";
import { greet, projectList } from "./api";
import {
  FolderIcon,
  MoonIcon,
  SettingsIcon,
  SunIcon,
  TranscriptIcon,
} from "./components/Icons";
import { AppErrorBoundary } from "./components/AppErrorBoundary";
import lumenCutMark from "./assets/lumen-cut.svg";
import { t, type Lang } from "./i18n";
import { ProjectsView } from "./views/ProjectsView";
import { SettingsView } from "./views/SettingsView";
import { TranscriptView } from "./views/TranscriptView";
import type { ProjectSummary } from "./types";
import { TaskCenterView } from "./views/TaskCenterView";

type View = "projects" | "transcript" | "tasks" | "settings";
type Theme = "dark" | "light";

const LAST_PROJECT_KEY = "lumen-cut.lastProject";

function initialTheme(): Theme {
  const stored = localStorage.getItem("lumen-cut.theme");
  if (stored === "dark" || stored === "light") return stored;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function App() {
  const [view, setView] = useState<View>("projects");
  const [pid, setPid] = useState<string | null>(null);
  const [version, setVersion] = useState<string>("—");
  const [recentProjects, setRecentProjects] = useState<ProjectSummary[]>([]);
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

  const openProject = (id: string) => {
    setPid(id);
    localStorage.setItem(LAST_PROJECT_KEY, id);
    setView("transcript");
    void projectList().then(setRecentProjects).catch(() => undefined);
  };

  const projectDeleted = (id: string) => {
    setRecentProjects((projects) => projects.filter((project) => project.pid !== id));
    if (pid !== id) return;
    setPid(null);
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
    { id: "settings" as const, label: t("settings", lang), icon: SettingsIcon },
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
                onClick={() => setView(item.id)}
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
          <button onClick={() => setView("settings")}>
            <span className="sidebar-utility-icon" aria-hidden="true">⌘</span>
            {lang === "zh" ? "自动化接口" : "Automation"}
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
                <span className="recent-project-glyph" aria-hidden="true">
                  {project.title.trim().slice(0, 1).toUpperCase() || "L"}
                </span>
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
        {view === "transcript" && (
          <TranscriptView
            lang={lang}
            pid={pid}
            onOpenSettings={() => setView("settings")}
            onProjectTitleChange={projectTitleChanged}
          />
        )}
        {view === "settings" && (
          <SettingsView lang={lang} pid={pid} />
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
