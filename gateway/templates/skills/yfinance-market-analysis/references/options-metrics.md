# Options Metrics

## Inputs

For each expiry, pull:

- calls table
- puts table
- spot price from `ticker.history(period="5d")`

## Core Metrics

- Put/Call OI ratio: `sum(put.openInterest) / sum(call.openInterest)`
- Put/Call volume ratio: `sum(put.volume) / sum(call.volume)`
- Max pain (approx): strike minimizing total option payout at expiry
- Term-structure slope: `IV_far - IV_near`

## Liquidity Filters

- Ignore rows with zero bid/ask and near-zero volume/OI.
- Require a minimum number of valid strikes before publishing skew/max pain.

## Confidence Tags

- `high`: dense chain, stable IV fields, good OI/volume
- `medium`: partial chain coverage
- `low`: sparse chain, many missing IV values
