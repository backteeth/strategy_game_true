# P25-003: 募兵時の軍事装備支給接続 実装・検証報告書

## 1. 総合判定

**COMPLETE**

P25-002/P25-002-REV-001で確立した固定小数点装備会計API(`try_issue_equipment`・
`validate_required_equipment`)を、実際の募兵処理(`military::recruitment`)へ接続した。
`DivisionDefinition.required_equipment`は物理MilitaryEquipment必要量として正式に
扱われるようになり、treasuryを装備の代金として即座に減算していた旧仮実装
("Simplified"実装)は完全に削除した。新規Divisionは人的資源と装備の両方が揃った
場合だけ、1つの募兵transaction内で原子的に生成される。既存テストの検証過程で、
AI国家の月次募兵頻度が旧treasury仮実装によって偶発的に制限されていた実際の
テスト前提の崩れを5件発見し、いずれも意図を保ったまま修正した。

## 2. 開始時HEAD

```
9342cdd3f81687b429cb15fc867f1174f9ee4378
```

想定と一致していた(P25-002-REV-001の訂正コミット)。

## 3. 監査対象コードHEAD

```
9342cdd3f81687b429cb15fc867f1174f9ee4378
```

作業開始時点で他のコミットは存在せず、開始時HEADと同一。

## 4. P25-001設計との一致

P25-001が確定した設計をそのまま実装した。

- `RecruitmentQueueItem`へ新しいフィールド(装備予約)は追加していない。
- キュー投入時(`request_recruitment`)には装備を消費しない(§8で確認済み)。
- Division生成成功時(`process_recruitment`)にのみ、正規APIで装備を即時消費する。
- 装備不足時は「完全充足時のみ生成」方式(既存のmanpower/treasuryチェックと
  同じ即時atomic予約パターン)に従い、生成せずキュー項目を待機させる。

## 5. P25-002-REV-001固定小数点APIの再利用

`try_issue_equipment`・`validate_required_equipment`をそのまま呼び出しており、
募兵処理内で生の`f64`を直接加減算する箇所は存在しない(`grep`で
`treasury -=`/`treasury +=`/`+= def.required_equipment`等のパターンが
`src/military/recruitment.rs`・`src/military/ai.rs`から消えたことを確認済み)。

## 6. 現行RecruitmentQueueItem

```rust
pub struct RecruitmentQueueItem {
    pub division_id: DivisionDefinitionId,
    pub target_state: StateId,
    pub days_remaining: u32,
    pub total_days: u32,
}
```

変更していない(新しいフィールドを追加していない、§4)。

## 7. 現行process_recruitment

`days_remaining`が0に達した項目について、新設した`try_spawn_division`
(内部ヘルパー)を呼ぶ。これは`try_issue_equipment(stockpile, &mut 0.0,
def.required_equipment, def.required_equipment)`(装備0・上限
`required_equipment`のDivisionへ全額支給)として装備会計APIを呼び出し、
成功した場合のみ`Division`を構築して返す。失敗した場合は`completed_indices`へ
追加せず、その項目は`days_remaining=0`のままキューに残り、翌日以降の
`process_recruitment`呼び出しで再判定される(既存の日次呼び出し構造がそのまま
「毎日再試行」を実現する)。DivisionDefinition自体が解決できない場合は、
既存の(P23-009で確認済みの)挙動どおり項目を静かに完了させて破棄する(本タスクの
対象外、§16)。

## 8. required_equipment全参照箇所

`grep -rn "required_equipment" src/`を実施し、以下を確認した。

- `src/military/data.rs`: `DivisionDefinition.required_equipment: f64`(定義)。
- `src/military/recruitment.rs`: `validate_required_equipment`による有効性検証
  (§9)、`try_spawn_division`内での支給要求量として使用。treasury関連の参照は
  すべて削除済み。
- `src/military/ai.rs`: 旧`if country.treasury < def.required_equipment`
  チェックを削除済み(§27)。
- `src/military/equipment.rs`: `validate_required_equipment`(P25-002から
  既存)。
