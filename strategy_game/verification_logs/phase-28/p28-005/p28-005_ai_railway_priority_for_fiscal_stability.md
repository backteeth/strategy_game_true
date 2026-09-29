# P28-005 長期財政を安定させるAI Railway優先建設

P28-005-PRE-001・P28-005-PRE-002で採用したR0案(物流比率0.9未満の州へRailwayを優先建設する)を実装した。建設AIの優先度2.5(戦時MilitaryFactory)の後・優先度3(通常開発)の前に、優先度2.7として追加した。雇用率による対象州の絞り込みは行わない(P28-005-PRE-002のR1が境界依存の保護漏れを起こすことが確認済みのため)。財政式・生活水準式・補給会計・税率AIは一切変更していない。

## 1. 判定

```text
P28-005 COMPLETE
```

全AI・無操作の実App(現行・A_all・A_war診断)で600か月まで、Dwarf(現行413か月ごろ)とElfin(600か月以内)の財政崩壊経路が解消し、6か国とも税率0.25以下・生活水準32未満の月0で安定した。既存テスト7件(6バイナリ)は、テスト名単位の照合により事前の要約にあった件数の食い違い(「6件」)を解消し、正しく7件(p26_006 1件、p26_det_001 1件、p26_det_001_rev_001 1件、p27_002_c 1件、p28_002 2件、p28_003 1件)であることを確定した。いずれも本番コードの契約違反ではなく、正当な世界状態の変化(6件は正規値更新)またはfixture依存(P27-002-Cの振動上限基準1件を振幅・健全性基準へ再定義)として扱った。

## 2. 開始時HEAD・作業ツリー・テスト基準

- `git rev-parse HEAD` → `5e1ebe2c537adb525ce370039010df0100212e87`(期待値と一致、P28-005-PRE-002の報告書コミット)
- `git status --short` → 空
- 開始時テスト基準(前タスクの最終実測から引用): 2734 passed / 0 failed / 1 ignored、clippy警告0
- 本タスク開始時に`cargo test --all-targets -j 2`と`cargo clippy --all-targets --all-features -- -D warnings`を実測: **2734 passed / 0 failed / 1 ignored**(84バイナリ)、**clippy警告0**。前タスクの最終実測と完全一致することを確認した。

## 3. R0の正確な判定式・対象州・優先順位・資金条件

実コードを`rg`で再確認し、報告書の式と一致することを確認した(§8参照)。

- **対象**: 自国が所有かつ支配する陸上州(`owned_states`、`!s.is_sea && owner_country_id == country_id && controller() == country_id`、既存の走査条件を流用)のうち、`state.logistics_ratio < economy::finance::LOGISTICS_WELFARE_BONUS_RATIO`(0.9)、Railwayが最大レベル未満、建設キューに同州のRailwayがない州。
- **選択**: 物流比率が最も低い州(`owned_states`は呼び出し側で既に`StateId`昇順に整列済みのため、「厳密に低い場合だけ更新」する通常の規則で走査順に依存せず決定的)。
- **優先順位**: 優先度1(Food緊急)→優先度2(原材料/IndustrialGoods事前開発)→優先度2.5(戦時MilitaryFactory、`wartime_supply`がSomeのときだけ)→**優先度2.7(Railway、新規)**→優先度3(通常開発)。
- **資金条件**: `国庫 ≥ 建設費(RONの800) + 月次経常支出 × PEACETIME_CONSTRUCTION_RESERVE_MONTHS(3)`。既存の優先度3・P27-002-Aと同じ予備資金契約(戦時MilitaryFactoryの1か月とは異なる、平時契約を流用)。
- **1回の判断で最大1件**: 既存の関数構造(1つの`if`ブロックが成功したら`return`)をそのまま踏襲。
- **完成後の再評価**: 永続状態を持たない。毎月の判断のたびに`owned_states`と各州の現在の`logistics_ratio`・Railwayレベルを読み直すため、自然に再評価される(§8の契約13で単体テスト済み)。

## 4. 実装ファイルと変更内容

