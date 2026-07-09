# Risk Formulas

## Return Series

- Daily return: `r_t = close_t / close_t-1 - 1`
- Portfolio return: `r_p = w' * r`

## Annualization

- Annualized return (mean approximation): `mean_daily * 252`
- Annualized vol: `std_daily * sqrt(252)`
- Sharpe: `(ann_return - rf) / ann_vol`

## Drawdown

1. Build cumulative return curve.
2. Track running max.
3. Drawdown: `cum / running_max - 1`.
4. Max drawdown is minimum drawdown value.

## VaR and CVaR (Historical)

- VaR at alpha: empirical quantile of returns
- CVaR: mean of returns worse than VaR cutoff

## Optimization Defaults

- Long-only bounds: `[0, 1]`
- Sum of weights constraint: `1`
- Optional max weight cap to prevent concentration
