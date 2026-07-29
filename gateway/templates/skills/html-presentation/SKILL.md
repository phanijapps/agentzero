---
name: html-presentation
description: >
  Create polished single-file HTML presentations matching the ARIA design system.
  Trigger on: "create a presentation", "make slides", "build a slide deck",
  "HTML presentation", or when the user wants to present information visually
  as a slide deck.
---

# ARIA Presentation Skill

Generate single-file HTML presentations that match the ARIA v1.0 design system — warm earth-tone palette, serif display headings, monospace labels, 1920x1080 canvas with JS viewport scaling, section nav legend, and staggered entrance animations.

## Output placement

- Use an exact task path only when the Active Ward Template permits it.
- Otherwise resolve a declared resource area or file rule that permits HTML.
- Never assume an output directory or filename.
- If no durable role is declared, write nothing and return
  `role_not_declared` with the template digest.

## Workflow

**Always follow this two-step intake before generating:**

### Step 1 — Ask for Context
Ask the user: *"What is this presentation about? Give me the content, topic, or talking points."*
Collect all the information they want to convey.

### Step 2 — Ask for Slide Layout
**Never assume layout from context.** After receiving context, present a numbered slide list and ask:
*"Here's the slide structure I'd suggest — please confirm or adjust:"*

```
1. Title slide (dark) — "Presentation Title"
2. Section divider (accent) — "Section Name"
3. Content slide — two-col layout with cards
...
```

For each slide, specify:
- Variant: title / dark / accent / content / image
- Layout: single-col / two-col / three-col / cards-grid / custom diagram
- Key content summary

Only generate HTML after the user confirms.

## Design System Quick Reference

### CSS Variables
```
--cream: #faf9f5          (light background)
--cream-dark: #f0eee6     (subtle alt background)
--ink: #141413             (dark text / dark bg)
--ink-light: #3d3d3a      (body text)
--terracotta: #d97757      (primary accent)
--terracotta-deep: #c6613f (hover/deep accent)
--muted: #75869680         (muted labels)
--sand: #e3dacc            (borders, dividers)
--warm-gray: #b0aea5       (secondary text)
--sage: #bcd1ca            (green accent)
--blush: #ebcece           (pink accent)
--lavender: #cbcadb        (purple accent)
--gold: #d4a27f            (gold accent)
```

### Fonts
```
--font-display: 'Cormorant Garamond', Georgia, serif   (h1-h3)
--font-body: 'DM Sans', system-ui, sans-serif           (body, UI)
--font-mono: 'JetBrains Mono', monospace                (labels, code, pills)
```

### Canvas
- Fixed 1920x1080 canvas, scaled to viewport via JS
- Slide padding: 60px top/bottom, 80px left/right
- Body background: #1a1a18 (dark surround)

### Typography Scale
| Element | Size | Weight | Extra |
|---------|------|--------|-------|
| h1 | 4.2rem | 400 | letter-spacing: -.02em |
| h2 | 3rem | 400 | letter-spacing: -.01em, mb: .6em |
| h3 | 1.9rem | 400 | mb: .4em |
| p | 1.25rem | 400 | line-height: 1.7, max-width: 720px |
| .label | .98rem | 500 | mono, uppercase, letter-spacing: .12em, terracotta |
| code | 1.25rem | 400 | mono |

### Animation Stagger
Elements get classes `anim-1` through `anim-6` for staggered fadeUp on slide entry:
- anim-1: 0.1s delay
- anim-2: 0.25s delay
- anim-3: 0.4s delay
- anim-4: 0.55s delay
- anim-5: 0.7s delay
- anim-6: 0.85s delay
- anim-fade: fadeIn at 0.3s delay

## Slide Variants

### 1. Title Slide (`.slide--title`)
Dark background (`--ink`), centered content, large serif heading (5.8rem, weight 300).
Use for: Opening slide, presentation title.

### 2. Section Divider (`.slide--accent`)
Terracotta background, centered, section number label, h1 at 4rem weight 300.
Use for: Chapter breaks between major sections.

### 3. Content Slide (default `.slide`)
Cream background. Supports layouts:
- **Single column**: Label + h2 + body content
- **Two column**: `.two-col` grid (1fr 1fr, gap 60px)
- **Three column**: `.three-col` grid (1fr 1fr 1fr, gap 40px)
- **Cards grid**: Cards in columns

### 4. Dark Slide (`.slide--dark`)
Dark background (`--ink`), cream text. Same layouts as content.
Use for: Emphasis, technical deep-dives, code-heavy slides.

### 5. Image Slide (`.slide--image`)
Full-bleed background image with gradient overlay. Content at bottom-left.
Use for: Screenshots, visual showcases.

