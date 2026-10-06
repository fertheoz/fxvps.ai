"""LMAX kapı sınavı (elle, tek seferlik): emirleri NATS üzerinden doğrudan
lp-gateway-lmax'e gönderir. cl_ord_id öneki "YUK" olduğu için çekirdek bu
yürütmeleri yok sayar (müşteri hesabı/teminat devreye girmez). Alış/satış
sırayla gider, net pozisyon 0 kalır. Hız LMAX 100/sn sınırının ve 80/sn
frenin altında tutulur.

Kullanım (sunucu, ağ host):
  docker run --rm --network host -v $PWD:/w python:3.12-alpine sh -c \
    'pip -q install nats-py && python /w/lmax-kapi-sinavi.py --rate 60 --duration 120'
"""
import argparse, asyncio, json, time, statistics

import nats

ap = argparse.ArgumentParser()
ap.add_argument("--url", default="nats://127.0.0.1:4222")
ap.add_argument("--lp", default="LMAX")
ap.add_argument("--symbol", default="EUR/USD")
ap.add_argument("--qty", default="0.1")
ap.add_argument("--rate", type=float, default=60)
ap.add_argument("--duration", type=float, default=120)
a = ap.parse_args()

pre = f"fx.lp.{a.lp}"
sent, acked, filled, rejected, timeouts = {}, {}, {}, {}, 0
replies, fill_qty = {"ok": 0}, {"Buy": 0.0, "Sell": 0.0}


async def main():
    global timeouts
    nc = await nats.connect(a.url)

    async def on_event(msg):
        try:
            ev = json.loads(msg.data)
        except Exception:
            return
        x = ev.get("execution") or ev.get("Execution") or ev
        if not isinstance(x, dict):
            return
        cl = x.get("cl_ord_id") or ""
        if not cl.startswith("YUK") or cl not in sent:
            return
        now = time.perf_counter()
        et = str(x.get("exec_type", ""))
        if et == "New":
            acked.setdefault(cl, now)
        elif et == "Trade":
            acked.setdefault(cl, now)
            if cl not in filled:
                filled[cl] = now
                fill_qty[x.get("side", "Buy")] = fill_qty.get(x.get("side", "Buy"), 0) + float(x.get("last_qty") or x.get("qty") or 0)
        elif et in ("Rejected", "Canceled"):
            acked.setdefault(cl, now)
            if et == "Rejected":
                rejected[cl] = x.get("text") or et

    sub = await nc.subscribe(f"{pre}.events", cb=on_event)
    run = f"YUK{int(time.time())}"
    gap = 1.0 / a.rate
    t0 = time.perf_counter()
    n = 0
    pending = []

    async def one(i):
        global timeouts
        cl = f"{run}-{i}"
        side = "Buy" if i % 2 == 0 else "Sell"
        cmd = {"type": "submit", "cl_ord_id": cl, "symbol": a.symbol, "side": side,
               "qty": a.qty, "ord_type": "Market", "limit_price": None, "tif": "ImmediateOrCancel"}
        sent[cl] = time.perf_counter()
        try:
            r = await nc.request(f"{pre}.orders", json.dumps(cmd).encode(), timeout=2)
            k = r.data.decode()[:60]
            replies[k] = replies.get(k, 0) + 1
        except Exception as e:
            timeouts += 1
            replies[type(e).__name__] = replies.get(type(e).__name__, 0) + 1

    while time.perf_counter() - t0 < a.duration:
        target = t0 + n * gap
        d = target - time.perf_counter()
        if d > 0:
            await asyncio.sleep(d)
        pending.append(asyncio.create_task(one(n)))
        n += 1
    if n % 2:  # çift sayı: alış = satış, net pozisyon 0
        pending.append(asyncio.create_task(one(n)))
        n += 1
    await asyncio.gather(*pending)
    await asyncio.sleep(5)  # son yürütmeler
    await sub.unsubscribe()
    await nc.close()
    el = time.perf_counter() - t0 - 5

    def lat(d):
        v = sorted((d[c] - sent[c]) * 1000 for c in d)
        if not v:
            return {}
        return {"p50": round(statistics.median(v), 1), "p99": round(v[int(len(v) * .99) - 1 if len(v) > 1 else 0], 1), "max": round(v[-1], 1)}

    out = {"sent": n, "rate_per_s": round(n / el, 1), "replies": replies, "acked": len(acked),
           "filled": len(filled), "rejected": len(rejected),
           "reject_samples": list(set(rejected.values()))[:5],
           "ack_ms": lat(acked), "fill_ms": lat(filled), "fill_qty": fill_qty}
    print(json.dumps(out, ensure_ascii=False))

asyncio.run(main())