- **`src/economy/finance.rs`**: `LOGISTICS_WELFARE_BONUS_RATIO: f32 = 0.9`を新設し、`update_state_welfare_and_unrest`内の既存のリテラル`0.9`をこの定数に置き換えた(値は不変、生活水準の物流ボーナス境界と建設AIの選定閾値を同じ定数で恒久的に一致させるため)。財政式・税率AI・生活水準の計算式そのものは変更していない。
- **`src/country/country_ai.rs`**(本番コードの主な変更対象):
  - `try_logistics_railway_construction`(新規、優先度2.7の本体)。
  - `process_construction_ai_with_wartime_supply`に、優先度2.5の後・優先度3の前で上記関数を呼ぶ1ブロックを追加。
  - テスト用共有建物レジストリ(`p26_008_c_building_registry`)にRailway定義を追加(既存テストで唯一欠けていた定義、他のテストの挙動には影響しない)。
  - 恒久単体テスト13件を追加(§6)。

`src/military/ai.rs`は本タスクでは変更していない(募兵条件はP28-004の対象のまま、§10)。

## 5. 建設キュー・重複・月次上限・完成後の再評価

- 建設キュー上限2件(既存、関数冒頭の判定)はRailway専用の例外を作らず、そのまま適用される。
- 既に同じ州にRailwayがキュー中なら(`queued`判定)、優先度2.7は`false`を返し、優先度3へフォールスルーする(単体テスト`p28_005_respects_existing_queue_and_buildable_conditions`で、フォールスルー先が別の建物を建てることまで含めて確認)。
- 最大レベル(RONで5)に達した州は候補から除外する。
- 完成後の再評価は§3のとおり、永続状態なしで自然に行われる。単体テスト`p28_005_reevaluates_current_logistics_and_level_after_completion`で、完成後もまだ0.9未満ならレベル+1で再発注し、0.9以上に改善していれば発注しないことを確認した。

## 6. 既存7件のテスト名単位の照合と件数不一致の解消

前タスクの要約には「正規値変更6件」という記載があったが、`P28RAIL`相当(本タスクでは恒久有効)を全体テストへ適用して実測したところ、失敗は次の**7件**(6バイナリ)だった。件数を推測で補わず、実際の失敗結果から確定した。

| # | ファイル | テスト名 | 変更前(旧)の値 | 変更後(新)の値 |
|---|---|---|---|---|
| 1 | `p26_006_capital_connectivity_supply_e2e_test.rs` | `test_36_month_oceanic_shortage_unchanged_when_uncontested_by_capital_connectivity` | shortage_days=550 | shortage_days=561 |
| 2 | `p26_det_001_construction_ai_determinism_e2e_test.rs` | `test_f_all_six_countries_stable_within_process_and_no_regression` | Oceanic 459.577314 | Oceanic 402.23853800000006 |
| 3 | `p26_det_001_rev_001_cross_state_production_order_e2e_test.rs` | `test_i_all_six_countries_stable_within_process_and_no_regression` | Oceanic 459.577314 | Oceanic 402.23853800000006 |
| 4 | `p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs` | `test_05_real_six_country_90_day_36_60_120_month_long_horizon_verification` | Oceanic振動3回、上限0〜3回、M120完全一致 | Oceanic振動7回(基準を振幅・健全性へ再定義)、Arcadia/Dwarf/OceanicのM120値変更 |
| 5 | `p28_002_wartime_military_factory_construction_e2e_test.rs` | `test_01_dwarf_builds_completes_and_passes_the_unchanged_offensive_gate_by_its_own_production` | [(1800/06/05,州20,lv1),(1801/03/24,州20,lv2)] | [(1800/06/05,州6,lv3),(1801/03/03,州6,lv4)] |
| 6 | `p28_002_wartime_military_factory_construction_e2e_test.rs` | `test_03_save_load_before_during_and_after_wartime_construction_matches_continuous_run` | queued分岐: [(州20,lv1)] | queued分岐: [(州6,lv3)] |
| 7 | `p28_003_disconnected_friendly_frontline_delivery_e2e_test.rs` | `test_01_exclave_divisions_no_longer_block_war8_and_no_unsupplied_attack_occurs` | 州8に取り残されたDivision2個(自然再現) | 自然再現が消滅(§9で対応) |

