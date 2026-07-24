import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import serializedProject from "../../test/fixtures/project.json";
import type { Doc, TaskStatus } from "../../types";
import { EnhancementPanel } from "./EnhancementPanel";

test("completed enhancements require explicit confirmation before rerunning", () => {
  const onStart = vi.fn(async () => undefined);
  const status: TaskStatus = {
  pending: 0,
  done: 1,
  failed: 0,
    kinds: [{
      kind: "chapters",
      state: "completed",
      calls: 1,
      pending: 0,
      done: 1,
      failed: 0,
    }],
  };

  render(
    <EnhancementPanel
      busy={false}
      configured
      doc={serializedProject as Doc}
      lang="zh"
      status={status}
      onOpenSettings={() => undefined}
      onStart={onStart}
    />,
  );

  fireEvent.click(screen.getByRole("button", { name: "再次运行" }));
  expect(onStart).not.toHaveBeenCalled();
  expect(screen.getByText("再次运行可能替换这一步的现有结果。")).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "确认再次运行" }));
  expect(onStart).toHaveBeenCalledWith("chapters", null);
});

test("running enhancements expose live provider activity", () => {
  const status: TaskStatus = {
    pending: 3,
    done: 1,
    failed: 0,
    kinds: [{
      kind: "chapters",
      state: "running",
      calls: 4,
      pending: 3,
      done: 1,
      failed: 0,
      queued: 2,
      inFlight: 1,
      retrying: 1,
      attempt: 2,
      maxAttempts: 3,
    }],
  };

  render(
    <EnhancementPanel
      busy={false}
      configured
      doc={serializedProject as Doc}
      lang="zh"
      status={status}
      onOpenSettings={() => undefined}
      onStart={async () => undefined}
    />,
  );

  expect(screen.getByText("1 个请求在途 · 2 个等待 · 重试 2/3")).toBeVisible();
});
