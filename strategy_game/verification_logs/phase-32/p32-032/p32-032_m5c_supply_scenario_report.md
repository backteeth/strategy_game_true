# P32-032 M5-c: 検証用シナリオからの到達判定

**完了したのは「検証用RONから主体ごとの補給源と通行可能範囲を読み込み、既存の到達判定(P32-030)とステート要約(P32-031)へ接続し、検証用CLIで確認できる」ことだけである。** 経済・AI・軍・保存への接続、輸送能力・補給量は未実装(後続)。到達可能は「十分な補給が届く」ではない。経済シナリオ・mapeco・静的識別・`ECONOMY_RULES_REVISION` は変更していない。**D10 は確定していない**(§9)。M4 は閉鎖済み、M5 と Phase 32 は継続。

## 1. 基準状態
- 開始 HEAD: `aaa7884`(main、origin/main と一致、未追跡は `hex_atlas/` のみ。ブランチ main)。AGENTS.md / CLAUDE.md は無し。P32-029〜031 の記録とコードを確認した。
- 終了 HEAD: コミット後に `git log` で確認(報告書自体にはハッシュを書けない)。
- 保護: `hex_atlas/`、地図、`saves/`(ユーザーセーブ。今回は一切使っていない)、過去の記録。検証用の保存は一時ディレクトリのみ。

## 2. 変更範囲
| ファイル | 内容 |
|---|---|
| `src/world/supply_scenario.rs`(新規) | RON の読込・検証・展開(`parse_supply_scenario` / `resolve_supply_scenario` / `load_supply_scenario`)と、既存の `reach` + `summarize_model` を呼ぶ `evaluate_supply_scenario`。探索・要約の処理は持たない |
| `src/world/mod.rs` | モジュールの公開(追加のみ) |
| `src/bin/world_reach_check.rs` | `--scenario <path>` / `--scenario-agent <label>` を追加。要約の表の表示を関数に切り出して共用(既存の出力は不変) |
| `assets/data/world/preview/supply_connectivity_scenario.ron`(新規) | 検証用シナリオ(主体 3: A / B / 複数補給源) |
| `assets/data/world/preview/README.md` | 新ファイルの説明を1行追加 |
| `tests/p32_032_supply_scenario_test.rs`(新規) | 12件 |
| `verification_logs/phase-32/p32-029/…design.md` | §12 追記(採用した形式。恒久的な決定ではない) |

## 3. 入力形式(`format_version: 1`)
```
( format_version: 1, description: "...",
  agents: [ ( label: 1, sources: [18309], passable_states: [20, 22], passable_provinces: [17903] ), ... ] )
```
- `label`(必須): 識別用ラベル。結果は**ラベル昇順**(ファイルの記述順に依存しない)。
- `sources`(必須・1件以上): 補給源の `ProvinceId`(複数可)。**すべて通行可能集合に含まれること**。
- `passable_provinces` / `passable_states`(少なくとも一方が必須。空リストの明示は可): `passable_states` は `WorldModel` の所属(正本)から `ExplicitPassable::from_states` で展開する名前つきの変換。経済シナリオの担当ステート・配列順から推定しない。
- 未知のフィールドは拒否(`deny_unknown_fields`)。ID は型ごとに解決し、`ProvinceId`/`MapStateId` を数値一致で変換しない。

## 4. 検証規則(すべての問題を集めて報告。種類は `code`)
| 失敗の種類 | code / 区分 | CLI の終了コード |
|---|---|---|
| ファイルが読めない / RON として不正・未知のフィールド・型違い / 非対応の版 | `Io` / `Parse` / `UnsupportedVersion` | 66 |
| 必須項目の欠落(label・sources・通行範囲・agents)、主体なし、ラベル重複 | `missing-field` / `no-agents` / `duplicate-label` | 65 |
| 補給源: 空、重複、未知の ID(退役済みは `retired` を併記) | `no-sources` / `duplicate-source` / `unknown-province` | 65 |
| `passable_provinces`: 重複、未知の ID | `duplicate-passable-province` / `unknown-province` | 65 |
| `passable_states`: 重複、未知のステート、所属が空のステート | `duplicate-passable-state` / `unknown-state` / `empty-state` | 65 |
| **併用時**: 展開後の重複(`passable_provinces` の ID が、同じ主体の `passable_states` に挙げたステートの所属でもある) | `overlap-province-in-state` | 65 |
| 展開後の通行範囲が空 / 補給源が通行範囲に無い | `empty-passable` / `source-not-passable` | 65 |
| 個別指定との併用、`--scenario-agent` だけの指定 | 引数の誤り | 64 |
| 存在しない `--scenario-agent` ラベル | 入力不正 | 65 |
| 到達不能・不明(正常な判定) | — | **0** |

