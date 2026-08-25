# P23-005 監査報告: 募兵・補充・戦闘損耗・解散における人的資源ライフサイクル

## 1. 監査判定

**AUDIT COMPLETE**

主要な処理経路(募兵、部隊完成、戦闘損耗、部隊解散/撃破、動員解除、国家消滅、save/load)は
すべて実コードを直接読み、根拠となるファイル・関数・行を特定できた。到達不能な経路は
存在しなかった。

## 2. システム健全性

**INCONSISTENT**

`CountryData.mobilized_manpower`のフィールド自体のドキュメントコメント(「動員済みの
人的資源（軍隊に所属している合計）」)が明示する不変条件を、実コードは維持していない。
戦闘損耗・部隊撃破・部隊撤退失敗による消滅のいずれでも`mobilized_manpower`は一切減算
されず、これは特定の稀なエッジケースではなく、**戦争が起きて誰か一人でも戦死すれば
即座に発生する、通常プレイ経路上のずれ**である。加えて、補充・動員解除(個別部隊)・
Attritionは経路自体が存在しない(MISSING FEATURE)。これらを「PARTIALLY CONSISTENT」
として丸めず、`INCONSISTENT`と判定する。

## 3. 基準状態

* 開始時HEAD: `76b37bebaf15e405bc7f2892e047136f489af053`(P23-004完了)
* `git log -5 --oneline`は基準状態の説明と一致(`76b37be`→`697b0ec`→`1556ef4`→`eb825fb`→`5f0054d`)、`git status --short`は`saves/savegame_v1.ron`の変更のみ。重大な不一致なし。
* `saves/savegame_v1.ron`は監査中一切参照・変更・復元・整形・ステージ・コミットしていない。
* 監査中、本番Rustコード・テストコードは一切変更していない(調査は`Read`/`Grep`/`Bash`(`rg`相当の`grep`、`cargo test -- --list`)のみで実施)。

## 4. データモデル一覧

| 名称 | 種別 | 所在 | 役割 |
|---|---|---|---|
| `CountryData.available_manpower: u64` | フィールド | `src/country/mod.rs` | 募兵に使用できる未動員の人的資源 |
| `CountryData.mobilized_manpower: u64` | フィールド | `src/country/mod.rs` | ドキュメント上は「軍隊に所属している合計」。実際は「募兵コマンドで加算され、募兵キャンセルでのみ減算される、未解消の累計」 |
| `CountryData.recruitment_queue: Vec<RecruitmentQueueItem>` | フィールド | `src/country/mod.rs` | 完成待ちの募兵予約 |
| `RecruitmentQueueItem{division_id, target_state, days_remaining, total_days}` | 型 | `src/military/recruitment.rs` | 1件の募兵予約 |
| `Division{manpower, max_manpower, organization, max_organization, status, owner, ...}` | 型/Component相当(HashMap管理) | `src/military/data.rs` | 実在する1個の部隊 |
| `MilitaryRegistry{divisions: HashMap<DivisionId,Division>, definitions, next_division_id}` | Resource | `src/military/data.rs` | 全部隊の実体を保持 |
| `DivisionDefinition{required_manpower, required_equipment, recruitment_days, ...}` | 型 | `src/military/data.rs` | 部隊種別ごとの募兵コスト・完成日数 |
| `Army{owner, member_division_ids}` / `ArmyRegistry` | 型/Resource | `src/military/army.rs` | 複数Divisionの「編成」というグルーピングのみ。人的資源とは無関係 |
| `StateData.population: u64` | フィールド | `src/state/data.rs` | 州人口(P23-001で毎月成長のみ、減少経路なし) |
| `StateData.workforce_ratio` / `total_workforce()` / `civilian_workforce()` | フィールド/メソッド | `src/state/data.rs` | P23-001〜003の労働力指標 |
| `population::growth::compute_manpower_cap` | 純粋関数 | `src/population/growth.rs` | 所有人口の10%を人的資源上限とする |
| `population::growth::recover_manpower` | 純粋関数 | `src/population/growth.rs` | `available_manpower`を毎月上限へ向けて回復(上限低下時に切り下げない) |
| `population::mobilization::apportion_mobilized_manpower` | 純粋関数 | `src/population/mobilization.rs` | 国家の`mobilized_manpower`を所有州へ人口比按分 |
| `MonthChangedMessage` / `DayChangedMessage` | イベント | `src/app/time.rs` | 月次・日次処理の起点 |
| `BattleRegistry` / `Battle{attacker_division_ids, defender_division_ids, ...}` | Resource/型 | `src/military/battle.rs` | 進行中の戦闘 |
| `DivisionStatus::{Idle,Moving,Fighting,Occupying,Retreating,Disbanding,Destroyed}` | enum | `src/military/data.rs` | 部隊状態(`Disbanding`/`Destroyed`は事実上未使用箇所あり、後述) |

## 5. 処理経路

### 5.1 募兵(request_recruitment)

`src/military/recruitment.rs:74-109`。

* 入口: `src/ui/military_panel.rs:1500`(プレイヤーのボタン押下)のみ。AI側からの呼び出しは無い(§9参照)。
* `country.is_active()`(消滅国家を拒否)→`available_manpower >= required_manpower`→`treasury >= required_equipment`の順に検証。
* 検証通過後、**同一関数内で原子的に**:
  ```text
  available_manpower -= required_manpower   (行97)
  mobilized_manpower += required_manpower   (行98)
  treasury           -= required_equipment  (行99)
  recruitment_queue へ RecruitmentQueueItem を push (行101-106)
  ```
