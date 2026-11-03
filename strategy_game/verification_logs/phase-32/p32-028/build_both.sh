#!/usr/bin/env bash
# P32-028: 新旧の測定用 release バイナリを作る(逐次。ビルド・テスト・測定と並走させない)。strategy_game/ で実行。
#  new: 現在の作業ツリー(高速経路あり)
#  old: コミット 551eaa2(P32-027 の終了時点)を別の作業ツリーに取り出したもの。測定コマンド側の `save_timing_bench.rs` だけ、
#       現在のもの(inspect の時間の追加のみ。製品コードは含まない)に差し替える。製品の decode は 551eaa2 のまま。
set -eu
OUT="${P28_EXES:-$HOME/p28_exes}"
mkdir -p "$OUT"
ROOT="$(pwd)"
TARGET="$ROOT/target"
echo "== build new"
cargo build -j 1 --release --bin economy_save_timing --bin economy_parse_compare
cp target/release/economy_save_timing.exe "$OUT/new_economy_save_timing.exe"
cp target/release/economy_parse_compare.exe "$OUT/new_economy_parse_compare.exe"
echo "== old worktree (551eaa2)"
BASE="$OUT/base551"
if [ ! -d "$BASE" ]; then git -C "$ROOT/.." worktree add --detach "$BASE" 551eaa2; fi
cp src/economy/save_timing_bench.rs "$BASE/strategy_game/src/economy/save_timing_bench.rs"
echo "== build old"
( cd "$BASE/strategy_game" && CARGO_TARGET_DIR="$TARGET" cargo build -j 1 --release --bin economy_save_timing )
cp target/release/economy_save_timing.exe "$OUT/old_economy_save_timing.exe"
echo "== done"; ls -la "$OUT"
