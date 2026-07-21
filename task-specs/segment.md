# Paragraph segmentation task

Group the supplied sentences into natural paragraphs without changing any
sentence text.

Return one JSON object:

```json
{"paragraphs":["First sentence. Second sentence.","Next paragraph."]}
```

Each string must be the exact space-joined text of one or more consecutive
input sentences. Cover every input sentence once, preserve order, and never
split inside a sentence.