* 人的資源は「募兵コマンドを出した時点」で即座に消費される(部隊完成時ではない)。
* 同一フレーム内で複数回募兵しても過剰消費は起きない: `handle_recruit_buttons`(`src/ui/military_panel.rs`)は`Changed<Interaction>`ごとに`request_recruitment`を順に呼び、各呼び出しが直前の呼び出しで更新済みの`available_manpower`を再読するため、逐次的に正しく検証される。
* キャンセル(`cancel_recruitment`、同ファイル111-133行)は`available_manpower += required_manpower`・`mobilized_manpower = mobilized_manpower.saturating_sub(required_manpower)`・`treasury += required_equipment`で全額返還する。**ただしキャンセルできるのは`recruitment_queue`にまだ残っている(未完成の)予約のみ**であり、完成済み部隊はこの経路の対象外。

### 5.2 部隊完成・配備(process_recruitment)

`src/military/recruitment.rs:16-72`。`src/military/update.rs:30-33`(`handle_daily_military`内、日次処理の最初のステップ)から全国家に対して毎日呼ばれる。

* `recruitment_queue`の各項目の`days_remaining`を1減らし、0になった項目から`Division`を新規生成する。
* 新規`Division.manpower = def.required_manpower`・`max_manpower = def.required_manpower`——募兵時に予約した人数とここで一致する。
* **人的資源の追加消費は一切ない**(`available_manpower`/`mobilized_manpower`はこの関数内で読まれることすらない)。これは正しい: 消費は§5.1の時点で既に完了しているため、ここで再度消費すれば二重消費になる。
* 完成した`Division`は`military_registry.add_division(division)`でIDを採番されて追加される(実DivisionIdは§5.1のRecruitmentQueueItemには含まれておらず、この時点で初めて割り当てられる)。

### 5.3 補充(reinforcement)

**未実装。** リポジトリ全体を`reinforce|replenish`(大文字小文字無視)で検索した結果、
唯一のヒットは`src/military/invasion.rs:140`のログメッセージ「Division {:?} joined ongoing
battle {:?}...as reinforcement」であり、これは「進行中の戦闘へ新たな部隊が合流した」という
意味のログであって、既存部隊の兵員を最大値へ向けて補充する仕組みではない。`Division.manpower`
を`max_manpower`へ近づける処理はコードベースのどこにも存在しない。一度減った部隊の兵員は
(新規に別部隊を募兵する以外に)二度と回復しない。

### 5.4 戦闘損耗

**重要な訂正**: このリポジトリには外見上「本番コードらしい」戦闘関数が2つ存在するが、
実際にDailySimulationSetへ登録され稼働しているのは片方だけである。

* `src/war/combat.rs::process_combat(military_registry, war_registry)`——1対1の単純な
  ペアワイズ戦闘。`src/war/tests.rs`からのみ呼ばれており、`grep -rn "add_systems"`で
  `war/mod.rs`を確認した結果、**このSystemはどこにも登録されていない(未使用の死んだ
  コード)**。損耗ペアの構築が`military_registry.divisions.values()`(`HashMap`、既定の
  ランダムシードハッシャー)の走査順に依存しており、もし将来これがSystemとして配線
  された場合、同一州に3か国以上の部隊が存在するケースでプロセス実行ごとに結果が
  変わり得る(非決定性のリスクを内包した死んだコード)。
* **実際に稼働している経路**: `src/military/update.rs::handle_daily_military`
  (`src/military/mod.rs`で`DailySimulationSet::MilitaryAction`へ登録)→
  `process_daily_battles`(同ファイル75-177行)→
  `combat_calc::resolve_combat_day_multi`(`src/military/combat_calc.rs`)。
  * `BattleId`昇順(`battle_ids.sort_by_key(|id| id.0)`)で決定的に処理する。
  * 各バトルにつき攻撃側・防御側の参加`Division`を**クローンしてから**まとめて
    `resolve_combat_day_multi`へ渡し、結果(`manpower_loss`・`org_loss`のペア)を
    まとめて受け取ってから実`Division`へ適用する——同一tick内で「先に処理された
    ペアの結果が後続ペアの計算に影響する」順序依存は生じない設計になっている。
  * 適用: `division.manpower = division.manpower.saturating_sub(manpower_loss)`
    (161・167行)、`organization`も同様に`saturating_sub`相当(`max(0.0)`)。
  * **`country.mobilized_manpower`・`available_manpower`はこの関数内で一切参照・
    変更されない。**
* `resolve_finished_battles`(同ファイル183-291行、`process_daily_battles`の直後に
  同じ`handle_daily_military`内で呼ばれる)→`resolve_participant_outcome`
  (298-349行)が、参加師団ごとに継続可否を判定する:
  * `manpower == 0` → `military_registry.remove_division(division_id)`
    (実際にHashMapからEntity相当のレコードを削除する。ゴーストとして残らない)。
  * `organization == 0.0`で退路が無い、または撤退時追加損耗
    (`RETREAT_MANPOWER_LOSS_RATIO = 0.1`、`max_manpower`の10%相当)を差し引いた後の
    残存兵員が0以下 → 同じく`remove_division`で完全撤去(「包囲撃破」)。
  * 撤退成立時は`manpower = remaining_manpower`(追加損耗差引後)のまま生存継続。
  * **いずれの分岐でも`country.mobilized_manpower`・`available_manpower`は一切
    参照・変更されない。**

### 5.5 Attrition・非戦闘損耗

**未実装。** `attrition`(大文字小文字無視)でリポジトリ全体を検索した結果、
本番コード・テストコードともに一致無し。補給切れ・悪天候・過剰駐留などによる
非戦闘損耗の仕組みは存在しない。

### 5.6 部隊解散・削除

3つの異なる「解散/削除」概念が存在し、混同しやすいため明確に区別する。