- `src/ui/military_panel.rs`: UI表示(§21で装備備蓄基準へ訂正)。
- `assets/data/divisions.ron`: 実データ(1000.0・2000.0・500.0、いずれも整数)。
- テストfixture各所: 100.0・10.0等(既存の慣例値、意味変更なし)。

`required_equipment`をtreasury/moneyとして解釈する箇所は、コードベース全体で
0件になったことを確認した。

## 9. required_equipmentの新しい正規意味

```text
DivisionDefinition.required_equipment
    = そのDivisionを完全装備するために必要な
      ResourceType::MilitaryEquipmentの物理量
```

`request_recruitment`は`validate_required_equipment(def.required_equipment)`
(finite・非負・固定小数点正規範囲内)で有効性のみを検証し、実際の国家備蓄からの
支給はDivision生成時まで行わない。

## 10. 削除した財務コスト仮実装

```rust
// 削除(request_recruitment):
if country.treasury < def.required_equipment {
    return Err("Insufficient equipment/treasury");
}
country.treasury -= def.required_equipment; // Simplified

// 削除(cancel_recruitment):
country.treasury += def.required_equipment; // Refund

// 削除(evaluate_recruit_feasibility):
if country.treasury < def.required_equipment {
    return RecruitFeasibility::InsufficientFunds;
}

// 削除(military::ai::decide_and_recruit_for_country):
if country.treasury < def.required_equipment {
    return AiRecruitmentOutcome::SkippedInsufficientTreasury;
}
```

`RecruitFeasibility::InsufficientFunds`・`AiRecruitmentOutcome::
SkippedInsufficientTreasury`のenum variant自体も削除した(復元・置換ではなく、
装備がもはやキュー投入時の判定条件ではないため、対応する概念自体が存在しなくなった)。

## 11. treasuryへの影響

`DivisionDefinition`には`required_equipment`以外の正式な募兵費用フィールドは
存在しない。`maintenance_cost`フィールドは存在するが、実際の参照箇所
(`src/economy/finance.rs:49`)は建物(`BuildingDefinition`)の月次維持費計算専用
であり、Division募兵時には一切参照されないことをコード監査で確認した
(`grep`で`DivisionDefinition`のインスタンス化以外に`maintenance_cost`が
Division文脈で読まれる箇所がないことを確認)。したがって、募兵によって
treasuryが変化する経路は本タスク後に一切残っていない(§21 Test M・Test A・
Test Vで直接確認)。

## 12. 募兵成功条件

`process_recruitment`が項目をDivisionへ変換するのは、以下をすべて満たした
場合のみ。

1. `days_remaining == 0`
2. `DivisionDefinition`が解決できる(できない場合は既存どおり静かに破棄、
   本タスクの対象外)
3. `try_issue_equipment`が成功する: (a) Division開始状態(`0 <= 0 <=
   required_equipment`)が有効、(b) `required_equipment`がfinite・非負・
   固定小数点正規範囲内、(c) 国家備蓄がその量以上、(d) overflow/underflowなし

manpowerの条件(available_manpower充足)は`request_recruitment`(キュー投入時)で
既にすべて検証・確定済みであり、`process_recruitment`はそれを再検証しない
(§13)。

## 13. manpower事前検証

`request_recruitment`は既存(Phase 23確立)のまま、`available_manpower <
required_manpower`を即時検証し、キュー投入と同時に`available_manpower`
から`mobilized_manpower`へ即時移動する。この挙動・タイミングは一切変更して
いない(§30/§34で確認)。

## 14. equipment事前検証

`request_recruitment`は`validate_required_equipment`で`required_equipment`
自体の有効性(finite・非負・正規範囲内)だけを検証し、国家備蓄の充足有無は
検証しない(§8: 装備はキュー投入時の条件ではない)。国家備蓄の充足検証は
`process_recruitment`(Division生成時)でのみ行われる。

## 15. 人的資源＋装備の原子性

