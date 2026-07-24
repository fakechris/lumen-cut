import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { ReviewFindings } from "./ReviewFindings";

const findings = Array.from({ length: 1_000 }, (_, index) => ({
  code: index % 2 === 0 ? "source-flash" : "timing-gap",
  severity: index % 10 === 0 ? "fail" : "warning",
  location: `cue-${index}`,
  message: `Finding ${index}`,
}));

test("keeps large audit reports bounded and filterable", () => {
  const { container } = render(<ReviewFindings findings={findings} lang="en" />);

  expect(screen.getByText("1000 findings, 1000 shown")).toBeVisible();
  expect(container.querySelectorAll(".virtual-list-row").length).toBeLessThan(30);

  fireEvent.change(screen.getByRole("combobox", { name: "Filter by severity" }), {
    target: { value: "fail" },
  });
  expect(screen.getByText("1000 findings, 100 shown")).toBeVisible();

  fireEvent.change(screen.getByRole("searchbox", { name: "Search review findings" }), {
    target: { value: "cue-990" },
  });
  expect(screen.getByText("1000 findings, 1 shown")).toBeVisible();
  expect(screen.getByText("Finding 990")).toBeVisible();
});
