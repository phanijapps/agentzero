# YFinance Recipes

## Symbol Formats

- US stocks/ETFs: `AAPL`, `SPY`
- Indices: `^GSPC`, `^IXIC`
- Forex: `EURUSD=X`
- Crypto: `BTC-USD`
- Futures: `GC=F`, `CL=F`

## Download Modes

- Batch mode: `yf.download([...], group_by="ticker", threads=True)`
- Single symbol fallback: `yf.Ticker(symbol).history(...)`

## Interval Constraints

- `1m` data is limited to recent windows.
- Intraday intervals have shorter lookback than daily/weekly/monthly intervals.

## Fallback Logic

1. Try `yf.download`.
2. If empty/error, call `Ticker.history`.
3. If still empty, attempt a shorter period and record the failure reason.

## Data Hygiene

- Convert index to UTC.
- Lowercase columns and standardize `adj_close`.
- Prefer both a columnar format and a portable format when persistence is needed.
- Convert all persisted payloads to JSON-safe values before `json.dump`.

## Portable JSON Helper

Use this pattern before writing yfinance-derived JSON artifacts:

```python
from datetime import date, datetime
import math

try:
    import numpy as np
except ImportError:
    np = None

try:
    import pandas as pd
except ImportError:
    pd = None


def json_safe(value):
    if value is None or isinstance(value, (str, bool, int)):
        return value
    if isinstance(value, float):
        return value if math.isfinite(value) else None
    if isinstance(value, (datetime, date)):
        return value.isoformat()
    if pd is not None:
        if isinstance(value, pd.Timestamp):
            return None if pd.isna(value) else value.isoformat()
        if isinstance(value, pd.DataFrame):
            return json_safe(value.reset_index().to_dict(orient="records"))
        if isinstance(value, pd.Series):
            return json_safe(value.to_dict())
    if np is not None:
        if isinstance(value, np.ndarray):
            return json_safe(value.tolist())
        if isinstance(value, np.generic):
            return json_safe(value.item())
    if isinstance(value, dict):
        return {str(k): json_safe(v) for k, v in value.items()}
    if isinstance(value, (list, tuple, set)):
        return [json_safe(v) for v in value]
    try:
        if pd is not None and pd.isna(value):
            return None
    except (TypeError, ValueError):
        pass
    return str(value)
```

Write `json.dump(json_safe(payload), f, indent=2)` and optionally keep
`default=str` as a last-resort guard. The helper is required for
`ticker.calendar`, `ticker.earnings_dates`, statement tables, option chains,
and news payloads because yfinance may return `date`, `Timestamp`, numpy
scalars, lists of dates, or pandas containers.

## News And Calendar Shapes

- `ticker.calendar` values may be lists containing `datetime.date` objects.
  Normalize the whole calendar with `json_safe` before extracting fields.
- For earnings dates, handle both scalar and list values:

```python
calendar = json_safe(ticker.calendar or {})
earnings_dates = calendar.get("Earnings Date")
next_earnings = (
    earnings_dates[0]
    if isinstance(earnings_dates, list) and earnings_dates
    else earnings_dates
)
```

- `ticker.news` may expose article data under a nested `content` object rather
  than top-level `title`, `publisher`, or `pubDate` fields. Normalize with a
  defensive extractor:

```python
def normalize_news_item(item):
    content = item.get("content") or item
    provider = content.get("provider") or {}
    canonical_url = content.get("canonicalUrl") or {}
    return json_safe({
        "title": content.get("title"),
        "publisher": provider.get("displayName") or item.get("publisher"),
        "published_at": (
            content.get("pubDate")
            or content.get("displayTime")
            or item.get("providerPublishTime")
        ),
        "url": canonical_url.get("url") or item.get("link"),
    })
```

Record a `missing_fields` or `data_quality` entry when normalized news items
lack title, publisher, timestamp, or URL fields.
