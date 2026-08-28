# P25-006: 復員時の生存装備返却と国家消滅時の装備会計

## 1. 総合判定

**COMPLETE(復員機能に限定、国家消滅の既知の矛盾はHigh findingとして記録・未修正)**

実装前監査で、P25-006のスコープに直接関わる矛盾を発見した。国家消滅時の既存実装
(`src/country/elimination.rs`、P21-021、Phase 25より前から存在)は、消滅国家の全
Divisionを`MilitaryRegistry::remove_division`で物理削除しており、P25-001が決定した
「国家消滅時の装備会計は凍結して残存」という設計と矛盾する(§14/§20参照)。ユーザーに
報告し判断を仰いだ結果、「P25-006は復員機能のみ完了する。`country::elimination`は
一切変更せず、この矛盾は別タスクの対象としてHigh findingとして記録する」という方針が
選択された(本報告書はその方針に基づく)。復員本体(§3〜§13)は完全に実装・検証済みで
COMPLETEである。

## 2. 開始時HEAD

```text
26a8629e793aa0d1451120433c85535f1b5d25e5
```

## 3. 監査対象コード

* `src/military/demobilization.rs::demobilize_division`(復員本体、実装前の唯一の本番呼び出し元は`src/ui/military_panel.rs:2081`)
* `src/military/equipment.rs::try_return_equipment`(P25-002-REV-001の正規返却API)
* `src/military/update.rs::resolve_participant_outcome`(P25-004の戦闘全滅despawn経路)
* `src/country/elimination.rs::apply_country_elimination`(国家消滅、P21-021)
* `src/military/data.rs::MilitaryRegistry::remove_division`(Division削除の唯一の実装)
* `src/military/army.rs`(Army編成整理、`ArmyRegistry::sanitize_references`)
* `src/war/frontline.rs`(前線参照整理、`FrontlineRegistry::sanitize_references`)

## 4. 稼働中の復員経路

```text
src/ui/military_panel.rs::execute_division_demobilization(ボタン押下System)
  └─ src/military/demobilization.rs::demobilize_division(唯一の本番Domain API呼び出し)
```

AIは復員経路を一切使用しない(`rg "demobilize_division"`の結果、呼び出し元は
`src/ui/military_panel.rs`と`src/military/demobilization.rs`自身のテストのみ)。

## 5. 復員可能性の既存条件

`demobilize_division`は以下を順に検証する(P23-007から無変更)。

1. 国家が存在し`is_active()`(消滅していない)
2. Divisionが存在し、要求国家が所有国と一致
3. `manpower>0`かつ`status != Destroyed`
4. `status == Idle`(Fighting/Moving/Retreating/Occupying/Disbanding/Destroyedは拒否)
5. `BattleRegistry`の進行中戦闘から参照されていない
6. 人的資源帳簿(`compute_manpower_ledger`)が計算可能かつ整合

P25-006はこの6条件のいずれも変更していない。新たに追加したのは、これらすべてに
成功した後段の装備返却検証(§10・§13)のみである。

## 6. Division全削除経路の分類

`rg -n "remove_division\("`で本番経路を全て洗い出し、以下のとおり分類した。

| 削除・停止理由 | 実際のコード箇所 | 装備処理 |
| --- | --- | --- |
| 生存者の正式な復員 | `military::demobilization::demobilize_division` | **P25-006で追加: `try_return_equipment`で所属国備蓄へ返却** |
| 戦闘による全滅(manpower==0) | `military::update::resolve_participant_outcome`(P25-004) | `try_destroy_equipment`で現在装備を全損、返却しない |
| 戦闘による撤退追加損失(組織率0での敗走) | `military::update::resolve_participant_outcome`(撤退分岐) | P25-004・P25-006とも未接続(§19で監査結果を報告) |
| 国家消滅 | `country::elimination::apply_country_elimination` | **既知のギャップ(§14参照)。装備会計APIを一切呼ばずDivisionを物理削除するため、現在装備が会計上消失する** |
| save/load時のワールド置換 | `save::write`/`save::load`(独立した永続化経路、Division実体を再構築するだけで会計APIを呼ばない) | 対象外(保存状態をそのまま復元するだけ) |
| テスト・セットアップ上の削除 | `military::army::tests`等の`#[cfg(test)]`内直接呼び出し | 本番会計対象外 |