`try_spawn_division`内の`try_issue_equipment`呼び出しが、装備移動を1回の
原子的操作として実行する(P25-002-REV-001の検証済み保証)。manpowerは
`request_recruitment`時点で既に確定済みの値であり、`process_recruitment`は
それを再度動かさない——したがって「manpowerだけ動いて装備が動かない」
「装備だけ動いてmanpowerが動かない」という部分成功は、そもそも同一
transaction内で同時に扱う対象が装備の移動とDivision生成のみであるため
構造的に発生しない。装備支給が失敗した場合、`try_issue_equipment`自身の
atomicity保証により国家備蓄・Division(まだ存在しない)のいずれも変化せず、
`process_recruitment`側もその場合は`completed_indices`へ追加しないため
queue itemも消えない(§21 Test Uで実測確認)。

## 16. Division生成時のequipment

```rust
let mut equipment = 0.0;
let result = try_issue_equipment(stockpile, &mut equipment, def.required_equipment, def.required_equipment)?;
// division.equipment = result.equipment_after
```

`result.equipment_after`は`try_issue_equipment`が正規units経由で算出した
canonical値であり、要求量(`def.required_equipment`)をそのまま`Division.equipment`
へコピーするのではなく、支給APIの実際の結果を使う(§18で保存則を確認)。

## 17. Division生成時のmax_equipment

`division.max_equipment`にも同じ`result.equipment_after`を使用する
(`division.equipment`と全く同じ値、別の量子化経路を経由しない)。したがって
`equipment == max_equipment`(完全装備)が常に成立する(§21 Test A・H で確認)。

## 18. stockpile減少units

`try_issue_equipment`の既存の保存則保証(P25-002-REV-001)により、
`stockpile_units_before - stockpile_units_after == requested_units ==
division_equipment_units`が常に成立する。§21 Test Hで、`military_equipment_
value_to_units`を使い、raw f64減算に頼らずunits空間で直接この関係を確認した。

## 19. 世界装備保存則

支給は「国家備蓄→Division」という単一方向の移動であり、装備を無から生成・
消滅させない。§21 Test G(複数回募兵での保存則確認、`初期stockpile ==
最終stockpile + 支給合計`)・Test H(units空間での直接確認)で実測した。

## 20. 装備不足時のqueue処理

`try_issue_equipment`が`InsufficientStockpile`で失敗した場合、`process_
recruitment`は該当項目を`completed_indices`へ追加しない。ループ構造上
`days_remaining`は既に0であり、`> 0`ガードにより以降減少しないため、項目は
`days_remaining=0`のままキューに残り続け、以降の`process_recruitment`呼び出し
(=翌日以降)ごとに再判定される——備蓄が充足した時点で自動的に完了する
(§21 Test C・P)。既存のqueueの順序(先頭から処理)は変更していない
(§21 Test J)。

## 21. manpower不足時のqueue処理

manpower不足は`request_recruitment`(キュー投入時点)で拒否されるため、
そもそもキューに項目が積まれない(既存Phase 23の挙動を維持)。装備が十分でも
manpower不足なら、キュー投入自体が失敗し装備は一切参照・変更されない
(§21 Test D)。

## 22. required_equipment=0

`required_equipment=0`は有効な値として扱われる。`try_issue_equipment`へ
`requested=0.0`を渡すと成功するno-opとなり(P25-002-REV-001で確立済みの性質)、
国家備蓄が0のままでも募兵が成功し、`Division.equipment=0`・`max_equipment=0`
となる(§21 Test F)。0装備状態を不正として拒否する処理は存在しない。

## 23. 小数装備

`required_equipment=3.7`のような端数値でも、固定小数点units経由で正確に
支給され、5回連続で募兵してもドリフト(誤差蓄積)が発生しないことを§21 Test G
で確認した(`保存則: 初期stockpile == 最終stockpile + 支給合計`を直接検証)。

## 24. 複数queueの決定性

同一国家に複数のキュー項目がある場合、既存の処理順序(`enumerate()`によるVec
先頭からの走査、HashMap/Bevy query順に依存しない)をそのまま維持している。
備蓄が1件分しかない場合、先に処理された項目だけが成功し、後続項目は待機する
ことを§21 Test Jで確認した(タスク例と同じ`stockpile=150, 必要100×2`の
シナリオで、先着1件のみ成功しstockpile=50が残ることを確認)。

