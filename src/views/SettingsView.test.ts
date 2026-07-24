import { afterEach, expect, test, vi } from "vitest";
import type { SetupJobStatus } from "../types";
import { setupTransferLabel } from "./SettingsView";

afterEach(() => {
  vi.useRealTimers();
});

test("setup transfer status shows trustworthy size speed and elapsed time", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-01-01T00:02:05Z"));
  const job: SetupJobStatus = {
    kind: "asr-models",
    state: "running",
    phase: "downloading",
    startedAt: new Date("2026-01-01T00:00:00Z").getTime() / 1000,
    updatedAt: new Date("2026-01-01T00:02:05Z").getTime() / 1000,
    progress: 42,
    detail: "model.safetensors",
    current: 512 * 1024 * 1024,
    total: 2 * 1024 * 1024 * 1024,
    unit: "bytes",
    bytesPerSecond: 16 * 1024 * 1024,
    error: null,
  };

  expect(setupTransferLabel(job, "zh")).toBe("512 MB / 2.0 GB · 16.0 MB/s · 2 分 5 秒");
});
