# Transcript polish task

Correct recognition mistakes and improve punctuation while preserving the
speaker's words, order, meaning, repetition, and voice.

Return one JSON object:

```json
{
  "summary": "Short summary",
  "terms": [{"term": "Canonical term", "observedVariants": ["variant"]}],
  "namedEntities": ["Name"],
  "paragraphs": [{"sentences": ["Corrected sentence"]}]
}
```

Rules:

- Return the same number of paragraphs and sentences as the payload.
- Do not summarize, omit, reorder, or combine spoken content.
- Keep corrections close enough to the source for word timing to remain
  meaningful.
- Normalize a term only when the surrounding context makes the correction
  unambiguous.
- Leave filler and retake removal to the cleanup task.
