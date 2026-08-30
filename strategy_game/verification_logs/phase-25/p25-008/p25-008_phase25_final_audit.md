# P25-008: Phase 25装備ライフサイクル最終監査 報告書

## 1. 総合監査判定

**AUDIT COMPLETE, PHASE 25 READY**

## 2. Phase 25固定可否

固定可。装備の生産(P25-002-REV-001)・募兵支給(P25-003)・戦闘損失(P25-004)・
日次補充(P25-005)・復員返却(P25-006)・強制削除/国家消滅時破壊
(P25-006-REV-001)・戦闘力反映(P25-007)を一続きのライフサイクルとして、
本タスクは実コード・実関数・既存テスト・新規一時検証テストにより最終監査した。
Critical/High findingは0件、装備会計の中核に関係するMedium findingも0件で
あった。§4のREADY判定基準13項目をすべて満たすことを確認した。

## 3. 開始時HEAD

```text
0adcfcc54512d288b7ef3ee19e5eb9ac1bbf4171
```

開始直後に`git rev-parse HEAD`・`git status --short`(` M saves/savegame_v1.ron`
のみ)・`git log --oneline --decorate -15`を実行し、想定と完全に一致することを
確認した。

## 4. 監査対象コードHEAD

```text
0adcfcc54512d288b7ef3ee19e5eb9ac1bbf4171
```

本タスクは読み取り専用の最終監査であり、本番Rustコード・既存テストコードは
一切変更していないため、監査対象コードHEADは開始時HEADと同一のまま。

## 5. ユーザーセーブの保護

`saves/savegame_v1.ron`は監査全工程を通じて一切読み書き・復元・ステージ・
コミットしていない。作業開始時・全カテゴリのテスト実行後・コミット直前の
いずれの`git status --short`でも` M saves/savegame_v1.ron`のみが(または
一時検証ファイル追加時のみそれに加えて`??`が)残り続けたことを確認した。
このファイルの内容は一度も読み込んでいない。

## 6. P25-001〜P25-007の最終的な扱い

| タスク | 最終的な扱い |
| --- | --- |
| P25-001 | 設計確定。国家消滅時「Division装備凍結」の前提はP25-001自身の設計段階での見落としであり、P25-006-REV-001で「破壊してからDivision削除」へ正式に訂正済み(履歴として保持) |
| P25-002 | COMPLETE WITH CORRECTION REQUIRED(直接`f64`加減算による`2^53`付近の装備消失リスクを内包していた) |
| P25-002-REV-001 | fixed-point(scale 1,000,000、`u128` checked演算)装備会計へ訂正済み。実コード確認済み |
| P25-003 | 募兵完成時の装備支給(`try_issue_equipment`経由)を実装済み。実コード確認済み |
| P25-004 | 通常戦闘損耗による装備破壊(`calculate_proportional_equipment_loss`+`try_destroy_equipment`)を実装済み。実コード確認済み |
| P25-005 | 日次装備補充(`apply_daily_equipment_reinforcement`)を実装済み。実コード確認済み |
| P25-006 | 生存復員時の装備返却(`try_return_equipment`)を実装済み。実コード確認済み。国家消滅・撤退強制削除のHigh 2件は本タスク範囲外として記録(直後のP25-006-REV-001で解消) |
| P25-006-REV-001 | 撤退・強制削除・国家消滅時の装備破壊を訂正済み。`NaN > 0.0`ガードによる検証スキップの実バグも同一タスク内で発見・修正済み。実コード確認済み(ガード除去を確認) |
| P25-007 | 装備充足率(`min(manpower比率, equipment比率)`)の戦闘力反映を実装済み。実コード確認済み |
| P25-008 | 本タスク。Phase 25全体の最終監査、判定は本報告書のとおり |

旧報告書・旧コミットはいずれもrevert/amend/削除しておらず、履歴としてそのまま
保持されていることを`git log --oneline --stat -- verification_logs/phase-25`で
確認した。

## 7. 過去の訂正履歴

* P25-002 → P25-002-REV-001: `f64`直接演算の精度崩壊(`2^53`反例を実測)を
  fixed-point units化で解消。
* P25-006 → P25-006-REV-001: 国家消滅時のDivision装備が会計APIを経由せず
  物理削除される問題(High)、撤退追加manpower損失で非ゼロ装備Divisionが
  未処理削除され得る問題(High)の2件を解消。同タスク内で新規発見した
  `NaN > 0.0`ガードスキップの実装バグ(常時呼び出しへ修正)も含む。

いずれも既に解消済みの過去findingであり、本タスクでは現行findingとして
再計上していない(履歴としてのみ§6/本節に記載)。

## 8. 正規装備型とfixed-point仕様

実コード(`src/economy/resources.rs`)で確認した保存形式・正規仕様。

```text
保存値: CountryStockpile[MilitaryEquipment] = f64
        Division.equipment = f64
        Division.max_equipment = f64

正規units: MILITARY_EQUIPMENT_SCALE = 1_000_000
           units = round(value × scale)  (military_equipment_value_to_units)
           value = units / scale         (military_equipment_units_to_value)

正規最大units: MILITARY_EQUIPMENT_MAX_UNITS = 1,000,000,000,000,000 (10^15)
              (equipment値換算で10億)
```

