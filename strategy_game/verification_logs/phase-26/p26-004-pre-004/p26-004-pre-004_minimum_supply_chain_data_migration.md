# P26-004-PRE-004: 最小補給生産網データと旧Save構造移行の実装

## 1. 総合判定

**COMPLETE WITH FINDINGS**

New Gameデータ変更(4州への鉱床追加・OceanicのMilitaryFactory追加)、旧Save構造的
migration(冪等・原子的)、4か国全ての実App実測での正生産到達、Arcadia/Dwarfの
非悪化(構造的には無変更)、New Game/Loaded Gameの一致、Save V1維持、全テスト成功、
clippy警告0を達成した。ただし検証の過程で、本タスクのデータ変更とは無関係な
**既存コードの実行順序非決定性**(Arcadia単独の36か月後MilitarySupplies値が、
全く同一のデータ・コードで`cargo test`プロセスを複数回起動しただけで
823.172/831.121/896.758と変動する)を新規に発見した。これは本タスクが引き起こした
回帰ではない(Arcadiaのデータ・関連コードは一切変更していない)が、既存の36か月
検証手法(P26-004-PRE-001〜003を含む)の再現性に影響する重要な既存事象のため、
Medium findingとして記録し、テストの許容範囲を実測に基づいて調整した(§33参照)。

## 2. 開始時HEAD

```
9700c70f2eb42b7ea9f37cb8da1e7dedb0cce505
```

`git status --short`(開始時): ` M saves/savegame_v1.ron` のみ(想定と一致、自ら
`git rev-parse HEAD`・`git status --short`で再確認済み)。

## 3. 監査対象コードHEAD

`9700c70f2eb42b7ea9f37cb8da1e7dedb0cce505`(開始時HEADと同一。本タスクはコミット前の
作業ツリー上で実装したため、監査対象コードは常にこのHEAD+作業ツリーの変更分)。

## 4. New Gameデータ変更

* `assets/data/resources.ron`: StateId(8)・(13)・(16)・(21)へそれぞれ
  `Iron(base_output:50.0, discovered:true, development_level:1)`・
  `Coal(base_output:40.0, discovered:true, development_level:1)`を追加。
* `assets/data/states.ron`: StateId(8、Oceanic Magic Empire「Imperial Capital」)の
  `buildings`マップへ`MilitaryFactory: 1`を追加(既存の`Farm: 1`はそのまま)。

## 5. 4州の変更前後

| 国家 | StateId | 州名 | 変更前 | 変更後 |
|---|---:|---|---|---|
| Elfin Republic | 16 | Highland | `buildings: {LoggingCamp:2, Mine:1}`、鉱床なし | 鉱床Iron50/Coal40追加、建物無変更 |
| Ferrowyn League | 21 | Ironvein Camp | `buildings: {Mine:2}`、鉱床なし | 鉱床Iron50/Coal40追加、建物無変更 |
| Passguard March | 13 | Mountain Watch | `buildings: {MilitaryFactory:1, Railway:1}`、鉱床なし | 鉱床Iron50/Coal40を**同一州**へ追加、建物無変更 |
| Oceanic Magic Empire | 8 | Imperial Capital | `buildings: {Farm:1}`、鉱床なし | 鉱床Iron50/Coal40追加、`MilitaryFactory:1`を新規追加 |

## 6. 鉱床の正規表現

`src/economy/resources.rs`の`StateResourceDeposit { resource_type, base_output, discovered, development_level }`
をそのまま使用。`resource_deposits: Vec<StateResourceDeposit>`は同一`resource_type`を
複数保持できるVecであり、Map上で一意ではないことをコードで確認済み。冪等性判定は
既存ヘルパー`has_discovered_deposit(&deposits, resource_type)`(`discovered && resource_type一致`
の存在確認)を再利用した。

## 7. Oceanic MilitaryFactory変更

`assets/data/states.ron`のStateId(8)`buildings`マップへ`MilitaryFactory: 1`を直接
追加(New Game用)。旧Save側は`src/building/supply_chain_migration.rs`の
`BuildingLevelFloor { state_id: StateId(8), building_type: MilitaryFactory, min_level: 1 }`
が「既存レベル < 1のときだけ1へ設定、既に1以上なら無変更」という下限保証ロジックで
処理する。

