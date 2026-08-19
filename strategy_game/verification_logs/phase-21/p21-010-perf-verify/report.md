# P21-010-PERF-VERIFY: 外交危機接続後の2000州性能再検証

計測条件: warmup_days=10, measured_days=60, seed=0x00C0FFEE12345678, release build
(`profile_1000_states` / `profile_crisis_scaling`、いずれも本番と同一の`DailySimulationSet`順序)

## 0. 事前に見つかった不整合(前提の再確認)

セッション開始時点で作業ツリーには以下が**未追跡ファイルとして既に存在していた**:

- `verification_logs/p20-008/baseline_verify_run1..5`
- `verification_logs/phase-21/p21-010-perf-verify/baseline_verify_run1..5`(本レポートが元々あった場所)

両者をdiffしたところ**完全に同一内容**だった。後者は前者を単純コピーしたものであり、
P21-010専用のA/B比較でも段階測定でもない。さらに25ファイルすべてのmtimeが0.05秒以内に
収まっており、5回の独立した`cargo run --release`(2000州で60日×8シナリオ計測)がこの時間で
終わることは物理的にありえない。よってこのディレクトリの中身は本タスクの根拠として使用せず、
以下は全て本セッション中に新規取得した実測値のみで構成している。

タスク説明にある「P21-010後に観測された2000州normal mean 0.56〜0.72ms」という記述は、
`verification_logs/p20-008/p21-010/`・`p21-010-rerun/`・`p21-010-final/`(いずれもコミット済み、
d83a05c)のmean=0.7224/0.5958/0.5650msに対応しており、出所は確認できた。

## 1. 測定: 現行実装(作業ツリー、未変更)を2000州で5回

`strategy_game/verification_logs/phase-21/p21-010-perf-verify/01_realtree_5x/run1..5/`

| run | normal mean(ms) | normal median | normal ticks/s | high_load mean(ms) | high_load median |
|---|---|---|---|---|---|
| 1 | 0.2423 | 0.1902 | 4126.6 | 1.0555 | 1.0009 |
| 2 | 0.2365 | 0.1846 | 4228.3 | 1.1007 | 1.0404 |
| 3 | 0.2438 | 0.1889 | 4101.9 | 1.0575 | 0.9960 |
| 4 | 0.2244 | 0.1807 | 4456.4 | 1.0865 | 1.0357 |
| 5 | 0.2387 | 0.1864 | 4190.0 | 1.0795 | 1.0261 |
| **平均** | **0.2371** | **0.1862** | **4220.6** | **1.0759** | **1.0198** |
| 中央値 | 0.2387 | 0.1864 | 4190.0 | 1.0795 | 1.0261 |
| 最小 | 0.2244 | 0.1807 | 4101.9 | 1.0555 | 0.9960 |
| 最大 | 0.2438 | 0.1902 | 4456.4 | 1.1007 | 1.0404 |

現在の環境でクリーンに測ると、5回とも0.22〜0.24ms(normal)に収まり、タスク記述にある
0.56〜0.72msより明確に低い。CPU負荷は事前確認で4.5〜7.7%(Vivaldi・EA系プロセス・
VSCode/rust-analyzer等がバックグラウンドで稼働、詳細は`environment.txt`)。
これらのプロセスは停止せず稼働させたまま計測した(ユーザー環境への無断介入を避けるため)。

## 2. 同一環境A/B(一時worktree、作業ツリー非変更)

### 手法

- `git worktree add <temp> HEAD --detach` で作業ツリー外に一時コピーを作成
- A = 現行`update.rs`(Crisis日次ループそのまま)、B = 同ループのみを`let _ = &crisis_registry;`
  に置換した比較版(diffは1関数15行→3行のみ、他は無変更)
- **既知の落とし穴**: 当初A用worktreeとB用worktreeを別ディレクトリに用意し`--target-dir`を
  共有する方式を試みたが、Cargoのローカルパッケージfingerprintが絶対パスに依存しない
  (少なくともこの環境のCargoでは)ため、Bを指しているはずのビルドがAの成果物を
  無変更のまま再利用してしまうバグを実測で確認した(exe SHA256が完全一致、ビルド時間0.88秒)。
  そのため**同一パス内でソースを書き換えて再ビルドする方式**に切り替えて解決した
  (this file: `report.md` 末尾の補足参照)。
