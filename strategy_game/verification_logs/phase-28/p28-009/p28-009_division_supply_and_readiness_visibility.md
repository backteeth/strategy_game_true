# P28-009：選択Divisionの補給・装備・組織率の可視化

## 1. 判定
実装完了(自動テスト合格)。実ウィンドウでの目視確認は **MANUAL VERIFICATION PENDING**。

- 開始HEAD: `2d07f032362a7f0970e99c3ef051dd4ed001182e`(作業ツリーclean確認済み)

## 2. 問題
軍事パネルは兵力・組織・状態のみを表示し、補給充足率・装備充足率は一切表示されなかった。
複数選択時は値自体が出なかった。このため補給0のDivisionを画面で判別できなかった。

## 3. 表示内容とデータ源(独自計算式なし)
| 表示 | データ源 |
|---|---|
| 兵力 現在/最大・比率 | `Division`のmanpower |
| 装備 現在/必要・充足率(必要0は「装備不要」) | `Division`の装備量 |
| 組織 現在/最大・比率 | `Division`のorganization |
| 補給(最新の日次配分値) | `MilitaryRegistry::division_supply_fulfillment`(Division単位) |
| 戦闘準備率と制限要因(兵力/装備/補給) | `calculate_combat_readiness_with_supply`(正規関数) |

- 補給は「Divisionが最新の日次配分で得た値」と明記。国家備蓄(MilitarySupplies日数)ではない。
- 未計算 / 不正値 / 消滅は0%と区別して表示。補給0%のDivisionも隠さない。平均化なし。
- 戦闘準備率は「最終戦闘力」ではなく、制約要因表示として表記。
- 複数選択: Divisionごとに1行の要約(`compact_line`)。単数選択: 詳細行(`detail_lines`)。
- 実装: `src/ui/division_status_view.rs`(純粋ビューモデル)、`src/ui/military_panel.rs`(組込)。

## 4. 更新タイミング
既存のパネル更新System(開いている間の毎フレーム再構築)に従う。日次補給配分後の値が次の更新で反映される。
load直後はパネルが閉じるため、再度開くまでテキストは古いまま(既存挙動)。

## 5. 変更していないもの
移動入力、日次補給、装備補充、組織回復、戦闘準備率・ダメージ・退却、AI、Save形式(Save V1、永続フィールド追加なし)。

## 6. ローカライズ
`military_panel.*`に22キーを日英で追加。既存キー(`military_panel.strength`/`organization`)は残置(未使用)。キー集合・プレースホルダはJA/EN一致をテストで確認。

## 7. テスト
- 単体(`division_status_view`、14件): 補給0%・部分・満額、装備不足、組織0、必要装備0、制限要因、不正/未計算ほか。
- UI統合(`tests/p28_009_division_status_panel_e2e_test.rs`、7件): 複数選択でA(補給0)/B(満額)が各自のIDに結び付く(順序入替も)、単数詳細、日次更新追従・未計算/消滅の区別、必要装備0、実日次補給Systemの配分値との一致、開閉で状態不変、save→load後。
- 全体: `cargo test --all-targets -j 2` = 2794 passed / 0 failed / 1 ignored(基準2773から+21)。
- `cargo clippy --all-targets --all-features -- -D warnings` = 警告0。

## 8. 実ウィンドウ確認
この環境では実ウィンドウ操作ができないため **MANUAL VERIFICATION PENDING**。
確認手順: 2個以上のDivisionを選択→補給/装備/組織/戦闘準備率が各Divisionごとに表示されること、補給0%のDivisionが隠れないこと、攻撃時の制限要因表示が実際の戦闘結果と整合すること。

## 9. 初回ゼロダメージ戦闘(state 3)の原因
**未確定(UNDETERMINED)**。計測データが無く、断定しない。
正規のゼロダメージ条件: 攻撃側の有効戦闘力=0(防御側へのダメージ0)、または防御側有効戦闘力+地形=0(攻撃側へのダメージ0)。有効戦闘力=戦力×準備率×組織比。
再現時は新パネルで該当Divisionの補給/装備/組織/準備率を確認し、どれが0かを記録する。

## 10. P27-005 手動確認
**PENDING のまま**。補給の画面表示と実際の攻撃での観測が揃っていないため完了とは主張しない。

## 11. 保護事項
`saves/savegame_v1.ron`・`tests/land_war_combat_peace_test.rs`は未変更。ワークスペース全体のformatなし。再生成PNGは個別に復元済み。
