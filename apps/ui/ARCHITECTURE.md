# UI Architecture Principles

## SOLID Principles for UI

### Single Responsibility
- **theme.css**: Only defines design tokens (colors, spacing, typography, shadows)
- **components.css**: Only defines reusable component classes
- **React components**: Only handle logic and structure, NOT styling decisions

### Open/Closed Principle
- Components are **open for extension** via CSS modifier classes (e.g., `.btn--primary`, `.card--interactive`)
- Components are **closed for modification** - don't change base classes, add modifiers
- To change UI appearance, edit CSS files, NOT component code

### Liskov Substitution
- Any component using `.btn` can use `.btn--primary` or `.btn--secondary` interchangeably
- Variant classes extend base behavior, don't break it

### Interface Segregation
- Small, focused CSS classes that do one thing
- Don't create "god classes" that handle everything
- Compose multiple classes: `class="btn btn--primary btn--sm"`

### Dependency Inversion
- Components depend on **abstract** design tokens (`var(--primary)`), not concrete values
- Never hardcode colors, spacing, or fonts in components

---

## File Structure

```
src/styles/
├── index.css          # Entry point - imports in correct order
├── theme.css          # Design tokens (CSS custom properties)
└── components.css     # Reusable component classes
```

### theme.css - Design Tokens

Contains ALL design decisions:
- Colors (with semantic names: `--primary`, `--destructive`, `--muted-foreground`)
- Typography (`--text-sm`, `--font-mono`)
- Spacing scale (`--spacing-1` through `--spacing-12`)
- Border radius (`--radius-sm`, `--radius-md`, `--radius-lg`)
- Shadows (`--shadow-card`, `--shadow-modal`)
- Layout dimensions (`--sidebar-width`, `--modal-width`)

**Rule**: To change how the app looks, edit ONLY this file.

### components.css - Component Classes

Defines reusable patterns using design tokens:

```css
.card {
  background-color: var(--card);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-card);
}

.btn--primary {
  background-color: var(--primary);
  color: var(--primary-foreground);
}
```

**Rule**: These classes use tokens from theme.css, never hardcoded values.

---

## Component Class Naming

Follow BEM-inspired conventions:

```
.block                    # Component root (e.g., .card, .btn, .modal)
.block--modifier          # Variant (e.g., .card--interactive, .btn--primary)
.block__element           # Child element (e.g., .card__header, .btn__icon)
.block__element--modifier # Child variant (e.g., .modal__header--compact)
```

### Examples:

```html
<!-- Card with interactive hover state -->
<div class="card card--interactive card__padding">
  <div class="card__header">
    <div class="card__icon card__icon--primary">...</div>
  </div>
</div>

<!-- Primary button, medium size -->
<button class="btn btn--primary btn--md">
  <span class="btn__icon">...</span>
  Submit
</button>

<!-- Split panel layout -->
<div class="split-panel">
  <div class="split-panel__sidebar">...</div>
  <div class="split-panel__main">...</div>
</div>
```

---

## What NOT to Do

### Never put styles inline in components

```jsx
// BAD - styles mixed in component
<div className="bg-[var(--card)] rounded-xl p-4 shadow-lg hover:shadow-xl">

// GOOD - semantic class names
<div className="card card--interactive card__padding">
```

### Never hardcode values

```css
/* BAD */
.card { background: #ffffff; border-radius: 12px; }

/* GOOD */
.card { background: var(--card); border-radius: var(--radius-lg); }
```

### Never mix concerns

```jsx
// BAD - component decides styling
const Card = () => (
  <div style={{ padding: isLarge ? '24px' : '16px' }}>

// GOOD - component uses CSS classes
const Card = ({ size }) => (
  <div className={`card card__padding${size === 'lg' ? '--lg' : ''}`}>
```

---

## Available Component Classes

### Layout
- `.app-shell` - Main app container (flex, full height)
- `.page` - Page wrapper with scroll
- `.page-container` - Centered content with max-width
- `.page-header` - Title + actions layout
- `.split-panel` - Sidebar + main content layout

### Sidebar
- `.sidebar` - Dark sidebar container
- `.nav-link`, `.nav-link--active` - Navigation items
- `.connection-status` - Status indicator

