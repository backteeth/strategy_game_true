"""P32-029: 検証地域(プレビューの29プロヴィンス)の隣接グラフの事実を、実ファイルから数える(読み取りのみ。何も書かない)。
使い方(strategy_game/ で): python -I verification_logs/phase-32/p32-029/region_graph_facts.py
前提: 隣接は軸座標の6近傍(src/hexmap/data.rs の RIVER_NEIGHBOR_OFFSETS)。陸水は map の terrain(sea/lake が水域)。"""
import json, re, collections

OFF = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)]
m = json.load(open("assets/data/hex_atlas_map.json", encoding="utf-8"))
cells = {(c["q"], c["r"]): c for c in m["cells"]}
water = lambda k: k not in cells or cells[k]["terrain"] in ("sea", "lake")
rivers = {(r["q"], r["r"], r["direction"]) for r in m["rivers"]}

def river_between(a, b):
    for d, (dq, dr) in enumerate(OFF):
        if (a[0] + dq, a[1] + dr) == b:
            return (a[0], a[1], d) in rivers or (b[0], b[1], (d + 3) % 6) in rivers
    return False

prov = open("assets/data/world/preview/provinces.ron", encoding="utf-8").read()
P = {int(i): (int(q), int(r)) for i, q, r in re.findall(r"id: (\d+),\s*q: (-?\d+),\s*r: (-?\d+)", prov)}
st = open("assets/data/world/preview/states.ron", encoding="utf-8").read()
S = {}
for sid, body in re.findall(r"id: (\d+),\s*name_key: \"[^\"]+\",\s*provinces: \[(.*?)\]", st, re.S):
    for p in re.findall(r"\d+", body):
        S[int(p)] = int(sid)
cell2p = {v: k for k, v in P.items()}
fac = open("assets/data/world/preview/province_facilities.ron", encoding="utf-8").read()
F = collections.defaultdict(list)
for kind, pid in re.findall(r"kind: (\w+), province: (\d+)", fac):
    F[int(pid)].append(kind)

print("provinces:", len(P), "assigned:", len(S), "unassigned:", sorted(set(P) - set(S)))
edges = set(); outside = collections.Counter(); water_nb = collections.Counter(); river_edges = 0
for p, c in P.items():
    for dq, dr in OFF:
        n = (c[0] + dq, c[1] + dr)
        if n in cell2p:
            q = cell2p[n]
            if p < q:
                edges.add((p, q))
                river_edges += river_between(c, n)
        elif water(n):
            water_nb[p] += 1
        else:
            outside[p] += 1
print("province-province land edges (undirected):", len(edges), " of which with a river on the shared edge:", river_edges)
cross = [(a, b) for a, b in edges if S.get(a) != S.get(b)]
print("edges crossing state/unassigned boundaries:", len(cross), "| between two different assigned states:", sum(1 for a, b in cross if a in S and b in S))
print("provinces touching water (sea/lake/unmapped cell):", sorted(p for p in P if water_nb[p]))
print("provinces adjacent to out-of-scope land:", sorted(p for p in P if outside[p]))

def comps(nodes, edge_set):
    adj = collections.defaultdict(set)
    for a, b in edge_set:
        if a in nodes and b in nodes:
            adj[a].add(b); adj[b].add(a)
    seen = set(); out = []
    for n in sorted(nodes):
        if n in seen:
            continue
        comp = set(); stack = [n]
        while stack:
            x = stack.pop()
            if x in comp:
                continue
            comp.add(x); stack.extend(adj[x] - comp)
        seen |= comp; out.append(sorted(comp))
    return out

print("components of the whole 29-province graph:", len(comps(set(P), edges)))
for sid in sorted(set(S.values())):
    ns = {p for p, s in S.items() if s == sid}
    print(f"state {sid}: {len(ns)} provinces, internal components {len(comps(ns, edges))}")
# 未割当を通れない場合(割り当て済みプロヴィンスだけ)の連結成分
assigned = set(S)
print("components when unassigned provinces are not traversable:", [len(c) for c in comps(assigned, edges)])
for sid in sorted(set(S.values())):
    others = {p for p, s in S.items() if s != sid}
    print(f"without state {sid}: components of the remaining assigned provinces:", [len(c) for c in comps(assigned - {p for p, s in S.items() if s == sid}, edges)])
print("facilities by province:", {p: k for p, k in sorted(F.items())})
print("facility provinces touching water:", sorted(p for p in F if water_nb[p]))