- 併用の規則: 両方を併用してよく、通行範囲は**和集合**。展開後の重複はエラー(意図の取り違えを検出するため)。別のステートの所属を明示するのは重複ではない。ステート未所属の既知の `ProvinceId` は、`passable_provinces` に明示すれば通行でき、補給源にもできる。
- 水域・範囲外の陸セルには `ProvinceId` が無く、名指しできない(その番号は未知の ID として拒否)。範囲外の陸の扱いは既存の判定規則のまま(橋渡しにしない。「不明」の根拠にだけ使う)。
- 問題のある主体は、通行範囲の空・補給源の検査を重ねず(連鎖した誤報を避ける)、他の主体の検査は続ける。

## 5. CLI の実際の起動と出力
`cd strategy_game && cargo run --release --bin world_reach_check -- ...`。ログ(1行目に実コマンド、末尾に `exit=`)はこのフォルダの `cli_*.txt`。
- `--scenario <path>` は、`--source` `--passable*` `--blocked*` `--target*` とは**併用できない**(終了コード 64。`cli_16`)。シナリオ入力ではステート要約を常に出す(`--no-list` は有効)。既存の引数・出力は変えていない。
- `cli_01_scenario_all.txt`(3主体)/ `cli_02_…agent1_A.txt` / `cli_03_…agent2_B.txt`(`--scenario-agent`)。個別指定の対照: `cli_04_individual_A.txt`(`--source 18309 --passable-states 20,22 --state-summary`)、`cli_05_individual_B.txt`(同 + `--passable-provinces 17903,18110,18501`)。

| 条件(起点 18309) | RON の主体 | 到達 | ステート20 | ステート21 | ステート22 | 未所属3 |
|---|---|---|---|---|---|---|
| A: ステート20・22の13件 | label 1 | 7、不明6 | 全域(7/7) | 到達不能(通行不可13) | 不明(6) | 到達不能(通行不可) |
| B: A + 未所属17903・18110・18501 | label 2 | 16、不明0 | 全域(7/7) | 到達不能(通行不可13) | 全域(6/6) | 到達可能3 |
| 複数補給源(18309 と 17266、20・22) | label 3 | 13、不明0 | 全域 | 到達不能 | 全域(2つ目の補給源から着く) | 到達不能(通行不可) |

- `compare_scenario_vs_individual.txt`: A・B とも、シナリオ出力の「到達件数・不明件数・到達/不明の一覧・ステート表・未所属行」が、個別指定の出力と**完全に一致**(A は8行、B は7行を比較。差分なし)。自動テストでも、既存の `ExplicitPassable` + `reach` + `summarize_by_state` の結果と `Reach`・要約が等しいことを確認。
- 異常系の実出力: `cli_10`(未知の ID・未知のステート・重複・展開後の重複、4件)、`cli_11`(欠落・空・補給源が範囲外・ラベル重複・空の通行範囲など9件)、`cli_12`(版違い)、`cli_13`(未知のフィールド)、`cli_14`(RON として不正)、`cli_15`(ファイルなし)、`cli_16`〜`cli_18`(引数の誤り・未知のラベル)。入力は `bad_inputs/`(記録用。アセットではない)。

## 6. 自動検証(新規12件。探索と要約は既存の処理を使う)
形式の失敗の区別 / 全種類の入力不正をそれぞれの code で報告(複数問題の集約を含む)/ 退役済み ID の区別 / ステートからの展開が `WorldModel` の所属と `ExplicitPassable::from_states` に一致 / 併用の和集合と展開後の重複 / 同梱RONの A・B・複数補給源が既存の個別指定の結果と一致(独立した総当たりでも確認)/ 補給源の1つでも範囲外なら解決で拒否 / 主体・補給源・ID の順序を入れ替えても同一の結果 / 通行条件の変更で要約が変わる / プロヴィンス指定だけの範囲と未所属の補給源 / 範囲外の陸に ID を発行せず「対象外」/ シナリオ層が経済・保存・軍・時刻・乱数・探索コードを持たない(ソース検査)。時間は合否条件にしていない。

