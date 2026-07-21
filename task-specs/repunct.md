# Punctuation repair task

Choose punctuation marks at the supplied seam candidates. Do not rewrite or
echo transcript text.

Return one JSON object:

```json
{"segs":[{"id":4,"cuts":[{"id":"c-word-id","m":"，"}]}]}
```

Rules:

- Use only segment ids and candidate ids present in the payload.
- Do not repeat a segment or candidate id.
- Valid marks are `，。 、？！；：…` and `,.?!;:`.
- Include only changes that improve punctuation; an empty `cuts` array is
  valid when a segment needs no change.
