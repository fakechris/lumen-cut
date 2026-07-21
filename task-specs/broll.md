# B-roll suggestion task

Suggest optional supporting visuals for concrete moments in the transcript.

Return one JSON object:

```json
{
  "suggestions": [{
    "start": "first-word-id",
    "end": "last-word-id",
    "mode": "fullscreen",
    "query": "specific visual search phrase",
    "reason": "why this visual helps"
  }]
}
```

Rules:

- Return at most eight suggestions.
- `mode` must be `fullscreen` or `pip`.
- Use valid, ordered word ids. Suggestions must not overlap.
- Each span must be 1.5 to 20 seconds long, start after the first 3 seconds,
  and end before the final 3 seconds.
- `query` and `reason` must be specific and non-empty.
- Return an empty array when supporting visuals would not improve the edit.