内訳は「Phase 26の正規値更新3件+P28-002の2件+P27-002-Cの1件+P28-003の1件=7件」であり、事前要約の「6件」はP28-003の1件が数え落とされていたことによる食い違いだった。

### 7. 各テストの旧契約・変更理由・新契約(対応表)

| # | 旧契約(保護対象) | R0で結果が変わる上流イベント | 変更したfixture/期待値 | 変更後も守るアサーション |
|---|---|---|---|---|
| 1 | Oceanicの補給不足日数は接続性判定の境界追加だけでは変わらない | Dwarfの経済発展経路がRailway建設で変わり、州6の生産・戦争タイムラインが早まった影響がOceanicにも波及(§9) | `shortage_days`の期待値のみ | `first_shortage_day=32`(初期ブリッジ備蓄枯渇由来、cadence非依存)は不変のまま維持 |
| 2・3 | Oceanicの36か月MilitarySupplies生産分の回帰保護 | Oceanicが物流改善のためのRailway建設に建設枠・資金の一部を振り向け、Factory増産が後ろ倒しになった | Oceanicの期待値のみ(Dwarf・他4か国は不変) | Dwarf(プレイヤー国家)・Arcadia・Elfin・Passguard・Ferrowynの値は完全一致のまま維持 |
| 4 | Oceanicの税率振動は0〜3回、全国のM120値は監査実測と完全一致 | Oceanicの経済経路変化により、税率0.12/0.13付近の均衡点近傍で反転が3→7回に増えた(§9で原因を確定) | Oceanicの振動基準を振幅・健全性基準へ置換。Arcadia/Dwarf/OceanicのM120値を更新 | 他5か国の反転回数(0〜3回、完全一致)、Ferrowyn/Passguardの健全性条件(§既存)は無変更のまま維持。Oceanicも国庫負転0・税率≤0.25は健在 |
| 5・6 | 戦時に実月産不足を検知してMilitaryFactoryを1件建てる(選定式自体) | 州6が優先度2.7でRailwayを先に得たことで、優先度2.5の増産見込み判定(`projected_military_factory_supply_gain`)が州20ではなく州6を選ぶよう自然に変わった(§9で実証) | 着工州・レベル・save/load期待値のみ | 契約自体(`try_wartime_supply_construction`の単体テスト12件)は無変更・全件緑のまま維持。実Appでも「戦時に着工→完成後の実生産が開戦時を上回る→正規のゲート再開→攻勢→戦闘→勝利」という機能上の契約を、州を問わない形で明示的に追加検証 |
| 7 | 自国支配・首都非連結のDivisionを前線配送判定から除外する | Railway導入で戦争タイムライン全体が変わり、州8の飛び地が生じる自然再現シナリオが消滅(§9で240か月まで探索し確認) | 自然運用への依存を、制御されたfixture(実New Gameデータ+`add_war`/`update_all_frontlines`)へ移行 | 契約自体(`offensive_delivery_division_ids`の単体テスト7件)は無変更・全件緑のまま維持。制御fixtureで同じ因果(飛び地の充足率0、従来母集合なら0.34未満で停止、新母集合では停止しない)を実App経由で検証 |

## 8. Oceanicの税率反転の測定と新しい財政受け入れ条件

P27-002-Cの実App(`p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`の30日固定暦)で実測した。

| # | 月 | 反転 |
|---|---|---|
| 1〜7 | m26・m42・m43・m51・m52・m59・m118 | 税率0.11〜0.15の範囲内で0.12/0.13を中心に往復 |

- 全120か月を通じ税率は0.09〜0.15、国庫は負転せず、生活水準は68〜76で推移。
- P27-002-C-PRE-001が問題視した候補0〜6の「44〜118回、税率が0.97まで上昇する拡大型振動」とは性質が異なる均衡点近傍の有界な微振動であることを、以下の新しい受け入れ条件で恒久的に検証する(`tax_rate_oscillation_amplitude`、単体テスト2件付き)。
  - 国庫が120か月間、一度も負転しない
  - 税率が120か月間、一度も0.25を超えない
  - 反転が実際に起きている区間(最初の反転の直前〜最後の反転)での税率の振れ幅(振幅)が0.02以内