1. **編成(Army)の解散**(`src/military/army.rs::ArmyRegistry::disband`、211-225行):
   UIの「Disband」ボタン(`src/ui/military_panel.rs`)から呼ばれる。これは
   **複数Divisionをまとめる「編成」というグルーピングを解くだけ**であり、
   個々の`Division`エンティティは一切削除されず、`Division.manpower`も
   `CountryData`の人的資源フィールドも一切変更されない。「部隊(Division)を
   解散して人的資源を返還する」というプレイヤー操作は**存在しない**。
2. **戦闘による撃破**(§5.4参照): `military_registry.remove_division`で実際に
   削除される。返還処理は一切ない(生存兵員・戦死者いずれも人的資源へ戻らない)。
3. **戦後の孤立部隊処理**(`src/war/peace.rs::relocate_hostile_territory_divisions`、
   176-254行、講和成立後に呼ばれる): 敵地に取り残され、隣接する自国領土も
   自国領土そのものも一切無い部隊は`status = DivisionStatus::Destroyed`・
   `manpower = 0`とマークされる**が、`remove_division`は呼ばれずレコードは
   HashMapに残り続ける**。これは②の「実削除」と一貫しない別の「死」の表現
   であり、以降のあらゆる「生存判定」が`division.manpower == 0 ||
   division.status == DivisionStatus::Destroyed`という2条件のORで書かれている
   理由がここにある(②では②のケースを拾うためにmanpower==0だけで十分だが、
   ③のような「レコードは残るが死んでいる」ケースを拾うにはstatusの確認も
   必要)。いずれの経路でも人的資源フィールドへの影響は無い。
4. **国家消滅時の一括解散**(§5.8参照)。

### 5.7 動員解除・戦争終了・講和

* **国家全体の「動員解除」ボタン/コマンドは存在しない。** `mobilized_manpower`が
  減る経路は§5.1の`cancel_recruitment`(募兵キュー未完成分のキャンセル)のみ。
* 部隊を維持したまま`mobilized_manpower`だけ減らす経路は無い(そもそも
  「維持したまま減らす」の逆に近い状態、すなわち「部隊が消えても
  `mobilized_manpower`が減らない」状態が常態化している——§8 F1参照)。
* 戦争終了・講和による自動動員解除は存在しない。`war/peace.rs`は占領地の
  返還・州所有権の移転・孤立部隊の帰還処理は行うが、`mobilized_manpower`/
  `available_manpower`には一切触れない。
* 生存兵員・損耗兵員・募兵中兵員の区別: `mobilized_manpower`という単一の
  カウンタしか存在せず、これらを区別する内訳データは無い。
* 民間労働力の回復: `civilian_workforce()`は`employed_workforce`+
  `unemployed_workforce`(P23-002で確定)であり、これは`economy::handle_monthly_economy`
  内で毎月`population::mobilization::apportion_mobilized_manpower`(現在の
  `country.mobilized_manpower`を使用)から再計算される。したがって仮に
  `mobilized_manpower`が正しく減っていれば、その**翌月の月次経済処理**で
  民間労働力が回復する設計である(P23-002/003で確認済み)。

### 5.8 国家消滅(country::elimination)

`src/country/elimination.rs::apply_country_elimination`、350行台。

* 消滅国家の`recruitment_queue`をクリアする(355-357行、Save検証が募兵中項目の
  残存を拒否するため)。
* 消滅国家が所有する全`Division`を`military_registry.remove_division`で実削除
  (359-369行、`divisions_disbanded`カウントとしてテストで検証済み——
  `country::elimination::tests::apply_disbands_all_remaining_divisions_of_the_eliminated_country`)。
* `army_registry.sanitize_references`で空になった編成を自動解散。
* 進行中戦闘・占領支配の整合も同トランザクション内で処理する。
* **消滅国家の`available_manpower`・`mobilized_manpower`は一切変更されない**
  (フィールドはそのまま凍結される。`CountryStatus::Eliminated`により
  `is_active()`ゲートで以後の募兵操作からは拒否されるため実害は無いが、
  値そのものは無意味なまま残り続ける)。
* **併合側(annexer)の`available_manpower`・`mobilized_manpower`にも変化は
  無い**(敵軍を吸収して自国の人的資源が増える、という設計は存在しない)。

### 5.9 Save/Load V1

* `CountryData`(`available_manpower`・`mobilized_manpower`含む全フィールド)は
  `#[derive(Serialize, Deserialize)]`により丸ごとシリアライズされ、
  `src/save/apply.rs`で`world.insert_resource(CountryRegistry { countries: ... })`
  として**そのまま復元**される(再計算・再集計は無い)。
* `mobilized_manpower`は`#[serde(default)]`(デフォルト0)、`available_manpower`は
  `#[serde(default = "default_available_manpower")]`(デフォルト100,000)——
  旧V1セーブに欠落していても読み込み自体は失敗しない。
* `MilitaryRegistry::from_saved_parts`(`src/save/apply.rs:391,700`)で
  `Division`群もそのまま復元される。
* `src/save/validate.rs`を"manpower"で検索した結果、実際に検証しているのは
  募兵定義の存在チェック(`unknown_recruitment_division_definition_is_rejected`)
  や消滅国家の募兵キュー空チェックのみで、**`mobilized_manpower`と実際の
  Division兵員合計が一致するかを検証するロジックは存在しない**。
* 結論: セーブ前に生じていた`mobilized_manpower`とDivision合計のずれ
  (§8 F1)は、save/loadを経ても**そのまま維持される**(悪化も改善もしない)。
  ロード時の再集計・不整合検出のどちらも無い。

## 6. 状態遷移表

