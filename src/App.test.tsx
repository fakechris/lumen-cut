import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import App from "./App";

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn<(command: string) => Promise<unknown>>(),
}));
let projectDoc: Record<string, unknown>;

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

beforeEach(() => {
  localStorage.clear();
  invoke.mockReset();
  projectDoc = {
    id: "project-1",
    schema: 1,
    media: {
      path: "/Users/example/Interview.mp4",
      durationSeconds: 2212.792018,
      sampleRate: 44100,
      channels: 2,
    },
    meta: {
      title: "Interview",
      description: "",
      language: null,
      createdAt: "2026-07-21T10:08:00Z",
      updatedAt: "2026-07-21T10:08:00Z",
    },
    paragraphs: [],
    translations: {},
  };
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
        return {};
      case "task_status":
        return { pending: 0, done: 0, kinds: [], polishQuality: null };
      case "config_show":
        return { llmEndpoint: "", llmModel: "" };
      case "transcription_status":
        return { pid: "project-1", state: "completed", phase: "completed", progress: 100 };
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