## 7. 既存の m4d/m4e の回帰確認
既存の手順(P32-022 と同じ `world_economy_check` の保存・復元・連続実行。保存先は一時ディレクトリで、確認後に削除。`saves/` は不使用)。コマンド: `regression_m4d_m4e_commands.txt`、出力: `reg_*_m4d.txt` / `reg_*_m4e.txt`、要約: `regression_m4d_m4e_summary.txt`。
- m4d・m4e とも、保存(100日)・復元(60日)・連続実行(160日)が exit 0。**復元後の60行と連続実行の60行が完全一致**(日次進行・保存・復元後のダイジェスト)。静的入力の判定は両方とも「一致 21 項目 / 警告 0 / 拒否 0」。
- m4d の先頭・末尾のダイジェストは `2f1b1910201ce473` / `2fce25ceb982483b`で、`tests/p32_019_session_api_test.rs` が固定している値と一致。m4e は `59759ddc267451ff` / `3688f1125fce9d23`(m4e の固定値は既存テストに無い)。
- **注記**: P32-022 の記録(m4d `0efefcfcfc398595`/`7f6f2d0ddbd2cc19`、m4e `3f98d4cfb71c2bf6`/`60b0759c8c6efa26`)とは値が異なる。現在のテストが固定する値は新しい値で、原因の調査は今回していない(P32-022 以降の変更に由来するとみられる)。**今回の変更は `src/economy`・`assets/data` を1行も変えていない(`git diff` 0行)**ので、今回の差ではない。
- 全体テストに含まれる既存の回帰(p32_013 / 015 / 016 / 017 / 019 / 020〜022 など)もすべて成功。経済シナリオ・mapeco の形式・静的識別・`ECONOMY_RULES_REVISION` は不変。

## 8. 必須チェック
- 実行前: 空きメモリ 約15〜17GB、残存の cargo/rustc なし。逐次実行(並走なし)。
- `cargo clippy -j 1 --all-targets --all-features -- -D warnings`: **警告 0**(初回で成功)。`clippy.log`。
- `cargo test -j 1 -- --test-threads=1`: **3443 成功 / 0 失敗 / 6 無視**(基準 3431 + 新規 12)。`full_test_j1_threads1.log`(約15分)。
- **GPUテスト**(`ui_headless_render_produces_real_pixels`): 今回は**再発せず成功**。前回の失敗の原因は未特定のまま(再発した場合は結果と条件を記録する)。
- **追跡済みPNG**: 実行前の差分は `world_reach_check.rs` と `world/mod.rs` のみで、PNGの差分は無かった(`pretest_tracked_diffs.txt`)。全体テストはこれまでと同様に追跡済みのスクリーンショット(`p20-007`・`p20-009`・`p21-save-002e`の9枚)を再生成して変更したので、**今回のテストが生成した変更だけを `git checkout` で元に戻し、コミット対象から外した**。
- 事前に、新規テストだけを先に実行して12件成功を確認した。

## 9. M5 の完了条件に対して確認できた範囲と、D10 の確定に残る判断
元計画のM5の完了条件は「D10 を確定できる性能データ」(P32-029 §1)。
| 段階 | 確認できたこと |
|---|---|
| M5-a(P32-030) | プロヴィンス隣接グラフ、明示的な通行条件、純粋な到達判定(到達可能/不能/不明/対象外) |
| M5-b(P32-031) | ステート単位の要約(再探索なし)、旧小規模ケースとの一致、約3.5万ノードの日次相当の測定(最悪側 100主体で約0.28秒、未最適化) |
| M5-c(今回) | 検証用RONの入力から、複数補給源・ステート展開・併用規則・入力検証つきで、既存の判定と要約へ接続。A/B を再現し個別指定と一致 |

**D10 の確定に残る判断(ユーザー決定。今回も確定していない)**:
1. 計算単位: プロヴィンス探索+ステート集約(本実装の方式)を採るか。ステート単位のみは、内部の分断が見えない(P32-031 §5)ため推奨しない、という材料までは揃った。
2. 再計算の頻度と1日の時間予算: 下記の提案を採るか。
3. 通行範囲を将来どう導くか(D6: 所有・実効支配・通行権)。現在は検証入力のみ。
4. 本体(経済・AI・軍)への接続と、輸送能力・補給量(L3/L4)。M5 の範囲外。

**本体での再計算頻度の提案(P32-031 の測定が根拠。決定ではない)**: 既定は「入力(通行集合・補給源・所属)が変わった主体だけを再計算」とし、変更の検出(主体ごとの版)を持つ。根拠: 1主体あたり約0.8〜2.8 ms(未最適化)で、変わった主体数に比例する。支配が毎日大きく動く時期(M6/M7)は毎日全主体を再計算にフォールバックしてよい(3主体 約5〜9 ms、30主体 約28〜91 ms、100主体 約80〜282 ms)。1日の実時間の予算が厳しい場合は、P32-031 §8 の最適化候補(通行条件のビット列化、`Reach` の複製削減、集約の差分化)を先に検討する。本実装は、この頻度を固定しない(呼び出し側が決める純粋な関数)。

## 10. M5 の残りと次
本体(経済・AI・軍)への接続は後続(M6 以降)。今回の接続先は検証用CLIだけで、`MapEconomySimulation`・保存形式は不変。