| イベント | available | mobilized | 部隊現在兵員 | 州人口 | 民間労働力 | 実装箇所 | 判定 |
|---|---:|---:|---:|---:|---:|---|---|
| 募兵開始 | `-= required_manpower` | `+= required_manpower` | 変化なし(未生成) | 変化なし | 変化なし | `military/recruitment.rs:97-98` | CONFIRMED CORRECT |
| 部隊完成 | 変化なし | 変化なし | `= required_manpower`(新規) | 変化なし | 変化なし | `military/recruitment.rs:34-59` | CONFIRMED CORRECT(二重消費なし) |
| 補充 | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | — | 未実装 |
| 戦闘損耗 | 変化なし | **変化なし(要修正候補)** | `saturating_sub(manpower_loss)` | 変化なし | 変化なし | `military/update.rs:159-170` | CONFIRMED DEFECT(F1) |
| 部隊撃破/撤退失敗 | 変化なし | **変化なし(要修正候補)** | Divisionレコード削除 | 変化なし | 変化なし | `military/update.rs:307-346` | CONFIRMED DEFECT(F1) |
| Attrition | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | — | 未実装 |
| 編成(Army)解散 | 変化なし | 変化なし | 変化なし(Division自体は無傷) | 変化なし | 変化なし | `military/army.rs:211-225` | CONFIRMED CORRECT(ただし「部隊個体の解散」機能は未実装) |
| 動員解除(国家全体) | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | — | 未実装 |
| 募兵キャンセル | `+= required_manpower` | `-= required_manpower`(saturating) | 変化なし(未生成のまま消滅) | 変化なし | 変化なし | `military/recruitment.rs:125-129` | CONFIRMED CORRECT |
| 国家消滅 | 変化なし | **変化なし(不明瞭)** | 全Divisionレコード削除 | 変化なし | 変化なし | `country/elimination.rs:355-369` | CONFIRMED DEFECT寄り(消滅国家分は実害小、DESIGN UNDECIDED寄りの側面もあり) |
| Save/Load | そのまま復元 | そのまま復元(ずれも保持) | そのまま復元 | そのまま復元 | 再計算(月次のみ) | `save/apply.rs` | CONFIRMED CORRECT(復元自体は正しいが、既存のずれは検出されない) |

## 7. 不変条件の判定

### 候補A: `available_manpower + mobilized_manpower <= manpower_cap`

**成立しない(保証なし)。** 根拠:

* 実データ`assets/data/countries.ron`は`available_manpower`を明示せず、全国家が
  `default_available_manpower() = 100_000`(`src/country/mod.rs:77-79`)から開始する。
  一方`manpower_cap`は所有人口の10%(`compute_manpower_cap`)であり、州最小人口が
  25,000程度の国家では真の上限が100,000を大きく下回り得る——**ゲーム開始時点で
  既に超過し得る**。
* `recover_manpower`(`population/growth.rs`)は上限が現在値を下回った場合でも
  既存の余剰を削らない設計であり、これは既存テスト
  `recover_manpower_does_not_delete_a_surplus_when_the_cap_drops_below_current`
  で意図的に固定されている(バグではなく設計判断)。
* `mobilized_manpower`側にも上限チェックは無い(`request_recruitment`は
  `available_manpower`の残高のみ検証し、`mobilized_manpower`が`cap`を超えるか
  どうかは一切見ない)。

この不変条件を「常に成立すべきもの」として今後扱うかどうかはユーザー判断が必要
(DESIGN UNDECIDED、§13参照)。

### 候補B: `mobilized_manpower == 国家が所有する全Divisionの現在兵員合計`

**成立しない(確認されたバグ)。** `mobilized_manpower`は募兵コマンド発行時に
一度加算されるだけで、その後の戦闘損耗・部隊撃破では一切減算されない
(§5.4・§5.6参照)。フィールド自身のコメント「動員済みの人的資源（軍隊に
所属している合計）」がこの等式を意図していたことを示しているが、実装は
それを満たしていない。差分は「これまでに戦闘等で失われた累計兵員数」を意味する
ことになるが、その値自体はどこにも記録されておらず、遡って再構築することもできない
(死者数の記録が存在しないため)。募兵キュー・予備役・駐屯兵といった「正当な
不一致理由」は無く、単純に同期処理が欠落している。

### 候補C: 人的資源の保存則

* 募兵: `募兵前available = 募兵後available + 新規動員兵員`——**成立する**
  (`available_manpower -= def.required_manpower`のみの単純減算、他の同時変更なし)。
* キャンセル: `解散前mobilized = 解散後mobilized + 返還された兵員`——**成立する**
  (`saturating_sub`だが、`mobilized_manpower`はこの経路以外で減らないため
  アンダーフローは実質発生しない)。
* 「解散(部隊個体)」自体が存在しないため、「生存兵員だけを返還する解散」の
  保存則は**該当なし**。
* 戦死者(戦闘損耗・撃破)は、どの保存則の記録にも現れない——`Division.manpower`
  という「今この瞬間の値」からただ消えるだけで、`mobilized_manpower`側にも
  他のどのフィールドにも転記されない。

### 候補D: 他国非波及

**成立する。** `available_manpower`・`mobilized_manpower`は`CountryData`ごとの
フィールドであり、`request_recruitment`/`cancel_recruitment`/戦闘処理の
いずれも自国(または直接の交戦相手の`Division`)以外を変更するコードパスは
存在しない。`apportion_mobilized_manpower`は国家ごとに独立して計算され、
既存のP23-002統合テスト(`one_countrys_mobilization_does_not_affect_another_
countrys_employment`等)で実経路検証済み。戦闘は交戦国同士が互いの`Division`を
変更するが、これは「無関係な第三国」への波及ではなく戦闘の直接の結果であり、
本候補が問題にする非波及性とは別の話である。

### 候補E: 決定性