### 6. End/Thank-You Slide
Light background, centered, minimal. Label + large h1 + divider + attribution.

## Output Structure

Every generated presentation must follow this exact HTML skeleton:

```
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>PRESENTATION_TITLE</title>
  <!-- Google Fonts: Cormorant Garamond, DM Sans, JetBrains Mono -->
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Cormorant+Garamond:ital,wght@0,300;0,400;0,500;0,600;0,700;1,300;1,400&family=DM+Sans:ital,opsz,wght@0,9..40,300;0,9..40,400;0,9..40,500;0,9..40,600;0,9..40,700;1,9..40,300;1,9..40,400&family=JetBrains+Mono:wght@300;400;500&display=swap" rel="stylesheet">
  <style>
    /* Full CSS from css-template.md */
    /* Plus any slide-specific CSS */
  </style>
</head>
<body>
<div class="deck" id="deck">

  <!-- SLIDES HERE -->

  <!-- Section Nav Legend -->
  <div class="section-nav hidden" id="sectionNav">
    <!-- nav items + separators -->
  </div>

  <!-- Progress bar -->
  <div class="progress" id="progress"></div>
  <div class="slide-counter" id="counter">1 / N</div>

</div><!-- /deck -->

<script>
  /* Navigation engine from js-template.md */
</script>

<script>
  /* Diagram editor from diagram-editor.md (if presentation has diagrams) */
</script>
</body>
</html>
```

## Key Rules

1. **Always ask for layout** — Never assume slide layout from content. Ask explicitly.

2. **Concise text** — Slide text must be concise. No paragraphs of prose. Use bullet points, short phrases, keywords. If user provides long text, distill it.

3. **Base64 images** — All images must be base64 encoded inline. Never use external image URLs. If user provides an image file, read it and encode. If no image is available, use CSS-only visuals.

4. **Diagrams get the editor** — Any slide with a positioned diagram layout must use `class="diagram-editable"` on the container and `class="diagram-el"` on positioned children. Include the diagram editor script (from `diagram-editor.md`). This lets the user press `e` to enable drag/resize editing and `d` to end edit and copy CSS positions to clipboard.

5. **Section nav reflects structure** — The section nav at the top must list all major sections. Update the `slideSection` array and `sectionFirstSlide` map in the JS to match actual slide structure.

6. **Decorative layers** — Every slide should include:
   - `<div class="noise"></div>` — subtle texture overlay
   - `<div class="grid-lines"></div>` — faint 80px grid (optional, good for content slides)
   - Corner glyphs on select slides (optional decorative touch)

7. **Stagger animations** — Use `anim-1` through `anim-6` on slide children for entrance animations. First element is `anim-1`, second is `anim-2`, etc.

8. **Multi-step slides** — For slides that reveal content progressively, add `data-steps="N"` to the slide div. Use CSS `step-1`, `step-2` classes to show/hide content. The JS navigation engine handles step progression automatically.

9. **Responsive scaling** — The 1920x1080 canvas scales to fit any viewport via the JS `resizeDeck()` function. Never use viewport units inside slides. All sizing is in rem/px relative to the 24px root.

10. **Single file** — Everything must be in one `.html` file. No external CSS, JS, or assets.

11. **Declared destination** — Verify the final path is task-specified or
    template-declared before delivery.

## Reference File Guide

Read these files from `references/` in this skill folder when you need more detail:

| File | When to Read |
|------|-------------|
| `design-system.md` | When you need exact color values, spacing rules, or typography details beyond the quick ref above |
| `css-template.md` | **Always read this** when generating a presentation — it contains the complete base CSS to paste into `<style>` |
| `js-template.md` | **Always read this** when generating — contains the navigation engine JS |
| `slide-variants.md` | When you need exact HTML templates for each slide type |
| `component-library.md` | When the user wants cards, pills, tables, code blocks, terminals, or other UI components |
| `diagram-editor.md` | When the presentation includes any positioned/diagrammatic layouts that should be editable |
| `animation-patterns.md` | When the user wants advanced animations beyond basic fadeUp (e.g., rail draw, runner dots, swimlanes) |

### For Simple Presentations (5-10 slides, no diagrams)
Read: `css-template.md` + `js-template.md`. The quick ref in this file covers everything else.

### For Complex Presentations (diagrams, animations, many components)
Read: `css-template.md` + `js-template.md` + `component-library.md` + `diagram-editor.md` + `animation-patterns.md`.

## Noto Sans KR

If the presentation contains Korean text, add `Noto Sans KR` to the Google Fonts link:
```
family=Noto+Sans+KR:wght@400;500;700
```
And use `font-family: 'Noto Sans KR', sans-serif` for Korean text elements.
