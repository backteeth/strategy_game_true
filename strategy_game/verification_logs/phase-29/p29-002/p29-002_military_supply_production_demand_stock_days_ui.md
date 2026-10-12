# P29-002：軍用補給物資の生産・需要・備蓄日数の可視化

プレイヤー国家の MilitarySupplies(画面名「軍用補給物資」)について、**前回の月次生産(実績)・現在の日次需要・備蓄日数**を、既存の国家経済パネル(左側の常時表示パネル)へ追加した。生産式・入力配分・FIX1・D4・日次補給・募兵AI・建設AI・戦闘・講和・Save V1・本番 RON データは変更していない。

```text
P29-002 COMPLETE
SUPPLY BALANCE UI IMPLEMENTED
GAMEPLAY CONTRACT PRESERVED（生産結果・備蓄・研究力は従来経路とビット同一。回帰テストで確認、§7）
MANUAL VERIFICATION PENDING（実ウィンドウは未操作。自動テストを画面確認済みとは扱わない）
```

## 1. 基準状態

| 項目 | 値 |
|---|---|
| 期待HEAD | `642acd661f245ee30b8ab8dd18e9f071be84a28d` |
| 開始時の実HEAD | `642acd661f245ee30b8ab8dd18e9f071be84a28d`(一致、作業ツリーはクリーン) |
| テスト基準(引用) | 2836 passed / 0 failed / 1 ignored、clippy 警告 0(P28-012 の引用値) |
| 今回の最終実測 | §8 |

## 2. 変更ファイル

| ファイル | 内容 |
|---|---|
| `src/economy/production.rs` | `CountryProductionOutcome`(研究力 2 値 + MilitarySupplies 実産出)と `process_country_production_with_outcome` を追加。既存の `process_country_production` は同じ処理を呼ぶ薄いラッパで、戻り値・呼び出し元は不変。生産式・処理順・配分は変更なし。実産出は、既存の国家単位の commit(`add_military_supplies_produced`)が成功した量だけを返す |
| `src/economy/supply_balance.rs`(新規) | 表示専用の月次生産実績 `MilitarySupplyProductionLedger`(非永続 Resource)、表示ビュー構築 `build_military_supply_balance_view`(純粋関数)、単体テスト 6 件 |
| `src/economy/mod.rs` | `handle_monthly_economy` が実産出と対象年月を ledger へ記録(`Option<ResMut<...>>` なので ledger の無い既存テストは影響を受けない)。`EconomyPlugin` が ledger を登録し、国家選択画面への進入(New Game)で消去 |
| `src/save/runtime.rs` | ロード成功時(`reset_terminal_ui_state`)に ledger を消去 |
| `src/ui/economy_panel.rs` | 軍用補給物資の収支ブロックを整形(`format_military_supply_balance`)し、`economy_panel.info` へ挿入。単体テスト 8 件 |
| `assets/localization/ja-JP.ron`・`en-US.ron` | `economy_panel.supply_*` の 9 キーを追加、`economy_panel.info` に `{supply_balance}` を追加 |
| `tests/p29_002_military_supply_balance_e2e_test.rs`(新規) | 9 件(実産出の契約 5、実 App の月次記録・ロード・New Game 4) |
| `verification_logs/phase-29/p29-001-ver-001/...results.md` | 手動結果報告書の限定的訂正(§10) |
| 本報告書 | — |

## 3. 表示場所と名称

国家経済パネル(画面左、常時表示)の「備蓄」一覧の直下。既存の備蓄表示(軍用補給物資・軍需物資を含む全資源)と、Division 詳細(軍事パネル)は変更していない。

| | 日本語 | English |
|---|---|---|
| 見出し | `[軍用補給物資の収支]` | `[Military Supplies Balance]` |
| 生産 | `前回の月次生産(実績・1800/03分): 36.0` | `Last monthly production (actual, 1800/03): 36.0` |
| 需要 | `現在の需要: 1.0 / 日` | `Current demand: 1.0 / day` |
| 備蓄日数 | `備蓄: 180.0日分(追加生産なしで現在の需要が続く場合)` | `Stock: 180.0 days (if current demand continues with no new production)` |
| 注記 | `※各Divisionの補給充足率とは別の、国家備蓄の日数です` | `Note: national stockpile days, separate from each division's supply fulfillment` |

- 見出しは「軍用補給物資(MilitarySupplies)」。「軍需物資(MilitaryEquipment)」と混同しない名称にした。
- 生産に月次(対象年月つき)、需要に日次の単位を付けた。色には頼らず、文字と数値で表す。
- 技術用語・内部のタスク ID は表示していない。数値の表示は小数 1 桁(丸めは表示のみ)。

## 4. 3 項目の定義・単位・算定元