### Cards
- `.card` - Base card with shadow
- `.card--interactive` - Hover state
- `.card--bordered` - Border instead of shadow
- `.card__padding`, `.card__padding--lg` - Padding variants
- `.card__icon--primary/warning/success/destructive` - Icon backgrounds

### Buttons
- `.btn` - Base button
- `.btn--sm/md/lg` - Sizes
- `.btn--primary/secondary/ghost/outline/destructive` - Variants
- `.btn--icon`, `.btn--icon-ghost` - Icon-only buttons

### Forms
- `.form-group` - Label + input wrapper
- `.form-label` - Label styling
- `.form-input` - Text input
- `.form-textarea` - Textarea
- `.form-select` - Select dropdown

### Feedback
- `.badge`, `.badge--primary/success/warning/destructive` - Status badges
- `.alert`, `.alert--info/success/warning/error` - Alert messages
- `.empty-state` - Empty content placeholder
- `.loading-spinner` - Loading indicator

### Lists
- `.list-item`, `.list-item--active` - Clickable list items
- `.list-category` - Category header

### Modal
- `.modal-backdrop` - Overlay
- `.modal`, `.modal--sm/lg` - Dialog container
- `.modal__header/body/footer` - Modal sections

### Shared Slide-over
- `.slideover-backdrop`, `.slideover-backdrop--open` - Overlay
- `.slideover`, `.slideover--open` - Right-side panel (540px)
- `.slideover__header/body/footer` - Panel sections (--spacing-8 padding)
- `.slideover__section` - Section with bottom border separator
- `.slideover__section-title` - Uppercase label with line divider
- `.slideover__icon/title/subtitle/close` - Header elements

### Shared Components
- `.tab-bar` - Horizontal tab container with active underline
- `.tab-bar__tab`, `.tab-bar__tab--active` - Tab items with count badges
- `.action-bar` - Search + filter + action button row
- `.action-bar__search`, `.action-bar__search-input` - Search field
- `.filter-chip`, `.filter-chip--active` - Filter toggle chips
- `.help-box`, `.help-box__icon`, `.help-box__content` - Contextual help
- `.meta-chip`, `.meta-chip--{variant}` - Metadata badges (model/skills/mcps/schedule/tools/stdio/http/sse/plugin/worker/enabled/disabled/running/error/stopped/starting)
- `.card-grid`, `.card-grid--2col` - Responsive card grid layout
- `.status-dot`, `.status-dot--idle` - Pulsing status indicator
- `.info-tip` - Inline (i) tooltip circle
- `.add-link` - "+ Add" styled link button
- `.toggle-switch`, `.toggle-switch--on/off` - Toggle switch control

### Page Layout (v2)
- `.page-header-v2` - Page title area (--spacing-8 padding)
- `.page-title-v2` - Page heading (text-2xl, weight 600)
- `.page-subtitle-v2` - Descriptive subtitle
- `.page-content-v2` - Scrollable content area

### Settings Page (tabbed: Providers | General | Logging)
- Provider components in `features/settings/`: ProviderCard, ProviderSlideover, ProvidersGrid, ProvidersEmptyState, providerPresets
- `.provider-grid`, `.provider-card`, `.model-chip` - Provider-specific classes
- `.settings-info-card`, `.settings-toggle-option` - Settings form controls

### Agents Page (tabbed: My Agents | Skills Library | Schedules)
- `.agent-card`, `.agent-card__top/avatar/info/name/id/desc/meta/footer` - Agent cards
- `.skill-card`, `.skill-card__header/name/desc` - Skill cards
- `.schedule-card`, `.schedule-card__icon/info/name/cron/agent/meta` - Schedule items
- `.skill-toggle`, `.skill-toggle--on` - Toggle rows for MCP/skill selection

### Integrations Page (tabbed: Tool Servers | Plugins & Workers)
- `.ts-card`, `.ts-card__top/icon/info/name/desc/meta/footer` - Tool server cards
- `.ts-card__icon--stdio/http/sse` - Transport type icon variants
- `.ts-detail-row`, `.ts-test-result`, `.ts-tool-item` - Slide-over detail elements
- `.pw-card`, `.pw-card__top/icon/info/name/desc/meta/footer` - Plugin/worker cards
- `.pw-card__icon--plugin/worker` - Source type icon variants

