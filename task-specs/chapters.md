# Chapter generation task

Create concise chapter titles for meaningful topic changes in the supplied
ordered segments.

Return newline-delimited JSON, one object per line, without a surrounding
array or Markdown fence:

```json
{"title":"Introduction","startSeg":"first-segment-id"}
{"title":"Main topic","startSeg":"later-segment-id"}
```

The first chapter must begin at the first segment. Later `startSeg` values
must exist in the payload and be strictly increasing. Titles must be non-empty.
