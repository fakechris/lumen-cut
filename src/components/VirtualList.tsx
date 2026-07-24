import {
  memo,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type AriaRole,
  type ReactNode,
} from "react";

interface Props<T> {
  activeKey?: string | null;
  ariaLabel?: string;
  className: string;
  estimateHeight: number;
  followActive?: boolean;
  id?: string;
  itemKey: (item: T) => string;
  items: T[];
  overscan?: number;
  role?: AriaRole;
  renderItem: (item: T, index: number) => ReactNode;
}

interface MeasuredRowProps {
  itemKey: string;
  top: number;
  onMeasure: (key: string, height: number) => void;
  children: ReactNode;
}

const MeasuredRow = memo(function MeasuredRow({
  itemKey,
  top,
  onMeasure,
  children,
}: MeasuredRowProps) {
  const ref = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => onMeasure(itemKey, element.getBoundingClientRect().height);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [itemKey, onMeasure]);

  return (
    <div
      className="virtual-list-row"
      data-virtual-key={itemKey}
      ref={ref}
      style={{ transform: `translateY(${top}px)` }}
    >
      {children}
    </div>
  );
});

export function VirtualList<T>({
  activeKey = null,
  ariaLabel,
  className,
  estimateHeight,
  followActive = false,
  id,
  itemKey,
  items,
  overscan = 6,
  role,
  renderItem,
}: Props<T>) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const sizesRef = useRef(new Map<string, number>());
  const [measureVersion, setMeasureVersion] = useState(0);
  const [viewport, setViewport] = useState({ height: 600, top: 0 });

  const measurements = useMemo(() => {
    const offsets = new Array<number>(items.length);
    const heights = new Array<number>(items.length);
    const indices = new Map<string, number>();
    let total = 0;
    items.forEach((item, index) => {
      const key = itemKey(item);
      indices.set(key, index);
      offsets[index] = total;
      const height = sizesRef.current.get(key) ?? estimateHeight;
      heights[index] = height;
      total += height;
    });
    return { heights, indices, offsets, total };
  }, [estimateHeight, itemKey, items, measureVersion]);

  useLayoutEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const update = () => setViewport((current) => ({
      height: element.clientHeight || current.height,
      top: element.scrollTop,
    }));
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const firstVisible = useMemo(() => {
    let low = 0;
    let high = measurements.offsets.length - 1;
    let answer = 0;
    while (low <= high) {
      const middle = Math.floor((low + high) / 2);
      if (measurements.offsets[middle] + measurements.heights[middle] < viewport.top) {
        low = middle + 1;
      } else {
        answer = middle;
        high = middle - 1;
      }
    }
    return Math.max(0, answer - overscan);
  }, [measurements, overscan, viewport.top]);

  const lastVisible = useMemo(() => {
    const edge = viewport.top + viewport.height;
    let index = firstVisible;
    while (index < items.length && measurements.offsets[index] < edge) index += 1;
    return Math.min(items.length, index + overscan);
  }, [firstVisible, items.length, measurements.offsets, overscan, viewport]);

  useEffect(() => {
    if (!followActive || !activeKey) return;
    const index = measurements.indices.get(activeKey) ?? -1;
    const container = containerRef.current;
    if (index < 0 || !container || container.contains(document.activeElement)) return;
    const top = measurements.offsets[index];
    const bottom = top + measurements.heights[index];
    if (top >= container.scrollTop && bottom <= container.scrollTop + container.clientHeight) {
      return;
    }
    const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    const nextTop = Math.max(0, top - (container.clientHeight - measurements.heights[index]) / 2);
    const isLongJump = container.clientHeight > 0
      && Math.abs(nextTop - container.scrollTop) > container.clientHeight * 2;
    if (typeof container.scrollTo === "function") {
      container.scrollTo({
        behavior: reducedMotion || isLongJump ? "auto" : "smooth",
        top: nextTop,
      });
    } else {
      container.scrollTop = nextTop;
    }
  }, [activeKey, followActive, measurements]);

  const onMeasure = useMemo(() => (key: string, height: number) => {
    if (!Number.isFinite(height) || height <= 0) return;
    const previous = sizesRef.current.get(key);
    if (previous !== undefined && Math.abs(previous - height) < 0.5) return;
    sizesRef.current.set(key, height);
    setMeasureVersion((version) => version + 1);
  }, []);

  return (
    <div
      className={`${className} virtual-list`}
      id={id}
      aria-label={ariaLabel}
      onScroll={(event) => {
        const element = event.currentTarget;
        setViewport({ height: element.clientHeight, top: element.scrollTop });
      }}
      ref={containerRef}
      role={role}
    >
      <div className="virtual-list-spacer" style={{ height: `${measurements.total}px` }}>
        {items.slice(firstVisible, lastVisible).map((item, relativeIndex) => {
          const index = firstVisible + relativeIndex;
          const key = itemKey(item);
          return (
            <MeasuredRow
              itemKey={key}
              key={key}
              onMeasure={onMeasure}
              top={measurements.offsets[index]}
            >
              {renderItem(item, index)}
            </MeasuredRow>
          );
        })}
      </div>
    </div>
  );
}
