import { expect, test } from "vitest";
import type { TaskStatus } from "../types";
import { agentActivityLabel } from "./TaskCenterView";

test("agent activity distinguishes provider requests from queued batches", () => {
  const task: TaskStatus["kinds"][number] = {
    kind: "translate",
    state: "running",
    calls: 6,
    pending: 5,
    done: 1,
    failed: 0,
    queued: 3,
    inFlight: 2,
    retrying: 1,
    attempt: 2,
    maxAttempts: 3,
  };

  expect(agentActivityLabel(task, "zh")).toBe(
    "2 个请求在途 · 3 个等待发送 · 1 个正在重试 · 当前第 2/3 次尝试",
  );
});