**実際に稼働している経路では成立する。** `military::update::process_daily_battles`
は`BattleId`昇順で処理し、各バトルの参加`Division`をクローンしてから結果を
まとめて適用するため、同一tick内の処理順序が最終結果へ影響しない設計になって
いる。`population::mobilization::apportion_mobilized_manpower`もP23-002で
`StateId`昇順のタイブレークが検証済み。

ただし、**未使用の`war::combat::process_combat`(§5.4)が仮に将来Systemとして
配線された場合は成立しない**——`military_registry.divisions.values()`という
`HashMap`(既定のランダムシードハッシャー)の走査順に、州内の交戦ペア処理順序が
依存する。現状は死んだコードのため実害は無いが、リスクとして記録する。

## 8. 戦死者と人口・民間労働力の数学的検証

### 8.1 現在のコードでこの問題は発生するか

**発生しない(発生させる経路自体が存在しないため)。** §5.4・§5.6・§8候補Bで
確認したとおり、現在の戦闘損耗・部隊撃破は`Division.manpower`のみを変更し、
`StateData.population`・`CountryData.mobilized_manpower`のいずれにも触れない。
したがって「兵士の死亡でpopulationとmobilized_manpowerを共に1減らす」という
処理自体が今は存在せず、以下の検証は**将来そのような同期処理(§9で候補に
挙げるF1の修正、または人口連動による損耗モデル)を設計する際に踏まえるべき
制約の可視化**である。

### 8.2 使用する実関数と丸め規則

* `population::update::calculate_workforce(population, workforce_ratio)`:
  `((population as f64) * (workforce_ratio as f64)).round().max(0.0) as u64`
  (四捨五入、`.5`は絶対値が大きい方へ丸める)。
* `StateData::civilian_workforce()` = `employed_workforce + unemployed_workforce`
  であり、これは`population::update::update_state_population_and_employment`の
  構成上、常に厳密に
  `calculate_workforce(population, workforce_ratio).saturating_sub(mobilized_burden)`
  **と一致する**(`employed`と`unemployed`の和は、需要で頭打ちされるかどうかに
  関わらず必ず`total_wf`に戻るため)。よって以下では
  `P = calculate_workforce(population, workforce_ratio)`(四捨五入)、
  `M = 州別mobilized_burden`、`C = P.saturating_sub(M)`として計算する。
* 州別按分は`population::mobilization::apportion_mobilized_manpower`
  (最大剰余法、`StateId`昇順タイブレーク)。

### 8.3 具体例(最低3ケースの指示に対し、境界を含む5ケースを検証)

**ケース1: `workforce_ratio = 1.0`(丸め誤差なし)**

```text
population=100,000, ratio=1.0 → P_before = round(100,000) = 100,000
死亡想定: population -= 1, mobilized -= 1 (単一州・単純モデル)
population=99,999 → P_after = round(99,999) = 99,999
M_before=99,999 → C_before = 100,000 - 99,999 = 1
M_after=99,998  → C_after  = 99,999 - 99,998 = 1
結果: Cは不変。ratio=1.0では populationとPが1:1で動くため、
      MがP同様に1減れば矛盾は起きない。
```

**ケース2: 一般的な`workforce_ratio = 0.45`**

```text
population=100,000, ratio=0.45 → P_before = round(45,000.0) = 45,000(端数なし)
死亡想定: population=99,999 → P_after = round(99,999 * 0.45) = round(44,999.55) = 45,000
          (0.55は切り上げられ、Pは population が1減っても変化しない)
M_before=40,000 → C_before = 45,000 - 40,000 = 5,000
M_after=39,999 (mobilizedを1減らした場合) → C_after = 45,000 - 39,999 = 5,001
結果: 兵士が1人死亡したにもかかわらず、Cが5,000→5,001へ「増加」する。
      これがユーザー指摘の逆説そのものである。
```

**ケース3: 人口が非常に小さく丸めの影響が大きい州**

```text
population=11, ratio=0.45 → P_before = round(4.95) = 5
死亡想定: population=10 → P_after = round(10 * 0.45) = round(4.5) = 5
          (Rustのf64::roundは0.5を絶対値の大きい方=切り上げへ丸めるため、
           4.5は5になり、Pは population が11→10になっても変化しない)
M_before=4 → C_before = 5 - 4 = 1
M_after=3  → C_after  = 5 - 3 = 2
結果: ケース2と同型の逆説が、潜在労働力が5人程度という極小規模の州では
      「1人分」がCの100%相当の変動として現れる——絶対値は小さいが、
      相対的な歪みはケース2より遥かに大きい。
```

**ケース4: 複数州へ動員負担を按分している国家(再按分込みの実式)**

```text
国家が州A(population=100,000)・州B(population=200,000)を所有、
国家のmobilized_manpower=90,000。
apportion_mobilized_manpower(90,000, [(A,100000),(B,200000)]) より
  burden_A = 30,000, burden_B = 60,000 (端数なし、300,000を3等分の比)

州Aで兵士が1人死亡: population_A=99,999、国家のmobilized_manpower=89,999 とし、
再按分すると(最大剰余法):
  population_total=299,999
  base_A=29,999(剰余140,000)、base_B=59,999(剰余159,999)
  不足1人は剰余の大きいB側へ配分 → burden_A=29,999、burden_B=60,000(不変)

P_A_before = round(100,000*0.45) = 45,000
P_A_after  = round(99,999*0.45)  = round(44,999.55) = 45,000(ケース2と同じ理由で不変)
C_A_before = 45,000 - 30,000 = 15,000
C_A_after  = 45,000 - 29,999 = 15,001 → 増加(逆説再現)

州B(死亡が起きていない側)も注目に値する:
P_B は不変(population未変更)=90,000。burden_Bも60,000のまま(再按分の結果
たまたま不変)だったため C_B は変化しなかったが、**他州での死亡が自州の
按分負担を変化させ得る**(今回はB側が繰上りを引き受けた側)——複数州を持つ
国家では、1州での死亡が国内の別州の民間労働力にまで波及し得るという、
逆説とは別の実務上の注意点も同時に確認された。
```