`military_equipment_value_to_units`はfinite・非負・値域内・往復可能性
(`U(V(u))==u`)のすべてを検証し、いずれかに違反すると`Err`(`NonFinite`・
`Negative`・`OutOfRange`・`RoundTripFailed`)を返す。`try_issue_equipment`・
`try_return_equipment`・`try_destroy_equipment`・`calculate_proportional_
equipment_loss`・`calculate_combat_readiness`はいずれも内部演算を`u128`
`checked_add`/`checked_sub`/`checked_mul`のみで行い、生の`f64`加減算による
装備会計更新は本番経路に残っていないことを`rg`で確認した(唯一の例外は
`src/war/combat.rs`の死んだ経路、§33参照)。`2^53`境界問題は再現テスト
(`two_pow_53_counterexample_is_rejected_not_silently_lossy`等)により再導入
されていないことを確認した。失敗時は元の`f64`値・stockpileが一切変化しない
(原子性、§26参照)。

## 9. 全mutation経路の棚卸し

`rg`で`CountryStockpile[MilitaryEquipment]`・`Division.equipment`・
`Division.max_equipment`・`DivisionDefinition.required_equipment`を書き換える
全経路を洗い出した。

* `CountryStockpile[MilitaryEquipment]`: `try_issue_equipment`(消費)・
  `try_return_equipment`(加算)・`add_military_equipment_produced`(生産)の
  3経路のみ(いずれも`src/economy/resources.rs`/`src/military/equipment.rs`の
  正規API)。他に`stockpile.add`/`.set`/`.consume`を`MilitaryEquipment`へ
  直接適用する本番経路は0件。
* `Division.equipment`: `try_spawn_division`(募兵完成、`src/military/
  recruitment.rs`)・`apply_combat_casualty`(通常戦闘損失、`src/military/
  update.rs`)・`apply_equipment_reinforcement_for_country`(日次補充、
  `src/military/equipment_reinforcement.rs`)・`demobilize_division`(復員
  返却、`src/military/demobilization.rs`)・`resolve_participant_outcome`
  (撤退追加損失・強制削除全損、`src/military/update.rs`)・
  `apply_country_elimination`(国家消滅全損、`src/country/elimination.rs`)の
  6経路。いずれも正規API(`try_issue_equipment`/`try_destroy_equipment`/
  `try_return_equipment`)を経由する。
* `Division.max_equipment`: `try_spawn_division`(募兵生成時に1回だけ設定、
  `try_issue_equipment`の結果をそのまま使用)のみ。日次補充・戦闘損失・復員
  はいずれも読み取りのみで一切書き込まないことをコード上確認した
  (`equipment_reinforcement.rs`は`max_equipment`を参照のみ、`demobilization.rs`
  は`max_equipment`を返却量計算に一切使わない)。
* `DivisionDefinition.required_equipment`: `assets/data/divisions.ron`から
  起動時に読み込まれる静的データであり、実行時に書き換える経路は存在しない
  (Save対象外、§29)。

`required_equipment`をtreasury/moneyコストとして扱う旧仮実装は、
`request_recruitment`・`cancel_recruitment`・`evaluate_recruit_feasibility`・
`military::ai::decide_and_recruit_for_country`のいずれからも完全に削除済み
であることを`rg -n "req_treasury|InsufficientFunds|InsufficientTreasury|
SkippedInsufficientTreasury" src assets`で再確認した(0件)。`Food`等の他
`ResourceType`への副作用は`src/economy/production.rs`の`res ==
ResourceType::MilitaryEquipment`分岐が唯一の変更であり、他資源の生産式は
無変更のまま`CountryStockpile::add`(既存の生`f64`加算)を使い続けている。
`war::combat.rs`の死んだ旧戦闘経路(§33)は本タスクでも未接続のままである
ことを、`add_systems`登録の`rg`検索で再確認した。`src/save/`配下は
`try_issue_equipment`/`try_return_equipment`/`try_destroy_equipment`/
`military::equipment::`のいずれも呼び出しておらず(`rg`で0件)、Save/Loadが
支給・返却・破壊を再実行する会計遷移になっていないことを確認した。

## 10. 正規APIと呼び出し元

`src/military/equipment.rs`・`src/economy/resources.rs`で定義された以下の
正規APIすべてについて、実コード上の全呼び出し元を確認した。

