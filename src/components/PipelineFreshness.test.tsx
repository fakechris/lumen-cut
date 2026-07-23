import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { PipelineFreshness } from "./PipelineFreshness";

test("shows a non-fatal stale warning only for active work with no fresh progress", () => {
  const { rerender } = render(
    <PipelineFreshness
      state="running"
      phase="transcribing"
      updatedAt={1}
      lang="zh"
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent("没有收到新进度");
  expect(screen.getByRole("status")).toHaveTextContent("安全取消后重试");

  rerender(
    <PipelineFreshness
      state="running"
      phase="waiting"
      updatedAt={1}
      lang="zh"
    />,
  );
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