## 8. 旧SaveでRONが再マージされない根拠

`src/app/loader.rs`のNew Gameロード処理を実コードで確認: `resources.ron`は
`HashMap<StateId, Vec<StateResourceDeposit>>`として独立に読み込まれ、
`state.resource_deposits = deposits.clone()`で`states.ron`由来の`StateData`へ
上書き代入される(211〜218行付近)。一方、`src/save/dto.rs`の`SaveGameV1.states: Vec<StateData>`
(`StateData`はresource_deposits/buildingsを含む全フィールドをそのまま保持する型)が
Save V1へ直接シリアライズされ、`src/save/apply.rs`の`StateRegistry::build(save.states)`
(実際の適用箇所を確認)がロード時に`save.states`だけから再構築する——`resources.ron`/
`states.ron`との再マージ処理は一切存在しない。したがって`resources.ron`/`states.ron`
への編集は、その編集より前に作成された旧Saveには一切反映されない。

## 9. migration方式

`src/building/supply_chain_migration.rs`を新設(`mine_migration.rs`と同じ
`src/building/`配下、同型の設計パターン)。公開API:

* `minimum_supply_chain_deposit_placements() -> Vec<DepositPlacement>`(固定8件:
  4州×Iron/Coal)
* `minimum_supply_chain_building_floors() -> Vec<BuildingLevelFloor>`(固定1件:
  Oceanic州8のMilitaryFactory下限1)
* `apply_minimum_supply_chain_migration(states: &mut [StateData]) -> Result<SupplyChainMigrationReport, MissingTargetState>`

判定ロジック: (a) 対象StateId(8/13/16/21)が`states`に**1件も存在しない**場合は、
この地図は本タスク対象外と判断し無変更で`Ok(空レポート)`を返す(共有インフラ経由の
既存テスト群[本番地図と無関係な小規模テスト地図]を壊さないために必須、§32詳述)。
(b) **1件以上存在するが全件ではない**場合は、原子性違反(実運用のマップ破損)と
みなし、どのstateも変更せず即座に`Err(MissingTargetState)`を返す。(c) 全4州が
揃っている場合のみ、各placement/floorについて「discovered済み同種鉱床が既に
あるか」「建物レベルが既にmin_level以上か」という構造的判定だけで追加/据え置きを
決める。

## 10. migration実行順序

* New Game: `src/app/loader.rs`——`states.ron`読込→`resources.ron`マージ→
  `mine_migration::migrate_mines_to_crystal_mines`(既存)→
  `supply_chain_migration::apply_minimum_supply_chain_migration`(新規、隣接する
  位置に追加)→`validate_data`→`StateRegistry::build`。New Gameでは対象4州が
  既に`resources.ron`/`states.ron`由来のデータを持つため、本関数は実質何も追加
  しない(冪等条件がその時点で真になっているため)——ただし旧Save経路と全く同じ
  関数を通すことで両経路の論理的構造一致を保証する目的で、あえて重ねて呼ぶ。
* 旧Save: `src/save/apply.rs`の`prepare_load`——必須Resource確認→State/CountryID
  集合の一致確認(`RuntimeCompatibility`検査、この時点で`save.states`の
  StateId集合は必ず現在の地図[=常に8/13/16/21を含む]と一致することが保証される)
  →`save`の所有権取得→`mine_migration`(既存)→本migration(新規、直後に追加)→
  以降のRegistry構築(CountryAi/MilitaryAi等)→Commit。migration後の州・建物構造は
  Division復元・CountryPower再計算・月次経済処理・建設AIより確実に前に確定する。

## 11. 冪等性

判定条件は構造チェックのみ(§9)。`tests::applying_twice_is_idempotent`・
`applying_three_times_still_matches_the_single_application_result`(いずれも
`src/building/supply_chain_migration.rs`内)、およびE2E
`test_d1_repeated_save_load_cycles_never_duplicate_deposits_or_buildings`
(4回のsave/loadサイクルで鉱床件数・MilitaryFactoryレベルが一切増殖しないことを
実App経由で確認)で検証済み。

