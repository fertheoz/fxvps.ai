#!/usr/bin/env python3
"""Back-fills chart history for the demo: writes core-data/candles-seed.txt.

The LP (LMAX FIX) gives no historical candles, so a fresh install shows empty charts.
This fetches recent OHLC bars from Yahoo Finance's public chart endpoint (no key) for
every FX instrument in lmax-instruments.txt and writes them in the client-gateway
candle format. The gateway loads the seed at start; its own bars (LP prices) replace
seed bars with the same open time, so the seed only fills what is missing.

    python3 seed-candles.py            # then: docker compose up -d --force-recreate trading

Demo use only: the prices are indicative (a pip or so off the LP) and the source is
not licensed for redistribution. Metals come from the futures contracts (scaled to the live spot level where the
live feed already has bars); crosses the source lacks are derived or inverted.
"""
import json
import os
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
INSTRUMENTS = os.path.join(HERE, "lmax-instruments.txt")
OUT = os.path.join(HERE, "core-data", "candles-seed.txt")
BARS = 600  # per symbol and timeframe (the terminal asks for 500)
# (source interval, range, derived timeframes in seconds)
FETCHES = [("1m", "2d", [60]), ("5m", "1mo", [300, 900, 1800]), ("60m", "2y", [3600, 14400, 86400])]
SCALE = 10**8  # domain::Fixed raw units


def fetch(ticker, interval, rng):
    url = f"https://query1.finance.yahoo.com/v8/finance/chart/{ticker}?interval={interval}&range={rng}"
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                res = json.load(r)["chart"]["result"]
            break
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return []
            time.sleep(5 * (attempt + 1))
        except (urllib.error.URLError, TimeoutError, ValueError):
            time.sleep(5 * (attempt + 1))
    else:
        return []
    if not res or not res[0].get("timestamp"):
        return []
    q = res[0]["indicators"]["quote"][0]
    rows = zip(res[0]["timestamp"], q["open"], q["high"], q["low"], q["close"])
    return [r for r in rows if None not in r and min(r[1:]) > 0]


def aggregate(rows, secs):
    """Source bars -> UTC-aligned buckets of `secs`, oldest first."""
    out = {}
    for t, o, h, l, c in rows:
        k = t - t % secs
        b = out.get(k)
        if b is None:
            out[k] = [o, h, l, c]
        else:
            b[1], b[2], b[3] = max(b[1], h), min(b[2], l), c
    return sorted(out.items())[-BARS:]


# Instruments the source has no spot symbol for: a futures contract, an inverted
# pair, or a cross derived from two series (metal in USD / USD pair).
ALIASES = {
    "XAUUSD": ("GC=F", None), "XAGUSD": ("SI=F", None), "XPTUSD": ("PL=F", None), "XPDUSD": ("PA=F", None),
    "XAUEUR": ("GC=F", "EURUSD=X"), "XAUAUD": ("GC=F", "AUDUSD=X"), "XAGAUD": ("SI=F", "AUDUSD=X"),
}
INVERTED = {"CNHSEK": "SEKCNH=X"}
CANDLES = os.path.join(HERE, "core-data", "candles.txt")


def own_close(symbol, secs=300):
    """Latest (time, close) of the live feed's bars for `symbol`, or None."""
    try:
        last = None
        with open(CANDLES, encoding="utf-8") as fh:
            for line in fh:
                f = line.split()
                if len(f) == 8 and f[0] == symbol and f[1] == str(secs):
                    t = int(f[2]) // 10**9
                    if last is None or t > last[0]:
                        last = (t, int(f[6]) / SCALE)
        return last
    except OSError:
        return None


def derive(rows, fx_rows):
    """metal/USD bars divided by a USD pair (bar-by-bar, by timestamp)."""
    fx = {t: c for t, _o, _h, _l, c in fx_rows}
    out = []
    for t, o, h, l, c in rows:
        r = fx.get(t)
        if r:
            out.append((t, o / r, h / r, l / r, c / r))
    return out


def invert(rows):
    return [(t, 1 / o, 1 / l, 1 / h, 1 / c) for t, o, h, l, c in rows if o and h and l and c]


def source_rows(symbol, interval, rng):
    """Bars for `symbol` from its direct ticker, alias, or inversion; None when unknown."""
    if symbol in INVERTED:
        return invert(fetch(INVERTED[symbol], interval, rng))
    if symbol in ALIASES:
        base, fx = ALIASES[symbol]
        rows = fetch(base, interval, rng)
        if fx:
            time.sleep(0.3)
            rows = derive(rows, fetch(fx, interval, rng))
        return rows
    return fetch(f"{symbol}=X", interval, rng)


def main():
    lines, done, skipped = [], 0, []
    for row in open(INSTRUMENTS, encoding="utf-8"):
        f = row.split()
        if len(f) < 3 or row.startswith("#"):
            continue
        symbol = f[0].replace("/", "")
        digits = len(f[2].split(".")[1]) if "." in f[2] else 0
        got = 0
        # Futures / derived series sit at a different level than the LP's spot
        # price: scale them so the seed meets the live bars without a step.
        factor = 1.0
        for interval, rng, frames in FETCHES:
            rows = source_rows(symbol, interval, rng)
            time.sleep(0.3)
            if not rows and interval == "1m":
                break  # unknown to the source: do not ask twice more
            if interval == "5m" and symbol in ALIASES:
                own = own_close(symbol)
                if own:
                    ref = [c for t, _o, _h, _l, c in rows if t <= own[0]]
                    if ref and ref[-1]:
                        factor = own[1] / ref[-1]
            for secs in frames:
                for t, (o, h, l, c) in aggregate(rows, secs):
                    o, h, l, c = (x * factor for x in (o, h, l, c))
                    raw = [int(round(round(x, digits) * SCALE)) for x in (o, max(o, h, c), min(o, l, c), c)]
                    lines.append(f"{symbol} {secs} {t * 10**9} {raw[0]} {raw[1]} {raw[2]} {raw[3]} 0" + chr(10))
                    got += 1
        if got:
            done += 1
        else:
            skipped.append(symbol)
        print(f"{symbol}: {got} bars" + (f" (x{factor:.5f})" if factor != 1.0 else ""), flush=True)
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    tmp = OUT + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline=chr(10)) as fh:
        fh.writelines(lines)
    os.replace(tmp, OUT)
    print(f"wrote {len(lines)} bars for {done} instruments to {OUT}; skipped: {' '.join(skipped) or '-'}")
    return 0 if done else 1


if __name__ == "__main__":
    sys.exit(main())
