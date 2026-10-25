# Summarises `ALLY probe` lines of a modloader log: python3 -I tools/probe_summary.py <log>
import re,sys,collections,statistics as st
f=sys.argv[1]
rows=[]
battle=0
for line in open(f,errors='replace'):
    if 'ALLY deploy' in line: battle+=1
    m=re.search(r'ALLY probe (ally |enemy) t=(\d+) c=(0x[0-9a-f]+) pos=\((-?\d+),(-?\d+)\) speed=(\d+) state=(-?\d+) mode=(-?\d+) target=(0x[0-9a-f]+) target_dist=(-?\d+) aggr=([\d.]+) retarget=([\d.-]+) flags=\[([\d, ]+)\] d60=([\d.-]+) launch_cnt=(-?\d+) \| zenith n=(\d+) loaded=([\d.-]+) reserve=(-?\d+) ready=(\d+) launches=(\d+) \| guns n=(\d+) loaded=([\d.-]+) reserve=(-?\d+) ready=(\d+) shots=(\d+)',line)
    if m:
        g=m.groups(); rows.append(dict(b=battle,who=g[0].strip(),t=int(g[1]),c=g[2],x=int(g[3]),y=int(g[4]),speed=int(g[5]),state=int(g[6]),mode=int(g[7]),target=g[8],td=int(g[9]),aggr=float(g[10]),flags=g[12],d60=float(g[13]),zn=int(g[15]),zl=float(g[16]),zr=int(g[17]),zready=int(g[18]),launches=int(g[19]),gn=int(g[20]),gl=float(g[21]),gr=int(g[22]),gready=int(g[23]),shots=int(g[24])))
print(len(rows),"rows")
for b in sorted(set(r['b'] for r in rows)):
    for who in ('ally','enemy'):
        rs=[r for r in rows if r['b']==b and r['who']==who]
        if not rs: continue
        ids=collections.Counter(r['c'] for r in rs)
        print(f"battle {b} {who}: n={len(rs)} ships={dict(ids)} t={rs[0]['t']}..{rs[-1]['t']}")
        print(f"   speed mean={st.mean(r['speed'] for r in rs):.0f} max={max(r['speed'] for r in rs)} | states={dict(collections.Counter(r['state'] for r in rs))} | target_dist med={st.median(r['td'] for r in rs if r['td']>=0) if any(r['td']>=0 for r in rs) else None}")
        print(f"   zenith n={rs[-1]['zn']} ready_mean={st.mean(r['zready'] for r in rs):.1f} loaded {rs[0]['zl']}->{rs[-1]['zl']} reserve {rs[0]['zr']}->{rs[-1]['zr']} launches(last)={rs[-1]['launches']} | guns n={rs[-1]['gn']} loaded {rs[0]['gl']}->{rs[-1]['gl']} reserve {rs[0]['gr']}->{rs[-1]['gr']} shots(last)={rs[-1]['shots']}")
        print(f"   flags seen={dict(collections.Counter(r['flags'] for r in rs).most_common(3))} d60 vals={sorted(set(r['d60'] for r in rs))[:6]}")
    # who do the enemies target
    ally=set(r['c'] for r in rows if r['b']==b and r['who']=='ally')
    en=[r for r in rows if r['b']==b and r['who']=='enemy']
    if en:
        tc=collections.Counter('ALLY' if r['target'] in ally else ('none' if r['target']=='0x0' else 'other(player?)') for r in en)
        print("   enemy targets:",dict(tc))