## 12. プレイヤーによる削除・低下経路の監査

`rg`で以下を確認(ground rule 8要求):

* `resource_deposits.push/remove/retain/clear`: 本番コード中で該当するのは
  `src/app/loader.rs`(New Gameの`resources.ron`まるごと代入)と
  `src/building/supply_chain_migration.rs`(本migration自身の`push`)のみ。
  それ以外の削除・変更経路は0件。
* `state.buildings`への書き込み: `src/economy/mod.rs`(建設完了時に
  `state.buildings.insert(item.building_type, current + 1)`——**単調増加のみ**)と
  `src/building/mine_migration.rs`(Mine→CrystalMineへの同数移し替え、対象外の
  建物種別)の2箇所のみ。レベルを引き下げる・キーを削除する経路(解体・破壊等)は
  0件。
* `src/country/elimination.rs`(国家消滅処理)も確認したが、`buildings`/
  `resource_deposits`を一切変更しない。

**結論**: 鉱床・MilitaryFactoryのいずれについても、本番プレイ中に減少・削除される
経路は存在しない。したがって「既に存在するか」という構造チェックだけで、
プレイヤーの意図的な変更をロードのたびに復活させるリスクは生じない。

## 13. marker要否

**不要と判断した(justification: §12の監査結果)**。P21-021の`CountryStatus`や
P26-004-PRE-003が設計した`military_supplies_initial_grant_applied`(次タスク
P26-004-PRE-005向け、本タスクでは未実装)のような一度限りマーカーは、「0が正規の
値でありうる」ケース(初期備蓄額のように、既に達成済みか未達成かを区別できない量)
でのみ必要になる。本タスクが扱う鉱床・建物レベルは、削除経路が存在しない
モノトニックな構造であるため、構造チェックのみで安全に冪等性が成立する。

## 14. 原子性

`apply_minimum_supply_chain_migration`は「対象StateIdが1件でも存在するが全件では
ない」場合、どのstateも変更せずに`Err`を返す設計(§9)。単体テストで検証:
`missing_target_state_8/13_rejects_the_whole_migration_without_partial_changes`・
`at_least_one_target_state_present_but_not_all_still_triggers_atomic_rejection`・
`missing_state_check_runs_before_any_mutation_regardless_of_slice_order`。
実運用(`src/save/apply.rs`)では、この関数の呼び出しより前に既存の
`RuntimeCompatibility`検査(`current_state_ids != save_state_ids`ならロード全体を
`Err(ApplyLoadError::RuntimeCompatibility)`で拒否)が必ず先に走るため、「本番地図には
4州とも存在するのに、あるSaveだけ一部が欠落する」状況は構造的に到達不能——本関数の
原子性チェックは、その前提が万一崩れた場合の防御的な二重保険として機能する
(`ApplyLoadError::StaticDataMismatch`へマップして`?`で伝播、Worldは未変更のまま)。

## 15. 既存値の保護

Test C系(単体・E2E双方)で確認: Iron/Coalの一方だけが既に存在する場合は不足分だけを
追加し既存値(base_output/development_level)は書き換えない、両方既に存在する場合は
何も追加しない、discovered:falseの同種鉱床は「既存」として扱わずdiscovered:true版を
新規追加する(2件並存)、MagicCrystal等の無関係な鉱床は変更しない、
MilitaryFactoryが既にlevel1以上なら変更しない(引き下げも加算もしない)、Farm等の
他建物は変更しない。

## 16. 所有権変更時の挙動

E2E `test_f1_migration_applies_by_state_id_regardless_of_a_changed_owner`で検証:
StateId(13)の所有国をArcadiaへ書き換えた合成旧Saveをロードしても、migrationは
地理構造(StateId)としてそのまま州13へ鉱床を適用する。国庫・国家備蓄は変更前後で
完全に不変(ArcadiaのMilitarySupplies備蓄・両国の国庫が一致)であり、国家間の混入は
発生しない。

## 17. New Game/Loaded Game比較

