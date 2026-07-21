import { useEffect, useState } from "react";
import { greet, projectList } from "./api";
import {
  FolderIcon,
  MoonIcon,
  SettingsIcon,
  SunIcon,
  TranscriptIcon,
} from "./components/Icons";
import { t, type Lang } from "./i18n";
import { ProjectsView } from "./views/ProjectsView";
import { SettingsView } from "./views/SettingsView";
import { TranscriptView } from "./views/TranscriptView";

type View = "projects" | "transcript" | "settings";
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
  const [projectTitle, setProjectTitle] = useState<string | null>(null);
  const [version, setVersion] = useState<string>("—");
  const [theme, setTheme] = useState<Theme>(initialTheme);
  const [lang, setLang] = useState<Lang>(
    () => (localStorage.getItem("lumen-cut.lang") as Lang) || "zh",
  );

  useEffect(() => {
    void greet()
      .then((result) => setVersion(result.version))
      .catch(() => setVersion("—"));

    const previous = localStorage.getItem(LAST_PROJECT_KEY);
    if (!previous) return;
    void projectList()
      .then((projects) => {
        const project = projects.find((candidate) => candidate.pid === previous);
        if (project) {
          setPid(previous);
          setProjectTitle(project.title);
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

  const openProject = (id: string, title?: string) => {
    setPid(id);
    setProjectTitle(title || id);
    localStorage.setItem(LAST_PROJECT_KEY, id);
    setView("transcript");
  };

  const projectDeleted = (id: string) => {
    if (pid !== id) return;
    setPid(null);
    setProjectTitle(null);
    localStorage.removeItem(LAST_PROJECT_KEY);
  };

  const navigation = [
    { id: "projects" as const, label: t("projects", lang), icon: FolderIcon },
    { id: "transcript" as const, label: t("editor", lang), icon: TranscriptIcon },
    { id: "settings" as const, label: t("settings", lang), icon: SettingsIcon },
  ];

  return (
    <div className="app-shell" data-theme={theme}>
      <aside className="sidebar">
        <div className="brand-block">
          <div className="brand-mark" aria-hidden="true">S</div>
          <div>
            <strong>lumen-cut</strong>
            <span>{t("tagline", lang)}</span>
          </div>
        </div>

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

        {pid ? (
          <button className="current-project" onClick={() => setView("transcript")}>
            <span>{t("currentProject", lang)}</span>
            <strong title={pid}>{projectTitle || pid}</strong>
          </button>
        ) : (
          <div className="sidebar-tip">
            <span>{t("firstStep", lang)}</span>
            <p>{t("chooseMediaHint", lang)}</p>
          </div>
        )}

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
            onProjectTitleChange={setProjectTitle}
          />
        )}
        {view === "settings" && (
          <SettingsView lang={lang} pid={pid} />
        )}
      </main>
    </div>
  );
}

export default App;
