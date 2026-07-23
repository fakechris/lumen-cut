import { render } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import type { Doc } from "../../types";
import { TimelineWorkspace } from "./TimelineWorkspace";

describe("long-project timeline", () => {
  test("keeps thousands of cues out of the DOM while preserving the active cue", () => {
    const sentences = Array.from({ length: 5_000 }, (_, index) => ({
      id: `cue-${index}`,
      text: `Cue ${index}`,
      words: [{
        id: `word-${index}`,
        text: `Cue ${index}`,
        start: index * 2,
        end: index * 2 + 1,
      }],
    }));
    const doc: Doc = {
      id: "long-project",
      schema: 1,
      media: { path: "/tmp/long.mp4", durationSeconds: 10_000 },
      meta: {
        title: "Long project",
        description: "",
        createdAt: "2026-01-01T00:00:00Z",
        updatedAt: "2026-01-01T00:00:00Z",
      },
      paragraphs: [{ id: 1, sentences }],
      translations: {},
    };

    const { container } = render(
      <TimelineWorkspace
        busy={false}
        currentTime={0.5}
        cuts={[]}
        doc={doc}
        lang="en"
        onRestoreCut={vi.fn()}
        onSeek={vi.fn()}
      />,
    );

    expect(container.querySelectorAll(".timeline-overview-raster")).toHaveLength(1);
    expect(container.querySelectorAll(".cue-region")).toHaveLength(1);
    expect(container.querySelectorAll(".virtual-list-row").length).toBeLessThan(30);
    expect(container).toHaveTextContent("Cue 0");
    expect(container).not.toHaveTextContent("Cue 4999");
  });
});