`military::army.rs`・`war::frontline.rs`内の`remove_division`呼び出しは全て
`#[cfg(test)]`ブロック内のテストコードであることを確認済み(本番despawn経路ではない)。

## 7. 返却対象がcurrent equipmentだけであること

`demobilize_division`は`division.equipment`(復員実行時点の現在値、`max_equipment`
ではない)を`division_equipment`としてローカルに捕捉し、これを`try_return_equipment`の
`returned`引数へそのまま渡す。損耗Division(§16.2の数値例、`max_equipment=100,
equipment=40`)では40だけが返却され、破壊済みの60は一切復活しない(Test 2/3)。

## 8. max_equipmentを返却しないこと

`demobilize_division`のコード上、`division.max_equipment`は一切参照されない
(返却量の計算にもtry_return_equipmentの呼び出しにも登場しない)。Test 9で
`returned_equipment != max_equipment(=100.0)`であることを明示的に確認した。

## 9. fixed-point unitsの使用

`try_return_equipment`(P25-002-REV-001が確立済み)を再利用しており、装備量の
比較・加減算は全て正規units(scale 1,000,000)上で行われる。P25-006はこのAPI内部の
式を一切変更・複製していない。小数装備(3.7)の返却がunits単位で正確に一致すること
をTest 6で確認した。

## 10. `try_return_equipment`の接続箇所

`src/military/demobilization.rs::demobilize_division`内、manpower会計の検証
(`available_after`/`workforce_burden_after`の計算)が全て成功した直後、かつ
実状態への実際のcommit(`country_registry.get_mut`以降)より前に、`country.stockpile`
をcloneした一時値と、`division.equipment`をコピーしたローカル`f64`へ
`try_return_equipment`を適用する形で接続した。

```rust
let mut stockpile_after = country.stockpile.clone();
let mut equipment_after_local = division_equipment;
try_return_equipment(&mut stockpile_after, &mut equipment_after_local, division_equipment)
    .map_err(DemobilizationError::EquipmentReturnFailed)?;
```

成功した場合のみ、`country.stockpile = stockpile_after`として実状態へcommitする。
損失式(destroy)・返却式(return)とも既存APIをそのまま呼ぶだけで、計算式自体を
`demobilization.rs`側へ複製していない。

## 11. manpower＋equipment＋despawnの原子性

`demobilize_division`全体の処理順序:

```text
1. request validation(国家/Division存在・所有権)
2. ownership/eligibility validation(status・進行中戦闘参照)
3. manpower会計の事前計算(ledger検証・available/burden算出、まだcommitしない)
4. equipment/stockpile validation(cloneした一時値でtry_return_equipment、まだcommitしない)
5. 3・4がともに成功した場合のみ、country registryへ実際にcommit
6. military_registry.remove_division(Division削除)
7. army_registry.sanitize_references / frontline_registry.sanitize_references
```

manpower検証(3)が失敗した場合は装備検証(4)へ到達せず即座に`Err`。装備検証(4)が
失敗した場合は、3で計算済みの`available_after`/`workforce_burden_after`はまだ
どこにもcommitされていない(ローカル変数のまま)ため、country/Division/registryの
いずれも一切変更されずに`Err`を返す(Test 13/14/15/16/18で確認)。

## 12. 失敗時のrollback

装備返却が失敗した場合(NaN装備・NaN備蓄・stockpile overflow等)、以下を
テストで確認した。

* 国家の`available_manpower`/`mobilized_manpower`は変化しない(Test 16/18)
* `CountryStockpile[MilitaryEquipment]`は変化しない(Test 14/15)
* `Division.equipment`は変化しない(Test 13/15)
* `Division.manpower`は変化しない(Test 13/15/16)
* Divisionは削除されない(全失敗テストで`military.divisions.contains_key`を確認)
* panicしない(全テストが正常終了)

