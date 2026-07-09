# Fundamental Field Map

## Core Sources

- `ticker.info`: broad metadata and valuation fields
- `ticker.fast_info`: lean market fields
- `ticker.financials`: annual income statement table
- `ticker.quarterly_financials`: quarterly income statement table
- `ticker.balance_sheet`: annual balance sheet table
- `ticker.cashflow`: annual cash flow table

## Common Mapping

- Revenue: look for `Total Revenue`
- Gross profit: `Gross Profit`
- Operating income: `Operating Income`
- Net income: `Net Income`
- Total debt: `Total Debt`
- Cash and equivalents: `Cash And Cash Equivalents`

## Fallback Strategy

1. Prefer quarterly metrics for near-term trend.
2. Fall back to annual tables if quarterly is sparse.
3. If a field is unavailable, set `null` and add note in `missing_fields`.

## Suggested Derived Metrics

- Revenue growth YoY
- Gross margin and operating margin trend
- Free cash flow trend
- Debt-to-equity proxy
- Forward/trailing valuation spread
