import json, sys, collections
S = r"C:\Users\hirom\AppData\Local\Temp\claude\C--Users-hirom--local-bin\df30284d-eb05-4a0d-9621-4ae94df538b2\scratchpad"
old = json.load(open(S + r"\old_map.json", encoding="utf-8"))
new = json.load(open(r"C:\Users\hirom\Desktop\strategy_game\strategy_game\assets\data\hex_atlas_map.json", encoding="utf-8"))
out = []
def P(*a):
    s = " ".join(str(x) for x in a)
    print(s); out.append(s)

P("grid equal:", old["grid"] == new["grid"], new["grid"])
oc = {(c["q"], c["r"]): c for c in old["cells"]}
nc = {(c["q"], c["r"]): c for c in new["cells"]}
P("cells old/new:", len(oc), len(nc), "same coordinate set:", set(oc) == set(nc))
W = ("sea", "lake")
def land(c): return c["terrain"] not in W
ol = {k for k, c in oc.items() if land(c)}
nl = {k for k, c in nc.items() if land(c)}
P("land old/new:", len(ol), len(nl), "gained:", len(nl - ol), "lost:", len(ol - nl))
for t in sorted({c["terrain"] for c in list(oc.values()) + list(nc.values())}):
    P("  terrain", t, sum(1 for c in oc.values() if c["terrain"] == t), "->", sum(1 for c in nc.values() if c["terrain"] == t))
flips = collections.Counter((oc[k]["terrain"], nc[k]["terrain"]) for k in oc if land(oc[k]) != land(nc[k]))
P("land/water flips by terrain pair:", dict(flips))
for f in ("terrain", "climate", "vegetation"):
    P("changed", f, sum(1 for k in oc if oc[k][f] != nc[k][f]))
P("countryId non-null cells:", sum(1 for c in nc.values() if c.get("countryId") is not None))
orv = {(r["q"], r["r"], r["direction"]) for r in old["rivers"]}
nrv = {(r["q"], r["r"], r["direction"]) for r in new["rivers"]}
P("rivers old/new:", len(orv), len(nrv), "added:", len(nrv - orv), "removed:", len(orv - nrv))
# preview region
import re
txt = open(r"C:\Users\hirom\Desktop\strategy_game\strategy_game\assets\data\world\preview\provinces.ron", encoding="utf-8").read()
pv = [(int(a), int(b), int(c)) for a, b, c in re.findall(r"id: (\d+),\s*q: (-?\d+),\s*r: (-?\d+)", txt)]
P("preview provinces:", len(pv))
# neighbors (pointy, odd-r? use axial)
dirs = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)]
def dist(q1, r1, q2, r2):
    dq, dr = q1 - q2, r1 - r2
    return max(abs(dq), abs(dr), abs(dq + dr))
region_old = {k for k in ol if dist(k[0], k[1], 83, 81) <= 3}
region_new = {k for k in nl if dist(k[0], k[1], 83, 81) <= 3}
P("land within dist 3 of (83,81): old", len(region_old), "new", len(region_new), "same:", region_old == region_new)
pcells = {(q, r) for _, q, r in pv}
P("preview cells == old region:", pcells == region_old, " == new region:", pcells == region_new)
P("preview cells still land in new:", all(k in nl for k in pcells), " attributes changed in preview cells:",
  [(k, {f: (oc[k][f], nc[k][f]) for f in ('terrain','climate','vegetation') if oc[k][f] != nc[k][f]}) for k in sorted(pcells) if any(oc[k][f] != nc[k][f] for f in ('terrain','climate','vegetation'))])
# neighbor change near region (dist<=4)
near = {k for k in oc if dist(k[0], k[1], 83, 81) <= 5}
P("cells within dist<=5 with any change:", [(k, oc[k]['terrain'], nc[k]['terrain']) for k in sorted(near) if any(oc[k][f] != nc[k][f] for f in ('terrain','climate','vegetation'))])
rr_old = [r for r in orv if dist(r[0], r[1], 83, 81) <= 4]; rr_new = [r for r in nrv if dist(r[0], r[1], 83, 81) <= 4]
P("rivers near region old/new, added, removed:", len(rr_old), len(rr_new), sorted(set(rr_new) - set(rr_old)), sorted(set(rr_old) - set(rr_new)))
# id analysis: old indices vs new generation
def gen(land_set):
    l = sorted((r, q) for q, r in land_set)
    return {(q, r): i for i, (r, q) in enumerate(l)}
go, gn = gen(ol), gen(nl)
bad_old = [(i, q, r) for i, q, r in pv if go.get((q, r)) != i]
P("stored preview ids == OLD map initial numbering:", not bad_old, "mismatch", len(bad_old))
shift = collections.Counter(gn[(q, r)] - i for i, q, r in pv)
P("stored id vs NEW-map initial numbering (delta histogram):", dict(shift))
P("max stored id:", max(i for i, _, _ in pv), "next_province_id in file: 34503; new land count:", len(nl))
P("new land cells with initial index < 34503 in NEW numbering (would collide with stored IDs):", sum(1 for k, i in gn.items() if i < 34503))
open(S + r"\cmp_maps_out.txt", "w", encoding="utf-8").write("\n".join(out) + "\n")
