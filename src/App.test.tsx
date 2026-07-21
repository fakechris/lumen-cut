import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import App from "./App";
import serializedProject from "./test/fixtures/project.json";

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn<(command: string) => Promise<unknown>>(),
}));
let projectDoc: Record<string, unknown>;
let asrReady: boolean;

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
  invoke,
}));

beforeEach(() => {
  localStorage.clear();
  asrReady = true;
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
          path: "/projects/project-1",
          duration_seconds: 2212.792018,
          word_count: 0,
          paragraph_count: 0,
        }];
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
  for (const label of ["转写稿", "翻译", "样式", "属性", "时间线", "审查与修复", "导出"]) {
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