**ケース5: 動員負担が潜在労働力を超えている州(飽和境界)**

```text
population=1,000, ratio=0.45 → P_before = round(450.0) = 450
M_before=500(Pを超過) → C_before = saturating_sub(450,500) = 0(下限で飽和)
死亡想定: population=999 → P_after = round(999*0.45) = round(449.55) = 450(不変)
M_after=499 → C_after = saturating_sub(450,499) = 0(引き続き飽和)
結果: この境界では逆説は「見えなくなる」——PもMも動いているが、
      saturating_subの下限0への飽和がその差分を吸収してしまうため、
      Cの値としては変化が観測できない。飽和域では問題が隠れるだけで
      解消されているわけではない点に注意。
```

### 8.4 結論

* 現行コードでは、population・mobilized_manpowerを兵士の死亡に連動させる処理
  自体が存在しないため、この逆説は**現在は発生しない**。
* しかし、`workforce_ratio < 1.0`の州で「populationとmobilized_manpowerを
  単純に1:1で減らす」設計を採用した場合、**ケース2・3・4で示したとおり、
  日常的に(特別な入力を必要とせず)Cが死亡後に増加し得る**ことを実式で確認した。
* 原因はP側が`.round()`という離散的な丸めを経由するのに対し、M側(もし将来
  同期させる場合)は整数の直接減算になるため、両者の「1人あたりの実効的な
  減少幅」が一致しないことにある。
* 飽和域(候補5)では症状が隠れるが解消されているわけではなく、飽和が外れた
  瞬間に矛盾が再度顕在化し得る。
* この制約は、§9で推奨する次タスク(戦闘損耗とmobilized_manpowerの同期)の
  設計時に**population非連動のまま`mobilized_manpower`だけを同期する**か、
  **population連動も同時に設計し、この逆説を回避する丸め方式(例:
  国家単位で端数を繰り越す、月次バッチでのみpopulationへ反映する等)を
  別途決める**かを、実装前に明示的に決めておく必要があることを示す。

## 9. 既存テストの証拠

### 9.1 実行コマンドと結果

```bash
cargo test --lib -- --list
```
上記出力を`recruit|disband|combat|casualt|attrition|demobiliz|elimination.*divis|mobiliz`
で絞り込み、実在するテスト名のみを対象とした(存在しないターゲットは実行していない)。

```bash
cargo test --all-targets
```
結果: **1361 passed, 0 failed, 1 ignored**(基準状態から変化なし。監査中は
コード変更をしていないため当然の結果)。既知の非決定的PNG9枚が再生成されたため、
個別ファイル単位で`git checkout --`により復元した(ディレクトリ単位の復元はしていない)。
`tests/land_war_combat_peace_test.rs`は変更・整形していない。

### 9.2 各テストが証明する範囲(抜粋、代表例)

| テスト | 証明する範囲 | 経路 |
|---|---|---|
| `military::tests::test_recruitment_conditions` | 募兵の資金/人的資源不足時の拒否 | 実Domain API経由 |
| `military::tests::test_recruitment_cancellation` | キャンセル時の`available_manpower`/`treasury`の正確な返還 | 実Domain API経由(ただし`mobilized_manpower`はassertしていない——TEST GAP) |
| `military::tests::test_recruitment_progress_and_completion` | 募兵キューの日数消化と部隊生成 | 実Domain API経由(`mobilized_manpower`はassertしていない——TEST GAP) |
| `military::combat_calc::tests::test_zero_manpower_no_damage` | 兵員0の部隊が損耗計算に参加しても追加ダメージを生まない | 純粋関数のみ |
| `military::tests::test_phase15_manpower_does_not_go_below_zero` | 戦闘損耗の`saturating_sub`が負値を作らない | 実Domain API経由 |
| `country::elimination::tests::apply_disbands_all_remaining_divisions_of_the_eliminated_country` | 消滅国家の全Division実削除、併合側の部隊は無傷 | 実Domain API経由(`available_manpower`/`mobilized_manpower`はassertしていない——TEST GAP) |
| `military::army::tests::disband_returns_all_members_to_unassigned_and_removes_group` | 編成解散でDivisionが「未所属」へ戻る(=Division自体は存在し続ける) | 実Domain API経由 |
| `population::mobilization::tests::*`(11件) | `apportion_mobilized_manpower`の按分アルゴリズム自体の正しさ(0除算・overflow・タイブレーク等) | 純粋関数のみ |
| `war::tests::attacker_side_supporter_division_can_attack_defender_via_process_combat` | **未使用の`war::combat::process_combat`が動くこと自体は証明するが、これが本番Systemとして登録されているとは証明していない**(誤解を招きやすいテスト名、後述) | `war::combat::process_combat`直接呼び出し(死んだコード経由) |

### 9.3 テストが存在しない/不十分な領域(TEST GAP)

* `mobilized_manpower`が戦闘損耗・部隊撃破後にどう振る舞うべきかを検証する
  テストは**皆無**(現状「変化しない」ことを積極的に固定するテストすら無い
  ——バグを許容していることが暗黙のままで、明文化されていない)。
* 国家消滅時の`available_manpower`/`mobilized_manpower`の扱いを検証する
  テストは無い。
* 補充・Attrition・個別部隊解散(人的資源返還)は機能自体が無いため、
  当然テストも無い。