```text
military_equipment_value_to_units / military_equipment_units_to_value
  (src/economy/resources.rs)
  → military::equipment全関数、combat_calc::calculate_combat_readiness、
    equipment_reinforcement.rs から共通利用。計算式の複製なし。

try_issue_equipment
  → recruitment.rs::try_spawn_division(募兵支給)
  → equipment_reinforcement.rs::apply_equipment_reinforcement_for_country
    (日次補充)

try_return_equipment
  → demobilization.rs::demobilize_division(復員返却)

try_destroy_equipment
  → update.rs::apply_combat_casualty(通常戦闘損失)
  → update.rs::resolve_participant_outcome(撤退追加損失・強制削除全損、
    2箇所)
  → country/elimination.rs::apply_country_elimination(国家消滅全損)

equipment_shortfall / validate_equipment_state / validate_required_equipment
  → equipment_reinforcement.rs(不足量算出・国家単位preflight)、
    recruitment.rs(required_equipment有効性検証)、combat_calc.rs
    (readiness計算の内部検証)から利用

add_military_equipment_produced
  → economy/production.rs::process_country_production
    (MilitaryEquipment生産加算の唯一の経路)
```

類似計算・会計処理の複製は見つからなかった(損失式は`calculate_proportional_
equipment_loss`一箇所のみ、readiness式は`calculate_combat_readiness`一箇所
のみ、いずれも呼び出し元は結果を使うだけで式自体を再実装していない)。

## 11. System順序

`src/military/update.rs::handle_daily_military`(`DailySimulationSet::
MilitaryAction`に登録)の実際の呼び出し順序を確認した。

```text
1. process_recruitment (募兵完了、装備支給)
2. apply_daily_reinforcement (manpower日次補充)
2.5. apply_daily_equipment_reinforcement (equipment日次補充)
3. validate_and_stop_invalid_movements
4. process_movement
5. process_daily_battles (戦闘力計算[readiness反映]・通常戦闘casualty・装備損失)
6. resolve_finished_battles (撤退・強制削除の装備損失/全損)
7. cleanup_finished_battles
8. process_org_recovery
```

`DailySimulationSet`自体のチェーン順序(`Economy → ... → MilitaryAction →
...`)も含め、P25全体を通じて既存順序を一切変更していないことを確認した。
同日生産装備は同日募兵に使用可能(Economyが先)、募兵後の余剰のみが同日の
既存Division補充へ回る(§16参照)、補充後の値がその日の戦闘力計算
(`process_daily_battles`内の`calculate_combat_readiness`)に使われる
(§24の実装ノート、P25-007報告書)、戦闘損失は同じ`process_daily_battles`
呼び出し内で確定するため同一戦闘より前へ巻き戻ることはない。HashMap反復順
依存は排除されており(挿入順を変えても結果不変のテストで確認済み)、
`DivisionId`昇順tie-breakは`apportion_reinforcements`の既存実装を一貫して
再利用している。

## 12. 世界装備保存式

一時検証テスト`combined_world_conservation_across_full_chain`(§13参照、
実行後削除)で、生産→2師団への募兵支給→片方の戦闘損失→もう片方の生存復員、
という複合連鎖を実際の正規API呼び出しで実行し、次を確認した。

```text
Σ国家備蓄_after + ΣDivision装備_after
= Σ国家備蓄_before + 生産量 - 破壊量
```

fixed-point units単位で厳密に一致した(端数なし)。募兵支給・日次補充・復員
返却はいずれも純粋な移動として世界合計を変化させず、生産のみが増加、通常
戦闘損失・撤退追加損失・強制削除時全損・国家消滅時全損のみが減少させる
ことを、コード上の全mutation経路(§9)と組み合わせて確認した。

## 13. 整数値ライフサイクルの通し結果

一時検証テスト`integer_lifecycle_matches_spec_section_9_1_exactly`
(`tests/zzz_temp_p25_008_audit_verification_test.rs`、監査完了後に削除)で、
実際の`try_issue_equipment`・`calculate_proportional_equipment_loss`・
`try_destroy_equipment`・`try_issue_equipment`(補充)・`try_return_equipment`
を直接連鎖させ、仕様書§9.1の数値例(S=1000, R=100, M=100, C=25)を
fixed-point units単位で検証した。

```text
募兵完成:  S=900,  E=100, 世界合計=1000  ✅
実casualty C=25: destroyed=25, S=900(不変), E=75, 世界合計=975  ✅
日次補充25: S=875, E=100, 世界合計=975(不変)  ✅
生存復員:  S=975, Division削除, 世界合計=975  ✅
```

全段階が仕様書の期待値と正確に一致した。

## 14. 小数値・反復ドリフトの結果

一時検証テスト`fractional_lifecycle_matches_spec_section_9_2_without_drift`
で、初期備蓄10.0・募兵支給3.7・戦闘破壊(E=3.7,M=10,C=1相当)・日次補充・
復員という仕様書§9.2の小数連鎖を実行した。

```text
最終備蓄 = 9.63 (仕様書の期待値と正規units単位で完全一致)
Divisionなし(equipment=0確認)
```

さらに、初期備蓄1,000,000.0に対し「支給5.0→返却5.0」を1,000回反復する
ドリフト検証を実行し、1,000回後も備蓄が1単位もズレず初期値と完全一致する
ことを確認した(fixed-point units整数演算のため理論上もドリフトし得ない)。
これは既存の`repeated_issue_return_cycles_do_not_drift`(内部テスト、
100,000回反復)と独立に、外部から実関数を直接呼び出して再確認したもの。

## 15. 募兵支給