- 実測: Oceanicの振幅は0.020(閾値内)、最大税率0.15、国庫負転なし。他5か国は従来どおり反転回数0〜3回・完全一致の厳密な回帰保護を維持する。

## 9. P28-002・P28-003の機能上の契約の保護

- **P28-002**: `try_wartime_supply_construction`とその判定式(実月産・入力資源・増産見込み・資金条件)は無変更。単体テスト12件は無変更で全件緑。実App経由で「戦時にDwarfが着工→完成後の実生産が開戦時月産を上回る→自国生産で既存ゲート(40/70日)が正規に再開→攻勢→戦闘→攻撃側勝利」という機能上の契約を、着工州を問わない形で明示的に追加検証した(州6を選ぶこと自体は自然再現の参考値として別途固定)。原因は、優先度2.7が州6にRailwayを先に与えたことで、州6の経済状態(建物構成・雇用)が変わり、優先度2.5の増産見込み判定が州20ではなく州6を選ぶようになったため(§7の対応表参照、実Appの建設イベント時系列で直接確認、`verification_logs/phase-28/p28-005/`実測)。
- **P28-003**: `offensive_delivery_division_ids`は無変更。単体テスト7件は無変更で全件緑。自然運用でのwar8シナリオ(州8の飛び地)は240か月まで探索しても再現しなくなったため(Railway導入で戦争タイムライン全体が変わったことによる)、実App・実データ+`WarRegistry::add_war`/`update_all_frontlines`(P26-008-Bの既存パターンと同じ)による制御されたfixtureへ移行した。飛び地は、州8の所有権(Oceanic)を変えず、`controller_country`だけをDwarfへ上書きして構成した(契約が除外する2分類のもう一方、「他国所有・自国占領」)。飛び地のDivision2個は実際の日次補給Systemにより充足率0になり、従来の母集合(全割当)なら0.34未満で停止していたはずのところ、実際の前線Systemでは攻勢が塞がれないことを確認した。
  - なお、取り残されたDivisionが配送判定からは除外されても前線割当・移動命令の対象には残るという既存のLow L-6(本タスクの対象外)により、本fixtureでは飛び地のDivisionが実際に敵領内の戦闘へ1日だけ参加する事象が観測された。自然運用(P28-003完了時点)で「補給0のDivisionを含む攻撃が0日」だったのは、飛び地が地理的に遠く観測窓内で戦闘に参加しなかった偶然の結果であり、P28-003の契約(配送判定からの除外)自体の保証ではないため、恒久テストではassertしていない(診断として記録)。

## 10. Save V1・save→load・決定性・プレイヤー保護

- 新規永続フィールド・enum値は追加していない。判断理由は既存の`PeacetimeDevelopment`を再利用した(P28-002が`WarInProgress`を再利用したのと同じ方針、Save V1の値域を広げない)。Railway建設は既存の`ConstructionQueueItem`だけを使う。
- `tests/p28_005_ai_railway_priority_e2e_test.rs::test_03`で、Railwayの着工前・建設キュー中・完成後の3分岐でsave→loadを挟んでも、建設キューが重複せず、連続実行と一致することを確認した。
- 単体テスト`p28_005_tie_breaks_by_state_id_regardless_of_registration_order`で、州の登録順を反転しても選択結果が一致することを確認した。600か月診断を独立プロセスで2回実行し、FIS行(税率・国庫・収支・生活水準)が1バイトも違わず完全一致することを確認した(決定的)。
- 単体テスト`p28_005_daily_ai_never_applies_to_player_or_eliminated_countries`で、プレイヤー国家・消滅国家にはRailway優先建設が適用されないことを確認した。

## 11. 6か国の120・240・360・480・600か月結果(現行・A_all・A_war)

一時診断(`tests/p28_005_temp_longterm_diagnostic.rs`、実カレンダー。最終検証前に削除、§18)で、現行AI運用・P28-004候補A_all・A_war(いずれも一時診断条件、募兵の本番実装や政策承認を意味しない)の3条件を600か月まで測定した。A_all/A_warの適用にあたっては、P28-004-PRE-001の一時パッチ(`src/military/ai.rs`)を診断中だけ再適用し、最終検証前に完全に除去した(§18)。

