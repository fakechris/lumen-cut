import { render } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { VirtualList } from "./VirtualList";

interface Item {
  id: string;
  label: string;
}

const itemKey = (item: Item) => item.id;
const renderItem = (item: Item) => <span>{item.label}</span>;

test("keeps long projects to a bounded number of mounted rows", () => {
  const items = Array.from({ length: 5_000 }, (_, index) => ({
    id: `cue-${index}`,
    label: `Cue ${index}`,
  }));

  const { container } = render(
    <VirtualList
      className="test-list"
      estimateHeight={40}
      itemKey={itemKey}
      items={items}
      renderItem={renderItem}
    />,
  );

  expect(container.querySelectorAll(".virtual-list-row").length).toBeLessThan(30);
  expect(container.querySelector(".virtual-list-spacer")).toHaveStyle({ height: "200000px" });
  expect(container).toHaveTextContent("Cue 0");
  expect(container).not.toHaveTextContent("Cue 4999");
});

test("jumps immediately when playback moves far outside the viewport", () => {
  const items = Array.from({ length: 200 }, (_, index) => ({
    id: `cue-${index}`,
    label: `Cue ${index}`,
  }));
  const scrollTo = vi.fn();
  const clientHeight = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "clientHeight",
  );
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 200,
  });
  Object.defineProperty(HTMLElement.prototype, "scrollTo", {
    configurable: true,
    value: scrollTo,
  });

  try {
    render(
      <VirtualList
        activeKey="cue-180"
        className="test-list"
        estimateHeight={40}
        followActive
        itemKey={itemKey}
        items={items}
        renderItem={renderItem}
      />,
    );

    expect(scrollTo).toHaveBeenCalledWith({
      behavior: "auto",
      top: 7120,
    });
  } finally {
    if (clientHeight) {
      Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
    } else {
      Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
    }
    Reflect.deleteProperty(HTMLElement.prototype, "scrollTo");
  }
});