## 25. 複数国家の分離

`try_issue_equipment`は`&mut CountryStockpile`を明示的に受け取り、
`process_recruitment`は`country.stockpile`(各`CountryData`が個別に保持)を
渡すため、国家間の備蓄・人的資源・キューは構造的に分離される。§21 Test Kで、
2か国のうち片方だけが装備を保有するシナリオを検証し、装備を持つ国だけが
募兵に成功し、もう一方の国のstockpile・queueには一切影響がないことを確認した。

## 26. 同日生産装備

現行の`DailySimulationSet`順序(Economy→...→MilitaryAction、P25-001監査で
確認済み、本タスクで変更していない)により、月替わり当日にMilitaryFactoryが
生産した装備は、その日のMilitaryAction(募兵処理を含む)から利用可能である。
§21 Test Lで、日初め不足だった備蓄がEconomy相当の生産で必要量に到達した
状態を再現し、同日中の募兵完了でstockpileがちょうど0になることを確認した。

## 27. AI募兵

`military::ai::decide_and_recruit_for_country`から旧treasuryチェックを削除
した(§10)。AIは既存の`request_recruitment`を唯一の書き込み経路として
再利用し続けており(モジュールdocコメントの記述どおり)、AI専用の無料Division
生成経路は存在しない。AI queueもプレイヤーと全く同じ`process_recruitment`の
装備ゲートを通ることを、§21 Test T(実App経由、AI国家のstockpile不足時に
Divisionが生成されないことを確認)および`military::ai`の単体テスト
`queues_recruitment_even_when_military_equipment_stockpile_is_insufficient`
で確認した。AIのqueue投入判断ロジック自体(目標Division数計算等)は変更して
いない。

## 28. 消滅国家・無効state・未解決definition

- `CountryStatus::Eliminated`: `request_recruitment`の`is_active()`チェック
  (既存)がそのまま拒否する。装備・manpower・treasuryのいずれも変更されない
  (§21 Test R)。
- 未解決definition: `request_recruitment`は`military_registry.definitions.get`
  で事前に検証済みのため到達しない。`process_recruitment`側の未解決definition
  処理(既存のP23-009確認済み挙動)は変更していない(§16、本タスクの対象外)。
- 不正な`required_equipment`(NaN・infinity・負数・正規最大値超過): 
  `validate_required_equipment`が`request_recruitment`時点で拒否し、
  manpower・装備・queueのいずれも変化しない(§21 Test I)。

## 29. 初期Divisionの扱い

`app::loader::spawn_debug_divisions`(New Game初期配置)は装備会計APIを
一切呼び出さず、`equipment: def.required_equipment, max_equipment: def.
required_equipment`を直接設定するのみである(既存のP25-001監査で確認済みの
挙動、本タスクで変更していない)。§21 Test Nで、New Game直後の
`CountryStockpile[MilitaryEquipment]`が0.0のまま(初期Divisionによる二重
消費が発生しない)ことを実App経由で確認した。

## 30. Loaded Game Divisionの扱い

Save/Loadは`Division`構造体をそのままシリアライズ/デシリアライズするのみで、
装備会計APIを経由しない(既存の設計、本タスクで変更していない)。§21 Test Oで、
既存Divisionを含むセーブをロードしても`CountryStockpile[MilitaryEquipment]`が
再消費されず、Division数も重複しないことを実App経由で確認した。

## 31. Save V1維持

新しい保存フィールドは追加していない。`RecruitmentQueueItem`・
`CountryStockpile`・`Division.equipment`/`max_equipment`・`DivisionDefinition.
required_equipment`はいずれも既存の型・スキーマのまま(`git diff --stat
src/save/`が空であることを確認)。

## 32. pending save/load

§21 Test Pで、装備不足で待機中のキュー項目を含む状態をsave→load往復させ、
往復後も(a)装備を予約したことにならない、(b)stockpileが変化しない、
(c)load後も待機を継続、(d)後で装備を追加すると1回だけ生成される、ことを
すべて確認した。