`src/military/recruitment.rs::try_spawn_division`のコードを確認した。
`request_recruitment`(キュー投入時)は`validate_required_equipment`で
`required_equipment`自体の有効性のみを検証し、国家備蓄は消費しない。
`process_recruitment`(Division生成時)が`try_issue_equipment(stockpile,
&mut 0.0, def.required_equipment, def.required_equipment)`を呼び、成功
した場合のみ`Division`を構築する。`division.equipment`/`max_equipment`は
どちらも`try_issue_equipment`が返した`result.equipment_after`(同一の
canonical値)を使うため`equipment == max_equipment`(完全装備)が常に
成立する。装備不足時は`try_issue_equipment`が`InsufficientStockpile`を
返し、`process_recruitment`は該当項目を`completed_indices`へ追加しない
ため`days_remaining=0`のままキューに残り、翌日以降再判定される(部分支給
なし)。`required_equipment=0`は`try_issue_equipment(requested=0.0)`が
成功するno-opとして扱われるため有効。初期Division(`spawn_debug_divisions`)
は装備会計APIを一切呼ばず直接フィールドを設定するため二重消費なし。
Loaded Game DivisionもSave/Loadが会計APIを経由しないため再支給されない
(§9で確認済み)。AI(`military::ai`)も`request_recruitment`/
`process_recruitment`という同一経路を通るため、プレイヤーと同じ装備ゲート
を受ける。国家間分離は`&mut CountryStockpile`を明示的に受け取る構造上、
借用モデルにより保証される。

## 16. 完了済みキューの飢餓防止

`src/military/recruitment.rs::process_recruitment`の戻り値`HashSet<CountryId>`
(純粋な`InsufficientStockpile`失敗があった国のIDを収集)を、
`src/military/update.rs::handle_daily_military`が
`apply_daily_equipment_reinforcement`へ渡し、該当国のその日の既存Division
向け装備補充予算を丸ごと0にする実装を確認した。新しい永続予約フィールドは
`RecruitmentQueueItem`へ一切追加されておらず、当日ごとに動的に判定される。
待機キューは備蓄を蓄積でき、募兵完成後の余剰は同日中に既存Divisionへ回る
(既存System順序どおり)。Save/Loadでこの優先順位が変化しないことは、判定
自体が当日の`process_recruitment`実行結果から都度導出される一時的な値
(永続状態ではない)であるため構造的に保証される。複数キュー時の処理順は
既存のVec先頭からの`enumerate()`走査のまま決定的。

**方式の妥当性評価**: 本方式(当日ブロック)は、備蓄が少数の完成待ちキューへ
独占され既存Divisionへ届かなくなる反例(仕様書§6の反例)を実際に解消して
おり、Phase 25のREADYを妨げる欠陥は見つからなかった。ただし、これは
「装備不足国はその日、既存Divisionの補充を一切受けられない(部分的な
優先度調整ではなく完全ブロック)」という単純な方式であり、将来複数の
完成待ちキューと多数の既存Divisionが同時に存在する場合の公平性チューニング
(例: 予算の一部だけを既存Divisionへ回す等)は依然として未実装である。
これは既存の設計判断(P25-005で確定済み)であり、Lowの将来拡張事項として
§34へ記載する(READY判定を妨げない)。

## 17. 日次補充

`src/military/equipment_reinforcement.rs::apply_equipment_reinforcement_
for_country`のコードを確認した。予算`T = min(stockpile_units, Σshortfall_
units)`をfixed-point units上で計算し、既存の`apportion_reinforcements`
(最大剰余法、`DivisionId`昇順tie-break)をmanpower補充と共通再利用する。
補充対象条件は`is_eligible_for_daily_reinforcement_by_position`
(所有・存続・非戦闘・支配地域一致)をmanpower補充と共有しつつ、
manpower不足条件とequipment不足条件はそれぞれ独立(`max_equipment>0.0`の
みequipment補充適格条件へ追加)。国単位で2段階(事前検証→全件成功時のみ
commit)の原子的処理であり、1つのDivisionまたは国家備蓄が不正状態の場合は
その国全体が変更されずに中断される。`max_equipment`は読み取りのみで
一切書き込まれない。消滅国家(`is_active()==false`)は対象外。複数国は
`owner`ごとに独立処理され、HashMap挿入順への非依存はソート
(`sort_by_key`)で保証されている。

## 18. 通常戦闘損失