manpower会計自体が失敗した場合(既存のP23-007の各種エラー)も、装備返却処理
(§10のtry_return_equipment呼び出し)へは到達しないため、装備側は当然無変更のまま。

## 13. 同一フレーム重複要求への対応

`MilitaryRegistry::remove_division`はBevy Commandsを介さない同期的な
`HashMap::remove`であり(`Division`は`#[derive(Component)]`を持たないプレーンな
データ)、復員成功時点で即座にDivisionがレジストリから消える。したがって、同一
Divisionへの2件目の復員要求は、1件目の処理完了直後には既に`DivisionNotFound`に
なる(Test 20)。新しい「処理済みID」追跡集合や新規Saveフィールドは一切追加して
いない——既存のアーキテクチャ(deferred Commandsではなく即時削除)が、この要求
自体を構造的に満たしていた。

## 14. 国家間分離

`try_return_equipment`は`demobilize_division`が渡した`country.stockpile`
(要求国家自身のもの)のクローンにのみ作用する。他国の`CountryStockpile`へは
一切アクセスしない構造(Rustの借用境界に加え、実際に複数国家E2Eテストで検証)。

* Test 10/11: 正規所属国のみ増加、別国家は不変
* Test 12: 偽装された他国復員要求(所有権不一致)を`NotOwner`で拒否、両国とも不変
* Test 22: 1国の装備会計失敗が別国の復員処理へ波及しない

## 15. 装備保存則

Test 7/8で、復員単独処理において次の等式がunits単位で厳密に成立することを確認した。

```text
stockpile_before + Σ division_equipment_before
=
stockpile_after + Σ surviving_division_equipment_after
```

復員したDivisionは右辺の合計から除外される。世界装備総量は増減しない(生存
Division(id=1)の装備が復員処理と無関係に不変であることも同時に確認)。

## 16. P23-007 manpower会計の維持

Test 23/25で、`src/military/demobilization.rs`の既存内部テスト
(`demobilizing_a_worn_division_returns_only_its_surviving_manpower`)と全く同じ
数値例(初期募兵10,000・戦闘損失3,000・生存7,000復員)を使い、`returned_manpower`・
`available_manpower`・`mobilized_manpower`の数値がP25-006前後で完全に不変で
あることを確認した(装備返却を同時に検証しつつ)。既存の22件のDomain単体テスト
(`src/military/demobilization.rs`)・24件のE2Eテスト
(`tests/p23_007_division_demobilization_e2e_test.rs`)は1件も変更せず全て合格。

## 17. 戦闘全滅との分離

`demobilize_division`は`try_return_equipment`のみを呼び、`try_destroy_equipment`
(P25-004の戦闘損失専用API)を一切呼び出さない。戦闘全滅は`military::update`側で
既に完結しており、`demobilize_division`が介入する余地は構造的にない(全滅した
Divisionは`resolve_finished_battles`が既に`MilitaryRegistry`から削除済みのため、
事後の復員要求は必ず`DivisionNotFound`になる、Test 28/29)。

## 18. P25-004全滅装備0の再確認

P25-004の内部テスト(`src/military/update.rs::tests::test_i_overkill_damage_...`
等)が、全滅Divisionのdespawn直前に`equipment == 0.0`であることを既に確認済み
(全滅時、実装は数学的にD=E(現在装備を全損)することを保証している)。本タスクでは
P25-004のコード・テストを一切変更しておらず(`cargo test --lib military::update::`
が全テスト合格)、`process_daily_battles`/`resolve_finished_battles`の
実装は無変更のまま再検証された。

## 19. 撤退時追加損失の監査結果

`RETREAT_MANPOWER_LOSS_RATIO`による追加manpower損失(`resolve_participant_outcome`
内、組織率0での撤退時)は、P25-004と同様、本タスクでも一切変更していない。
Test 31で、`demobilization.rs`のソースコード自体に`RETREAT_MANPOWER_LOSS_RATIO`
という文字列参照が存在しないことを確認し、復員コードが撤退処理に一切触れていない
ことを保証した。

