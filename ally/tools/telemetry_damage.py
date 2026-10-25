#!/usr/bin/env python3
"""Damage and composition report from hf_ally_parts_<unix>.csv (written next to the telemetry CSV).

usage: telemetry_damage.py hf_ally_parts_<unix>.csv
Prints, per ship id: the parts it was seen with (by oid), the ship-name candidates (strings found in the ship object),
and a timeline of health lost (sum of hp drops per second). hp numbers are the game's own per-part values.
"""
import csv, sys, collections

path = sys.argv[1]
rows = list(csv.DictReader([l for l in open(path) if not l.startswith("#")]))
comp = collections.defaultdict(collections.Counter)
names = {}
hp_by = {}
lost = collections.defaultdict(list)
drops = collections.defaultdict(lambda: collections.defaultdict(float))
for r in rows:
    k, ship = r["kind"], r["ship"]
    if k == "N": names[ship] = r["oid"]
    elif k == "S":
        comp[ship][r["oid"]] += 1
        hp_by[(ship, r["part"])] = float(r["hp"])
    elif k == "H":
        key = (ship, r["part"]); new = float(r["hp"])
        old = hp_by.get(key, new)
        if new < old: drops[ship][int(float(r["t"]))] += old - new
        hp_by[key] = new
    elif k == "L":
        lost[ship].append((float(r["t"]), r["part"]))
for ship in sorted(set(comp) | set(names), key=lambda s: int(s)):
    print(f"== ship {ship}" + (" (player)" if ship == "0" else ""))
    print("  parts:", ", ".join(f"{o or '?'} x{n}" for o, n in comp[ship].most_common(12)))
    strs = [s for s in names.get(ship, "").split("|") if s]
    print("  strings in ship object:", "; ".join(strs[:20]) or "none")
    d = drops[ship]
    tot = sum(d.values())
    print(f"  total part health lost: {tot:.0f}; parts removed from list: {len(lost[ship])}")
    if d:
        print("  health lost per second (t: hp):", ", ".join(f"{t}:{v:.0f}" for t, v in sorted(d.items())[:30]))
