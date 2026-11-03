"""P32-028: raw/*.txt の生の記録から表を作る。使い方(strategy_game/ で): python -I verification_logs/phase-32/p32-028/aggregate.py > verification_logs/phase-32/p32-028/tables.md
全体と工程の比較は、同じ呼出(同じプロセス・同じ試行・同じ操作)の値だけを使う。"""
import re, os, glob, collections, statistics as st

RAW = os.path.join("verification_logs", "phase-32", "p32-028", "raw")

def kv(line):
    return {m.group(1): m.group(2) for m in re.finditer(r"(\w+)=(\S+)", line)}

def rd(name):
    return open(os.path.join(RAW, name), encoding="utf-8", errors="replace").read().splitlines()

out = []
P = out.append

# ---------------------------------------------------------------- 1. decode 単体
P("## 1. decode 単体(新: 高速経路 / 従来: ヘッダー解析+本体解析。同じ文章・同じ試行。ms)\n")
P("| 入力 | 条件 | 試行 | 新 decode | 従来 decode | 従来の内訳(ヘッダー + 本体) | 新/従来 | バイト |")
P("|---|---|---|---|---|---|---|---|")
for name in ["decode_100x3_pinned", "decode_300x3_pinned", "decode_1000x3_pinned_a", "decode_1000x3_unpinned", "decode_1000x100_pinned"]:
    for l in rd(name + ".txt"):
        if l.startswith("PARSE "):
            d = kv(l)
            n, lg = float(d["new_decode_wall_ms"]), float(d["legacy_decode_wall_ms"])
            P(f"| {name} | {'固定なし' if 'unpinned' in name else '固定(0xFFFF)'} | {d['trial']} | {n:.3f} ({d['new_spans']}) | {lg:.1f} | {d['legacy_header_ms']} + {d['legacy_body_ms']} | 1/{lg/n:.0f} | {d['bytes']} |")

# ---------------------------------------------------------------- 2. フォールバック
P("\n## 2. 小さな不正入力(100ステートの保存を壊した入力。5試行の中央値 ms)\n")
P("| 入力 | バイト | 結果 | 新 decode(高速経路を試して従来経路へ) | 従来 decode | 差(新 − 従来) | 同じ結果 |")
P("|---|---|---|---|---|---|---|")
rows = collections.defaultdict(list)
for l in rd("fallback_100x3_pinned.txt"):
    if l.startswith("FALLBACK "):
        d = kv(l); rows[d["name"]].append(d)
for n, v in rows.items():
    a = st.median(float(x["new_decode_ms"]) for x in v); b = st.median(float(x["legacy_decode_ms"]) for x in v)
    P(f"| {n} | {v[0]['bytes']} | {v[0]['outcome']} | {a:.3f} | {b:.3f} | {a-b:+.3f} | {all(x['same_result']=='true' for x in v)} |")

# ---------------------------------------------------------------- 3. 保存・復元・確認の全体
runs_pinned = ["e2e_pinned_old_a", "e2e_pinned_new_a", "e2e_pinned_new_b", "e2e_pinned_old_b", "e2e_unpinned_old", "e2e_unpinned_new"]

def parse_series(name):
    """-> {(case, mode): {'trials': {t: TRIAL dict}, 'spans': {(t, op): {name: ms}}, 'totals': {(t, op): total_ms}, 'cont': dict}}"""
    res = collections.defaultdict(lambda: {"trials": {}, "spans": collections.defaultdict(dict), "totals": {}, "cont": None, "exit": None})
    cur = []
    for l in rd(name + ".txt"):
        if l.startswith("EXIT "):
            d = kv(l)
            key = (d["case"], d["mode"])
            for x in cur:
                dd = kv(x)
                if x.startswith("TRIAL "):
                    res[key]["trials"][int(dd["trial"])] = dd
                elif x.startswith("SPAN "):
                    res[key]["spans"][(int(dd["trial"]), dd["op"])][dd["name"]] = float(dd["ms"])
                elif x.startswith("TOTAL "):
                    res[key]["totals"][(int(dd["trial"]), dd["op"])] = float(dd["total_ms"])
                elif x.startswith("CONTINUITY "):
                    res[key]["cont"] = dd
            res[key]["exit"] = d["code"]
            cur = []
        else:
            cur.append(l)
    return res

S = {n: parse_series(n) for n in runs_pinned}

P("\n## 3. 保存・復元・確認の全体(外側から計った時間 ms。各試行。計時なし = off、計時あり = on。同じ操作・同じ試行どうしを並べる)\n")
P("| 実行 | 条件 | ケース | モード | 保存(試行1/2/3) | 確認 inspect(試行1/2/3) | 復元(試行1/2/3) | バイト |")
P("|---|---|---|---|---|---|---|---|")
for n in runs_pinned:
    for (case, mode), r in sorted(S[n].items()):
        t = r["trials"]
        if not t:
            continue
        f = lambda k: " / ".join(f"{float(t[i][k]):.1f}" for i in (1, 2, 3))
        P(f"| {n} | {'固定なし' if 'unpinned' in n else '固定'} | {case} | {mode} | {f('save_wall_ms')} | {f('inspect_wall_ms')} | {f('restore_wall_ms')} | {t[1]['bytes']} |")