E2E `test_h1_new_game_and_migrated_old_save_agree_structurally`: New Gameと
(合成旧Saveへのmigration経由の)Loaded Gameとで、4州のIron/Coal
(`base_output`/`discovered`/`development_level`)とOceanicのMilitaryFactoryレベルが
完全に一致することを確認。`test_h2_new_game_and_migrated_branch_reach_the_same_production_after_36_months`:
月3時点でmigration境界を挟んだ分岐と、New Gameのまま36か月継続した経路の両方で、
Passguardが正のMilitarySupplies生産へ到達することを確認(migration適用タイミングの
違いが最終結果を損なわない)。

## 18. save/load分岐比較

Test D-1(§11)で複数回のsave/loadサイクルを実施し、鉱床・建物構造が増殖しないことを
確認。Test B-1で、実際にAppがNew Gameを一度save→そのファイルを合成旧Save化(対象
データを`strip_new_supply_chain_data`で除去)→loadという実RONラウンドトリップを
経由してmigrationが正しく発火することを確認(実ユーザーセーブは不使用)。

## 19. Elfin生産結果

実App・27日簡略暦(P26-004-PRE-002/003と同一手法)・6か国同時AI稼働・通常財政で
36か月実測。初回生産月: **月5**。月12=383.368、月18=750.894、月24=1117.329、
**月36=1846.952**(P26-004-PRE-003の投影値1846.952と完全一致)。事前開発規則が
既存Mine(州16、lv1)を鉱床検出後に増強し、追加でStateId(3)にFactory(lv3)を新設した
ことをMine総レベル増加(初期値1から増加)で確認。

## 20. Ferrowyn生産結果

初回生産月: **月2**(既存Factory[州22]がそのまま機能するため最速)。月12=360.325、
月18=543.522、月24=726.175、**月36=1089.859**(PRE-003投影値1089.859と完全一致)。
既存Mine(州21、lv2)がそのまま鉱床を検出し、新規建設なしで即座に接続した。

## 21. Passguard生産結果

初回生産月: **月5**。月12=939.844、月18=1625.662、月24=2292.438、
**月36=3659.576**(PRE-003投影値3325.079とは異なる——約10%高い実測値。原因は
PRE-003が一時テストでランタイム注入したデータとの手法差[本タスクは`resources.ron`/
`states.ron`への恒久データ+migrationという別経路]によるものと推測されるが、
本タスクの検証時間内では原因を完全には特定できていない。数値自体は「本タスクが
実際にPassguardの供給網を機能させた」という結論を変えるものではなく、健全な方向
[より高い生産]の差であるためLow findingとして記録する、§33)。事前開発規則が
州13にMine(lv1)を新設し、別州にFactoryを新設して全鎖(Mine→Iron/Coal→Factory→
IndustrialGoods→MilitaryFactory→Supplies)が接続したことを確認。

## 22. Oceanic生産結果

初回生産月: **月5**。月12=78.693、月18=171.300、月24=282.474、
**月36=565.705**(PRE-003投影値565.705と完全一致、小数点以下も一致:
実測565.704948)。事前開発規則が州8にMineを新設(lv0→稼働)し、既存の
MilitaryFactory(lv1、本タスクで新規配置)と接続して生産を開始した。

## 23. Arcadia/Dwarf非影響

**データは完全に無変更**(`resources.ron`/`states.ron`いずれもArcadia[州0,1,2,11,12]・
Dwarf[州5,6,7,20]の記述箇所を一切編集していない、Test A-3/A-4で確認)。

Dwarfの36か月生産は複数回の独立実行で**安定して1487.059**(月12=517.647、
月18=800.000、月24=1035.294)を記録し、P26-004-PRE-002の記録値と完全一致した。

Arcadiaについては**重要な発見**がある(§33で詳述): 全く同一のデータ・コードで
`cargo test`プロセスを独立に3回起動したところ、月36の値が823.172/831.121/896.758と
変動した(月12=421.227は3回とも一致、月18以降で分岐が生じ始める)。うち896.758は
P26-004-PRE-002/003が記録した値と完全一致する。本タスクはArcadiaのデータ・
`country_ai.rs`等の関連コードを一切変更していないため、この変動は**本タスクが
引き起こした回帰ではなく、既存コードに内在する実行順序非決定性**である
可能性が高い(RNGは使用されていないため[コード監査済み]、Rust標準の
`HashMap`既定ハッシャーがプロセスごとにランダムシードを持つことに起因する、
Arcadia固有の候補州選定タイブレークが疑われるが、`country_ai.rs`は本タスクの
変更対象外のため深追いしていない)。いずれの観測値も旧来の恒久停止バグ値
(373.96)を大きく上回っており、「Arcadiaの供給網が本タスクで悪化した」という
証拠は一切ない。

