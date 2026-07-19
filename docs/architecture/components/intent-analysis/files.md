# Intent Analysis — File Reference

## Backend (Rust)

### Core Middleware
| File | What |
|------|------|
| `gateway/gateway-execution/src/middleware/intent_analysis.rs` | Types, LLM prompt, `format_intent_injection()`, `analyze_intent()` (3 params), `index_resources()`, `search_resources()`, `strip_markdown_fences()`, unit tests |

### Runner Integration
| File | What |
|------|------|
| `gateway/gateway-execution/src/runner.rs` | `OnSessionReady` type alias, `invoke_with_callback()`, session gate (`has_intent_log`), `format_intent_injection` call, `index_resources` call, event emission, fallback handling |

### Crate Exports
| File | What |
|------|------|
| `gateway/gateway-execution/src/lib.rs` | Exports `ExecutionRunner`, `OnSessionReady` |

### Gateway Events
| File | What |
|------|------|
| `gateway/gateway-events/src/lib.rs` | `IntentAnalysisStarted`, `IntentAnalysisComplete` event variants |

### WebSocket Protocol
| File | What |
|------|------|
| `gateway/gateway-ws-protocol/src/messages.rs` | `ServerMessage::IntentAnalysisStarted`, `ServerMessage::IntentAnalysisComplete` |

### WebSocket Handler
| File | What |
|------|------|
| `gateway/src/websocket/handler.rs` | Builds `OnSessionReady` closure for early subscription, calls `invoke_with_hook_and_callback()` |

### Runtime Service
| File | What |
|------|------|
| `gateway/src/services/runtime.rs` | `invoke_with_hook_and_callback()` — threads callback to runner |

### Log Service
| File | What |
|------|------|
| `services/api-logs/src/repository.rs` | `has_category_log()` — efficient `SELECT 1 ... LIMIT 1` |
| `services/api-logs/src/service.rs` | `has_intent_log()` — wraps repo call |
| `services/api-logs/src/types.rs` | `LogCategory::Intent` |

### Integration Tests
| File | What |
|------|------|
| `gateway/gateway-execution/tests/intent_analysis_tests.rs` | 5 tests: full enrichment + injection, LLM failure, malformed JSON, simple request, skills recommended |

## Frontend (TypeScript/React)

### Hook & State
| File | What |
|------|------|
| `apps/ui/src/features/research-v2/useResearchSession.ts` | Subscribes to WS, hydrates from snapshot, drives reducer |
| `apps/ui/src/features/research-v2/event-map.ts` | Maps `intent_analysis_started` / `intent_analysis_complete` / `intent_analysis_skipped` (and `ward_changed`) into `ResearchAction`s |
| `apps/ui/src/features/research-v2/reducer.ts` | Applies actions; sets `intentAnalyzing` flag and `intentClassification` |
| `apps/ui/src/features/research-v2/session-snapshot.ts` | Builds replay snapshot (incl. ward name from `/api/sessions/:id/state`) |

### UI Components
| File | What |
|------|------|
| `apps/ui/src/features/research-v2/ResearchPage.tsx` | Renders `IntentLine` (analyzing… / classification chip) and the `IntentInfoButton` next to the title |
| `apps/ui/src/features/research-v2/IntentInfoButton.tsx` | Popover button that surfaces full intent metadata for the active session |
| `apps/ui/src/features/research-v2/SessionTurnBlock.tsx` | Per-turn renderer used for both live and replayed turns |

### Styling
| File | What |
|------|------|
| `apps/ui/src/styles/components.css` | `.intent-analysis-block` styles, `.intel-section` collapsible styles |

## Planning Shards
| File | What |
|------|------|
| `gateway/templates/shards/planning_autonomy.md` | Rule: "Follow Intent Analysis" — agent reads the injected `## Intent Analysis` section |
