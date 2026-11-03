"""P32-027: series_run1.txt の同一呼出ごとの整合性確認(工程 ≤ 全体 ≤ 外側の時間 など)。"""
import re, sys, collections
def kv(l): return {m.group(1): m.group(2) for m in re.finditer(r"(\w+)=(\S+)", l)}
runs=[];cur=[]
for ln in open(sys.argv[1],encoding='utf-8').read().splitlines():
    if ln.startswith("EXIT "):
        d=kv(ln); runs.append((d["case"],d["mode"],int(d["pass"]),cur)); cur=[]
    else: cur.append(ln)
bad=0; n=0
print("case pass trial op | wall_ms total_ms covered_ms header_ms | header/total header/wall(別の値どうし) sum_spans")
for case,mode,pas,lines in runs:
    if mode!="on": continue
    wall={}
    for l in lines:
        if l.startswith("TRIAL "):
            d=kv(l); wall[int(d["trial"])]={"save":float(d["save_wall_ms"]),"restore":float(d["restore_wall_ms"])}
    tot={}; sp=collections.defaultdict(float)
    for l in lines:
        if l.startswith("TOTAL "):
            d=kv(l); tot[(int(d["trial"]),d["op"])]=(float(d["total_ms"]),float(d["covered_ms"]))
        if l.startswith("SPAN "):
            d=kv(l); sp[(int(d["trial"]),d["op"],d["name"])]+=float(d["ms"])
    for (t,op),(T,C) in sorted(tot.items()):
        hn="save.verify_decode_header" if op=="save" else "load.decode_header"
        H=sp[(t,op,hn)]; W=wall[t][op]
        s=sum(v for (tt,oo,_),v in sp.items() if tt==t and oo==op)
        ok = H<=T+1e-6 and C<=T+1e-6 and T<=W+1e-6 and abs(s-C)<0.01
        n+=1; bad+= (not ok)
        if case in ("1000x100","1000x3") and pas==2 or not ok:
            print(f"{case} p{pas} t{t} {op} | {W:.1f} {T:.1f} {C:.1f} {H:.1f} | {H/T:.4f} {H/W:.4f} {s:.1f} {'OK' if ok else 'VIOLATION'}")
print("checked calls:",n,"violations:",bad)