**監査結果(§13が要求する「撤退追加損失だけでDivisionが0 manpowerとなり非ゼロ装備の
ままdespawn可能か」の確認)**: `resolve_participant_outcome`のコードを確認したところ、
撤退追加損失で`manpower`が0になった場合(`remaining_manpower == 0`または撤退先が
見つからない場合)は`military_registry.remove_division(division_id)`が呼ばれる
(`src/military/update.rs`の該当分岐、P25-004investigationで確認済みのコード)。
この経路は装備会計API(`try_destroy_equipment`/`try_return_equipment`のいずれも)を
呼ばずにDivisionを削除しており、非ゼロ装備のまま未処理削除される経路が**実際に
到達可能**である。これはP25-004の時点で既に存在していた既知の追跡事項
(「撤退時追加manpower損失は装備損失へ未接続」)の一部であり、P25-004報告書でも
明示的に対象外として記録済みである。本タスクの指示(§13)に従い、これを隠さず
Critical/High相当の追跡事項として§28へ記載する(本タスクの対象範囲では修正しない、
§15対象外「撤退時追加manpower損失への装備損失接続」のとおり)。

## 20. 国家消滅時の凍結(既知のギャップ)

実装前監査(§4/§14)で発見したとおり、`country::elimination::apply_country_elimination`
は以下のとおり動作する。

* `CountryStockpile[MilitaryEquipment]`: **一切触れない(正しく凍結される)**
* 消滅国家が保有していた全Division: `military_registry.remove_division`で
  **物理削除される(凍結されない)**

`country_elimination_gap_stockpile_is_frozen_but_division_equipment_is_lost_not_frozen`
テストで、実際に`preflight_country_elimination`→`apply_country_elimination`を実行し、
次を確認した。

* 消滅国家の備蓄(50.0)は凍結され不変 ✅ (P25-001設計どおり)
* 併合国の備蓄(100.0)は増えない ✅ (P25-001設計どおり)
* 消滅国家のDivision(装備40.0)は物理削除され、どこにも残らない ❌ (P25-001設計と矛盾)
* 消滅前の世界装備総量190.0(50+100+40)が、消滅後は150.0になり、40.0が会計上消失する

この矛盾はP25-006が作った問題ではなく、Phase 21(P21-021)から存在する既存実装が
Phase 25(P25-001)の設計決定を満たしていなかったことの発覚である。ユーザーへ報告し、
「P25-006は復員機能のみで完了、`country::elimination`は変更せず、この矛盾は
別タスクの対象としてHigh findingとして記録する」という方針を確認・採用した。

## 21. 併合国へ移転しないこと

上記テストで、併合国(CountryId(2))の備蓄が消滅前後で全く変化しない(100.0のまま)
ことを確認した。既存実装はこの点(移転しないこと)については仕様どおりであり、
問題は「凍結すべきDivisionが削除される」点のみである。

## 22. New Game／Loaded Game比較

Test 37で、New Game直後に復員した結果と、New Game→save→load後に同じ復員を
実行した結果(返却装備量・stockpile)が完全に一致することを確認した。

## 23. 復員前後のsave/load分岐比較

* Test 38(復員前分岐): 中断なしの継続実行と、復員直前にsave/loadを挟んだ場合とで、
  返却装備量・stockpileが完全に一致することを確認。
* Test 39/41(復員後): 復員直後にsave/loadしても、stockpileが再度増加しないこと
  (二重返却なし)・復員済みDivisionがロード後も再出現しないことを確認。
* Test 43: ロードするだけ(復員要求を送らない)では装備が自動返却されないことを確認。

## 24. Save V1維持

新しい保存フィールドは一切追加していない。`DemobilizationOutcome`に
`returned_equipment: f64`フィールドを追加したが、これは関数の戻り値(呼び出し元へ
結果を伝える一時的な構造体)であり、Save V1には一切含まれない。既存の
`Division.equipment`/`CountryStockpile[MilitaryEquipment]`フィールドのみを使って
保存・復元される。