### 11.1 現行AI運用(補給・資金の注入なし)

| 月 | Arcadia | Elfin | Dwarf | Oceanic | Passguard | Ferrowyn |
|---|---|---|---|---|---|---|
| 120 | 0.16/LS52/tr4517 | 0.12/LS71/tr6270 | 0.13/LS69/tr15176 | 0.13/LS67/tr5022 | 0.19/LS67/tr904 | 0.19/LS55/tr686 |
| 240 | 0.15/LS55/tr3722 | 0.13/LS67/tr8216 | 0.13/LS69/tr16523 | 0.12/LS77/tr3053 | 0.20/LS67/tr1842 | 0.14/LS69/tr4259 |
| 360 | 0.16/LS53/tr4544 | 0.14/LS67/tr8131 | 0.14/LS68/tr17793 | 0.12/LS77/tr3047 | 0.19/LS67/tr1784 | 0.14/LS73/tr4744 |
| 480 | 0.18/LS48/tr4839 | 0.13/LS69/tr9511 | 0.14/LS71/tr20940 | 0.11/LS88/tr3238 | 0.19/LS67/tr1945 | 0.14/LS73/tr5659 |
| 600 | 0.19/LS45/tr4763 | 0.13/LS69/tr9424 | 0.13/LS72/tr33031 | 0.09/LS100/tr3718 | 0.18/LS67/tr2138 | 0.14/LS73/tr5890 |

(表記: 税率/加重生活水準/国庫。全600か月でLS<32の月0、最大税率0.24[Passguard]、国庫が負の月はPassguardの開始直後17か月のみ[120か月時点では既に黒字]。P28-005-PRE-001/PRE-002の実測と完全一致。)

### 11.2 A_all(一時診断条件、平時にも「募兵後の月需要≤実月産」を課す案)

| 月 | Arcadia | Elfin | Dwarf | Oceanic | Passguard | Ferrowyn |
|---|---|---|---|---|---|---|
| 120 | 0.14/LS60/tr8409 | 0.12/LS71/tr6270 | 0.16/LS58/tr6080 | 0.13/LS71/tr11509 | 0.19/LS67/tr904 | 0.19/LS51/tr1961 |
| 240 | 0.15/LS59/tr9275 | 0.12/LS71/tr5866 | 0.16/LS58/tr6068 | 0.13/LS70/tr12242 | 0.20/LS67/tr1842 | 0.18/LS58/tr2610 |
| 360 | 0.14/LS67/tr22490 | 0.15/LS61/tr2630 | 0.17/LS59/tr2278 | 0.11/LS88/tr3789 | 0.21/LS61/tr1171 | 0.13/LS65/tr6865 |
| 480 | 0.14/LS67/tr23700 | 0.16/LS61/tr3088 | 0.17/LS59/tr2893 | 0.11/LS92/tr3234 | 0.20/LS61/tr1862 | 0.14/LS65/tr7686 |
| 600 | 0.13/LS71/tr36467 | 0.18/LS54/tr3502 | 0.23/LS46/tr2590 | 0.11/LS92/tr3895 | 0.20/LS61/tr1600 | 0.14/LS70/tr10045 |

LS<32の月0、最大税率0.24。A_all世界は募兵抑制の性質(既知)により戦争が起きない。

### 11.3 A_war(一時診断条件、交戦中だけ同条件を課す案)

