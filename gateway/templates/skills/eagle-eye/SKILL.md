---
name: "eagle-eye"
description: "Visual intelligence skill — use when asked to analyze images, screenshots, diagrams, charts, PDFs with visuals, or any task requiring sight. Extracts structured insights from visual content using the multimodal_analyze tool. Works even when the current agent runs on a text-only model."
allowed-tools: multimodal_analyze
metadata:
  author: "agentzero"
  version: "1.0.0"
  tags: "vision,multimodal,image-analysis,document-understanding"
  trigger_keywords: "image,screenshot,diagram,chart,visual,picture,photo,pdf,analyze image,describe image,what do you see,look at this"
  domain_hints: "vision,multimodal,visual-analysis,document-understanding"
---

# Eagle Eye — Visual Intelligence

You have the ability to see. Use the `multimodal_analyze` tool to process any visual content — images, screenshots, diagrams, charts, PDF pages, or photos.

## When to Use

- User shares an image or screenshot and asks "what is this?"
- User wants data extracted from a chart, table, or diagram
- User asks you to review a UI screenshot for layout issues
- User provides a PDF with visual content (charts, figures, scanned pages)
- User wants to compare two images side by side
- Any task where understanding visual content is required

## How It Works

You call the `multimodal_analyze` tool with content items and a prompt. The tool routes your request to a vision-capable model configured in Settings > Multimodal.

### Analyzing a Single Image

```json
{
  "name": "multimodal_analyze",
  "arguments": {
    "content": [
      { "type": "image", "source": "/path/to/screenshot.png", "detail": "high" }
    ],
    "prompt": "Describe the UI layout. Identify interactive elements, navigation, and any visual issues."
  }
}
```

### Extracting Data from a Chart

```json
{
  "name": "multimodal_analyze",
  "arguments": {
    "content": [
      { "type": "image", "source": "/path/to/chart.png" }
    ],
    "prompt": "Extract all data points from this chart. Return as a table with columns and values.",
    "output_schema": {
      "type": "object",
      "properties": {
        "chart_type": { "type": "string" },
        "title": { "type": "string" },
        "data_points": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "label": { "type": "string" },
              "value": { "type": "number" }
            }
          }
        }
      }
    }
  }
}
```

### Comparing Two Screenshots

```json
{
  "name": "multimodal_analyze",
  "arguments": {
    "content": [
      { "type": "image", "source": "/path/to/before.png" },
      { "type": "image", "source": "/path/to/after.png" }
    ],
    "prompt": "What changed between these two screenshots? List every visual difference."
  }
}
```

### Reading a PDF Page

```json
{
  "name": "multimodal_analyze",
  "arguments": {
    "content": [
      { "type": "file", "source": "/path/to/document.pdf" }
    ],
    "prompt": "Read this page. Extract all text, tables, and describe any figures or diagrams."
  }
}
```

## Detail Levels

- `"low"` — 512px, fast, fewer tokens. Good for simple images or quick checks.
- `"high"` — full resolution with tiling. Use for detailed analysis, small text, complex diagrams.
- `"auto"` (default) — let the model decide based on image content.

Use `"high"` when precision matters (data extraction, small text, technical diagrams).
Use `"low"` for quick descriptions or large obvious content.

## Tips

1. **Be specific in your prompt.** "Describe this image" gives generic results. "List all navigation items and their positions" gives structured data.
2. **Use output_schema for structured extraction.** When you need data in a specific format, provide a JSON Schema.
3. **Multiple images in one call** are great for comparisons, before/after, or multi-page analysis.
4. **File paths** can be absolute paths, `file://` URIs, or URLs. The tool resolves them automatically.
5. **For large PDFs**, process one page at a time. Shard the document first, then analyze each page.

## Requirements

- A vision-capable model must be configured in Settings > Multimodal (e.g., GPT-4o, Claude with vision).
- If no multimodal model is configured, the tool returns a clear error with setup instructions.

## Limitations

- The tool makes a one-shot LLM call — it doesn't have conversation context. Include all relevant context in your prompt.
- Video and audio content are not supported in this version.
- Very large images may be resized by the provider. Use `"detail": "high"` to preserve resolution.
