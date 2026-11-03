"""P32-027: raw/*.txt の生の記録から表を作る。使い方(strategy_game/ で): python -I verification_logs/phase-32/p32-027/aggregate.py > verification_logs/phase-32/p32-027/tables.md"""
import re, glob, os, collections, statistics as st

D = os.path.join("verification_logs", "phase-32", "p32-027")
RAW = os.path.join(D, "raw")

def kv(line):
    return {m.group(1): m.group(2) for m in re.finditer(r"(\w+)=(\S+)", line)}

def lines(name):
    return open(os.path.join(RAW, name), encoding="utf-8", errors="replace").read().splitlines()

out = []
P = out.append

# 1. 製品の保存文章の解析(同一プロセス内の繰り返し・新しいプロセスの初回)
P("## 1. 製品の保存文章の解析時間(ms。解析のみ。ファイル読込・結果の破棄・出力を含まない)\n")
P("| 入力 | プロセス | 試行 | 製品ヘッダー解析 | 製品本体解析 | 複製ヘッダー型 | 全文を読み飛ばし(IgnoredAny) | 対照: 文章の線形走査 | 文章バイト |")
P("|---|---|---|---|---|---|---|---|---|")
for name in sorted(glob.glob(os.path.join(RAW, "trials_*.txt")) + glob.glob(os.path.join(RAW, "affinity_*[0-9A-F].txt"))):
    base = os.path.basename(name)
    for l in lines(base):
        if l.startswith("PARSE "):
            d = kv(l)
            P(f"| {base[:-4]} | | {d['trial']} | {d['product_header_ms']} | {d['product_body_ms']} | {d['replica_header_ms']} | {d['ignored_any_full_ms']} | {d['control_text_scan_ms']} | {d['bytes']} |")

# 2. 入力の構造
P("\n## 2. 入力の規模と構造\n")
P("| 入力 | 状態数 | 建物エントリ | 国家数 | 進行中の注文 | バイト | 行 | `(` | `[` | 最大の入れ子 | フィールド(`:`) |")
P("|---|---|---|---|---|---|---|---|---|---|---|")
for name in ["trials_100x3.txt", "trials_300x3.txt", "trials_1000x3_proc1.txt", "trials_1000x100.txt"]:
    case = inp = None
    for l in lines(name):
        if l.startswith("CASE "): case = kv(l)
        if l.startswith("INPUT "): inp = kv(l)
    P(f"| {name[7:-4]} | {case['state_entries']} | {case['building_entries']} | {case['country_entries']} | {case['queued_orders']} | {inp['bytes']} | {inp['lines']} | {inp['parens']} | {inp['brackets']} | {inp['max_depth']} | {inp['fields']} |")

# 3. メモリ(試行の前後。観測値であってピークではない)
P("\n## 3. プロセスのメモリ(試行の前後。計時区間の外。バイト)\n")
P("working_set = 現在ワーキングセット / private = プライベートのコミット済み / peak_ws = プロセス開始からのワーキングセット最大値(試行ごとのピークではない)\n")
P("| 入力 | 時点 | 試行 | working_set | private | peak_ws(開始から) |")
P("|---|---|---|---|---|---|")
for name in ["trials_1000x3_proc1.txt", "trials_1000x3_proc2.txt", "trials_1000x3_proc3.txt", "trials_1000x100.txt"]:
    for l in lines(name):
        if l.startswith("MEM "):
            d = kv(l)
            P(f"| {name[7:-4]} | {d['point']} | {d['trial']} | {d['working_set_bytes']} | {d['private_bytes']} | {d['peak_working_set_since_process_start_bytes']} |")

# 4. 変えた保存内容
P("\n## 4. 同じ保存の内容を変えた妥当な文章(製品の encode で書き出し。ms)\n")
P("| 入力 | 文章 | バイト | `(` | `[` | 試行1 ヘッダー | 試行2 | 試行3 | 本体(試行1) |")
P("|---|---|---|---|---|---|---|---|---|")
for name in ["variants_300x3.txt", "variants_1000x3.txt"]:
    info = {}; rows = collections.defaultdict(dict)
    for l in lines(name):
        d = kv(l)
        if l.startswith("INPUT "): info[d["label"]] = d
        if l.startswith("VARIANT "): rows[d["name"]][int(d["trial"])] = d
    for v, t in rows.items():
        i = info[v]
        P(f"| {name[9:-4]} | {v} | {i['bytes']} | {i['parens']} | {i['brackets']} | {t[1]['product_header_ms']} | {t[2]['product_header_ms']} | {t[3]['product_header_ms']} | {t[1]['product_body_ms']} |")

# 5. 形と量を変えた fixture
P("\n## 5. kind・版を保ち payload の形と量だけを変えた妥当なRON(複製ヘッダー型の解析時間 ms。試行の中央値。未完了は—)\n")
P("| 形 | 125KB | 250KB | 500KB | 1000KB | 250/125 | 500/250 | 1000/500 | 要素の特徴(1000KB) |")
P("|---|---|---|---|---|---|---|---|---|")
for name in sorted(glob.glob(os.path.join(RAW, "shape_*.txt"))):
    rows = collections.defaultdict(list); feat = ""
    inc = [l for l in lines(os.path.basename(name)) if l.startswith("INCOMPLETE")]
    for l in lines(os.path.basename(name)):
        if l.startswith("SHAPE "):
            d = kv(l); rows[int(d["target_kb"])].append(float(d["header_ms"]))
            if d["target_kb"] == "1000": feat = f"`(`={d['parens']} `[`={d['brackets']} 深さ={d['max_depth']}"
    med = {k: st.median(v) for k, v in rows.items()}
    cell = lambda k: f"{med[k]:.1f}" if k in med else "—"
    ratio = lambda a, b: f"×{med[b]/med[a]:.2f}" if a in med and b in med and med[a] > 0 else "—"
    shape = os.path.basename(name)[6:-4]
    note = " (1000KBは試行3回の途中で上限に達し、取れた分の中央値)" if inc else ""
    P(f"| {shape}{note} | {cell(125)} | {cell(250)} | {cell(500)} | {cell(1000)} | {ratio(125,250)} | {ratio(250,500)} | {ratio(500,1000)} | {feat} |")
print("\n".join(out))
