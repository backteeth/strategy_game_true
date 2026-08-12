# P21-MAP-001 実装サマリー: 6か国・28州へのマップ拡張

日付: 2026-08-11
対象: `p21-map-001_investigation_report.md` および `p21-map-001_addendum_6countries.md` で
設計・承認済みの内容を実装。

---

## 1. 変更内容

| ファイル | 変更内容 |
|---|---|
| `assets/data/states.ron` | 既存10州(ID0-9)は**id/name/owner_country_id等すべて無変更**、`neighbors`のみ追記(削除なし、`git diff`で確認済み)。新規18州(ID10-27)を追加、合計28州 |
| `assets/data/countries.ron` | 既存4か国は完全無変更(`git diff`で削除行ゼロを確認済み)。新興2か国(id4 Passguard March、id5 Ferrowyn League)を追加、合計6か国 |
| `src/app/loader.rs` | `validate_data`に`capital_state_id`の存在確認・所有権確認を追加(addendum §5決定事項) |
| `src/map/rendering.rs` | `MAP_WIDTH` 1800→2400、`MAP_HEIGHT` 1200→1600(28州のレイアウトに合わせて拡大) |
| `src/app/settings.rs` | `CameraSettings::map_bound_x` 1200→1600、`map_bound_y` 900→1200(同上、既存比率を維持) |
| `tests/land_war_combat_peace_test.rs` | (保護対象、addendum §4で事前合意済みの変更) 国数assert 4→6、「全ペア直接隣接」assertを「大陸単一連結性」BFSテストへ置換。新規テスト2件追加(双方向隣接、自己隣接なし)。降伏判定の占領州数を2→4に修正(下記§3参照) |

新興2か国のロールプレイ設定はaddendum §1のとおり(パスガード辺境国=山岳の小国・State13が唯一の入口、フェロウィン自由同盟=三国境の緩衝国家)。

---

## 2. 発見した追加の必須修正(addendum作成時点では未検出)

実装中に、`test_land_war_declaration_combat_and_peace_flow`が`DefenderCapitulated`を
成立させるために「防御側(Oceanic)所有州の60%以上を攻撃側が占領する」という
`check_defender_capitulation`(`src/war/capitulation.rs`)の条件に依存していることが判明した。

- 拡張前: OceanicはState 8・9の2州のみ所有 → 2州とも占領すれば100%で60%条件を満たす
- 拡張後: OceanicはState 8,9,23,24,25,27の6州を所有 → 従来どおり2州(8,9)のみ占領では33%にとどまり、
  条件を満たせず`CapitulationResult::None`になってしまう

これはaddendumで事前確認した「保護対象テストへの必須変更」(国数assert・連結性チェック)とは
別に、実装時に初めて判明した派生的な影響だった。対応として、State 8・9に加えて隣接する
State 23・24(いずれもOceanicの新規州)も占領するようテストを修正し、66.7%(4/6)で
条件を満たすようにした。テストが検証する内容(開戦→前線構築→侵攻→占領→降伏→講和の一連の
フロー、および講和成立時の`AttackerVictory`/州割譲)自体は変更していない。

---

## 3. 検証結果

- `cargo check --all-targets`: 成功
- `cargo test --lib`: **148 passed**(既存の合成データテストのみのため無変化、想定どおり)
- `cargo test --test land_war_combat_peace_test`: **4 passed**(既存2件を書き換え+新規2件、上記§2の修正含む)
- その他の安全な統合テスト(`daily_system_integration_test`/`diplomacy_tests`/`economy_tests`/
  `p20_009_hardcoded_string_scan_test`/`p20_009_localization_resource_test`/
  `profile_workload_correctness_test`/`research_and_politics_tests`): **全55件 passed**
- `ui_headless_render_test`・`p20_009_localization_headless_render_test`は、既知のリスク
  ([[project-headless-test-output-risk]]、コミット済みP20-007/P20-009スクリーンショットを
  実行時に上書きしてしまう)のため今回も**意図的に未実行**(既存の運用方針を踏襲)
- `cargo clippy --all-targets --all-features -- -D warnings`: **0 warnings**
- `cargo fmt --check`: 新規追加コードは手動整形済み。残る差分は本ファイルの既存の
  累積ベースライン(過去セッションで`cargo fmt`を一度も実行せず、各セッションが自分の
  追加分のみ手整形してきた結果、未整形のまま残っている既存行)のみで、今回変更していない
  行のみから構成されることを確認済み
- `cargo build --release --all-targets`: 成功
- 保護対象ファイル(`assets/data/divisions.ron`、`src/app/time.rs`、`src/war/military_ai.rs`)
  の`git diff --stat`: 空(無変更)を確認
- `states.ron`/`countries.ron`の`git diff`から削除行(`-`行のうち内容変更)を抽出し、
  既存10州は`neighbors`行のみが(追記の形で)変更されていること、既存4か国は
  1行も変更されていないことを確認済み

### 実機起動確認(`cargo run --release`, 約40秒間)

- `[DataLoader] Successfully loaded 8 buildings, 22 technologies, 6 countries, 28 states, 3 diplomatic relations`
  — RONパース・`validate_data`(新設のcapital_state_id検証含む)を実際にパスすることを確認
- `[DEBUG] Spawned 6 initial armies` — 新興2か国を含む全6か国の首都に初期軍が配置されたことを確認
- ウィンドウ表示中に(操作環境の残留マウス状態と思われる要因で)Army 0が
  `StateId(1)→StateId(11)→StateId(12)`と経路移動するログが記録され、新規州への
  移動・経路探索がクラッシュなく機能することを確認
- 40秒間クラッシュ・パニックなし

**注記(証拠の正直な報告として明記)**: 上記は自動起動+ログ確認によるものであり、
人手によるクリック操作を伴うインタラクティブな目視確認ではない。マップ全体のパン/ズーム、
全28州のクリック可否、要衝(State13)・突出部(State12)・袋小路の挙動、UIパネルの見た目、
ドラッグ選択・軍事パネルなどは**未確認**。addendumの§6手動確認項目は依然として
ユーザーによる実機プレイでの確認が必要。

---

## 4. 未実施・今後の課題

- addendumの手動確認項目(カメラパン一周、要衝/突出部/袋小路の実プレイ確認、
  UIの見た目、軍事パネル・ドラッグ選択の28州環境での違和感有無)
- 拡張前後の処理時間比較(既存`profiling.rs`基盤を使った定量比較は未実施。
  ただし本セッションの40秒稼働では体感上の遅延は見られなかった)
- 経路探索が複数経路から正しく最短路を選ぶことの専用テスト(既存`military::pathfinding`の
  単体テストは合成データのままで、本番28州データに対する専用テストは追加していない)
- 全州クリック・ドラッグ選択回帰の28州環境での専用テスト(既存のドラッグ選択テスト群は
  合成データのままのため無変化を確認したのみで、本番28州データに対する専用テストは
  追加していない)

これらはaddendum §9(自動テスト計画)の一部として将来のセッションで追加可能。
今回は「州数拡張そのものが安全に実装できること」を最優先で確認した。