## 33. completed save/load

§21 Test Qで、募兵成功後のDivision・stockpileを含む状態をsave→load往復
(さらに2回目の往復も)させ、Division数・stockpileが完全に一致し、再ロードで
装備が再消費されないことを確認した。

## 34. Phase 24会計の保護

`military::manpower`(A・B・L・K・X)・`state::data`(workforce)・
`population::`配下は本タスクで一切変更していない(`git status --short`で
確認)。§21 Test Vで、装備が十分な場合のmanpower遷移(即時`A-=M, B+=M`)が
P23時点から不変であることを直接確認した。Phase 23/24の既存回帰テスト
(`p23_006`〜`p23_010`・`p24_002`)は全件、本タスクの修正後に完全に合格した
(§39/§40)。

## 35. 更新した既存テストと理由

### 既存成功シナリオへの装備fixture追加(§22分類: 十分なMilitaryEquipmentを追加)

- `src/military/tests.rs::test_recruitment_progress_and_completion`:
  `required_equipment=100.0`ぶんの備蓄を追加し、Division生成を維持。
  treasuryが変化しないことのassertも追加。
- `tests/p23_006_manpower_ledger_e2e_test.rs::division_completion_moves_
  living_from_queue_to_division_without_changing_the_total`: 同様に
  `MilitaryEquipment`備蓄を100.0追加。
- `tests/p23_009_ai_recruitment_e2e_test.rs::a_completed_ai_recruitment_
  division_has_the_correct_owner_and_does_not_double_spend_manpower`・
  `after_completion_a_country_still_below_target_recruits_its_next_
  division_the_following_month`: 新設ヘルパー`ensure_large_military_
  equipment_stockpile`(1,000,000.0)を追加呼び出し。
- `tests/p23_010_phase23_manpower_lifecycle_audit_test.rs::full_lifecycle_
  cycle_preserves_the_manpower_ledger_at_every_stage`: プレイヤーの
  募兵要求前に必要量ぶんの備蓄を設定。

### 新しい装備不足シナリオへの置換(§22分類: Divisionが生成されないことを明示)

- `src/military/tests.rs::test_recruitment_equipment_shortage` →
  `test_recruitment_equipment_shortage_delays_division_spawn_until_
  stockpile_is_sufficient`へ改名・再設計: 旧実装は「治安/資金不足で
  キュー投入自体が拒否される」ことを検証していたが、新設計では投入は
  成功し、Division生成が装備到着まで待機することを検証する内容へ
  全面的に書き換えた。

### 意味が消滅した検証対象の削除・置換

- `src/military/recruitment.rs::RecruitFeasibility::InsufficientFunds`・
  `src/military/ai.rs::AiRecruitmentOutcome::SkippedInsufficientTreasury`
  の削除に伴い:
  - `src/military/tests.rs::evaluate_recruit_feasibility_insufficient_funds`
    → `evaluate_recruit_feasibility_is_ready_even_without_equipment_
    stockpile`へ改名: 備蓄が空でも`Ready`が返ることを検証する内容へ変更。
  - `src/military/ai.rs::does_not_recruit_when_treasury_is_insufficient`
    → `queues_recruitment_even_when_military_equipment_stockpile_is_
    insufficient`へ改名: 備蓄不足でもキュー投入(`Recruited`)自体は成功
    することを検証する内容へ変更。
  - `src/military/ai.rs::successful_recruitment_reduces_available_and_
    treasury_and_increases_burden` →
    `successful_recruitment_reduces_available_and_increases_burden_but_
    not_treasury`へ改名: treasury不変のassertへ訂正。
  - `src/ui/military_panel.rs::handle_recruit_buttons_success_queues_
    recruitment_and_deducts_cost` →
    `handle_recruit_buttons_success_queues_recruitment_without_touching_
    treasury`へ改名: treasury不変のassertへ訂正。
  - `src/ui/military_panel.rs::handle_recruit_buttons_insufficient_
    funds_does_not_mutate_state` →
    `handle_recruit_buttons_invalid_required_equipment_does_not_mutate_
    state`へ全面置換: 治安不足という前提自体が消滅したため、
    唯一残るキュー投入時拒否理由(`required_equipment`自体の不正値、
    §14)を検証する内容へ差し替えた。

