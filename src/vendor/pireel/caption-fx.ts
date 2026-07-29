// Ported from pireel/pireel (AGPL-3.0), packages/studio-engine/src/caption-fx.ts
// Excerpt: the pure word logic only (segmentation / timing / line breaking).
// The canvas rendering half of caption-fx.ts (drawCaptionFx, kinetic slam/pop
// animations) is intentionally NOT ported — lumen-cut renders static preset
// styles + current-word highlight in the DOM preview and in exported ASS.

// lumen-cut's tsconfig targets ES2020 libs, which lack Intl.Segmenter types —
// declare the slice this module uses (all target runtimes ship full ICU).
declare namespace Intl {
  interface Segmenter {
    segment(text: string): Iterable<{ segment: string; isWordLike?: boolean }>;
  }
  // eslint-disable-next-line no-redeclare
  var Segmenter: new (locale: string, options: { granularity: 'word' }) => Segmenter;
}

/** Word + time window (absolute edited seconds). */
export interface FxWord {
  text: string;
  start: number;
  end: number;
  emphasis?: boolean;
  /** Original index within the source sentence's words (stamped on mapped/derived copies; edit/translation write-back key). */
  si?: number;
}

/** Two adjacent words both Latin/digit → a real space between them (adjacent CJK doesn't need one). Shared predicate
 *  across three places: render-layer .sp spacing, sentence rebuild (joinWords), transcript panel word stream. */