## 25. Phase 23・24会計の保護

* P23-006(`compute_manpower_ledger`): 無変更、`tests/p23_006_manpower_ledger_e2e_test.rs`全22件合格
* P23-007(`demobilize_division`のmanpower部分): §16で確認したとおり数値不変
* P23-008(日次manpower補充): `tests/p23_008_daily_reinforcement_e2e_test.rs`全21件合格
* P23-010(Phase 23ライフサイクル): `tests/p23_010_phase23_manpower_lifecycle_audit_test.rs`全9件合格
* P24-002(workforce計算): `tests/p24_002_workforce_rounding_unification_e2e_test.rs`全17件合格

## 26. P25-003〜P25-005回帰結果

* P25-002-REV-001(固定小数点装備会計): `tests/p25_002_equipment_accounting_foundation_e2e_test.rs`(15件)・`tests/p25_002_rev_001_fixed_point_equipment_accounting_e2e_test.rs`(6件)全合格
* P25-003(募兵装備支給): `tests/p25_003_recruitment_equipment_issue_e2e_test.rs`全22件合格
* P25-004(戦闘装備損失): `tests/p25_004_combat_equipment_loss_e2e_test.rs`全5件、`cargo test --lib military::update::`全14件合格
* P25-005(日次装備補充): `tests/p25_005_daily_equipment_reinforcement_e2e_test.rs`全10件合格

いずれのソースファイルも本タスクで変更していない(`src/military/equipment.rs`・
`src/military/update.rs`・`src/military/recruitment.rs`・
`src/military/equipment_reinforcement.rs`・`src/military/reinforcement.rs`は無変更)。

## 27. 対象外を先行実装していないこと

装備充足率の戦闘力反映・装備種別・装備品質・鹵獲・修理回収率・復員時装備損耗率・
国家消滅時の併合国への装備移転・国家降伏時の戦利品・補給線/輸送時間・日次消耗品補給・
UIの装備表示拡張・AIの復員判断/装備備蓄目標変更・Save V2/migration・Attrition・
負傷者/疾病・戦死者population連動・撤退時追加manpower損失への装備損失接続・
P25-007以降の先行実装は、いずれも行っていない。

## 28. Critical／High／Medium／Low

**Critical: 0件**

**High: 2件(いずれも本タスク新規実装のバグではなく、既存コードの監査で発見した
未接続領域。ユーザーの判断によりP25-006の対象範囲外として記録するに留める)**

1. **国家消滅時のDivision装備が凍結されず物理的に消失する**(§14/§20)。
   `country::elimination::apply_country_elimination`がP25-001の「凍結して残存」
   設計と矛盾し、消滅国家のDivisionが保有していた装備量が会計上消失する
   (世界装備総量の保存則が国家消滅を挟むと破れる)。修正には`country::elimination`
   への変更が必要(本タスクの宣言スコープ外、ユーザーが別タスクでの対応を選択)。
2. **撤退時追加manpower損失(`RETREAT_MANPOWER_LOSS_RATIO`)だけで、非ゼロ装備の
   Divisionが装備会計APIを経由せず未処理のまま削除される経路が実在する**(§19)。
   `resolve_participant_outcome`の撤退失敗分岐が装備を一切考慮せず
   `remove_division`を呼ぶため、その装備量も会計上消失する。P25-004時点から
   既知の追跡事項(「撤退時追加manpower損失は装備損失へ未接続」)の一部であり、
   本タスクの対象外(§15)として指示されている。

**Medium: 0件、Low: 0件**

## 29. 追加・変更したファイル

* `src/military/demobilization.rs`(装備返却の接続、`DemobilizationError::EquipmentReturnFailed`追加、`DemobilizationOutcome::returned_equipment`追加、既存テスト名を1件訂正)
* `src/ui/military_panel.rs`(新エラーバリアントのUI表示マッピング追加、1行)
* `tests/p25_006_demobilization_equipment_return_e2e_test.rs`(新規、29件)
* 本報告書(新規)

