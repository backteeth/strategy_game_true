"""P32-026: series_run*.txt の生の記録(SPAN/TOTAL/TRIAL 行)から表を作る。使い方: python -I aggregate.py series_run1.txt > tables.md
計時あり(mode=on)の2回のパスを区別して出し、各試行の値を省略しない(平均だけにしない)。"""
import re, sys, collections

def kv(line):
    d = {}
    for m in re.finditer(r"(\w+)=(\S+)", line):
        d[m.group(1)] = m.group(2)
    return d

text = open(sys.argv[1], encoding="utf-8").read().splitlines()
# パス番号: EXIT 行の pass= を、その直前の出力に割り当てる
runs = []  # list of dict(case, mode, pass, lines)
cur = []
for ln in text:
    if ln.startswith("EXIT "):
        d = kv(ln)
        runs.append({"case": d["case"], "mode": d["mode"], "pass": int(d["pass"]), "lines": cur})
        cur = []
    else:
        cur.append(ln)

cases = ["100x3", "300x3", "1000x3", "1000x100"]
out = []
P = out.append

P("## 1. 計時なし/あり(外側から計った保存・復元の時間 ms)\n")
P("| ケース | パス | モード | 保存 試行1 / 2 / 3 | 復元 試行1 / 2 / 3 | バイト数 |")
P("|---|---|---|---|---|---|")
for r in runs:
    tr = [kv(l) for l in r["lines"] if l.startswith("TRIAL ")]
    if not tr:
        continue
    P(f"| {r['case']} | {r['pass']} | {r['mode']} | " + " / ".join(f"{float(t['save_wall_ms']):.1f}" for t in tr) +
      " | " + " / ".join(f"{float(t['restore_wall_ms']):.1f}" for t in tr) + f" | {tr[0]['bytes']} |")

def spans(run):
    res = collections.OrderedDict()
    for l in run["lines"]:
        if l.startswith("SPAN "):
            d = kv(l)
            res.setdefault((d["op"], d["name"]), {})[int(d["trial"])] = (float(d["ms"]), int(d["calls"]))
    tot = {}
    for l in run["lines"]:
        if l.startswith("TOTAL "):
            d = kv(l)
            tot[(d["op"], int(d["trial"]))] = (float(d["total_ms"]), float(d["covered_ms"]), float(d["uncovered_ms"]), d["outcome"])
    return res, tot

for pas in (2, 3):
    P(f"\n## 工程別(計時ありのパス{pas}、各試行の絶対時間 ms)\n")
    for c in cases:
        run = next((r for r in runs if r["case"] == c and r["mode"] == "on" and r["pass"] == pas), None)
        if not run:
            continue
        res, tot = spans(run)
        for op in ("save", "restore"):
            P(f"\n### {c} {op}(パス{pas})\n")
            P("| 工程 | 呼出 | 試行1 | 試行2 | 試行3 |")
            P("|---|---|---|---|---|")
            for (o, name), v in res.items():
                if o != op:
                    continue
                P(f"| {name} | {v[1][1]} | " + " | ".join(f"{v[t][0]:.2f}" for t in (1, 2, 3)) + " |")
            P("| **全体(関数の入口〜出口)** | | " + " | ".join(f"{tot[(op, t)][0]:.2f}" for t in (1, 2, 3)) + " |")
            P("| 工程で覆った合計 | | " + " | ".join(f"{tot[(op, t)][1]:.2f}" for t in (1, 2, 3)) + " |")
            P("| 覆っていない時間 | | " + " | ".join(f"{tot[(op, t)][2]:.2f}" for t in (1, 2, 3)) + " |")
            P("| 結果 | | " + " | ".join(tot[(op, t)][3] for t in (1, 2, 3)) + " |")

# 継続一致・同一性
P("\n## 計時なし/あり・パス間の同一性(継続・保存ファイル)\n")
P("| ケース | パス | モード | 継続比較日数 | 不一致日数 | 最終状態一致 | 試行間のファイル一致 | 日別ダイジェスト列(BLAKE3、先頭16桁) |")
P("|---|---|---|---|---|---|---|---|")
for r in runs:
    for l in r["lines"]:
        if l.startswith("CONTINUITY "):
            d = kv(l)
            P(f"| {r['case']} | {r['pass']} | {r['mode']} | {d['compared_days']} | {d['mismatch_days']} | {d['final_equal']} | {d['file_hashes_equal']} | {d['digest_chain'][:16]} |")
print("\n".join(out))

# ---- 追加: 計時の追加負荷(外側から計った時間の、計時あり − 計時なし。同じ試行番号どうし)と、header 解析の割合
import statistics as st
def walls(mode, case, key):
    v = {1: [], 2: [], 3: []}
    for r in runs:
        if r["case"] == case and r["mode"] == mode:
            for l in r["lines"]:
                if l.startswith("TRIAL "):
                    d = kv(l); v[int(d["trial"])].append(float(d[key]))
    return v
print("\n## 計時の追加負荷(外側から計った時間。試行番号ごとの、計時あり平均 − 計時なし平均。ms)\n")
print("| ケース | 操作 | 試行1 | 試行2 | 試行3 | 計時なし平均(試行1/2/3) |")
print("|---|---|---|---|---|---|")
for c in cases:
    for key, lab in (("save_wall_ms", "保存"), ("restore_wall_ms", "復元")):
        on, off = walls("on", c, key), walls("off", c, key)
        diffs = [st.mean(on[t]) - st.mean(off[t]) for t in (1, 2, 3)]
        print(f"| {c} | {lab} | " + " | ".join(f"{d:+.1f}" for d in diffs) + " | " + " / ".join(f"{st.mean(off[t]):.1f}" for t in (1, 2, 3)) + " |")
print("\n## ヘッダー解析・本体解析・その他(計時ありの全パス・試行の平均 ms、全体に対する割合)\n")
print("| ケース | 操作 | 試行 | ヘッダー解析 | 本体解析 | 識別情報生成 | それ以外の工程の合計 | 全体 | ヘッダー解析の割合 |")
print("|---|---|---|---|---|---|---|---|---|")
for c in cases:
    for op, hn, bn, idn in (("save", "save.verify_decode_header", "save.verify_decode_body", "save.identity"), ("restore", "load.decode_header", "load.decode_body", "load.identity")):
        for t in (1, 2, 3):
            H, B, I, T = [], [], [], []
            for r in runs:
                if r["case"] == c and r["mode"] == "on":
                    res, tot = spans(r)
                    H.append(res[(op, hn)][t][0]); B.append(res[(op, bn)][t][0]); I.append(res[(op, idn)][t][0]); T.append(tot[(op, t)][0])
            h, b, i, tt = st.mean(H), st.mean(B), st.mean(I), st.mean(T)
            print(f"| {c} | {op} | {t} | {h:.1f} | {b:.1f} | {i:.1f} | {tt-h-b-i:.1f} | {tt:.1f} | {100*h/tt:.1f}% |")