`src/military/equipment.rs::calculate_proportional_equipment_loss`の式
`D = floor((E×C + floor(M/2)) / M)`(`E`=現在装備units、`C`=actual casualty、
`M`=manpower_before)を確認した。`actual_casualty`は`process_daily_battles`
側の`saturating_sub`によるclamp後の実減少量であり、combat_calcが要求した
予定damageではない。`current_equipment`基準(`max_equipment`不参照)、
`u128` checked演算、half-up丸め(`+floor(M/2)`)、`casualty=0`または
`equipment=0`ならdestroyed=0、`C=M`なら数学的に必ず`D=E`(全滅時全損)、
attacker/defenderとも同一の`apply_combat_casualty`ヘルパーを経由(§22の
Test Qで非対称statsでも確認済み)、国家備蓄は一切参照されない(戦闘関数が
`CountryStockpile`を引数に取らない構造)、manpower・equipment・organization
の更新は`apply_combat_casualty`内で原子的(装備損失計算/`try_destroy_
equipment`のいずれかが失敗すればmanpower/organizationも一切更新しない)、
新しいRNGは導入されていない(このコードベースの戦闘計算はそもそも決定的)。
装備量がcasualty計算(実効attack/defense)へ影響するのはP25-007の
`calculate_combat_readiness`経由の戦闘力反映経路だけであり、装備損失式
自体はcasualtyの結果を受け取るだけで逆方向のフィードバックはない。

## 19. 撤退追加損失

`src/military/update.rs::resolve_participant_outcome`の撤退分岐を確認した。
`RETREAT_MANPOWER_LOSS_RATIO`による予定retreat_lossに対し、実際の
`saturating_sub`後の減少量を`actual_casualty`として使い、通常戦闘と同じ
`calculate_proportional_equipment_loss`をそのまま再利用する(損失式の複製
なし)。二重計上防止は、撤退追加損失が必ず「同日の`process_daily_battles`
適用後」の`division.manpower`/`division.equipment`を基準に計算されることで
構造的に保証されている(元のM/Eから再計算しない)。撤退全滅時
(`remaining_manpower==0`)も同じ恒等式により数学的に必ず全損。撤退先が
ない場合の強制削除は、比例損失を経ずに現在装備を1回でまとめて全損させる
設計(P25-006-REV-001の意図的な単純化、§20参照)であり、これも仕様の要求
(削除前equipment=0)を満たす。

## 20. 正式復員

`src/military/demobilization.rs::demobilize_division`を確認した。
`division.equipment`(現在値、`max_equipment`ではない)のみを`try_return_
equipment`経由で返却する。返却は、manpower会計の事前検証(ledger・
available/burden算出)が全て成功した後、かつ実際のcommit前に、cloneした
一時値へ適用する2段階構成であり、いずれかが失敗すれば国家/Division/
registryのいずれも変更されない。破壊済み装備(`max_equipment - equipment`)
は復活しない。所有国偽装(所有権不一致要求)は`NotOwner`で拒否。
`MilitaryRegistry::remove_division`が同期的`HashMap::remove`であるため、
同一Divisionへの2件目の復員要求は1件目完了直後に`DivisionNotFound`となり
二重返却は構造的に発生しない。復員以外の削除経路(戦闘全滅・撤退・強制
削除・国家消滅)はいずれも`try_return_equipment`を呼ばず、逆に復員経路は
`try_destroy_equipment`を呼ばないという明確な排他性を確認した。

## 21. 強制削除

`src/military/update.rs::resolve_participant_outcome`の全滅防御分岐
(`division.manpower==0`)・撤退失敗分岐(撤退先なしまたは撤退追加casualty
で全滅)の両方で、`try_destroy_equipment(&mut d.equipment, remaining_
equipment)`(`remaining_equipment = d.equipment`)を常に呼ぶ実装であること
をコード上で確認した。以前の実装ミス(`if remaining_equipment > 0.0 {...}`
という条件付き呼び出し、NaN装備で検証をスキップするバグ)は、P25-006-REV-001
で常時呼び出しへ修正済みであり、現行コードにこのガードは一切残っていない
ことを`rg -n "if.*equipment.*> 0\.0"`で確認した(0件)。

## 22. 国家消滅

`src/country/elimination.rs::apply_country_elimination`の解散ループを
確認した。`military_registry.remove_division(division_id)`を呼ぶ直前に
常に`try_destroy_equipment(&mut division.equipment, remaining_equipment)`
を呼ぶ(NaNガード問題なし、§21と同型の修正済みパターン)。消滅国家の
`CountryStockpile[MilitaryEquipment]`には一切書き込まない(凍結)。併合国の
`CountryStockpile`にも一切触れない(移転しない)。§0.5として新設された
preflightステップが、Registry変更前に全Divisionの装備状態を
`validate_equipment_state`で検証し、1件でも不正なら`CountryEliminationError::
InvalidDivisionEquipment`を返して州所有権移転・国家status遷移・Division
削除のいずれも実行しない原子性を確認した(コード上の到達順序で確認、
既存のNaN装備テストが§21と対で存在)。

## 23. 削除直前equipment=0不変条件

復員を除く全ての本番Division削除経路(通常戦闘全滅・撤退追加casualty全滅・
撤退強制解散・国家消滅)で、`military_registry.remove_division`呼び出し
直前に必ず`try_destroy_equipment`(常時呼び出し、NaNガードなし)が実行され、
`division.equipment`が0へ確定してから削除されることをコード上確認した
(§18〜22)。復員経路のみ、返却によって0へ到達してから削除される
(`try_return_equipment`が現在値を全額返却するため`equipment_after=0`)。
`equipment > 0.0`のような素通りガードは現行コードに一切存在しない
(§21で確認済み)。

