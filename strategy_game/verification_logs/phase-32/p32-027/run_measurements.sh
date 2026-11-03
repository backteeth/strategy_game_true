#!/usr/bin/env bash
# P32-027 の測定を逐次実行する(ビルド・テスト・他の測定と並走させない)。strategy_game/ で実行: bash verification_logs/phase-32/p32-027/run_measurements.sh
# `cargo run` 経由で実行する(直接 exe を起動すると動的リンクの DLL が見つからず失敗する)。1条件=1プロセス、上限120秒(超えたら未完了として記録)。
set -u
D=verification_logs/phase-32/p32-027/raw
mkdir -p "$D"
CMP="cargo run -q -j 1 --release --bin economy_parse_compare --"
run() { out="$1"; shift; echo "== $out: $*" ; "$@" > "$D/$out" 2>&1; echo "exit $?" >> "$D/$out"; }

# 1. 同一プロセス内の繰り返し(3試行)と、新しいプロセスの初回の比較: 1000x3 を新しいプロセスで3回
for p in 1 2 3; do run "trials_1000x3_proc$p.txt" $CMP --section trials --states 1000 --countries 3 --trials 3; done
run trials_100x3.txt  $CMP --section trials --states 100 --countries 3 --trials 3
run trials_300x3.txt  $CMP --section trials --states 300 --countries 3 --trials 3
run trials_1000x100.txt $CMP --section trials --states 1000 --countries 100 --trials 3
# 2. 同じ保存の内容を変えた妥当な文章(状態だけ/国家だけ/状態の前半)
run variants_300x3.txt  $CMP --section variants --states 300 --countries 3 --trials 3
run variants_1000x3.txt $CMP --section variants --states 1000 --countries 3 --trials 3
# 3. kind・版を保ち payload の形と量だけを変えた妥当なRON
for s in flat_ints struct_list tuple_list list_of_lists deep_nest long_string named_variants; do
  run "shape_$s.txt" $CMP --section shapes --shape $s --sizes-kb 125,250,500,1000 --trials 3
done
# 4. 計時の整合性の再確認(同じ呼出の CHECK 行つき): 1000x3 と 1000x100
D2=verification_logs/phase-32/p32-027
cargo run -q -j 1 --release --bin economy_save_timing -- --series --cases 1000x3,1000x100 --modes off,on > "$D2/series_recheck.txt" 2>&1
echo "series_exit $?" >> "$D2/series_recheck.txt"
echo "all done"
