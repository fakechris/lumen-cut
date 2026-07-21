# Translation alignment task

Review each entry in `pairs` that reports a problem. Return a `pairs` array
covering every problem entry. Each item uses one of two actions.

Boundary-only adjustment:

```json
{"pairs":[{"id":"group-id","action":"recut","cuts":[{"s":"source-marker","t":"target-marker"}]}]}
```

Use only marker ids present in the input `sm` and `tm` strings. Markers in
each list must be strictly increasing. Each resulting target unit must fit
within 20 projected cells.

Translation rewrite:

```json
{
  "pairs": [{
    "id": "group-id",
    "action": "rewrite",
    "reasonCode": "grammar",
    "reason": "Brief explanation",
    "pieces": [{"through": "source-marker", "t": "Text"}, {"through": "end", "t": "Text"}]
  }]
}
```

Allowed `reasonCode` values are `mistranslation`, `omission`, `terminology`,
`grammar`, `translationese`, and `reorder`. Piece boundaries must be strictly
increasing and the final piece must use `through: "end"`. Every piece must be
non-empty, preserve the source meaning, and fit within 20 projected cells.
Avoid ASCII ellipses, `⋯`, `?!`, `!?`, and full-width digits.
