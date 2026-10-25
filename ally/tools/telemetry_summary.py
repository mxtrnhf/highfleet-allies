#!/usr/bin/env python3
"""Summary of hf_ally telemetry CSVs (Modloader/logs/hf_ally_telemetry_*.csv).

usage: telemetry_summary.py FILE.csv [...]

Everything here is derived from the raw columns; speeds and accelerations come from position differences
(units per game second). Range bands, 'hit' counts and similar are this script's own bookkeeping, not game values.
"""
import csv, math, sys
from collections import defaultdict

BANDS = [(0, 500), (500, 1000), (1000, 2000), (2000, 1e9)]

def rows(path):
    with open(path, newline="") as f:
        lines = [l for l in f if not l.startswith("#")]
    want = lines[0].count(",")
    for n, l in enumerate(lines[1:], 1):
        if l.count(",") != want:
            print(f"WARNING {path}: data row {n} has {l.count(',') + 1} fields, header has {want + 1}: {l.strip()[:120]}")
            break
    return list(csv.DictReader(lines))

def fnum(v, d=0.0):
    try: return float(v)
    except (TypeError, ValueError): return d

def summarize(path):
    data = rows(path)
    by_t = defaultdict(list)
    marks = []
    for r in data:
        if r["kind"] == "M":
            marks.append((fnum(r["t"]), int(fnum(r["id"]))))
        else:
            by_t[fnum(r["t"])].append(r)
    times = sorted(by_t)
    if len(times) < 3:
        print(f"{path}: too few samples ({len(times)})"); return
    dur = times[-1] - times[0]
    print(f"== {path}\n  {len(times)} samples, {dur:.1f} s game time, marks: {len(marks)}")

    prev = None
    speeds, accs, vels = [], [], []
    band_t = [0.0] * len(BANDS)
    boost_t = fire_t = 0.0
    flares = launches = 0
    shots = defaultdict(int)
    last = {}
    launches_all = defaultdict(int)
    lead_mx = []
    for t in times:
        grp = by_t[t]
        p = next((r for r in grp if r["kind"] == "P"), None)
        if not p: continue
        x, y = fnum(p["x"]), fnum(p["y"])
        if prev:
            dt = t - prev[0]
            if 0 < dt < 1:
                v = ((x - prev[1]) / dt, (y - prev[2]) / dt)
                speeds.append(math.hypot(*v))
                if len(vels) >= 5:
                    a = ((v[0] - vels[-5][0]) / (5 * dt), (v[1] - vels[-5][1]) / (5 * dt))
                    accs.append(math.hypot(*a))
                vels.append(v)
                dts = dt
                boost_t += dts * (fnum(p["boost"]) > 1.5)
                fire_t += dts * (p["lmb"] == "1" or p["rmb"] == "1")
                enemies = [r for r in grp if r["kind"] == "E" and fnum(r["side"]) < 0 and (fnum(r["gun_n"]) + fnum(r["zen_n"]) > 0)]
                if enemies:
                    d = min(math.hypot(fnum(r["x"]) - x, fnum(r["y"]) - y) for r in enemies)
                    for i, (lo, hi) in enumerate(BANDS):
                        if lo <= d < hi: band_t[i] += dts
        prev = (t, x, y)
        for r in grp:
            key = (r["kind"], r["id"])
            g = fnum(r["gun_reserve"]) + fnum(r["gun_loaded"])
            z = fnum(r["zen_loaded"])
            fl = fnum(r["fl_loaded"]) + fnum(r["fl_reserve"])
            zn = fnum(r["zen_n"]); gn = fnum(r["gun_n"])
            if key in last:
                # a destroyed gun part removes its rounds from the sums: only count drops while the gun count is unchanged
                if gn != last[key][3]: last[key] = (g, last[key][1], last[key][2], gn, last[key][4]); 
                if zn < last[key][4]: launches_all[key] += 1
                if fl < last[key][2] - 0.5: flares += key[0] == "P"
                if g < last[key][0] - 0.5: shots[key] += round(last[key][0] - g)
                if z < last[key][1] - 0.5: launches += key[0] == "P"
            last[key] = (g, z, fl, gn, zn)
    n = max(len(speeds), 1)
    print(f"  player speed (computed from positions) avg {sum(speeds)/n:.0f} max {max(speeds, default=0):.0f}; accel over ~0.5 s (computed, noisy when parts are shot off) avg {sum(accs)/max(len(accs),1):.0f}")
    tot = sum(band_t) or 1
    print("  time to nearest enemy: " + ", ".join(f"{lo:.0f}-{hi if hi<1e8 else 'inf':}: {100*b/tot:.0f}%" for (lo, hi), b in zip(BANDS, band_t)))
    print(f"  lmb/rmb held {fire_t:.1f} s, boost on {boost_t:.1f} s, flares dropped {flares} (loaded+reserve drop of the flare launchers)")
    print(f"  player rounds spent {shots.get(('P','0'),0)}; Zenith launchers lost/launched (zen_n drops): " + (", ".join(f"{k[0]}{k[1]}:{v}" for k,v in launches_all.items()) or "none"))
    others = {k: v for k, v in shots.items() if k[0] == "E"}
    print(f"  other ships: rounds spent {sum(others.values())} over {len(others)} ships (includes friends; only ships with guns/zenith count for range bands)")
    for t, m in marks: print(f"  MARK {m} at t={t:.1f} (+{t-times[0]:.1f} s)")