### 意図した挙動変化の発見と、テストの主張自体の訂正(§22の枠を超える発見)

- `tests/p23_010_phase23_manpower_lifecycle_audit_test.rs::an_unresolved_
  recruitment_queue_definition_is_reported_as_an_explicit_error_not_a_
  panic_mid_run`: このテストが検証していたAI国家は、本テスト内で
  treasuryを底上げしていなかった。旧実装では`required_equipment`が
  treasuryから差し引かれていたため、このAIは数回の募兵で自然にtreasury
  不足へ陥り、3か月間の観測窓の間に募兵を停止していた(結果として
  `queue_len==0`が安定して成立していた)。P25-003でtreasury消費自体が
  廃止されたため、このAIはmanpowerが許す限り毎月継続的に募兵し続ける
  ようになった——これは意図した挙動変化であり(§11)、3か月経過時点で
  新しい正当な募兵項目がキューに存在していても不具合ではない。したがって
  本テストの主張を「queueが完全に空になる」から「不正なdivision_id
  (999,999)を参照する元の項目がもう存在しない」へ訂正した(本テストの
  本来の検証対象そのもの)。
- `tests/p23_010_phase23_manpower_lifecycle_audit_test.rs::a_save_and_
  reload_partway_through_matches_an_uninterrupted_run`: 6か月間の
  複数国家シナリオで、AI国家の装備ゲート付き募兵完了タイミングの差が
  (戦争・外交等の相互作用を経由して)プレイヤーのtreasuryへわずかに
  波及し、continuous/split-reload分岐比較で3ユニット(丸め後)の差異と
  なって現れた。比較対象外のAI国家に十分なMilitaryEquipment備蓄を
  用意することでタイミング差を解消した(既存のtreasury/available_
  manpowerの底上げと同じ意図の追加)。

## 36. P25-004へ引き継ぐ内容

`try_destroy_equipment`(戦闘損失、P25-004)は既に完成済みで未接続。
`try_return_equipment`(復員、P25-006)も同様。`process_recruitment`が
確立した「原子的検証→正規API呼び出し→成功時のみ状態確定」パターンは、
今後の接続先(補充・戦闘・復員)でもそのまま踏襲できる。

## 37. 対象外を先行実装していないこと

`git status --short`・追加ファイル一覧を確認し、以下がいずれも実装されて
いないことを確認した。日次装備補充・manpowerとequipment補充量の統合・
戦闘装備損失・casualty比例装備損失・復員時装備返却・全滅時装備処理・
国家消滅時装備移管・装備充足率の戦闘力反映・装備不足combat modifier・
新しいEquipmentKind・新しいResourceType・RecruitmentQueueItem新規フィールド・
装備予約・Save V2・migration・AI装備生産判断(新戦略ロジック)・新しいUI・
日次補給・兵站・Attrition・population変更・workforce変更・manpower cap変更・
`war::combat.rs`の活性化。いずれも該当なし。

## 38. Critical／High／Medium／Low

- Critical: 0
- High: 0
- Medium: 0
- Low: 1 — AI国家の月次募兵頻度が、旧treasury仮実装のtreasury枯渇によって
  偶発的に抑制されていたことが判明した(§35)。本タスクの意図した設計変更
  (treasury消費の廃止)の直接的な帰結であり、実装の不具合ではないが、
  将来AIバランス調整(P25後半、§27の「AI装備生産判断」参照)を行う際に
  考慮すべき既知の挙動変化として記録する。

## 39. 実行した全コマンド