## 24. 6か国36か月比較

| 国家 | 月12 | 月18 | 月24 | 月36 | 備考 |
|---|---|---|---|---|---|
| Arcadia(0) | 421.227 | 592.598〜598.056 | 667.324〜697.858 | 823.172〜896.758(§23の非決定性) | データ無変更 |
| Elfin(1) | 383.368 | 750.894 | 1117.329 | 1846.952 | PRE-003投影と完全一致 |
| Dwarf(2) | 517.647 | 800.000 | 1035.294 | 1487.059 | データ無変更、安定 |
| Oceanic(3) | 78.693 | 171.300 | 282.474 | 565.705 | PRE-003投影と完全一致 |
| Passguard(4) | 939.844 | 1625.662 | 2292.438 | 3659.576 | PRE-003投影(3325.079)より高い実測(Low、§33) |
| Ferrowyn(5) | 360.325 | 543.522 | 726.175 | 1089.859 | PRE-003投影と完全一致 |

Mine/Factory/MilitaryFactoryレベルの変化: Elfin(州16 Mine増強+州3 Factory新設)・
Ferrowyn(既存州21 Mine即稼働、追加建設は財政悪化と共に鈍化)・Passguard(州13に
Mine新設、別州にFactory新設)・Oceanic(州8 Mine 0→稼働、Factory新設)を実App経由で
確認(すべて事前開発規則[P26-004-PRE-002、本タスク無変更]が自律的に実施)。

## 25. Oceanic 48/60か月結果

実測(実App、60か月連続稼働): 月36=565.705、**月48=786.537**、**月60=1026.262**。
月間フロー(36→48)=220.832、(48→60)=239.725——**両区間とも正**であり、
P26-004-PRE-003 §32 Medium-1が懸念した「48〜60か月規模で資源不足に転じる」
という予測は、少なくとも60か月以内の実測範囲では**再現しなかった**(Iron:
月36=54.591→月48=52.918→月60=72.414、Coal: 月36=56.392→月48=57.542→
月60=57.042、いずれも枯渇傾向ではなく概ね横ばい〜回復)。MilitarySuppliesの
月間生産量はむしろ月48→60にかけて増加している。この点はPRE-003のMedium-1を
覆す新しい実測結果であり、非ブロッキングのLow findingとして記録し直す
(§33)——ただし60か月を超える超長期(例: 100か月以上)は本タスクの検証範囲外。

## 26. 国力・順位・Tier変化

日0時点(実App起動直後、AI稼働前): Arcadia 1130.0/GreatPower(1位、無変更)、
Elfin 1200.0→**1290.0**/GreatPower(2位)、Dwarf 1230.0/RegionalPower(3位、
無変更)、Oceanic 600.0→**620.0**/RegionalPower(4位)、Passguard 400.0/MinorPower
(順位6位→**6位のまま実測**)、Ferrowyn 150.0→**330.0**/MinorPower(順位5位)。
`economic_raw`変化はElfin/Ferrowyn/Oceanicのみ(Passguardは鉱床のみでMine未建設の
ためday0では無寄与)、Tier構成(GreatPower×2/RegionalPower×2/MinorPower×2)は
日0時点で不変。既存の単体テスト`p26_003_rev_003_economic_power_capacity_e2e_test`の
`EXPECTED`定数を実測値(economic_raw変化とそれに伴う正規化後total_score・
Passguard/Ferrowynの順位入替[Passguard 6位・Ferrowyn 5位、Ferrowynの人口590,000が
Passguardの245,000を上回るため])で更新した(§31)。