### Waterfall Interactive Components
- `.waterfall--interactive` - Interactive waterfall container (position: relative for overlay positioning)
- `.waterfall-dot`, `.waterfall-dot:hover` - Clickable tool dots with hover brightness
- `.waterfall-dot--selected-ring` - Pulsing ring on selected dot
- `.waterfall-dot--delegation-highlight` - Subtle pulse on dots within hovered delegation range
- `.waterfall-bar`, `.waterfall-bar:hover` - Clickable delegation bars with hover opacity
- `.waterfall-bar--selected-ring` - Pulsing ring on selected bar
- `.waterfall-tooltip` - Floating HTML tooltip positioned over SVG
- `.waterfall-tooltip__header/body/status` - Tooltip layout elements
- `.waterfall-tooltip__status--success/error` - Status color variants
- `.waterfall-slideout` - Right-side detail panel (320px, slides in from right)
- `.waterfall-slideout__header/section/label/code/event/nav` - Panel layout elements
- `.waterfall-slideout__header--error` - Error-themed header
- `.waterfall-slideout__code--error` - Error-themed code block
- `.waterfall-slideout__event--highlight/error/warn` - Event row variants
- `.waterfall-slideout__meta-row` - Key-value metadata display
- `.waterfall-slideout__nav-counter` - Navigation position indicator

---

### Memory Tab — Command Deck

Three-column layout under `apps/ui/src/features/memory/command-deck/`:

- **Top bar:** `SearchBar` (FTS/HYBRID/SEMANTIC mode toggle, auto-fts for quoted phrases) + `ScopeChips` (per-type filter).
- **Left (`WardRail`):** wards list with counts, global (`__*`) wards grouped separately.
- **Center (`ContentDeck`):** tabs Facts / Wiki / Procedures / Episodes (+ Graph ↗ passthrough). Facts rendered via `ContentList` grouped by age bucket (TODAY / LAST 7 DAYS / HISTORICAL) with opacity decay.
- **Right (`WriteRail` + `AddDrawer`):** `+ Fact / + Instruction / + Policy` presets (`+ Policy` maps to `instruction` category because the `MemoryCategory` union has no `policy` value).

**Gating:** behind feature flag `memory_tab_command_deck` (see `useFeatureFlag.ts`). The `MemoryTabGate` component picks legacy vs command-deck. Barrel `features/memory/index.ts` re-exports the gate as `MemoryTab`, so future consumers of the symbol get the gated version transparently.

**Data sources:**
- `GET /api/wards/:ward_id/content` → aggregated ward content with server-stamped `age_bucket`
- `POST /api/memory/search` → unified hybrid search across 4 types with `match_source` provenance
- `GET /api/wards` → ward id + fact-count list for the rail
- Existing `createMemory` transport → write-rail saves

**Recency decay** uses the server-stamped `age_bucket` classifier. The UI maps buckets to CSS classes `.memory-item.decay-1` (last_7_days), `.memory-item.decay-2` (historical); "today" gets full opacity. The opacity curve lives entirely in CSS — no JS tick.

**Search mode semantics:**
- `hybrid` (default) — FTS5 + sqlite-vec with reciprocal-rank fusion, degrades to FTS-only when embedding backend unavailable
- `fts` — never calls the embedding backend
- `semantic` — requires embedding, bubbles 400 on failure
- Quoted phrases (`"exact phrase"`) force FTS regardless of selected mode

---

## Adding New Components

1. Define CSS class in `components.css`
2. Use ONLY design tokens from `theme.css`
3. Follow BEM naming convention
4. Add modifiers for variants
5. Document in this file

---

## Theming

To create a new theme:
1. Copy the `:root` section from `theme.css`
2. Create a new class (e.g., `.theme-dark`, `.theme-blue`)
3. Override the CSS custom properties
4. Apply the class to `<html>` or `<body>`

```css
.theme-corporate {
  --primary: #0066cc;
  --primary-hover: #0052a3;
  --sidebar: #003366;
  /* ... */
}
```

---

## Migration Guide

When refactoring existing components:

1. Identify inline Tailwind classes
2. Map to semantic component classes
3. If no class exists, create one in `components.css`
4. Replace inline styles with class names
5. Test that appearance is unchanged