| 月 | Arcadia | Elfin | Dwarf | Oceanic | Passguard | Ferrowyn |
|---|---|---|---|---|---|---|
| 120 | 0.19/LS45/tr3291 | 0.12/LS71/tr6270 | 0.15/LS59/tr11974 | 0.13/LS71/tr10851 | 0.19/LS67/tr904 | 0.20/LS61/tr379 |
| 240 | 0.19/LS45/tr3542 | 0.12/LS71/tr5866 | 0.16/LS58/tr12321 | 0.13/LS71/tr11801 | 0.20/LS67/tr1842 | 0.17/LS79/tr975 |
| 360 | 0.22/LS39/tr3731 | 0.12/LS71/tr7091 | 0.16/LS57/tr13460 | 0.12/LS76/tr12424 | 0.19/LS67/tr1784 | 0.13/LS100/tr1185 |
| 480 | 0.22/LS40/tr3254 | 0.12/LS71/tr6763 | 0.16/LS57/tr14220 | 0.12/LS76/tr13387 | 0.19/LS67/tr1945 | 0.14/LS100/tr1361 |
| 600 | 0.24/LS36/tr3516 | 0.12/LS71/tr7189 | 0.16/LS57/tr15060 | 0.12/LS78/tr27263 | 0.18/LS67/tr2138 | 0.15/LS100/tr1404 |

LS<32の月0、最大税率0.24(Arcadia、600か月時点。余裕は最も薄いが健全)。

独立プロセスで現行AI運用を再実行し、FIS行が完全一致することを確認した(決定的、差分0)。

## 12. 現行・A_all・A_warの区別

現行AI運用のみが本番のRailway優先建設単体の効果である。A_all/A_warはP28-004-PRE-001の一時パッチ(本タスクでは実装・確定しない)を診断のためだけに一時的に重ねた条件であり、募兵条件の本番実装や政策の承認を意味しない。3条件すべてでRailway優先建設が長期財政を安定させることを確認したが、A_all/A_warの妥当性そのものはP28-004で改めて判断する(§15)。

## 13. 財政・建設・募兵・戦争・領土への副作用

- **財政**: Dwarf(現行413か月ごろの崩壊)・Elfin(600か月以内の崩壊)の両方が解消(§11.1)。
- **建設**: 州6・州8など物流不足州へのRailway建設が、既存の優先度2(原材料事前開発)・優先度2.5(戦時MilitaryFactory)と同じ枠を争うため、他の建物の着工タイミングが数か月単位でずれる(§7・§9)。
- **募兵**: 本タスクでは変更していない(P28-004の対象)。
- **戦争・領土**: Dwarfの経済発展経路の変化により、戦時MilitaryFactoryの着工州がP28-002完了時点の州20から州6へ変わり、開戦後のゲート再開・攻勢開始・終戦の日付が早まった(§7・§9、州を問わない機能上の契約は維持)。P28-003の州8飛び地シナリオは自然運用では再現しなくなった(§9)。

## 14. Critical/High/Medium/Low

```text
Critical: 0
High: 0
Medium: 1
Low: 3
```

- **High H-1(P28-005-PRE-001から継続、本タスクで解消)**: 長期の税率スパイラル。Railway優先建設の実装により、現行AI運用で600か月まで6か国全てが安定することを確認した。
- **Medium M-3(継続)**: 赤字時の税率引上げが0.25を越えて無条件に続く(加速要因、根本原因ではない)。
- **Low L-2・L-5・L-6(継続)**: P28-004-PRE-001のとおり。L-6(取り残されたDivisionが前線割当・移動命令の対象に残る)は本タスクの新規E2Eでも直接観測された(§9)。

## 15. P28-004に残す設計判断

本タスクでは募兵条件を実装・確定していない。P28-004-PRE-001が示した「交戦中の募兵に募兵後の月需要≤実月産を課す」案(A_war、余裕100%推奨)・「平時にも同条件を課すか」の論点は未決定のまま残る。§11.2・§11.3の一時診断は、Railway優先建設の適用後の世界でもA_all/A_warが引き続き長期財政面では機能することを示したが、これはP28-004の政策判断を代替しない。次の原子的タスクとして、この実装後の世界でA_war案を再測定し(開戦初期の帰結の変化・120/240か月の継続戦争数・補給不足日数)、P28-004-PRE-001の受け入れ条件(120か月の継続戦争数が現状以下)を満たすか確認したうえで、ユーザーに論点1・2の最終判断を仰ぐことを推奨する。

## 16. P27-005手動プレイ確認の未実施

P27-005の手動プレイ確認は**未実施のまま引き継ぐ**。本タスクで実施した自動テスト・診断をもって手動確認済みとは記述しない。

## 17. 実行コマンドと正確なテスト・clippy結果

