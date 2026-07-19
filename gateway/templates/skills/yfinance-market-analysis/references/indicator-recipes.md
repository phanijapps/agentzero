# Indicator Recipes

## Input Requirements

Need columns `open`, `high`, `low`, `close`, `volume` with a datetime index.

## Core Formulas

- EMA: `close.ewm(span=n, adjust=False).mean()`
- RSI:
  1. `delta = close.diff()`
  2. split gains and losses
  3. rolling means over `n`
  4. `100 - (100 / (1 + rs))`
- MACD: `EMA12 - EMA26`; signal is `EMA9(macd)`; histogram is `macd - signal`
- Bollinger: `SMA20 +/- 2 * rolling_std20`
- ATR: rolling mean of true range

## Multi-Timeframe Pattern

- Use higher timeframe for trend bias.
- Use lower timeframe for entry timing.
- Reject the setup if timeframes directly conflict.

## Expected Deliverables

- Technical summary narrative.
- Signal table with per-symbol/per-timeframe states.
- Optional price and indicator visualizations.