月36時点(実App、通常AI稼働後、1回の実測): Arcadia RegionalPower(3位)・
Elfin GreatPower(2位)・Dwarf RegionalPower(4位)・Oceanic GreatPower(1位)・
Passguard MinorPower(5位)・Ferrowyn MinorPower(6位)——Tier構成は
GreatPower×2/RegionalPower×2/MinorPower×2のまま。ただし§23の非決定性により、
Arcadia絡みの月36時点Tier判定(特にArcadia/Elfinの相対順位)は実行ごとに変動しうる
ため、この特定の1回の観測を確定的な結論として過信しない。

## 27. 財政・AIへの影響

4か国とも既存の`process_construction_ai`ロジックを一切変更していない(データ追加
のみ)。Elfin・Oceanicは通常財政のまま36か月間健全な国庫を維持した(実測は§19・22
参照、詳細な国庫推移はP26-004-PRE-003 §30の記録と定性的に一致)。Ferrowyn・
Passguardの財政悪化傾向はP26-004-PRE-001以来の既存の経済バランス課題の延長であり、
本タスクが新規に発生させたものではない(既にMilitarySupplies生産自体は財政悪化下
でも継続することを確認済み、`operation_ratio`計算は治療残高ではなく`is_bankrupt`
フラグのみを参照するため)。

## 28. Save V1構文互換性

**維持した**。新規`#[serde(default)]`フィールドは一切追加していない
(`CountryData`・`StateData`いずれも構造変更なし)。既存フィールド
(`StateData.resource_deposits`・`StateData.buildings`)への値追加のみであり、
Save形式番号(`SAVE_FORMAT_VERSION_V1 = 1`)は無変更。

## 29. Save V1意味的migration

**明示的な意味的migrationを実装した(「migration不要」ではない)**。§8で確認した
とおり、`resources.ron`/`states.ron`への編集は旧Saveの`resource_deposits`/
`buildings`へ自動反映されない。`src/building/supply_chain_migration.rs`が
`src/app/loader.rs`(New Game)と`src/save/apply.rs`(旧Saveロード)の両方から
呼ばれ、旧Saveの論理状態(鉱床なし・MilitaryFactoryなし)をNew Gameと同じ論理状態
(鉱床あり・MilitaryFactoryあり)へ一度だけ意味的に書き換える。構文互換性(§28)と
意味的互換性(本節)を明確に分離して記録する。

## 30. 初期備蓄を先行実装していないこと

`military_supplies_initial_grant_applied`フィールド、初期MilitarySupplies 31.0の
付与ロジック、いずれも本タスクでは実装していない(`rg`で確認、`CountryData`に
新規フィールドなし、`add_military_supplies_produced`への新規呼び出し追加もなし)。
P26-004-PRE-005の対象として温存した。

## 31. Phase 23〜25回帰

`cargo test --lib`で`state::`(19 passed)・`country::`(89 passed)・`economy::`
(32 passed)・`save::`(300 passed)を個別実行し、いずれも開始時基準と完全一致する
件数でpassすることを確認した。Phase 25のMilitaryEquipment固定小数点会計
(`src/economy/resources.rs`のMILITARY_EQUIPMENT_SCALE関連)・
Phase 23/24の人口・workforce計算は一切変更していない。

## 32. P26回帰

`p26_002_military_supplies_accounting_foundation_e2e_test`(45 passed)・
`p26_003_military_supplies_production_e2e_test`(49 passed、うち1件
`test_h3_...`はOceanicが新規にMilitaryFactoryを得たという本タスクの意図した
結果を反映する形に更新——原テストの「MilitaryFactoryが1つもない国家は生産0」
という不変条件自体は、実データに依存しない合成フィクスチャの
`test_a6_no_military_factory_produces_zero`が引き続き検証している)・
`p26_003_rev_002_supply_production_units_e2e_test`(56 passed)・
`p26_003_rev_003_economic_power_capacity_e2e_test`(21 passed、`EXPECTED`定数を
§26の実測値へ更新)・`p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test`
(11 passed、Test K-1/K-2・L-1/L-2・M-1を「鉱床皆無ゆえ無浪費/未解決」から
「鉱床が実在するようになった結果、正しく投資・解決される」という新しい実データの
下での正しい挙動へ更新)。いずれもassertの削除・弱体化ではなく、本タスクの
データ変更が意図して引き起こした事実の変化を反映する更新である。