- 依存クレート(bevy等)は初回ビルド時に外部target-dirへキャッシュし、以降のA/B切り替えは
  ローカルcrateのみの再コンパイル(約35秒)で済ませた。ビルド成果物2種(A.exe/B.exe)を
  取り出した後は同一exeを繰り返し実行するだけなので、切り替えごとの再ビルドは発生していない。
- 一時worktree・共有target-dirは比較完了後に完全削除済み(`git worktree remove --force`
  + `git worktree prune`)。作業ツリーの追跡ファイルは本タスク開始から一切変更していない
  (`git status`/`git diff --check`で確認)。

### 結果(2000州、n=15/変量、CSV: `02_ab_same_session/ab_all_results.csv`)

最初のA→B→A→B→A(n=5)だけでは外れ値(B側に2回、0.28ms・0.30ms)の影響で
normal meanの差が+11.7%まで見えたため、「測定回数を減らして通す」の逆(閾値ではなく
サンプル数を増やす)を行い、n=15/変量まで拡張して再判定した。

| scenario | 統計 | A (現行) | B (Crisis無効化) | 差分 (B-A)/A |
|---|---|---|---|---|
| normal | mean±sd | 0.23358 ± 0.00576 ms | 0.24035 ± 0.02305 ms | **+2.90%** |
| normal | median | 0.2353 ms | 0.2305 ms | **-2.04%** |
| high_load | mean±sd | 1.07174 ± 0.02881 ms | 1.08344 ± 0.03791 ms | **+1.09%** |
| high_load | median | 1.0609 ms | 1.0690 ms | **+0.76%** |

meanとmedianで差分の符号が反転している(normal: +2.9% vs -2.0%)こと自体が、
系統的な効果ではなく測定ノイズであることを示す典型的なパターン。BのsdがAの約4倍
(0.023 vs 0.0058ms)なのも、B側にたまたま2回(0.28ms・0.30ms)ノイズスパイクが載った
結果であり、コード差分(空のHashMap反復1行の有無)が原因とは物理的に整合しない
(空反復が数十マイクロ秒のコストを生むことはあり得ない)。

**判定: 全て5%以内 → 外部負荷/測定ノイズ。P21-010のCrisis日次接続コード自体による
実回帰ではない。**

## 3. Crisis件数スケーリング確認(2000州, normal, 3回反復)

一時worktree内に検証専用バイナリ`profile_crisis_scaling`を追加し(本流には反映せず、
`03_crisis_scaling/profile_crisis_scaling_source.rs`に証跡として保存)、World構築直後に
`CrisisRegistry`へN件の`DiplomaticCrisis`を直接挿入して計測した。

| crisis_count | mix | overall mean(ms, 3回平均) | Diplomacy-set mean(ms, 3回平均) |
|---|---|---|---|
| 0 | - | 0.23075 | 0.03044 |
| 1 | active | 0.22692 | 0.03075 |
| 100 | active | 0.23040 | 0.03008 |
| 1000 | active | 0.24068 | 0.03463 |
| 1000 | terminal(ResolvedPeacefully) | 0.23122 | 0.03364 |

- **0件時**: overall mean 0.231msは§1・§2のnormal実測(0.224〜0.244ms)の範囲内 →
  State/Country数に比例する新規走査は無い(コード上も`handle_daily_diplomacy`内で
  `DailySimulationSet::Diplomacy`に登録されているのはこの1関数のみで、Crisisループは
  `crisis_registry.crises`のみを反復し、既存のState/Country全件走査には触れない)。
- **線形性**: Diplomacy-set meanは0→1→100件でほぼ横ばい(0.0304〜0.0308ms、ノイズ内)、
  1000件で+13.8%(0.0346ms)。1000件あたりの増分は0.0042ms/1000crises ≈ **crisis 1件あたり
  約4.2ナノ秒**で、想定通りの線形かつ極小コスト。day_tick全体(約0.23ms)に対しては
  1.8%に過ぎない。
