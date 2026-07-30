# Automatic work surfaces: model selection smoke

- Date: 2026-07-28
- Model: local `gpt-oss:latest` via Ollama
- Endpoint: local `/api/chat`
- Streaming: disabled
- Thinking: disabled in request
- Tools: fixed `present_surface` and `respond` definitions
- System guidance: fixed positive structured-data cases and negative simple-fact,
  short-prose, code-only, and clarification cases

Only observable tool calls are retained here; model reasoning is intentionally
excluded.

## Eligible prompt

> Compare weekly signups for the last four weeks: 120, 180, 150, and 240.
> Highlight the trend and show me the data clearly.

First tool call:

```json
{
  "name": "present_surface",
  "arguments": {
    "surface_id": "weekly_signup_trend",
    "components": [
      {
        "id": "line1",
        "type": "LineChart",
        "props": {
          "data_path": "/weekly",
          "x_key": "week",
          "series": ["signup"]
        }
      }
    ],
    "data": {
      "weekly": [
        {"week": "Week 1", "signup": 120},
        {"week": "Week 2", "signup": 180},
        {"week": "Week 3", "signup": 150},
        {"week": "Week 4", "signup": 240}
      ]
    },
    "update": true
  }
}
```

After a successful surface tool result, the next observable tool call was
`respond`, preserving a complete canonical text answer.

## Paired simple factual prompt

> What is the capital of France?

The only tool call was:

```json
{
  "name": "respond",
  "arguments": {"message": "Paris"}
}
```

No `present_surface` call was produced.

## Result

Pass: under the fixed configuration, the chart-shaped prompt selected
`present_surface`, while the paired factual prompt stayed text-only.
