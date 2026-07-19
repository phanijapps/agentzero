---
name: yfinance-market-analysis
description: Unified yfinance market analysis workflows for market data, technical signals, fundamentals and valuation, catalysts and event studies, options chains, and portfolio risk. Use when an agent needs to fetch or analyze stocks, ETFs, indices, forex, crypto, futures, single-ticker valuation, multi-ticker comparisons, technical indicators, event reaction analysis, options diagnostics, or portfolio analytics with yfinance.
---

# YFinance Market Analysis

## Overview

Use direct `yfinance` calls as the evidence source for market-analysis work.
Load only the reference files needed for the requested analysis, then produce
structured outputs with explicit dates, fields, missing-data caveats, and
confidence notes.

## Route The Task

- Raw OHLCV, ticker normalization, batching, asset-class symbol formats, or
  data-quality checks: use **Data Foundation** and read
  `references/yfinance-recipes.md`.
- RSI, MACD, EMA, Bollinger, ATR, trend, momentum, volatility, or chart-ready
  signal tables: use **Technical Signals** and read
  `references/indicator-recipes.md`.
- Equity valuation, growth, profitability, leverage, statement fields,
  earnings quality, or peer metric comparisons: use **Fundamentals And
  Valuation** and read `references/fundamental-field-map.md`.
- News, earnings dates, analyst actions, upcoming events, sentiment shifts, or
  reaction-window analysis: use **Catalysts And Event Studies** and read
  `references/event-study-template.md`.
- Options expiries, chains, put/call ratios, max pain, implied-volatility term
  structure, skew, or options sentiment: use **Options Chain Analysis** and read
  `references/options-metrics.md`.
- Multi-asset returns, drawdowns, volatility, Sharpe, VaR/CVaR, correlations,
  stress tests, or allocation diagnostics: use **Portfolio Risk** and read
  `references/risk-formulas.md`.

## Core Rules

- Pull current data before making factual market claims. Do not rely on model
  training data for prices, metrics, events, or analyst context.
- Treat yfinance as a delayed and sometimes sparse data source. Mark missing
  fields as `null` or `missing`, never as zero.
- Record the data source, access date, period, interval, timezone assumptions,
  and ticker universe in every deliverable.
- Separate reported fields from computed custom ratios.
- Emit a `missing_fields` or `data_quality` block whenever fields are absent,
  stale, partially covered, or inferred.
- Before writing JSON artifacts, normalize yfinance, pandas, numpy, date, and
  datetime values into portable JSON types. Do not call `json.dump` directly on
  raw `ticker.calendar`, statement tables, news payloads, option chains, or
  pandas objects.
- Treat `ticker.news` shape as version-dependent. Prefer nested `content`
  fields when present, and record missing title, publisher, timestamp, or URL
  fields instead of fabricating values.
- Prefer reusable code for repeated calculations, but keep task-specific output
  paths and schemas governed by the active plan or ward.

## Standard Workflow

1. Define the universe, horizon, interval, and output schema.
2. Verify the Python environment can import `yfinance`; if not, add or document
   the dependency in the project environment rather than silently relying on a
   global install.
3. Pull raw data with `yf.download` or `yf.Ticker`, using fallback logic from
   the relevant reference file.
4. Normalize data into predictable tables before analysis.
5. Convert output payloads with the JSON-safe helper from
   `references/yfinance-recipes.md` before persistence.
6. Run the routed analysis modules.
7. Return machine-readable artifacts plus a concise narrative that distinguishes
   observed data from interpretation.

## Data Foundation

Use `yf.download` for batch OHLCV first and `Ticker.history` as the fallback.
Normalize columns to `open/high/low/close/adj_close/volume`, sort timestamps
ascending, convert datetimes to UTC, reject empty frames, and flag stale,
duplicate, or gapped data.

## Technical Signals

Compute transparent indicators from cleaned OHLCV data. Default indicators are
EMA(20/50/200), RSI(14), MACD(12,26,9), ATR(14), and Bollinger(20,2). Emit one
row per symbol/timeframe with trend, momentum, volatility, score, and risk
flags. Mark analysis incomplete when bars are fewer than the longest lookback
plus buffer.

## Fundamentals And Valuation

Use `ticker.info`, `ticker.fast_info`, statement tables, earnings dates, and
calendar fields. Produce business snapshot, growth quality, profitability,
balance-sheet risk, valuation context, catalyst calendar, and a data confidence
score. Quote report periods for statement metrics and keep reported values
separate from derived ratios.

## Catalysts And Event Studies

Pull yfinance news, calendars, earnings dates, recommendations, and upgrade or
downgrade feeds when available. Deduplicate events, normalize timestamps, build
reaction windows, and score impact with confidence. Treat event timing as
correlated evidence unless outside sources confirm causality.

## Options Chain Analysis

Use `ticker.options` and `ticker.option_chain(expiry)` for selected expiries.
Clean calls and puts, enforce numeric types, filter low-liquidity rows, then
compute put/call ratios, max pain estimates, ATM IV, term-structure slope, and
skew proxies. Label results as approximate when chains are sparse or Greeks are
missing.

## Portfolio Risk

Pull aligned adjusted-close series, convert to returns, and document missing
date handling. Compute annualized return and volatility, Sharpe, max drawdown,
rolling correlations, historical VaR/CVaR, and scenario stress outputs. Do not
optimize weights without explicit constraints.

## Output Contract

Include these blocks when applicable:

- `inputs`: symbols, date range, interval, benchmark, assumptions.
- `raw_sources`: yfinance objects or endpoints used.
- `tables`: normalized data, metrics, signals, chains, or risk matrices.
- `missing_fields`: unavailable fields and fallback behavior.
- `confidence`: high/medium/low with reasons.
- `narrative`: concise conclusions with caveats and dates.

## References

Read only the reference files needed for the requested route:

- `references/yfinance-recipes.md`
- `references/indicator-recipes.md`
- `references/fundamental-field-map.md`
- `references/event-study-template.md`
- `references/options-metrics.md`
- `references/risk-formulas.md`