P("\n## 4. 工程別(計時あり。同じ呼出の全体・工程。ms。新の高速経路の工程名 = 旧の「ヘッダー解析」+「本体解析」に対応)\n")
P("対応: 旧(P32-026・027)の `save.verify_decode_header` + `save.verify_decode_body` / `load.decode_header` + `load.decode_body` は、新では正常な保存で `save.verify_decode_fast` / `load.decode_fast`(型つきの完全解析の1工程)になる。従来経路は不正入力のフォールバックとして `header` / `body` の名前のまま残る。\n")
P("| 実行 | ケース | 操作 | 試行 | 全体 | 解析の工程 | 識別情報生成 | スナップショット+encode | 検証・候補構築・適用など | 覆っていない時間 |")
P("|---|---|---|---|---|---|---|---|---|---|")
for n in ["e2e_pinned_old_a", "e2e_pinned_new_a", "e2e_pinned_new_b", "e2e_pinned_old_b"]:
    for case in ["1000x3", "1000x100"]:
        r = S[n][(case, "on")]
        for op in ("save", "restore"):
            for t in (1, 2, 3):
                sp = r["spans"][(t, op)]; tot = r["totals"][(t, op)]
                if op == "save":
                    parse = sum(v for k, v in sp.items() if k.startswith("save.verify_decode"))
                    ident = sp.get("save.identity", 0); enc = sp.get("save.snapshot", 0) + sp.get("save.encode", 0)
                else:
                    parse = sum(v for k, v in sp.items() if k.startswith("load.decode"))
                    ident = sp.get("load.identity", 0); enc = 0
                covered = sum(sp.values())
                other = covered - parse - ident - enc
                P(f"| {n} | {case} | {op} | {t} | {tot:.1f} | {parse:.1f} | {ident:.1f} | {enc:.1f} | {other:.1f} | {tot-covered:.2f} |")

# ---------------------------------------------------------------- 5. 同一性
P("\n## 5. 保存ファイルのバイト列・復元・継続の一致(全実行・全試行)\n")
P("| 実行 | ケース | モード | 試行間のファイル一致 | 日別ダイジェスト列(先頭16桁) | 継続比較日数 | 不一致日数 | 最終状態一致 | 保存ファイル BLAKE3(先頭16桁) | 復元後=保存時 | inspect 完全一致・復元可能 |")
P("|---|---|---|---|---|---|---|---|---|---|---|")
chains = collections.defaultdict(set); hashes = collections.defaultdict(set)
for n in runs_pinned:
    for (case, mode), r in sorted(S[n].items()):
        c = r["cont"]; t = r["trials"]
        if not c:
            continue
        h = {t[i]["file_blake3"] for i in t}
        chains[case].add(c["digest_chain"]); hashes[case] |= h
        P(f"| {n} | {case} | {mode} | {c['file_hashes_equal']} | {c['digest_chain'][:16]} | {c['compared_days']} | {c['mismatch_days']} | {c['final_equal']} | {', '.join(x[:16] for x in sorted(h))} | {all(t[i]['restored_equal']=='true' for i in t)} | {all(t[i]['inspect_exact']=='true' and t[i]['inspect_restorable']=='true' for i in t)} |")
P("\n全実行(旧・新、固定・固定なし、計時あり・なし)にわたる、ケースごとの異なる値の数(1 なら完全に一致):\n")
for case in sorted(chains):
    P(f"- {case}: 保存ファイルの BLAKE3 の種類 = {len(hashes[case])}、日別ダイジェスト列の種類 = {len(chains[case])}")

# ---------------------------------------------------------------- 6. 旧/新の要約
P("\n## 6. 旧 → 新の要約(固定の4実行 off 計時なしの外側の時間。試行ごとの中央値 ms)\n")
P("| ケース | 操作 | 旧 a | 新 a | 新 b | 旧 b | 旧(中央値) | 新(中央値) | 新/旧 |")
P("|---|---|---|---|---|---|---|---|---|")
for case in ["1000x3", "1000x100"]:
    for key, lab in (("save_wall_ms", "保存"), ("inspect_wall_ms", "確認 inspect"), ("restore_wall_ms", "復元")):
        vals = {}
        for n in ["e2e_pinned_old_a", "e2e_pinned_new_a", "e2e_pinned_new_b", "e2e_pinned_old_b"]:
            t = S[n][(case, "off")]["trials"]
            vals[n] = st.median(float(t[i][key]) for i in (1, 2, 3))
        old = st.median([vals["e2e_pinned_old_a"], vals["e2e_pinned_old_b"]]); new = st.median([vals["e2e_pinned_new_a"], vals["e2e_pinned_new_b"]])
        P(f"| {case} | {lab} | {vals['e2e_pinned_old_a']:.1f} | {vals['e2e_pinned_new_a']:.1f} | {vals['e2e_pinned_new_b']:.1f} | {vals['e2e_pinned_old_b']:.1f} | {old:.1f} | {new:.1f} | 1/{old/new:.0f} |")
print("\n".join(out))