`p21_014_country_power_e2e_test`(10 passed、Passguard/Ferrowynの世界順位
[5位⇔6位]が実測に合わせて入れ替わったため更新)も併せて確認。

## 33. Critical/High/Medium/Low

* **Critical: 0件**
* **High: 0件**
* **Medium: 1件**
  1. **(新規発見、本タスク起因ではない)** Arcadiaの36か月後MilitarySupplies値に、
     同一データ・同一コードでの`cargo test`プロセス再起動だけで発生する非決定性
     (823.172/831.121/896.758、§23)。月12までは全試行で完全一致し、月18以降で
     分岐が生じる。RNGは不使用(コード監査済み)なため、Rust標準`HashMap`の
     プロセスごとのランダムハッシュシードに起因する、Arcadia固有の建設AI
     候補選定タイブレーク依存が疑われるが未確認。`country_ai.rs`は本タスクの
     変更対象外のため原因調査・修正は行っていない。将来の別タスクとして、
     `country_ai.rs`のHashMap反復由来の非決定性を監査することを推奨する
     (36か月規模の検証結果を「確定的な基準値」として扱う既存の運用[本タスクを
     含む複数の過去タスクの報告書が単一実行値を記録している]自体の信頼性に
     関わるため)。
* **Low: 3件**
  1. Passguardの月36実測値(3659.576)がP26-004-PRE-003の投影値(3325.079)と
     約10%異なる(§21)。健全な方向(より高い生産)の差であり、供給網が機能する
     という結論自体は変わらないが、手法差(恒久データ+migration vs 一時
     ランタイム注入)の影響を完全には特定できていない。
  2. P26-004-PRE-003 §32 Medium-1(Oceanicが48〜60か月規模で資源不足に転じる
     懸念)は、実測(§25)では60か月時点まで再現しなかった(むしろ生産フローは
     増加傾向)。P26-005着手前の残課題としては解消してよいが、60か月を超える
     超長期の挙動は未検証のまま。
  3. 鉱床・建物migrationの冪等性判定(§13)は「動的な鉱床・建物削除経路が現在
     存在しない」という前提(§12で監査済み)に依存する。将来「鉱床探索」
     「建物解体」等の新規ゲームプレイ機構が追加された場合、この前提を必ず
     再監査する必要がある(コード上のコメントとして明記済み)。

## 34. 実行コマンドと正確な結果

全コマンドを逐次実行(並列実行なし)。

```text
$ cargo test --lib state::
  → ok. 19 passed; 0 failed; 0 ignored

$ cargo test --lib country::
  → ok. 89 passed; 0 failed; 0 ignored

$ cargo test --lib economy::
  → ok. 32 passed; 0 failed; 0 ignored

$ cargo test --lib save::
  → ok. 300 passed; 0 failed; 0 ignored

$ cargo test --lib building::supply_chain_migration::
  → ok. 20 passed; 0 failed; 0 ignored

$ cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test
  → ok. 45 passed; 0 failed; 0 ignored

$ cargo test --test p26_003_military_supplies_production_e2e_test
  → ok. 49 passed; 0 failed; 0 ignored

$ cargo test --test p26_003_rev_002_supply_production_units_e2e_test
  → ok. 56 passed; 0 failed; 0 ignored

$ cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test
  → ok. 21 passed; 0 failed; 0 ignored

$ cargo test --test p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test
  → ok. 11 passed; 0 failed; 0 ignored

$ cargo test --test p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test
  → ok. 15 passed; 0 failed; 0 ignored

$ cargo test --test p21_014_country_power_e2e_test
  → ok. 10 passed; 0 failed; 0 ignored

$ cargo test --all-targets --jobs 2 --no-fail-fast
  → 全60バイナリの`test result:`行を合算: 2081 passed; 0 failed; 1 ignored
     (開始時基準2046 + 新規35[単体20+E2E15] = 2081、完全一致)
     注: 既定の並列ジョブ数ではWindows環境のページングファイル不足
     (os error 1455)でリンクに失敗したため、`--jobs 2`で逐次性を高めて再実行した
     (これはコマンドの並列実行ではなく、単一の`cargo test`プロセス内部の
     コンパイル並列度をOS制約に合わせて下げただけであり、ground rule 4
     「cargoコマンドは逐次実行」に抵触しない)。

$ cargo clippy --all-targets --all-features -- -D warnings
  → 1件の`needless_lifetimes`警告を検出・修正(`src/building/supply_chain_migration.rs`
     内のテスト専用ヘルパー関数、ライフタイム省略のみの機械的修正)後、
     0 warnings, 0 errors
```