def aim_report(path):
    """The cursor floats vc2/vc3 are a world-aligned aim offset: atan2(vc3, vc2) points at what you aim at
    (checked 2026-10-18: while firing it matched the bearing to the nearest armed enemy, concentration 0.95)."""
    data = rows(path)
    by = defaultdict(dict)
    for r in data:
        if r["kind"] in "PE": by[fnum(r["t"])][(r["kind"], r["id"])] = r
    err_f, err_n, mags, turn = [], [], [], []
    prev = None
    for t in sorted(by):
        g = by[t]; p = g.get(("P", "0"))
        if not p or p["vc2"] in ("", "nan"): continue
        vx, vy = fnum(p["vc2"]), fnum(p["vc3"])
        m = math.hypot(vx, vy); mags.append(m)
        a = math.atan2(vy, vx)
        if prev is not None and t - prev[0] < 0.2 and m > 15 and prev[2] > 15:
            d = (a - prev[1] + math.pi) % (2 * math.pi) - math.pi
            turn.append(abs(math.degrees(d)) / (t - prev[0]))
        prev = (t, a, m)
        px, py = fnum(p["x"]), fnum(p["y"])
        en = [r for k, r in g.items() if k[0] == "E" and fnum(r["side"]) < 0 and fnum(r["gun_n"]) + fnum(r["zen_n"]) > 0]
        if not en or m < 15: continue
        e = min(en, key=lambda r: math.hypot(fnum(r["x"]) - px, fnum(r["y"]) - py))
        b = math.atan2(fnum(e["y"]) - py, fnum(e["x"]) - px)
        d = abs(math.degrees((a - b + math.pi) % (2 * math.pi) - math.pi))
        (err_f if p["lmb"] == "1" or p["rmb"] == "1" else err_n).append(d)
    def pct(xs, lim): return 100 * sum(x < lim for x in xs) / max(len(xs), 1)
    print(f"  aim (cursor direction, from vc2/vc3): cursor offset avg {sum(mags)/max(len(mags),1):.0f} of max 100")
    for name, xs in (("firing", err_f), ("not firing", err_n)):
        if xs:
            xs2 = sorted(xs)
            print(f"    {name}: aim error to nearest armed enemy median {xs2[len(xs2)//2]:.0f} deg, within 10 deg {pct(xs,10):.0f}%, within 30 deg {pct(xs,30):.0f}% ({len(xs)} samples)")
    if turn:
        t2 = sorted(turn); print(f"    aim sweep speed median {t2[len(t2)//2]:.0f} deg/s, 90th percentile {t2[int(len(t2)*.9)]:.0f} deg/s")

for path in sys.argv[1:]:
    summarize(path)
    try: aim_report(path)
    except KeyError: print("  (no aim columns in this older file)")