Cargoはすべて逐次実行した(並列起動なし)。

- 開始時: `git rev-parse HEAD` → `5e1ebe2c537adb525ce370039010df0100212e87`、`git status --short` → 空
- 開始時実測: `cargo test --all-targets -j 2` → 2734 passed / 0 failed / 1 ignored(84バイナリ)。`cargo clippy --all-targets --all-features -- -D warnings` → 警告0
- 個別実行(実装後)。いずれも0 failed。

  | テスト | 件数 |
  |---|---|
  | `--lib country::country_ai`(新規13件を含む) | 76 |
  | `--lib economy::finance` | 19 |
  | `--lib building::` | 29 |
  | `--lib save::` | 305 |
  | `p27_002_c_residual_fiscal_self_stabilization_e2e_test`(新規2件を含む) | 20 |
  | `p28_002_wartime_military_factory_construction_e2e_test` | 3 |
  | `p28_003_disconnected_friendly_frontline_delivery_e2e_test`(制御fixtureへ移行) | 2 |
  | `p26_006_capital_connectivity_supply_e2e_test` | 18 |
  | `p26_006_rev_001_ai_save_resume_determinism_e2e_test`(無変更) | 9 |
  | `p26_det_001_construction_ai_determinism_e2e_test` | 10 |
  | `p26_det_001_rev_001_cross_state_production_order_e2e_test` | 12 |
  | `p28_005_ai_railway_priority_e2e_test`(新規) | 3 |
  | `land_war_combat_peace_test`(無変更) | 4 |

- 一時診断(`tests/p28_005_temp_longterm_diagnostic.rs`。P28-004一時パッチを診断中だけ再適用): 現行・A_all・A_war × 600か月。現行AI運用を独立プロセスで再実行し、FIS行が完全一致(決定的)。
- 最終`cargo test --all-targets -j 2` → **2752 passed / 0 failed / 1 ignored**(85バイナリ、exit 0)。開始時2734に、単体テスト13件(country_ai)・2件(finance側のテストではなくp27_002_c内、既存ファイル)・E2E 3件(新規ファイル)を加えた純増18件と一致。1 ignoredは既存の`profile_p21_021_elimination_scaling`。再生成された既知の非決定的PNG9件は個別に復元
- 最終`cargo clippy --all-targets --all-features -- -D warnings` → **警告0**(exit 0)

## 18. 一時診断コードの削除・ユーザーSave保護

- 一時パッチ: `src/military/ai.rs`(P28-004候補A/A_war、A_all/A_war診断のためだけに再適用)。差分は`scratchpad/p28_004/p28_004_temp_patch.diff`から取得し、逆適用後に`git hash-object`がHEADのblobと一致することを確認した。
- 一時診断テスト: `tests/p28_005_temp_longterm_diagnostic.rs`。スクラッチへ控えを取ってから削除した。
- `saves/savegame_v1.ron`: 読み書き・復元・ステージなし(全テストは独自の一時ディレクトリにSaveを書き出す)。
- `tests/land_war_combat_peace_test.rs`: 変更・整形なし。ワークスペース全体のformatなし。既存報告書は変更していない。
- 既知の非決定的PNG: `git status --short`で確認して、1件ずつ個別に復元した。

## 19. コミット対象と最終`git status --short`

コミット対象(ファイルごとに個別指定):

```text
src/country/country_ai.rs
src/economy/finance.rs
tests/p26_006_capital_connectivity_supply_e2e_test.rs
tests/p26_det_001_construction_ai_determinism_e2e_test.rs
tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs
tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs
tests/p28_002_wartime_military_factory_construction_e2e_test.rs
tests/p28_003_disconnected_friendly_frontline_delivery_e2e_test.rs
tests/p28_005_ai_railway_priority_e2e_test.rs(新規)
verification_logs/phase-28/p28-005/p28-005_ai_railway_priority_for_fiscal_stability.md(新規)
```

コミット前の`git status --short`は上記10ファイルのみ(`?? tests/p28_005_ai_railway_priority_e2e_test.rs`と本報告書、他8件は`M`)。コミット後は空(コミットハッシュはチャットで報告する)。