export const latinJoin = (a: string, b: string): boolean => /[A-Za-z0-9.,!?;:'")\]%]$/.test(a) && /^[A-Za-z0-9('"[$]/.test(b);

/** Word array → sentence text: add spaces at Latin word boundaries (join('') would run English together; the caption-track chip hit this). */
export function joinWords(texts: string[]): string {
  let out = '';
  texts.forEach((t, i) => {
    out += t;
    if (i < texts.length - 1 && latinJoin(t, texts[i + 1]!)) out += ' ';
  });
  return out;
}

/** ICU dictionary word segmentation (Intl.Segmenter, granularity 'word'): CJK breaks at real word
 *  boundaries (「科学家」 stays one token — the old fixed 2-char slicing cut through words), Latin splits
 *  on spaces, and punctuation glues onto the token before it (a subtitle token = word + trailing
 *  punctuation, never a standalone token). Runtimes without Intl.Segmenter (defensive; all our
 *  targets ship full ICU) fall back to whitespace + 2-char CJK slicing. */
let icuSegmenter: Intl.Segmenter | null | undefined;
export function segmentTokens(text: string): string[] {
  const t = text.trim();
  if (!t) return [];
  if (icuSegmenter === undefined) {
    try {
      icuSegmenter = new Intl.Segmenter('zh-Hans', { granularity: 'word' });
    } catch {
      icuSegmenter = null;
    }
  }
  if (!icuSegmenter) {
    const toks: string[] = [];
    for (const piece of t.split(/\s+/).filter(Boolean)) {
      if (/[一-鿿぀-ヿ가-힯]/.test(piece)) {
        for (let i = 0; i < piece.length; i += 2) toks.push(piece.slice(i, i + 2));
      } else toks.push(piece);
    }
    return toks;
  }
  const toks: string[] = [];
  for (const g of icuSegmenter.segment(t)) {
    const piece = g.segment.trim();
    if (!piece) continue;
    if (g.isWordLike || !toks.length) toks.push(piece);
    else toks[toks.length - 1] += piece;
  }
  return toks;
}

/**
 * Cut word-level timing from one caption's text + time window. Tokens come from ICU word
 * segmentation (segmentTokens); time is allocated linearly within [start,end] by char length.
 * This is the NO-ASR fallback — real word timing comes from ASR (groupAsrWords).
 */
export function wordsFromText(text: string, start: number, end: number): FxWord[] {
  const toks = segmentTokens(text);
  if (toks.length === 0) return [];
  const totalLen = toks.reduce((n, t) => n + t.length, 0) || 1;
  const span = Math.max(0.0001, end - start);
  let cur = start;
  const out: FxWord[] = [];
  for (const tk of toks) {
    const dur = (tk.length / totalLen) * span;
    out.push({ text: tk, start: round3(cur), end: round3(cur + dur) });
    cur += dur;
  }
  return out;
}

/** Letters+digits only (any script) — the alignment currency between ASR tokens and ICU words:
 *  punctuation/space counts are unreliable across tokenizers, letter counts are not. */
const alnumCount = (s: string): number => {
  let n = 0;
  for (const ch of s) if (/[\p{L}\p{N}]/u.test(ch)) n++;
  return n;
};

/** ASR word tokens (zh models often emit per-char/per-morpheme pieces) → ICU-word groups with merged
 *  timings: re-segment the sentence text into real words, then walk the ASR token stream and consume
 *  tokens by letter/digit count until each word is covered (first token's start / last token's end
 *  become the word's window). Any mismatch → return the raw ASR tokens unchanged (real timing beats
 *  pretty grouping). */
export function groupAsrWords(text: string, asr: { text: string; start: number; end: number }[]): FxWord[] {
  const flat = asr.filter((w) => w.text.trim() && w.end > w.start);
  const toks = segmentTokens(text);
  if (!toks.length || !flat.length) return flat.map((w) => ({ text: w.text.trim(), start: w.start, end: w.end }));
  const out: FxWord[] = [];
  let i = 0;
  for (const tk of toks) {
    const want = alnumCount(tk);
    if (want === 0) {
      // pure-punctuation token (only possible at sentence start — elsewhere punctuation is glued): merge into the previous word
      if (out.length) out[out.length - 1]!.text += tk;
      continue;
    }
    let got = 0;
    let first: number | null = null;
    let last = 0;
    while (i < flat.length && got < want) {
      if (first == null) first = flat[i]!.start;
      last = flat[i]!.end;
      got += alnumCount(flat[i]!.text);
      i++;
    }
    if (first == null) break;
    out.push({ text: tk, start: round3(first), end: round3(Math.max(first, last)) });
  }
  return out.length === toks.length ? out : flat.map((w) => ({ text: w.text.trim(), start: w.start, end: w.end }));
}

function round3(x: number): number {
  return Math.round(x * 1000) / 1000;
}

/** Group consecutive words into "screens". */
export function chunkWords(words: FxWord[], size: number): FxWord[][] {
  const out: FxWord[][] = [];
  const s = Math.max(1, size);
  for (let i = 0; i < words.length; i += s) out.push(words.slice(i, i + s));
  return out;
}

/** Target visual width of one caption line (in CJK-char units; a Latin char counts as half). */
export const CAPTION_LINE_UNITS = 13;
/** Visual width: CJK≈1, Latin/digit≈0.5. */
const visualWidth = (t: string) => [...t].reduce((a, ch) => a + (ch.charCodeAt(0) > 0x2e7f ? 1 : 0.5), 0);
const PUNCT_END = /[,。,.!?!?;;、::…]$/;

/**
 * Balanced line-breaking core (pretext-style: measure the whole sentence's total width → decide segment count →
 * break each segment near equal width, no "13+2" orphan tail; punctuation near the target width breaks first).
 * Don't split if the whole sentence fits. ONLY breaks within a sentence, NEVER pads across sentences (caller calls per sentence).
 * widthOf's unit is the caller's choice (coarse visual units / pixel estimate); limit shares widthOf's unit.
 */
export function chunkWordsBalanced<W extends { text: string }>(words: W[], limit: number, widthOf: (w: W) => number): W[][] {
  const widths = words.map(widthOf);
  const total = widths.reduce((a, b) => a + b, 0);
  if (total <= limit) return words.length ? [words] : [];
  const nSeg = Math.ceil(total / limit);
  const target = total / nSeg;
  const punctTol = limit * 0.15; // punctuation-priority band (scales with the unit)
  const out: W[][] = [];
  let cur: W[] = [];
  let len = 0; // current segment width
  let acc = 0; // total width consumed
  let segIdx = 1;
  for (let i = 0; i < words.length; i++) {
    cur.push(words[i]!);
    len += widths[i]!;
    acc += widths[i]!;
    if (i === words.length - 1) break;
    const boundary = segIdx * target;
    const nextW = widths[i + 1]!;
    // Break when: (1) punctuation and already near target, (2) current position is closer to the equal-width boundary than "swallow one more word", (3) hard-limit fallback.
    // If a balanced break is right next to punctuation, defer one word to break at the punctuation (readability first, still within tolerance).
    const punctBreak = PUNCT_END.test(words[i]!.text) && acc >= boundary - punctTol;
    const balancedBreak = acc >= boundary - target * 0.04 && Math.abs(acc + nextW - boundary) >= Math.abs(acc - boundary);
    const deferToPunct =
      !punctBreak && PUNCT_END.test(words[i + 1]!.text) && acc + nextW <= boundary + punctTol && len + nextW <= limit;
    if (!deferToPunct && (punctBreak || balancedBreak || len + nextW > limit + 1e-6)) {
      out.push(cur);
      cur = [];
      len = 0;
      segIdx++;
    }
  }
  if (cur.length) out.push(cur);
  return out;
}

/** Coarse visual-unit measure (CJK=1/Latin=0.5): the fallback when there's no font-size context. */
export function chunkWordsByWidth(words: FxWord[], maxUnits = CAPTION_LINE_UNITS): FxWord[][] {
  return chunkWordsBalanced(words, maxUnits, (w) => visualWidth(w.text));
}

/**
 * Per-char pixel width estimate (in em, times font size gives px): CJK/full-width (incl. full-width punctuation)=1;
 * uppercase/digit≈0.62; lowercase≈0.52; half-width punctuation/space≈0.34. One notch finer than "CJK=1/Latin=0.5";
 * combined with gap/padding it apportions precisely so split segments are guaranteed to fit the fixed-width caption box
 * (otherwise a visual line wrap = accident).
 */
export function estCharEm(ch: string): number {
  const c = ch.codePointAt(0) ?? 0;
  if (c >= 0x2e80) return 1; // from CJK radicals + full-width punctuation/kana/hangul
  if (/[A-Z0-9]/.test(ch)) return 0.62;
  if (/[a-z]/.test(ch)) return 0.52;
  return 0.34;
}
export function estWordEm(text: string): number {
  return [...text].reduce((a, ch) => a + estCharEm(ch), 0);
}