* Save/Load後に`mobilized_manpower`と実Division合計の整合性を検証する
  テストは無い(§5.9)。
* `war::combat::process_combat`関連のテスト(`war/tests.rs`)は、この関数が
  「本番で実際に使われている」かのように読める名前・コメントを持つが、
  Systemとして登録されていないため**本番挙動の証拠にはならない**。
  今回の監査で初めてこの区別を明示できた。

## 10. 発見事項

各項目とも、致命的なクラッシュやセーブ破損を伴うものではないため、
`Critical`(セーブ破損・クラッシュ相当)は使用しない。

### F1: 戦闘損耗・部隊撃破が`mobilized_manpower`を減算しない

* 分類: `CONFIRMED DEFECT`
* 重要度: **High**
* 影響イベント: 戦闘損耗(§5.4)、部隊撃破・撤退失敗による消滅(§5.4)
* 根拠: `src/military/update.rs:159-170`(損耗適用)、`src/military/update.rs:298-349`
  (`resolve_participant_outcome`、`remove_division`呼び出し)。いずれも
  `country.mobilized_manpower`を参照しない。フィールド自身のコメント
  (`src/country/mod.rs:140`「動員済みの人的資源（軍隊に所属している合計）」)
  と矛盾する。
* 再現条件: 戦争が発生し、いずれかの部隊が1兵でも損耗すれば即座に発生
  (特殊な設定不要)。
* 現在のゲームへの影響: `mobilized_manpower`は時間とともに実際の総兵力より
  過大な値へ乖離し続ける。これは表示(P23-003の軍事パネル)にも、経済用
  労働力比率には影響しない(民間労働力側は`employed_workforce`/
  `unemployed_workforce`という別経路で正しく動員按分される)が、
  「今どれだけの人的資源が軍に割かれているか」というプレイヤー向けの
  数値そのものが不正確になる。
* 必要となる設計判断: 死亡兵員数をどう特定するか(損耗発生箇所で直接減算/
  部隊削除時に`manpower`の残存値を減算、など)、§8の逆説をpopulationと
  同時に扱うかどうか。
* 推奨される後続タスク: §11参照(次タスクとして推奨)。

### F2: 個別部隊(Division)を解散して人的資源を返還する機能が存在しない

* 分類: `MISSING FEATURE`
* 重要度: **High**
* 影響イベント: 部隊解散(§5.6)
* 根拠: `military_panel.rs`の「Disband」ボタンは`ArmyRegistry::disband`
  (編成のグルーピング解除)のみを呼び、`Division`実体にも人的資源フィールドにも
  触れない(§5.6)。`Division`を削除して`available_manpower`/`mobilized_manpower`
  を調整するAPIはコードベースに存在しない。
* 再現条件: 常に(そもそも経路が無い)。
* 現在のゲームへの影響: プレイヤーは一度募兵した部隊の人的資源を、
  (部隊が全滅する以外の方法で)取り戻す手段が無い。
* 必要となる設計判断: 返還量は生存兵員のみか、返還時のペナルティ(例:
  一部を失う)を設けるか。
* 推奨される後続タスク: F1修正後の独立タスクとして。

### F3: 補充(reinforcement)機能が存在しない

* 分類: `MISSING FEATURE`
* 重要度: **High**
* 影響イベント: 補充(§5.3)
* 根拠: リポジトリ全体の`reinforce|replenish`検索(大文字小文字無視)で
  無関係なログメッセージ1件のみ。
* 現在のゲームへの影響: 一度損耗した部隊は永久に弱体化したままで、
  新規募兵以外に戦力を回復する手段が無い。
* 推奨される後続タスク: F1修正後、`mobilized_manpower`が正しい基準値に
  なってから設計するのが自然(補充消費分だけ再度動員する形になるため)。

### F4: Attrition(非戦闘損耗)が存在しない

* 分類: `MISSING FEATURE`
* 重要度: Medium
* 根拠: `attrition`の全文検索で該当無し。
* 現在のゲームへの影響: 補給切れ状態での駐留・進軍にペナルティが無く、
  無補給行軍が実質的にコスト無しになっている可能性がある(本監査は
  `supply_ratio`フィールドの用途までは深掘りしていない——別途確認が必要)。

### F5: AI国家が募兵(`request_recruitment`)を一切行わない

* 分類: `MISSING FEATURE`
* 重要度: **High**
* 影響イベント: 募兵(§5.1)、AI国家経路(§13関連)
* 根拠: `request_recruitment`の全呼び出し元は`src/ui/military_panel.rs`
  (プレイヤーのボタン)とテストのみ。`country_ai.rs`・`war/military_ai.rs`は
  `Division.manpower`を読むだけで、募兵を発行する経路が無い。
* 再現条件: 常に(AIは初期配置の部隊しか持てない)。
* 現在のゲームへの影響: AI国家の兵力は開戦後、損耗する一方で二度と増えない。
  長期戦・多数国間の戦争シミュレーションほど、AIが実質的に無力化されていく。
* 分類上の位置づけ: 「人的資源フィールドの帳簿が合っているか」という
  本監査の主題とは独立した問題(AIが人的資源を正しく扱っているかではなく、
  AIがそもそも人的資源を使う意思決定をしないという問題)だが、
  ユーザー指示の監査項目14に直接該当するため記録する。

### F6: 募兵コマンドの原子性・二重消費防止・キャンセル返還

* 分類: `CONFIRMED CORRECT`
* 重要度: informational
* 根拠: §5.1・§5.2。同一フレーム複数募兵、部隊完成時の再消費、キャンセル時の
  返還額、いずれも実コードで確認し、問題なし。

### F7: `available_manpower + mobilized_manpower <= manpower_cap`が保証されない