| 項目 | 定義 | 単位 | 算定元(新規の式は作っていない) |
|---|---|---|---|
| 前回の月次生産 | 直近の月次生産で**実際に備蓄へ反映された**MilitarySupplies の総量 | 1 か月あたり | `process_country_production_with_outcome` が、entry ごとに正規 units へ量子化して国家合計した値(FIX1・D4・入力不足・量子化を反映)。既存の commit(`add_military_supplies_produced`)が成功した量 |
| 現在の需要 | 現在の Division 構成で必要となる日次の補給量 | 1 日あたり | `military::supply_initialization::country_daily_demand_units`(日次補給処理と同じ正規関数。生存 Division のみ、manpower 按分、備蓄を参照しない) |
| 備蓄日数 | `現在の備蓄 units ÷ 現在の日次需要 units` | 日 | 上記需要と、国家備蓄の正規 units 変換(`military_supplies_value_to_units`)。将来の月次生産は含めない |

実績と推計値の区別:

- 表示するのは**実績のみ**。建物 Level からの最大能力や、稼働率からの見込みは表示しない・計算しない。
- **実産出と備蓄の純増減は別物**として扱う。生産後に日次消費で備蓄が減っても、記録済みの実績は変わらない(§7 のテストで固定)。

## 5. 未集計・需要 0・備蓄 0・異常値の扱い

| 状態 | 表示 | 根拠 |
|---|---|---|
| 月次生産をまだ観測していない(ゲーム開始直後、ロード直後) | `前回の月次生産(実績): 未集計(次の月次生産後に表示)`。**0 とは表示しない** | ledger に項目が無い |
| 集計・commit に失敗した月(量子化不能・overflow・上限超過) | `未集計`(0 ではない)。備蓄は従来どおり変更しない | `military_supplies_produced == None` |
| 軍需工場が無い等で実産出が 0 | `0.0`(実績 0。未集計とは別) | `Some(0.0)` |
| 需要 0(生存 Division なし) | `現在の需要: 0.0 / 日`、`備蓄: 消費なし(需要0)`。ゼロ除算・NaN・Infinity は出さない | `StockDays::NoConsumption` |
| 需要が正で備蓄 0 | 需要は 0 にせず表示し、`備蓄: 0.0日分` | 需要は備蓄を参照しない |
| 需要または備蓄が異常値(NaN・無限大・負数・範囲外) | `現在の需要: 算出不能` または `備蓄: 算出不能`。備蓄値は変更しない | 既存の正規 units 変換の `Err` をそのまま使う |

## 6. 更新タイミングとキャッシュのライフサイクル

| 項目 | 内容 |
|---|---|
| 更新 | 既存の `update_economy_panel`(毎フレーム)が、日付・軍事登録・実績・言語・プレイヤー国家のいずれかが変化したときだけ収支ブロックを再計算し、それ以外は直前のテキストを再利用する(毎フレームの全 Division 走査を避ける)。再クリック不要 |
| 一時停止中 | 表示のための時間経過は要求しない(日付が進まなければ再計算もしない) |
| 値の整合 | 備蓄・需要・実績は同じフレームのスナップショットから読む。実績と備蓄は同一 System(`handle_monthly_economy`)内で確定するため、異なる日の値は混在しない |
| 保存 | **非永続**(`SaveGameV1` に含めない。新規 Save フィールド・migration なし) |
| New Game | 国家選択画面へ進入するときに消去(`OnEnter(CountrySelection)`) |
| ロード | ロード成功時に消去。**ロード後は次の月次生産まで「未集計」と表示する仕様**(実績を Save から復元しない) |
| 国家切替 | ledger は国家 ID をキーとし、表示はプレイヤー国家の項目だけを読む。生産しない国家(州なし)の古い実績は月次処理で削除する |

## 7. 非回帰(ゲーム挙動への影響なし)

- 生産経路は、実績を返す関数と従来の関数が同じ本体を共有する。テスト `outcome_variant_is_bit_identical_to_the_legacy_entry_point` が、備蓄の全資源(ビット単位)・研究力・稼働率が従来の入口と一致することを確認する。
- 表示更新は読み取りのみ。テスト `updating_the_panel_has_no_side_effects_on_resources_or_divisions` が、パネルを複数回更新しても全国家の全資源備蓄・Division の兵力・補給充足率・日付が変わらないことを確認する。
- 変更していない領域: 生産量式・Iron 配分(D4)・入力の原子的会計(FIX1)・日次補給会計と配送・募兵 AI・建設 AI・攻勢ゲート・戦闘・講和・地形・Save V1 の永続構造・RON データ。AI の判断は ledger を一切参照しない(ledger を読むのはパネルだけ)。
- 既存の FIX1/D4 の恒久テスト(`p28_010_impl_001`・`p28_010_impl_002` の fixture・E2E)は変更していない。結果は §8。

## 8. 恒久テストと実行結果