## 24. 装備充足率の戦闘力反映

`src/military/combat_calc.rs::calculate_combat_readiness`を確認した。

```text
m = manpower / max_manpower, e = equipment_units / max_equipment_units
combat_readiness = min(m, e)  (積ではない、per-million fixed-point、u128 checked)
```

`resolve_combat_day`(1v1)・`effective_power_1000`(N対M)双方の
`atk_effective_1000`/`def_effective_1000`計算式で、旧`manpower_ratio_1000`
を`combat_readiness_1000`(内部ヘルパー)へ置き換えた単一代入で実装されて
おり、既存式への二重適用はない。attacker/defenderは同一関数を経由。
Divisionごとに個別補正してから合算(集団を先に平均化しない)。
`process_daily_battles`側に新設されたpreflightが、`resolve_combat_day_
multi`呼び出し前に全参加者へ`calculate_combat_readiness`を適用し、1件でも
`Err`ならそのBattleの当日解決を丸ごとskip(他Battle・他国家へ影響しない)
することをコード上確認した。organization・movement・RNGへは直接適用され
ていない(それぞれ既存式のまま、casualty経由の間接影響のみ)。両陣営装備0
でもcasualty自体が0になり安全に膠着する(panicなし)。

## 25. `max_equipment=0`の意味

`calculate_combat_readiness`のコードで、`max_equipment_units==0`の場合
`equipment_readiness_units = COMBAT_READINESS_SCALE`(100%)として扱われる
こと、`max_equipment_units==0`かつ`equipment_units>0`は
`CombatReadinessError::EquipmentExceedsMax`として明示的に拒否される
(黙って100%へclampしない)ことを確認した。これにより装備不要Division
(`max_equipment=0`)が誤って装備不足扱いされることはない。

## 26. 不正状態のpreflightと原子性

戦闘(`process_daily_battles`のBattle単位preflight、§24)・日次補充(国家
単位の2段階commit、§17)・国家消滅(Registry変更前の全Division§0.5
preflight、§22)のいずれも、不正状態(NaN・manpower/equipment超過等)を
検出した場合に該当スコープ(そのBattle/その国/その国家消滅トランザクション
全体)を原子的に中断し、他のスコープへ影響を与えないことを、それぞれの
コード上の検証順序(検証→ローカル変数算出→全件成功後のみcommit)で確認
した。

## 27. 複数国家分離

`try_issue_equipment`等の正規APIはいずれも`&mut CountryStockpile`・
`&mut f64`を呼び出し元から明示的に受け取り、グローバル状態・共有可変状態を
一切持たない構造(Rustの借用モデル上、他国のインスタンスへ影響を与える
ことは構造的に不可能)。`apply_equipment_reinforcement_for_country`は
`owner`ごとに独立呼び出し。`apply_country_elimination`は消滅国・併合国
それぞれの`CountryStockpile`を明確に区別して扱う。既存の複数国家テスト
(P25-002〜P25-007各報告書のTest群)がいずれもこれを直接検証済みであること
を報告書で確認した。

## 28. 決定性

Division間の配分は`apportion_reinforcements`(最大剰余法+`DivisionId`昇順
tie-break)を一貫して再利用しており、HashMap挿入順に依存しない
(`equipment_reinforcement.rs`は明示的な`sort_by_key`でこれを保証)。全ての
正規APIは純粋関数(グローバル状態・乱数・時刻を参照しない)であり、同じ
入力に対して常に同じ出力を返す。既存のP25-004/P25-007報告書のTest R
(5日間bit-for-bit一致)・Test 26/27/28(挿入順・複数装備水準での決定性)を
確認した。

## 29. New Game／Loaded Game比較

各P25-00Xタスクの報告書で、New Game直後の実行結果とNew Game→save→load後
の実行結果(募兵・戦闘損失・日次補充・復員・撤退・国家消滅・戦闘readiness
それぞれについて)が完全一致することを実App経由で確認済みであることを
報告書の記述と対応するテスト名から確認した。本タスクではこれらの既存
テストを再実行し(§37)、全て合格することを確認した。

## 30. save/load分岐比較

同様に、各段階(募兵前後・補充前後・戦闘前後・復員前後・撤退前後・国家
消滅前後)のsave/load分岐比較テストが各P25-00Xタスクで実装・合格済み
であることを確認し、本タスクでも該当テストファイルを再実行して全合格を
確認した(§37)。

## 31. Save V1維持

`src/save/`配下のいずれのファイルも、P25-002〜P25-007のいずれのコミット
でも変更されていないことを`git log --stat`のファイル一覧から確認した
(いずれの報告書も`src/save/`への変更を報告していない)。装備会計に新しい
保存フィールドは一切追加されておらず、`Division.equipment`/`max_equipment`・
`CountryStockpile[MilitaryEquipment]`はいずれもDivision/CountryData構造体
導入時から存在する既存必須フィールドのまま。`SAVE_FORMAT_VERSION_V1`・
migration・load applyのいずれも無変更。

## 32. Phase 23・24会計の保護