* 分類: `DESIGN UNDECIDED`
* 重要度: Medium
* 根拠: §7候補A。実データの初期値(全国家一律100,000)と人口比10%上限の
  不整合、および`recover_manpower`の意図的な非切り下げ設計。
* 現在のゲームへの影響: 小規模国家は実質的に無制限に近い初期人的資源で
  ゲームを開始できる可能性がある(バランス上の実害があるかは未検証)。

### F8: 部隊の「死」の表現が2種類混在している

* 分類: `CONFIRMED DEFECT`(内部一貫性の問題)
* 重要度: Low-Medium
* 根拠: §5.6-②(実削除)と§5.6-③(`status=Destroyed`のままレコード残存)。
* 現在のゲームへの影響: 各所の生存判定が`manpower==0 || status==Destroyed`
  という防御的なOR条件で書かれており、現状はこれによって実害が抑えられている
  (今回の監査で見つけた全ての判定箇所がこの2条件を併記していた)。ただし
  新しいコードを書く際にどちらか一方だけをチェックすると見落としが起こり得る。

### F9: 未使用の`war::combat::process_combat`が死んだコードとして残存

* 分類: `CONFIRMED DEFECT`(デッドコード/保守リスク)
* 重要度: Low
* 根拠: §5.4。`war/mod.rs`のSystem登録に存在せず、`war::tests`からのみ
  呼ばれる。HashMap走査順依存の非決定性を内包したまま残っている。
* 現在のゲームへの影響: 現状ゼロ(未使用のため)。将来誤って配線されると
  §7候補Eの非決定性が再発するリスク。

### F10: 人的資源フィールドに関するテストカバレッジの欠落

* 分類: `TEST GAP`
* 重要度: Medium
* 根拠: §9.3。

### F11: population/mobilized_manpower同期の丸め逆説(将来設計への制約)

* 分類: `DESIGN UNDECIDED`
* 重要度: Medium
* 根拠: §8。現在は発生しないが、F1の修正方法次第で作り込みかねない。

## 11. 次に推奨する実装タスク

**推奨: F1「戦闘損耗・部隊撃破と`mobilized_manpower`の同期」**

* 推奨理由: F1は`CONFIRMED DEFECT`であり、他の`MISSING FEATURE`
  (補充F3・個別部隊解散F2・Attrition F4)よりも設計の自由度が低く着手しやすい。
  かつ、F2(解散時の返還)・F3(補充)はいずれも「今`mobilized_manpower`が
  正しい値であること」を前提にしないと正しく設計できない——先に基盤となる
  帳簿を合わせる必要がある。
* 解決する不整合: F1(候補B不変条件の回復)。
* 必要な事前設計: 「損耗発生時に即座に減算する」か「部隊削除時に残存
  `manpower`分だけ減算する」か(前者は損耗のたびに、後者は死亡/撤退失敗の
  瞬間にまとめて)。§8の逆説はpopulation側を同時に触らない限り発生しない
  ため、本タスクではpopulationには触れず`mobilized_manpower`のみを対象と
  することを推奨する(スコープの分離、P23-004と同じ教訓)。
* 最小実装範囲: `military::update::process_daily_battles`の損耗適用箇所、
  および`resolve_participant_outcome`のDivision削除箇所で、対応する
  `CountryData.mobilized_manpower`を`saturating_sub`する。
* 対象外(推奨): population変更、`available_manpower`への返還(戦死者は
  民間へ戻らないという前提を維持)、補充・Attrition・個別部隊解散・
  国家消滅時の処理・AI募兵。
* セーブV1を維持できるか: 維持できる(新規フィールド不要、既存
  `mobilized_manpower`の更新経路を追加するのみ)。
* 後方互換性への影響: 無し(旧セーブの`mobilized_manpower`はロード後、
  次の戦闘発生時から正しく減り始める)。
* 必要な自動テスト: 損耗量ぶんの`mobilized_manpower`減算(純粋関数/実経路
  双方)、部隊全滅時の減算、複数部隊同時損耗時の合計整合、他国非波及、
  save/load後の継続動作。
* 他候補より先に行う理由: F2/F3/F4は新しいゲームメカニクスの追加であり
  ユーザーとの目的合意が必要な設計変更(前回P23-004の議論と同じ理由で
  自動的に次タスク化すべきではない)。F1は既存フィールドの意味を実装へ
  一致させるだけの、比較的スコープの狭い修正である。

## 12. 対象外を変更していないこと

本番Rustコード・テストコード・セーブ形式・ゲームバランスはいずれも変更して
いない。`git status --short`(§14)がRust/セーブ関連ファイルの変更を含まない
ことがその証跡である。監査中に発見した不整合はその場で修正せず、すべて§10・
§11へ記録した。

## 13. 残る不明点とユーザー判断が必要な事項

* F1の修正方針: 損耗発生の都度減算するか、部隊消滅時にまとめて減算するか。
* F7(候補A不変条件)を今後強制すべきか、現状の「初期値と上限の不整合を
  許容する」設計のままとするか。
* F8(2種類の「死」表現)を統一すべきか、現状の防御的OR条件のままで
  実害が無いとみなし放置するかどうか。
* `supply_ratio`(補給比率)フィールドが実際に何のゲームプレイ上の効果を
  持つのかは今回の監査では深掘りしていない(Attrition候補の設計時に
  合わせて調査が必要)。
* 国家消滅時に併合側が敵軍の人的資源を一部でも吸収する設計を導入するか
  どうか(現状は吸収しない)。

## 14. 監査報告書作成前の`git status --short`

```
 M saves/savegame_v1.ron
```

`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、本監査で一切
参照・変更していない。それ以外の変更は無い(本報告書ファイル自体はこの後
個別にステージしてコミットする)。
