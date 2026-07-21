# Speech cleanup task

Suggest conservative, reversible cuts for clear filler, false starts,
abandoned retakes, and excessive silence.

Return one JSON object:

```json
{
  "cuts": [{
    "a": "first-word-id",
    "b": "last-word-id",
    "cat": "filler",
    "reason": "Brief evidence"
  }]
}
```

`cat` must be `retake`, `filler`, `falseStart`, or `silence`. Retakes also
require `"alt":["kept-start-word-id","kept-end-word-id"]`.

Rules:

- Use only word ids from the payload and keep `a` through `b` in order.
- A cut cannot cross a paragraph or speaker boundary.
- Suggested cuts cannot overlap.
- Preserve complete meaning, useful setup, transitions, emphasis, and short
  listener responses. When uncertain, return no cut.
- Every cut requires a specific, non-empty reason.
