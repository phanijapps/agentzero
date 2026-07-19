# Event Study Template

## Default Windows

- Pre-event baseline: `-20d` to `-6d`
- Event window: `-5d` to `+5d`
- Post-event drift: `+6d` to `+20d`

## Metrics

- Event-day gap: `(open_t / close_t-1) - 1`
- Event-day return: `(close_t / close_t-1) - 1`
- Forward returns: `t+1`, `t+5`, `t+20`
- Volatility shift: rolling std before vs after event

## Impact Scoring

- `high`: large absolute move plus elevated volume/volatility
- `medium`: moderate move with mixed confirmation
- `low`: small move or noisy setup

## Causality Hygiene

- Treat events as correlated evidence unless multiple sources confirm causality.
- Document conflicting or overlapping events in the same window.