| テスト | 件数 | 確認する意味 |
|---|---|---|
| `economy::supply_balance::tests` | 6 | 備蓄 0 でも需要を表示、備蓄日数 = 備蓄 ÷ 需要で生産を含めない、需要 0、国家別・生存 Division のみ、異常値、未集計と 0 の区別・消去 |
| `ui::economy_panel::tests`(追加分 8) | 8 | JA/EN の 3 項目、未集計・失敗・0 の区別、日付・備蓄・募兵完成・Division 消滅への追従(再クリックなし)、備蓄 0・需要 0、国家切替で他国の値が混ざらない、言語切替、副作用なし |
| `tests/p29_002_military_supply_balance_e2e_test.rs` | 9 | 実産出 = 備蓄増分(units)、複数州の合計、入力不足の反映、実績 0、commit 失敗は未集計、従来経路とビット同一、実 App の月次記録(全国家で備蓄増分と一致)、日次消費後も実績不変、ロードで消去、New Game で消去 |

| コマンド | 結果 |
|---|---|
| `cargo test --all-targets -j 2 --no-fail-fast` | **2859 passed / 0 failed / 1 ignored**(93 binaries。引用値 2836 + 新規 23: 単体 6 + パネル 8 + E2E 9) |
| `cargo clippy --all-targets --all-features -- -D warnings` | **警告 0**(初回は新規テストの `field_reassign_with_default` 2 件を指摘。構造体初期化へ修正して再実行し 0 を確認。修正後に `economy_panel` の単体テスト 9 件を再実行して合格) |
| 個別実行(`cargo test --lib supply_balance`、`--lib economy_panel`、`--test p29_002_...`) | 6 / 9(`economy_panel` 全体)/ 9 passed |

関連する既存テスト(FIX1・D4、ローカライズ、Save、決定性)は、上の全体テストに含まれる。表示追加を理由とする 600 か月診断・募兵候補比較は実施していない(生産結果が従来経路とビット同一であることを関数単位で確認したため)。

- 全体テストの途中で再生成された既知の非決定的 PNG があれば、対象を確認して個別に `git checkout --` で復元した(`verification_logs/p20-007`・`p20-009`・`phase-21/p21-save-002e` の 9 件を個別に復元)。
- `saves/savegame_v1.ron` は読み書きしていない。Save を使うテストは `std::env::temp_dir()` 配下の一意な一時ディレクトリだけを使う。

## 9. 実ウィンドウ確認

**未実施(MANUAL VERIFICATION PENDING)**。エージェントは実ウィンドウを操作していない。単体・実 App テストは、テキストの内容・更新・ライフサイクルを確認するが、画面の見切れ・重なり・実際の視認性は確認していない。パネルは幅 300px で、`LineBreak::AnyCharacter` による折り返しがあるが、注記行は長いため、折り返しの見え方は実画面で確認が必要。

## 10. 手動結果報告書の限定的訂正

`verification_logs/phase-29/p29-001-ver-001/p29-001-ver-001_manual_supply_combat_ui_results.md` の、P27-005 の判定表(§4)・条件表(§5)の記述を訂正した。

| 訂正前 | 訂正後 |
|---|---|
| 「Division 状態「戦闘中」の明示報告は無い」「条件 1 は間接観測」 | ユーザーの 07/07 の報告に「状態：戦闘」があり、その後「戦闘中は Arcadian Capital、攻撃側」と明示された。条件 1 は観測として記載。**画面の正確な文言、州の支配者(controller)表示、内部の controller・経路まで確認したという意味へは拡張しない** |

初期工場 Level の不一致(§8)、初期の「補給：まだ判定なし」、その他の未確認事項は、そのまま維持した。

## 11. 残課題と短い手動確認手順

残課題(本タスクの範囲外):

- 工場の稼働率の律速要因(労働力・物流・入力資源)の画面表示。
- 全資源の収支ダッシュボード、生産・消費の月別推移。
- 実際に増員してよいかの判断(需要・実績・備蓄日数から読めるが、目標 Division 数と持続的生産能力の設計は Phase 28 の ISS-3 のまま)。
- 実ウィンドウでの見切れ・重なりの確認。

手動確認手順(約 5 分):

1. `cargo run`(作業ディレクトリ `strategy_game\strategy_game`)。国家選択で **Dwarf Federation** を選んで「ゲーム開始」(セーブ/ロード/「続きから」は押さない)。
2. 画面左の国家経済パネルの「備蓄」の下に、`[軍用補給物資の収支]` の 4 行(前回の月次生産・現在の需要・備蓄・注記)があることを確認する。開始直後は「未集計」のはず。
3. `>1` で進め、1800/02/01 を過ぎた直後に Space で停止する。「前回の月次生産(実績・1800/02分)」に値が出ること、直前の備蓄から増えた分と見比べる。
4. さらに数日進め、備蓄が減ること、「備蓄: n日分」が追従すること(再クリック不要)を確認する。
5. 見切れ・重なり(特に注記行の折り返し)があれば記録する。言語切替ボタンで英語表示も確認する。

## 12. 判定

| 判定 | 結果 |
|---|---|
| P29-002 | COMPLETE |
| SUPPLY BALANCE UI | IMPLEMENTED |
| GAMEPLAY CONTRACT | PRESERVED(生産・備蓄・研究力は従来経路とビット同一。副作用なしをテストで確認) |
| MANUAL VERIFICATION | PENDING(実ウィンドウ未確認) |