`src/country/elimination.rs`は一切変更していない(§20参照、git diff空)。

## 30. 実行した全コマンドと正確な結果

```bash
git rev-parse HEAD                          # 26a8629e793aa0d1451120433c85535f1b5d25e5
git status --short                          # M saves/savegame_v1.ron のみ
rg -n "demobil|despawn|Eliminated|RETREAT_MANPOWER_LOSS_RATIO" src tests
rg --files tests | sort | rg 'p23_006|p23_007|p23_008|p23_010|p24_002|p25_00'

cargo check --lib
cargo test --lib military::demobilization::     # 22 passed
cargo test --lib military::                     # 251 passed
cargo test --test p25_006_demobilization_equipment_return_e2e_test  # 29 passed
cargo test --test p23_006_manpower_ledger_e2e_test                  # 22 passed
cargo test --test p23_007_division_demobilization_e2e_test          # 24 passed
cargo test --test p23_008_daily_reinforcement_e2e_test              # 21 passed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test     # 9 passed
cargo test --test p24_002_workforce_rounding_unification_e2e_test   # 17 passed
cargo test --test p25_002_equipment_accounting_foundation_e2e_test  # 15 passed
cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test # 6 passed
cargo test --test p25_003_recruitment_equipment_issue_e2e_test      # 22 passed
cargo test --test p25_004_combat_equipment_loss_e2e_test            # 5 passed
cargo test --test p25_005_daily_equipment_reinforcement_e2e_test    # 10 passed

cargo test --all-targets                     # 1769 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings  # 0 warnings

# clippyがtype_complexityを1件検出したため#![allow(clippy::type_complexity)]を追加後、
# cargo test --all-targetsを再実行して最終確認(結果は同じ1769 passed/0 failed/1 ignored)

git checkout -- (非決定的PNG 9枚を個別に)
```

全て逐次実行。並列実行なし。`cargo clean`は使用していない(incremental cache破損の
兆候はなかった)。

## 31. 全テストpassed／failed／ignored

* 開始時基準: `1740 passed / 0 failed / 1 ignored`
* 最終`cargo test --all-targets`結果: **`1769 passed / 0 failed / 1 ignored`**
* 新規追加: **29件**(`tests/p25_006_demobilization_equipment_return_e2e_test.rs`)
* 既存テストの失敗: **0件**

## 32. clippy警告数

**0件**(`cargo clippy --all-targets --all-features -- -D warnings`)。

## 33. Cargo逐次実行

厳守。並列実行なし。

## 34. ユーザーセーブの保護

`saves/savegame_v1.ron`は全工程で未読み書き・未ステージ・未復元。

## 35. 監査対象コードの`git status --short`

```text
 M saves/savegame_v1.ron
 M src/military/demobilization.rs
 M src/ui/military_panel.rs
?? tests/p25_006_demobilization_equipment_return_e2e_test.rs
```

## 36. P25-007への引き継ぎ事項

* 装備充足率(`equipment / max_equipment`)を戦闘力(attack/defense)へどう反映するか
* `max_equipment == 0`のDivisionの扱い(ゼロ除算回避、`equip_ratio`を1.0扱いにする等)
* manpower充足率と装備充足率の合成方法(P25-001設計どおり、単一`min()`への統合を避けるか要検討)
* organization・movement・戦闘RNGへ影響させるかどうかの明確な線引き
* P25-004の戦闘損失・P25-005の日次補充・P25-006の復員返却をいずれも維持したうえで接続すること
* 撤退時追加manpower損失への装備連動は依然保留(§19/§28のHigh finding参照)
* **新規: 国家消滅時のDivision装備凍結の欠落(§20/§28のHigh finding)は、P25-007着手前に
  別タスクとして対応するかどうかユーザーが判断する必要がある**(戦闘力へ装備充足率を
  反映する場合、消滅国家関連の状態異常[削除済みDivisionへの参照等]が新たな問題を
  引き起こさないか、P25-007の設計時に併せて検討すべき)