本タスクで`cargo test --test p23_002_mobilized_manpower_workforce_e2e_test`
(8件)・`p23_004_economic_workforce_ratio_e2e_test`(7件)・
`p23_006_manpower_ledger_e2e_test`(22件)・
`p23_007_division_demobilization_e2e_test`(24件)・
`p23_008_daily_reinforcement_e2e_test`(21件)・
`p23_009_ai_recruitment_e2e_test`(22件)・
`p23_010_phase23_manpower_lifecycle_audit_test`(9件)・
`p24_002_workforce_rounding_unification_e2e_test`(17件)を個別に実行し、
全件合格を実測した(§37)。`military::manpower`(A・B・L・K・X)・
`state::data::canonical_total_workforce`・`population::`配下は、いずれの
P25-00Xコミットでも変更されていないことを各報告書の変更ファイル一覧
(§28/§34/§29/§32/§31/§34/§34の各節)で確認した。

## 33. 死んだ旧戦闘経路の状態

`src/war/combat.rs::process_combat`(引数`military_registry`・`war_registry`
を取る古い1対1簡易戦闘実装、生の`f64`加減算で`equipment -= damage*0.1`)は、
`src/war/mod.rs::WarPlugin::build`の`add_systems`登録を確認したところ
依然として一切登録されておらず、実際に呼び出されるのは`src/military/
update.rs`の内部テスト(`test_u_dead_war_combat_process_combat_path_
remains_unconnected`)からのみであることを確認した。Phase 25全体を通じて
この死んだ経路は一度も活性化されておらず、実ゲームプレイでは実行されない。

## 34. 将来タスク・対象外

仕様書§18が列挙する将来タスク(弾薬・燃料・補給物資、複数装備カテゴリ、
装備品質・修理・鹵獲、補給網・輸送上限、AI装備生産/備蓄方針、UI表示、
Attrition連動、population/workforce連動、Save V2等)はいずれも現行実装に
到達可能な不整合を生んでおらず、Phase 25のREADYを妨げない。特に仕様書
§18が明示的に確認を求めた2点について、実コード監査の結果を明確にする。

* **AIの装備不足を考慮しない完成待ちキュー増加**: P25-003報告書§Low-1が
  記録したとおり、旧treasury仮実装の偶発的な抑制効果が意図的に廃止された
  結果であり(§35で確認したtreasury消費削除の直接的帰結)、バグではなく
  意図した挙動変化。AIは装備が不足していても`request_recruitment`への
  投入自体は成功し、Division生成は装備到着まで待機する(§15の設計どおり)。
  将来のAIバランス調整(装備備蓄目標の導入等)で対応すべき既知の技術負債
  であり、Lowとして記録する(非ブロッキング)。
* **装備0同士の戦闘膠着**: P25-007報告書§22で確認したとおり、
  `calculate_combat_readiness`が両陣営0%を返す場合、casualty自体が0となり
  安全に膠着する(panicなし、NaN/Infinityなし)。これは仕様書§14が明示的に
  要求した設計(「arbitraryな最低戦闘力で隠さない」)どおりの意図した現行
  仕様であり、バグではない。将来的に補給切れによる強制後退等のメカニクス
  を追加するかどうかは別タスクの範囲。

§16で述べた完了済みキュー飢餓防止方式の公平性チューニング余地もLowとして
ここに含める。

## 35. READY判定を妨げる事項の有無

なし。§4のREADY判定基準13項目(正規API使用・無から生成/消失なし・保存則
成立・不正値の明確な拒否・原子性・国家間分離・削除分類の一意性・装備
充足率が正規経路のみへ反映・Save意味的整合・Phase23/24保護・全テスト合格・
clippy 0・Critical/High/装備中核Medium 0件)をすべて満たすことを、本報告書
の§8〜§33の実コード確認および§36の実測テスト結果で確認した。

## 36. Critical／High／Medium／Low

**Critical: 0件**
**High: 0件**(P25-006が発見した2件はP25-006-REV-001で解消済み、§7参照。
現行findingとして再計上しない)
**Medium: 0件**
**Low: 2件**(いずれも非ブロッキング、§34参照)

1. 完了済み募兵キューの飢餓防止方式(当日の既存Division補充予算を丸ごと
   0にする)が単純な全ブロック方式であり、部分的な予算配分等の公平性
   チューニングは将来課題として残る。
2. AIの装備備蓄を考慮した募兵戦略・装備0同士の膠着に対する後退メカニクス
   等のAI/ゲームプレイバランス調整は将来課題として残る(バグではなく
   意図した現行仕様であることを§34で確認済み)。

## 37. 実行した全コマンドと正確な結果

すべて逐次実行(並列実行・同一メッセージ内での複数cargo呼び出しは一切
行っていない)。

