import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import App from "./App";
import serializedProject from "./test/fixtures/project.json";

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn<(command: string) => Promise<unknown>>(),
}));
let projectDoc: Record<string, unknown>;
let asrReady: boolean;
let versionCommitError: Error | null;
let brollOverview: {
  suggestions: Array<Record<string, unknown>>;
  accepted: Array<Record<string, unknown>>;
  errors: string[];
};
let brollListError: Error | null;

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
  invoke,
}));

beforeEach(() => {
  localStorage.clear();
  asrReady = true;
  versionCommitError = null;
  brollOverview = { suggestions: [], accepted: [], errors: [] };
  brollListError = null;
  invoke.mockReset();
  projectDoc = structuredClone(serializedProject);
  invoke.mockImplementation(async (command) => {
    switch (command) {
      case "greet":
        return { msg: "ready", version: "0.1.0" };
      case "project_list":
        return [{
          pid: "project-1",
          title: "Interview",
          description: "",
          path: "/projects/project-1",
          duration_seconds: 2212.792018,
          word_count: 0,
          paragraph_count: 0,
          updated_at: "2026-07-21T00:00:00Z",
          starred: false,
        }];
      case "project_search":
        return [{
          pid: "project-2",
          title: "Search match",
          description: "Customer notes",
          path: "/projects/project-2",
          duration_seconds: 32,
          word_count: 8,
          paragraph_count: 2,
          updated_at: "2026-07-21T01:00:00Z",
          starred: false,
        }];
      case "project_set_star":
        return {
          pid: "project-2",
          title: "Search match",
          description: "Customer notes",
          path: "/projects/project-2",
          duration_seconds: 32,
          word_count: 8,
          paragraph_count: 2,
          updated_at: "2026-07-21T01:00:00Z",
          starred: true,
        };
      case "timing_repair":
        return "3 fix(es)";
      case "project_show":
        return projectDoc;
      case "subtitle_list":
      case "speakers_list":
      case "cut_list":
        return [];
      case "style_get":
        return {
          name: "Default",
          fontname: "Arial",
          fontsize: 52,
          primaryColour: "&H00FFFFFF",
          outlineColour: "&H00000000",
          bold: false,
          italic: false,
          underline: false,
          strikeOut: false,
          alignment: 2,
          outline: 2,
          shadow: 2,
          marginL: 40,
          marginR: 40,
          marginV: 80,
        };
      case "task_status":
        return { pending: 0, done: 0, kinds: [], polishQuality: null };
      case "version_list":
        return { v: 1, head: null, activeBranch: null, branches: [], versions: [] };
      case "broll_list":
        if (brollListError) throw brollListError;
        return brollOverview;
      case "pick_broll_file":
        return "/Users/example/product.png";
      case "broll_accept_suggestion": {
        const placement = {
          id: "br-1",
          file: "/Users/example/product.png",
          start: 4,
          end: 7,
          mode: "pip",
          rect: null,
          fit: "cover",
          background: "black",
          sourceStart: 0,
          radius: 0,
          name: "product close-up",
        };
        brollOverview = { ...brollOverview, accepted: [placement] };
        return placement;
      }
      case "version_commit":
        if (versionCommitError) throw versionCommitError;
        return "v0";
      case "config_show":
        return {
          asrModel: "Qwen/Qwen3-ASR-0.6B",
          asrAligner: "Qwen/Qwen3-ForcedAligner-0.6B",
          diarizeModel: "pyannote/speaker-diarization-3.1",
          llmEndpoint: "",
          llmApiKey: "",
          llmModel: "gpt-4o-mini",
          workerCount: 3,
        };
      case "asr_status":
        return {
          pythonPath: "/Users/example/.lumen-cut/runtime/bin/python3",
          runtimeReady: asrReady,
          runtimeDetail: "mlx-qwen3-asr 0.3.5",
          modelId: "Qwen/Qwen3-ASR-0.6B",
          modelCached: asrReady,
          alignerId: "Qwen/Qwen3-ForcedAligner-0.6B",
          alignerCached: asrReady,
          ready: asrReady,
        };
      case "transcription_status":
        return { pid: "project-1", state: "completed", phase: "completed", progress: 100 };
      case "media_asset_allow":
        return "/Users/example/Interview.mp4";
      default:
        throw new Error(`unexpected command: ${command}`);
    }
  });
});

test("opening a project renders the serialized media details", async () => {
  render(<App />);

  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByRole("heading", { name: "Interview" })).toBeVisible();
  expect(screen.getAllByText("2212.8s")[0]).toBeVisible();
});

test("a project rendering error shows recovery UI instead of a white window", async () => {
  vi.spyOn(console, "error").mockImplementation(() => undefined);
  projectDoc = { ...projectDoc, media: { path: "/Users/example/broken.mp4" } };
  render(<App />);

  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByRole("alert")).toHaveTextContent("界面出现问题");
  expect(screen.getByRole("button", { name: "重新载入" })).toBeVisible();
});

test("corrupt optional B-roll data does not block the transcript editor", async () => {
  brollListError = new Error("invalid broll.json");
  render(<App />);

  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByRole("heading", { name: "Interview" })).toBeVisible();
  expect(screen.getByText(/B-roll 数据无法加载，转写稿仍可继续编辑/)).toBeVisible();
  expect(screen.queryByText("界面出现问题")).not.toBeInTheDocument();
});

