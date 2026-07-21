# Translation task

Translate every entry in `lines` into the requested target `lang`.

Return one JSON object:

```json
{
  "summary": "Short context summary",
  "terms": [{"term": "Product name", "observedVariants": []}],
  "namedEntities": ["Name"],
  "translations": {"line-id": "Translated text"}
}
```

Rules:

- `translations` must contain every input line id exactly once and no extra ids.
- Every translated value must be non-empty.
- Preserve meaning, negation, numbers, names, tone, and register.
- Strings listed in a line's `rt` array are locked and must appear verbatim.
- Prefer concise, natural phrasing that respects `maxChars`; do not omit
  meaning merely to meet the display budget.
- Do not insert manual line breaks.