- **terminal Crisisの扱い**: 1000件terminal(0.0336ms)は1000件active(0.0346ms)とほぼ同等
  (差3%未満、ノイズ内)→ terminal状態のCrisisを不要に重く処理してはいない
  (`matches!`分岐で早期にスキップされるのみで、他の追加処理は発生しない)。
- **HashMap反復順**: 5ケース×3回=15回の実行全てで、`days_in_phase`合計の実測値が
  期待値(non-terminal: crisis_count×70日、terminal: 0)と完全一致することを
  `assert_eq!`で検証済み(全15回パス、`03_crisis_scaling/crisis_scaling_run*/summary.txt`参照)。
  Rustの`HashMap`はプロセスごとに反復順がランダム化されるため、この一致は結果が
  反復順に依存しないことの直接的な証拠になる。

## 4. 総合判定

| 項目 | 結果 |
|---|---|
| 同一環境A/B差 | normal +2.9%(mean)/-2.0%(median)、high_load +1.1%(mean)/+0.8%(median) → **5%以内** |
| 0件時の新規走査 | 無し(コード確認 + 実測とも一致) |
| Crisis件数スケーリング | 線形、約4.2ns/crisis、極小 |
| terminal Crisis処理 | 追加コスト無し(activeと同等) |
| HashMap順依存 | 無し(15/15回で決定論的一致を確認) |

→ **タスク記述の「0.56〜0.72ms」は外部負荷/測定ノイズによるものと判定する。
P21-010のCrisis日次接続コード自体には実回帰がなく、修正は不要。**
修正が必要な場合の各チェック項目(即時return・全走査排除・重複処理排除・
allocation/clone/sort排除)は、そもそも現行コードが元から満たしていることを
本測定で確認した(コード変更なし)。

## 5. 最終検証(作業ツリー、無変更のまま)

`04_final_verification/`

| コマンド | 結果 |
|---|---|
| `cargo test --lib` | 590 passed; 0 failed |
| `cargo test --tests`(既定並列) | 全21バイナリ pass; 0 failed(内訳は`cargo_test_tests.log`) |
| `cargo clippy --all-targets --all-features -- -D warnings` | warning 0件 |
| `cargo build --release` | 成功 |
| `git diff --check` | 空白関連の問題なし(exit 0、追跡ファイルの差分自体が無い) |
| `cargo fmt --check`(開始時/終了時) | **20ファイル・68 diff hunksの既存整形差分あり(本タスク
  開始前から存在、本セッションでは1バイトも変更していないため開始時=終了時で同一)**。
  この「68-hunk/20-file」という数字は、直前のP21-008完了時点(2026-08-16、
  `project_phase21_status.md`記載)の既知ベースラインと完全一致しており、
  本タスクによる新規drift・regressionが無いことを裏付ける。diplomacy/crisis関連ファイルは
  対象に含まれない。ワークスペース全体への`cargo fmt`適用は無関係な大規模diffを生むため、
  本タスクの範囲外として意図的に未実施。 |

テスト実行後の`git status`で、追跡ファイルへの意図しない変更(スクリーンショット等)が
無いことも確認済み。

## 補足: git worktree + 共有target-dirの落とし穴

同一リポジトリから作成した2つの`git worktree`が、`--target-dir`で同一の外部ディレクトリを
指す場合、少なくとも本環境のCargo(rustc 1.97.1)では**ローカルパスパッケージの
fingerprintが呼び出し元の絶対パスを区別しない**ケースがあることを実測で確認した
(`.fingerprint/strategy_game-<hash>/`が2つのworktreeで同一ハッシュに収束し、
後から変更した側のビルドが前のビルド成果物をそのまま「fresh」と誤判定して再利用する)。
同一worktree内でソースを書き換えて同じ`--target-dir`に対して再ビルドする分には
正しく差分を検出して再コンパイルされるため、複数worktree構成でビルドキャッシュを
共有する場合は要注意(将来このリポジトリで同様の手法を使う際のために記録しておく)。
