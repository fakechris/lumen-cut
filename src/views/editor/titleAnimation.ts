import type { TitleClip } from "../../types";

export function titleOpacityAt(title: TitleClip, sourceTime: number): number {
  const elapsed = Math.max(0, sourceTime - title.start);
  const remaining = Math.max(0, title.end - sourceTime);
  const fadeIn = Math.max(0, title.fadeIn || 0);
  const fadeOut = Math.max(0, title.fadeOut || 0);
  return Math.min(
    1,
    fadeIn > 0 ? elapsed / fadeIn : 1,
    fadeOut > 0 ? remaining / fadeOut : 1,
  );
}