test("a serialized transcript project can open every editor surface", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Alice",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [
          { id: "w1", text: "Hello", start: 0, end: 0.5 },
          { id: "w2", text: "world", start: 0.5, end: 1 },
        ],
      }],
    }],
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  const tabs = await screen.findByRole("navigation", { name: "编辑步骤" });
  for (const label of ["转写稿", "翻译", "样式", "属性", "版本", "时间线", "补充画面", "审查与修复", "导出"]) {
    const tab = within(tabs).getByRole("button", { name: label });
    fireEvent.click(tab);
    expect(tab).toHaveAttribute("aria-current", "page");
    expect(screen.queryByText("界面出现问题")).not.toBeInTheDocument();
  }
});

test("setup blocks transcription until the local runtime and models are ready", async () => {
  asrReady = false;
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByText(/本地转写尚未准备好/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "开始转写" }));

  expect(await screen.findAllByText(/本地转写尚未准备好/)).not.toHaveLength(0);
  expect(invoke).not.toHaveBeenCalledWith("transcription_start", expect.anything());
});

test("settings exposes the real local transcription status", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));

  expect(await screen.findByRole("heading", { name: "本地转写" })).toBeVisible();
  expect(screen.getByText(/mlx-qwen3-asr 0.3.5/)).toBeVisible();
  expect(screen.getAllByText("模型已下载")).toHaveLength(2);
  expect(screen.queryByRole("button", { name: /start server/i })).not.toBeInTheDocument();
});

test("project library can search transcript content, star, and repair a project", async () => {
  render(<App />);

  const search = await screen.findByPlaceholderText("搜索项目、备注或转写内容");
  fireEvent.change(search, { target: { value: "customer phrase" } });

  expect(await screen.findByText("Search match")).toBeVisible();
  fireEvent.change(screen.getByRole("combobox", { name: "项目排序" }), {
    target: { value: "name" },
  });
  expect(localStorage.getItem("lumen-cut.projectSort")).toBe("name");
  fireEvent.click(screen.getByRole("button", { name: "收藏项目: Search match" }));
  expect(await screen.findByRole("button", { name: "取消收藏: Search match" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  fireEvent.click(screen.getByRole("button", { name: "更多项目操作: Search match" }));
  fireEvent.click(screen.getByRole("button", { name: "修复转写时间轴" }));
  expect(screen.getByText(/修改前自动保存恢复版本/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "确认修复" }));
  expect(await screen.findByText(/项目检查完成 · 3 fix\(es\)/)).toBeVisible();

  expect(invoke).toHaveBeenCalledWith("project_search", {
    query: "customer phrase",
    root: null,
  });
  expect(invoke).toHaveBeenCalledWith("project_set_star", {
    pid: "project-2",
    starred: true,
    root: null,
  });
  expect(invoke).toHaveBeenCalledWith("timing_repair", {
    pid: "project-2",
    root: null,
  });
});

test("project editor exposes recoverable version snapshots", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "版本" }));

  fireEvent.change(screen.getByPlaceholderText("例如：校对完成"), {
    target: { value: "Before batch polish" },
  });
  fireEvent.change(screen.getByPlaceholderText("记录这次修改的目的"), {
    target: { value: "Safe restore point" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存当前版本" }));

  expect(await screen.findByText("当前项目已保存为可恢复版本。")).toBeVisible();
  expect(invoke).toHaveBeenCalledWith("version_commit", {
    pid: "project-1",
    name: "Before batch polish",
    note: "Safe restore point",
    root: null,
  });
});

test("a failed version save preserves the draft for retry", async () => {
  versionCommitError = new Error("disk is temporarily unavailable");
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "版本" }));

  const name = screen.getByPlaceholderText("例如：校对完成");
  const note = screen.getByPlaceholderText("记录这次修改的目的");
  fireEvent.change(name, { target: { value: "Retry me" } });
  fireEvent.change(note, { target: { value: "Keep this note" } });
  fireEvent.click(screen.getByRole("button", { name: "保存当前版本" }));

  expect(await screen.findByText("disk is temporarily unavailable")).toBeVisible();
  expect(name).toHaveValue("Retry me");
  expect(note).toHaveValue("Keep this note");
});

test("B-roll suggestions have a discoverable asset acceptance flow", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Alice",
      sentences: [{
        id: "s1",
        text: "Show the product",
        words: [
          { id: "w1", text: "Show", start: 4, end: 5 },
          { id: "w2", text: "product", start: 6, end: 7 },
        ],
      }],
    }],
  };
  brollOverview = {
    suggestions: [{
      start: "w1",
      end: "w2",
      mode: "pip",
      query: "product close-up",
      reason: "Show the object being discussed",
    }],
    accepted: [],
    errors: [],
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "补充画面" }));

  expect(await screen.findByText("product close-up")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "选择素材并添加" }));

  await waitFor(() => {
    expect(invoke).toHaveBeenCalledWith("broll_accept_suggestion", {
      pid: "project-1",
      suggestion: {
        start: "w1",
        end: "w2",
        mode: "pip",
        query: "product close-up",
        reason: "Show the object being discussed",
      },
      file: "/Users/example/product.png",
      root: null,
    });
  });
  expect(await screen.findByText("素材已按建议时段加入 B-roll 轨道。")).toBeVisible();
  expect(screen.getByRole("heading", { name: "已加入成片" }).closest("header")).toHaveTextContent("1");
});