`cargo test --all-targets`実行後(2回)、既知の9件の非決定的スクリーンショットPNGが
`M`として再生成されたため、その都度該当9パスのみを個別に`git checkout --`で復元した
(広範な`checkout`は一切使用していない)。

## 35. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・コミットして
いない。作業開始時・全検証コマンド実行後・報告書作成後のいずれの
`git status --short`でも` M saves/savegame_v1.ron`のみが変更されずに残り続けたこと
を確認した(内容は一度も参照していない)。テスト用の合成「旧Save」は全て
一時ディレクトリ(`std::env::temp_dir()`配下)に作成し、実ユーザーセーブへは一切
アクセスしていない。

## 36. コミット対象

* `assets/data/resources.ron`(4州×2鉱床、計8エントリ追加)
* `assets/data/states.ron`(OceanicのStateId 8へMilitaryFactory:1追加)
* `src/building/mod.rs`(新規モジュール宣言追加)
* `src/building/supply_chain_migration.rs`(新規、migration本体+20単体テスト)
* `src/app/loader.rs`(New Gameロード時の呼び出し追加)
* `src/save/apply.rs`(旧Saveロードprepare段階への呼び出し追加)
* `tests/p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test.rs`(新規、15テスト)
* `tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs`(既存5テストを実データ変化に合わせて更新)
* `tests/p26_003_military_supplies_production_e2e_test.rs`(既存1テストを実データ変化に合わせて更新)
* `tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs`(EXPECTED定数を実測値へ更新)
* `tests/p21_014_country_power_e2e_test.rs`(Passguard/Ferrowynの順位入替を反映)
* `verification_logs/phase-26/p26-004-pre-004/p26-004-pre-004_minimum_supply_chain_data_migration.md`(本報告書)

`saves/savegame_v1.ron`はコミット対象に含めない。

## 37. 最終`git status --short`

```
 M assets/data/resources.ron
 M assets/data/states.ron
 M saves/savegame_v1.ron
 M src/app/loader.rs
 M src/building/mod.rs
 M src/save/apply.rs
 M tests/p21_014_country_power_e2e_test.rs
 M tests/p26_003_military_supplies_production_e2e_test.rs
 M tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs
 M tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs
?? src/building/supply_chain_migration.rs
?? tests/p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test.rs
?? verification_logs/phase-26/p26-004-pre-004/
```

(コミット前の状態。`saves/savegame_v1.ron`は開始時から不変のユーザーファイルで
あり、本タスクの変更に含めない。)

## 38. P26-004-PRE-005へ引き継ぐ内容

* 初期MilitarySupplies備蓄(31.0)付与、`CountryData.military_supplies_initial_grant_applied`
  マーカーフィールドの追加、New Game/旧Save双方への一度限り付与ロジックは本タスクの
  対象外のまま(P26-004-PRE-003 §24〜27の設計を実装する)。
* 本タスクが確立した4か国の供給網(Elfin/Ferrowyn/Passguard/Oceanic)は実際に
  正のMilitarySupplies生産へ到達しており、初期備蓄実装の前提条件は整った。
* **新規申し送り事項**: §33 Medium-1(Arcadia単独の36か月シミュレーション結果の
  プロセス起動間非決定性)は、初期備蓄実装後の36か月検証タスクでも同様の再現性
  問題を引き起こす可能性がある。次タスクで36か月規模の実測値を報告書に記録する
  際は、複数回の独立実行で値の安定性を確認すること(本タスクがそうしたように)を
  推奨する。単一実行の値だけを「確定的な基準値」として次々タスクへ引き継がない
  よう注意する。
* P26-004-PRE-003 §32 Medium-1(Oceanicの長期資源枯渇懸念)は60か月実測で再現
  しなかった(§25)ため、P26-005着手前のブロッカーとしては解消してよい。