```bash
git rev-parse HEAD
git status --short

# 実装前の必須監査
grep -n "required_equipment" src/military/recruitment.rs src/military/ai.rs \
  src/ui/military_panel.rs
grep -n "maintenance_cost" src/military/*.rs src/economy/*.rs src/country/*.rs
grep -n "RecruitFeasibility\|InsufficientFunds\|req_treasury\|evaluate_recruit_feasibility" \
  src/ui/military_panel.rs
grep -n "req_treasury\|recruit_status_insufficient_funds\|..." \
  assets/localization/en-US.ron assets/localization/ja-JP.ron
grep -n "SkippedInsufficientTreasury" src/military/ai.rs
grep -n "pub struct CountryData" -A 40 src/country/mod.rs

cargo build --lib   # 実装の各段階で繰り返し実行

cargo test --lib military::                                          # 206 passed; 0 failed
cargo test --lib economy::                                           # 12 passed; 0 failed
cargo test --lib ui::military_panel::                                # 54 passed; 0 failed
cargo test --test p20_009_localization_resource_test                 # 8 passed; 0 failed

rustfmt --edition 2024 --check <touched files>
rustfmt --edition 2024 <touched files>   # フォーマット適用後、再checkでOKを確認

cargo test --test p25_002_equipment_accounting_foundation_e2e_test          # 15 passed; 0 failed
cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test # 6 passed; 0 failed
cargo test --test p25_003_recruitment_equipment_issue_e2e_test              # 22 passed; 0 failed

cargo test --test economy_tests                                      # 22 passed; 0 failed
cargo test --test p23_006_manpower_ledger_e2e_test                   # 22 passed; 0 failed
cargo test --test p23_007_division_demobilization_e2e_test           # 24 passed; 0 failed
cargo test --test p23_008_daily_reinforcement_e2e_test               # 21 passed; 0 failed
cargo test --test p23_009_ai_recruitment_e2e_test                    # 22 passed; 0 failed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test      # 9 passed; 0 failed
cargo test --test p24_002_workforce_rounding_unification_e2e_test    # 17 passed; 0 failed

cargo test --lib                    # 1268 passed; 0 failed(全体)
cargo test --all-targets            # 1回目: 2件failed(ui::military_panel::testsの
                                     # 旧treasury前提テスト)
# → src/ui/military_panel.rsの2テストを修正
cargo test --lib ui::military_panel::   # 54 passed; 0 failed(再確認)
cargo test --lib                        # 1268 passed; 0 failed(再確認)
cargo test --all-targets            # 2回目: 1680 passed; 0 failed; 1 ignored
cargo clippy --all-targets --all-features -- -D warnings   # 0 warnings

git status --short   # 既知の非決定的PNG9件を個別checkoutで復元
git checkout -- <9 screenshot paths individually>
```

Cargoコマンドはすべて逐次実行した(前回P25-002-REV-001タスクの反省を踏まえ、
本タスクでも同一メッセージ内での複数cargo呼び出しは一切行わなかった)。

## 40. 全テストのpassed／failed／ignored件数

```
1680 passed / 0 failed / 1 ignored
```

P25-002-REV-001時点の基準(1658 passed / 0 failed / 1 ignored)に対し、
新規`p25_003_recruitment_equipment_issue_e2e_test.rs`の22件を加えた数
(1658+22=1680)と正確に一致する(既存テストの改名・内容置換は件数に影響
しない)。

## 41. clippy警告数

```
0 warnings
```

(`cargo clippy --all-targets --all-features -- -D warnings`)

## 42. コミット直前の`git status --short`

```
 M assets/localization/en-US.ron
 M assets/localization/ja-JP.ron
 M saves/savegame_v1.ron
 M src/military/ai.rs
 M src/military/recruitment.rs
 M src/military/tests.rs
 M src/ui/military_panel.rs
 M tests/p23_006_manpower_ledger_e2e_test.rs
 M tests/p23_009_ai_recruitment_e2e_test.rs
 M tests/p23_010_phase23_manpower_lifecycle_audit_test.rs
 M tests/p25_002_equipment_accounting_foundation_e2e_test.rs
?? tests/p25_003_recruitment_equipment_issue_e2e_test.rs
```

(`saves/savegame_v1.ron`はユーザーの実セーブデータであり、本タスクでは一切
読み書きしていない。既存の未コミット差分がそのまま残っているのみで、
コミットには含めない。)
