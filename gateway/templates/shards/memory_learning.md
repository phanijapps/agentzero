MEMORY & LEARNING

Persistent memory across sessions via injected context and the `memory_write` tool.

## Recall
- Before starting any task, read the injected context packet for relevant knowledge (corrections, strategies, domain context).
- After entering a ward, use the injected ward/context packet and ward files for ward-specific knowledge.
- After a delegation completes, use the delivered result and injected context updates to absorb new learnings.
- Save important facts and corrections during execution so future sessions benefit.

## Categories
Use these categories for `memory_write`:
- `user` — preferences, style, capabilities (permanent)
- `pattern` — how-to knowledge, error workarounds, workflows (reinforced by reuse)
- `domain` — domain knowledge with hierarchical keys: `domain.finance.lmnd.outlook` (decays with time)

## Key Format
Use dot-notation hierarchy: `{category}.{domain}.{subdomain}.{topic}`
Examples:
- `user.report_style` = "Professional HTML with charts"
- `pattern.yfinance.multiindex` = "Flatten: [c[0] for c in df.columns]"
- `domain.finance.lmnd.outlook` = "Bullish short-term, RSI 74.9"

## Save Immediately
Don't batch — save as you learn:
- `memory_write(category="pattern", key="pattern.yfinance.multiindex", content="...", confidence=0.9)`

## Error Patterns
- `pattern.error.powershell_heredoc` = "Use write_file, not heredocs"
- `pattern.error.delegation_overflow` = "Keep subagent tasks focused"

## Success Patterns
- `pattern.workflow.stock_analysis` = "data-analyst + yfinance-market-analysis + coding"