```text
git rev-parse HEAD                                        # 0adcfcc54512d288b7ef3ee19e5eb9ac1bbf4171
git status --short                                         #  M saves/savegame_v1.ron のみ
git log --oneline --decorate -15                            # 想定と一致

cargo test --lib economy::                                  # 12 passed; 0 failed
cargo test --lib military::                                 # 283 passed; 0 failed
cargo test --lib country::                                  # 79 passed; 0 failed
cargo test --lib save::                                     # 300 passed; 0 failed

cargo test --test p23_002_mobilized_manpower_workforce_e2e_test        # 8 passed
cargo test --test p23_004_economic_workforce_ratio_e2e_test            # 7 passed
cargo test --test p23_006_manpower_ledger_e2e_test                     # 22 passed
cargo test --test p23_007_division_demobilization_e2e_test             # 24 passed
cargo test --test p23_008_daily_reinforcement_e2e_test                 # 21 passed
cargo test --test p23_009_ai_recruitment_e2e_test                      # 22 passed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test        # 9 passed
cargo test --test p24_002_workforce_rounding_unification_e2e_test      # 17 passed

cargo test --test p25_002_equipment_accounting_foundation_e2e_test               # 15 passed
cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test      # 6 passed
cargo test --test p25_003_recruitment_equipment_issue_e2e_test                   # 22 passed
cargo test --test p25_004_combat_equipment_loss_e2e_test                         # 5 passed
cargo test --test p25_005_daily_equipment_reinforcement_e2e_test                 # 10 passed
cargo test --test p25_006_demobilization_equipment_return_e2e_test               # 29 passed
cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test # 8 passed
cargo test --test p25_007_equipment_combat_effectiveness_e2e_test                # 21 passed

cargo test --test land_war_combat_peace_test                # 4 passed (保護ファイル、無変更のまま再確認)

# 一時検証テスト(tests/zzz_temp_p25_008_audit_verification_test.rs、
# 監査完了後に削除・コミット未含有)
cargo test --test zzz_temp_p25_008_audit_verification_test  # 3 passed
  # - integer_lifecycle_matches_spec_section_9_1_exactly: 仕様書§9.1の整数
  #   数値例(S=1000→900→900→875→975等)が実際のtry_issue_equipment/
  #   calculate_proportional_equipment_loss/try_destroy_equipment/
  #   try_return_equipmentの連鎖でunits単位で正確に再現されることを確認
  # - fractional_lifecycle_matches_spec_section_9_2_without_drift: 仕様書
  #   §9.2の小数数値例(最終備蓄9.63)を実関数で再現、および1,000回の
  #   支給/返却反復でドリフトが1単位も発生しないことを確認
  # - combined_world_conservation_across_full_chain: 生産→募兵支給→戦闘
  #   損失→復員返却を連鎖させ、世界合計=生産-破壊量が正確に成立することを
  #   確認

cargo test --all-targets       # 1回目(一時テスト含む): 1836 passed / 0 failed / 1 ignored
                                # (git status --short で既知の非決定的PNG9件が再生成 → 個別checkoutで復元)
# 一時検証テストファイルを削除(tests/zzz_temp_p25_008_audit_verification_test.rs)
cargo test --all-targets       # 2回目(削除後、最終確認): 1833 passed / 0 failed / 1 ignored
                                # (既知の非決定的PNG9件が再度再生成 → 個別checkoutで復元)
cargo clippy --all-targets --all-features -- -D warnings   # 0 warnings(削除後の最終確認)

git status --short             #  M saves/savegame_v1.ron のみ
```

## 38. 監査対象コードの`git status --short`

```text
 M saves/savegame_v1.ron
```

(一時検証テストファイルは削除済みであり残っていない。上記コマンド実行後の
実測結果。)

## 39. コミットへ含めるファイル

```text
verification_logs/phase-25/p25-008/p25-008_phase25_final_audit.md
```

本報告書のみ。本番コード・既存テストコード・過去の報告書はいずれも変更
していない。`git diff --stat -- verification_logs/phase-25/p25-001
verification_logs/phase-25/p25-002 verification_logs/phase-25/p25-002-rev-001
verification_logs/phase-25/p25-003 verification_logs/phase-25/p25-004
verification_logs/phase-25/p25-005 verification_logs/phase-25/p25-006
verification_logs/phase-25/p25-006-rev-001 verification_logs/phase-25/p25-007`
が空であることを確認済み(過去報告書は無変更)。

## 40. Phase 25の最終結論

**AUDIT COMPLETE, PHASE 25 READY**

Phase 25(軍事装備会計ライフサイクル: 生産→国家備蓄→募兵支給→戦闘/撤退
損失→日次補充→復員返却→強制削除/国家消滅時破壊→戦闘力反映)は、実コード・
実関数・既存テスト(cargo test --all-targets: 1833 passed / 0 failed /
1 ignored、clippy: 0 warnings)・本タスクで新規作成し監査完了後に削除した
一時検証テスト(実際の正規API連鎖によるunits単位の数値例再現)のいずれに
よっても、READY判定基準13項目をすべて満たすことが確認された。Critical/
High findingは0件、装備会計の中核に関係するMedium findingも0件である。
残る2件のLow findingはいずれも非ブロッキングの将来課題(完了済みキュー
飢餓防止方式の公平性チューニング余地、AI装備戦略/ゲームプレイバランス
調整)であり、Phase 25のREADY判定を妨げない。
