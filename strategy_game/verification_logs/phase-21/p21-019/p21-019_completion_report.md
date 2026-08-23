# P21-019 完了報告: Crisis由来戦争の講和・Claim解決接続

## 1. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

全品質ゲート(`cargo check/test/clippy/build --release/diff --check/fmt`)を通過し、
新規テスト41件(lib 37件・integration 4件、§14/§21の対応表参照)が全てgreen。
実データ(6か国28州)によるE2Eテスト4件を含む。

2026-08-21、ユーザーレビューにより初回実装の3点(首都割譲Save破損の
「対象外」誤判定、Save validationの`.first()`依存による三者一致検証不足、
P21-015/016の完全E2E未達)を指摘され、いずれも修正済み(§21)。
GUI実ウィンドウでの手動確認は実施していない(§22参照)ため、COMPLETEとは
断定しない。

---

## 2. 実装前調査結果(§1の15項目)

1. **`War`の全フィールドと`source_crisis_id`**(`src/war/data.rs`):
   `id`, `name`, `attackers: HashSet<CountryId>`, `defenders: HashSet<CountryId>`,
   `primary_attacker/primary_defender: Option<CountryId>`(P21-016、未設定なら
   `primary_attacker_id()`/`primary_defender_id()`が集合内最小要素へフォールバック、
   空集合ならpanicする不変条件付き)、`source_crisis_id: Option<DiplomaticCrisisId>`
   (P21-016、`declare_war`が正当化の`source_crisis_id`をそのままコピー)、
   `war_goals: Vec<WarGoal>`、`status`、`winner`等。
2. **`DiplomaticCrisis`のClaim参照・initiator・target・target_state**:
   `initiator`/`target: CountryId`、`related_claim_id: Option<ClaimId>`。
   **`target_state`という直接フィールドは存在しない** — 仕様書の想定と異なり、
   `crisis.war_goals.first().and_then(|g| g.target_states.first())`経由でしか
   得られない(`start_crisis`が`war_goals: vec![WarGoal{target_states:
   vec![claim.target_state], ...}]`として構築するため、実質的にClaimの
   target_stateと一致するが、フィールドとしては間接参照)。本タスクはこの実際の
   形を正として実装した。
3. **Justificationが保持するCrisis・Claim参照**: `WarJustification.source_crisis_id:
   Option<DiplomaticCrisisId>`のみ(P21-016)。Claimへの直接参照は無く、
   `declare_war`が正当化のsource_crisis_idをWarへそのままコピーする一段階の
   受け渡しのみ。
4. **`TerritorialClaim`と`ClaimRegistry::mark_consumed`**: 既存(P21-011)。
   `mark_consumed(id) -> bool`は対象Claimが存在しない場合のみ`false`、
   存在すれば(既にConsumedでも)常に`status`を`Consumed`にして`true`を返す
   **既存の冪等な実装**——P21-019はこれをそのまま再利用し、独自の重複防止を
   追加していない(§7参照)。
5. **`execute_peace_settlement`の引数・戻り値・副作用**(`src/war/peace.rs`):
   変更前は`(war_id, term, end_reason, current_date_str, &mut state_registry,
   &mut war_registry, &mut military_registry, &mut battle_registry,
   &mut diplomacy_registry, &mut frontline_registry) -> Result<(), &'static str>`。
   冒頭の`war_not_found`/`already_ended`(二重適用防止、`Ok(())`で早期return)の
   2チェックを除けば残りは無条件に実行される(既存コードにこれ以上の失敗パスは
   無い)。P21-019は`crisis_registry: &CrisisRegistry`・
   `claim_registry: &mut ClaimRegistry`を追加した(§4参照)。
6. **既存の全講和種別**: `PeaceTerm`は`WhitePeace`/`CedeWarGoalRegion`/
   `AttackerConcedes`の3種のみ。**「降伏」は独立した種別ではない** —
   `handle_daily_war_resolution`(`src/war/update.rs`)が
   `evaluate_war_capitulation`の結果に応じて`CedeWarGoalRegion`
   (防御側降伏)または`AttackerConcedes`(攻撃側降伏)を選んで
   `execute_peace_settlement`を呼ぶだけで、専用の講和ロジックは無い。
7. **Warを終結状態へ移す全経路**: `execute_peace_settlement`の呼び出しは
   3箇所のみ — `ui/peace_panel.rs::handle_peace_action_buttons`
   (プレイヤーの講和申し入れ受諾)、`war/update.rs::handle_daily_war_resolution`
   内の2箇所(防御側/攻撃側の自動降伏講和、日次)。他にWarを削除・終結させる
   経路は存在しない(Warは終結後も`status`を変えて履歴として`WarRegistry`に残る)。
8. **州所有権を変更する共有API**: `StateRegistry::transfer_region_ownership
   (state_id, new_owner) -> Result<(), &'static str>`(海州は拒否するのみ、
   **現在の所有国が誰であるかは一切検証せず無条件に上書きする**)。
9. **勝者・敗者・領土受領国の決定方法**: `war.primary_attacker_id()`/
   `primary_defender_id()`(P21-016、HashSet走査順に依存しない明示的な代表国)。
   `CedeWarGoalRegion`は`war.war_goals.first().target_states.first()`を
   attacker(primary)へ、`AttackerConcedes`はdefenderを`winner`とする。
10. **多国間Warでの`primary_attacker_id()`／`primary_defender_id()`**:
    上記の通り。空集合なら実在しない国を捏造せずpanicする設計
    (`declare_war`が必ずinitiator/targetを挿入するため実運用では発生しない)。
11. **AI講和とプレイヤー講和が同じDomain APIを使うか**: **「AI講和」という
    独立した概念自体が存在しない**。AI国家が自発的に講和を提案・受諾する
    コードは無く、講和は(a)プレイヤーのUI操作、(b)国籍を問わない日次自動降伏
    判定、の2経路のみで、いずれも同一の`execute_peace_settlement`を呼ぶ。
12. **War・Crisis・ClaimのSave DTOとvalidation**: `SavedWarRegistry`/
    `SavedCrisisRegistry`/`SavedClaimRegistry`はいずれもRegistryの内部
    コレクション+`next_id`をそのまま保持する薄いDTO。`save/validate.rs`の
    `validate_wars`は`source_crisis_id`のdangling参照のみ検証済みで、
    **War自身とCrisisのinitiator/target一致、Crisisが参照するClaimとの
    整合性はP21-019着手前は未検証**(`validate_war_justifications`には
    同等のCrisis↔Claim整合性検証が既に存在しており、Warには無かった非対称)。
13. **War goal表示の既存UI**: `peace_panel.rs`は`war.name`
    (`declare_war`が生成する`"{attacker} Conquest of {state} ({defender})"`
    形式の**未翻訳固定文字列**)のみ表示し、claimant/対象国/Claim状態/
    Crisis由来である旨を構造化して示す箇所は存在しなかった。
14. **開始時テスト数**: lib 908 / integration 186 / 合計1094
    (`cargo test --lib`/`--tests`で実測)。
15. **fmtドリフト基準**: `cargo fmt --all -- --check`で44箇所
    (11ファイル、P21-018完了時と同一)。

既存の`execute_peace_settlement`が要求(§4)を全て満たしていたため、新しい
第2の講和APIは追加していない。

---

## 3. 実際の講和APIと仕様との差異

- 仕様書は「Crisisのtarget_state」という言葉を使うが、実際には
  `DiplomaticCrisis`に直接の`target_state`フィールドは無く、
  `war_goals.first().target_states.first()`経由で得る(§2項目2参照)。
- 「降伏」は独立した講和種別ではなく、既存2種別(`CedeWarGoalRegion`/
  `AttackerConcedes`)を自動選択して同じ`execute_peace_settlement`を呼ぶだけ。
- 「AI講和」という独立した概念・APIは存在しない(§2項目11参照)。

いずれも実装前調査で確認済みの実コードの形をそのまま正として実装した
(仕様書の言葉遣いと異なるだけで、目的とする動作自体に矛盾は無い)。

---

## 4. Claim消費規則

`resolve_crisis_war_lineage`(`src/war/peace.rs`、新設の`pub(crate)`ではない
モジュール内`fn`)が`war.source_crisis_id`を検証し、`Ok(Some(claim_id))`を
返した場合のみ、`execute_peace_settlement`の最後(既存の全ステップ完了後)で
`claim_registry.mark_consumed(claim_id)`を呼ぶ。

- `source_crisis_id == None`(通常War)は`Ok(None)`で即座に素通りし、
  Claim消費処理を一切行わない(§6の回帰防止)。
- 講和結果(`WhitePeace`/`CedeWarGoalRegion`/`AttackerConcedes`、降伏含む)を
  問わず、lineageが有効なら常に消費する(§2の要求どおり)。
- 既存の`mark_consumed`自体が冪等(既にConsumedなclaimへ再度呼んでも
  `true`を返すだけで副作用は増えない)なため、P21-019独自の重複防止コードは
  追加していない——そもそも`execute_peace_settlement`冒頭の
  `war.status != Active`二重適用防止により、同じWarに対してこのコード自体が
  二度目に到達することが無い(§7で理由を詳述)。

---

## 5. 領土移転規則

既存の`CedeWarGoalRegion`分岐(`state_registry.transfer_region_ownership
(target_id, attacker)`、`attacker = war.primary_attacker_id()`)は変更していない
——Crisis由来Warでは`primary_attacker_id()`が(lineage検証により)
`crisis.initiator`と一致することが保証されるため、既存コードがそのまま
「Crisisの要求州をCrisis initiatorへ移転する」を実現する。追加したのは
以下の安全確認のみ:

- `target_state_id`(`war.war_goals.first().target_states.first()`)が
  `crisis.war_goals.first().target_states.first()`(すなわちClaimの
  target_state)と一致することの事前検証。
- `term == CedeWarGoalRegion`の場合、対象州の**現在の**法的所有国が
  まだdefenderのままであることの事前検証(`crisis_lineage_state_owner_changed`
  エラー)——`transfer_region_ownership`自体は現在の所有者を検証しないため
  (§2項目8)、戦争中に第三国へ所有権が移っていた場合でも無条件に奪ってしまう
  既存の潜在ギャップがあり、Crisis由来Warについてはこの事前チェックで
  安全側に倒した(通常Warのこの挙動自体は変更していない、§6・§20参照)。

`WhitePeace`/`AttackerConcedes`は既存どおり領土移転を一切行わない
(Claim消費だけが新たに発生する)。

---

## 6. lineage検証

`resolve_crisis_war_lineage`が以下を順に検証し、最初の不整合で
`Err(&'static str)`(`war_error.peace.crisis_lineage_*`翻訳キー)を返す:

1. `source_crisis_id`のCrisisが存在する
2. `crisis.initiator == attacker`(War側から渡された`primary_attacker_id()`)
3. `crisis.target == defender`
4. `crisis.related_claim_id`が`Some`
5. そのClaimがClaim Registryに存在する
6. `claim.claimant_country == crisis.initiator`
7. `claim.target_state == crisis`自身のwar_goal target_state
8. そのtarget_stateがState Registryに存在する
9. (`CedeWarGoalRegion`のみ)対象州の現在の所有国がdefenderのまま

`War.primary_attacker_id()`/`primary_defender_id()`(attackers/defendersが
空だとpanicする設計)は`execute_peace_settlement`側で計算済みの値を渡す形にし、
`resolve_crisis_war_lineage`自体はpanicし得る呼び出しを一切含まない。

---

## 7. 原子性

`resolve_crisis_war_lineage`は`&War`/`&CrisisRegistry`/`&ClaimRegistry`/
`&StateRegistry`のみを借用する純粋関数で、Registryを一切変更しない。
`execute_peace_settlement`はこの検証を(`attacker`/`defender`/
`target_state_id`という既存の**読み取りのみ**の計算の直後、既存の
「1. 講和条件に応じた領土移転」より前)`?`で呼ぶため、検証失敗時は
その時点で関数全体からErrを返し、以降の領土移転・占領解除・戦闘中止・
前線削除・陸軍帰還・休戦作成・War状態更新のいずれも実行されない
(`lineage_failure_leaves_every_registry_completely_unchanged`テストで
Claim/War/State Registryがbefore/after完全一致することを確認)。

**既にConsumedのClaimへの再到達**: `mark_consumed`自体が冪等なため、
明示的な「既にConsumed」エラーは追加していない。この判断の根拠——
`execute_peace_settlement`冒頭の`war.status != WarStatus::Active`チェックが
同一Warへの二度目の到達を最初から防ぐため、「同じWar・同じClaimに対して
lineage解決が2回実行される」という経路自体が構造的に存在しない。
万一(手動Save改変等で)別の壊れたWarが同じClaim IDを指すような異常データが
あっても、`mark_consumed`は`true`を返すだけで安全(既存の冪等性、
`already_consumed_claim_is_handled_idempotently`で確認)。

---

## 8. 通常Warへの影響

`source_crisis_id.is_none()`のWarは`resolve_crisis_war_lineage`が
即座に`Ok(None)`を返すため、Claim消費もlineage検証も一切発生しない。
`execute_peace_settlement`の既存ステップ(領土移転・占領解除・戦闘中止・
前線削除・陸軍帰還・休戦作成・War状態更新)は一切変更していない
(`normal_war_peace_does_not_touch_unrelated_claims`、既存の
`test_peace_treaty_basic`等の既存テスト全件greenで確認)。

---

## 9. 多国間War対応

`primary_attacker_id()`/`primary_defender_id()`は既にP21-016で
HashSet走査順非依存の実装済みのため、P21-019は追加変更なしでこの性質を
そのまま継承する。2対2の多国間Crisis由来War(攻撃側{initiator,supporter}、
防御側{target,supporter})で、要求州がprimary attacker(Crisis initiator)へ
移転し支持国へは移らないこと、支持国のHashSet挿入順を変えても結果が
変わらないことを新規テストで確認した(§14参照)。既存の
`peace_settlement_clears_hostility_for_all_participants_including_supporters`
(P21-016、支持国含む全参加者の敵対解除)は無変更のままgreen。

---

## 10. Save/Load

新しいSave Resourceは追加していない。`War.source_crisis_id`/
`ClaimStatus`はいずれも既存フィールドで、既存の`SavedWarRegistry`/
`SavedClaimRegistry`のシリアライズにそのまま乗る。

`save/validate.rs::validate_wars`へ、`validate_war_justifications`が
既に持つCrisis↔Claim整合性検証と全く同じパターンで、War側の
検証を追加した(§7の既存非対称を解消):

- (既存)`source_crisis_id`のdangling参照
- (新規)Warのprimary attacker/defenderと由来Crisisのinitiator/targetの一致
- (新規)由来Crisisが参照するClaimのclaimant/target_stateとの一致

`primary_attacker`/`primary_defender`はSave検証内で
`war.primary_attacker.or_else(|| war.attackers.iter().min...)`という
**panicしない**フォールバックのみを使う(`primary_attacker_id()`自体は
呼ばない、§6参照)。

`source_crisis_id == None`の既存Saveは無条件に通過する(baseline_save自体が
これに該当し、既存テスト`baseline_save_passes_validation`が無変更でgreen)。

---

## 11. P21-017／018との接続

`mark_consumed`は`status`だけを書き換え、`created_date`は変更しない
——P21-018のcooldown計算(`claim_month_index_or_blocking`)は
`created_date`をそのまま読むため、講和成立日ではなく元Claimの
`created_date`から12か月のcooldownが正しく引き継がれることを
`mark_consumed_does_not_rewrite_created_date_and_cooldown_uses_the_original_date`
で確認。Active Claimによる生成ブロック(P21-018の
`claimants_with_active_claim`)は、`mark_consumed`が呼ばれた瞬間に
(次回の月次評価から)自動的に解除される——P21-018側のコード変更は
一切不要だった。P21-017のClaim走査(`consumed_claim_is_not_a_candidate`、
既存テスト)は無変更のままConsumedなClaimを候補から除外し続ける。

---

## 12. UI変更

`ui/peace_panel.rs`の`update_peace_panel_ui`へ、`Res<CrisisRegistry>`/
`Res<ClaimRegistry>`を追加し、`war.source_crisis_id`が`Some`の場合のみ
新しい1行(claimant国名・対象国名・target_state名・元Claimの状態)を
`peace_panel.crisis_war_goal`翻訳キー(JA/EN)で表示する。通常Warでは
一切表示しない。`ClaimStatus::display_name()`を新設(`claim_status.active`/
`claim_status.consumed`、`ClaimSource::display_name()`と同じ規約)。
既存のスクロール構造・講和アクションボタン・戦勝点内訳・降伏進捗表示は
無変更。

---

## 13. 変更ファイル

**新規**:
- `tests/p21_019_crisis_war_peace_resolution_e2e_test.rs`(E2E 4件、
  §21で追加した2件を含む)

**既存ファイルへの追記**:
- `src/war/peace.rs`: `resolve_crisis_war_lineage`新設、
  `execute_peace_settlement`のシグネチャ拡張(`crisis_registry`/
  `claim_registry`追加)+ Claim消費ステップ追加。**§21-Fix1**:
  `resolve_capital_reassignment`新設 + シグネチャへ`country_registry`
  追加(13番目の引数)+ 首都再割当ステップ追加
- `src/war/tests.rs`: P21-019テスト20件追加、`execute_peace_settlement`
  既存3呼び出し箇所をシグネチャ変更に追従。**§21-Fix1**: 首都再割当
  テスト3件追加、`country_registry`引数追加への呼び出し箇所全件追従
  (`p21_019_valid_env`拡張・新規`p21_019_capital_env`フィクスチャ)
- `src/war/update.rs`: `handle_daily_war_resolution`へ`CrisisRegistry`/
  `ClaimRegistry`追加、`execute_peace_settlement`呼び出し2箇所を更新。
  **§21-Fix1**: `CountryRegistry`引数を同2箇所へ追加
- `src/ui/peace_panel.rs`: `update_peace_panel_ui`へCrisis由来戦争目標表示
  + `handle_peace_action_buttons`のレジストリ追加・呼び出し更新、
  レンダリングテスト5件新規追加。**§21-Fix1**: `handle_peace_action_buttons`
  へ`CountryRegistry`引数追加
- `src/diplomacy/claims.rs`: `ClaimStatus::display_name()`新設
- `src/save/validate.rs`: `validate_wars`へWar↔Crisis↔Claim整合性検証追加、
  テスト5件追加。**§21-Fix2**: `.first()`依存の検証を三者(Claim/Crisis
  war_goals/War war_goals)完全一致検証へ置き換え、テスト2件追加
- `src/diplomacy/ai_claim_generation/tests.rs`: P21-017/018接続テスト2件追加
- `tests/land_war_combat_peace_test.rs`(既存fmtドリフト15箇所を保持する
  「保護対象」ファイル): `execute_peace_settlement`呼び出し1箇所を
  シグネチャ変更に追従(該当行のみ手動編集、rustfmtは実行していない)。
  **§21-Fix1**: `country_reg`を`mut`化し引数追加(実データ・実際の首都州
  `StateId(8)`に対して再割当ロジックがregressionなく動作することを確認)
- `assets/localization/en-US.ron`/`ja-JP.ron`: `war_error.peace.
  crisis_lineage_*`(10キー)、`peace_panel.crisis_war_goal`、
  `claim_status.active`/`claim_status.consumed`を追加。**§21-Fix1**:
  `war_error.peace.cede_would_leave_country_without_a_capital`(EN/JA)追加

**P21-019が触れていない、セッション開始時から未コミットのまま残る
P21-016〜018由来の変更**: `src/country/country_ai.rs`,
`src/diplomacy/{ai_claim_generation,ai_crisis_start,crisis,mod,
player_demand_alert}.rs`, `src/map/*`, `src/military/{invasion,tests,
update}.rs`, `src/profiling.rs`, `src/save/{apply,export}.rs`,
`src/ui/{diplomacy_panel,military_panel}.rs`, `src/war/{data,frontline,
military_ai}.rs`, `tests/p21_005/008/010/011/012/013/015/016/017/018_*`,
`verification_logs/phase-21/p21-016/*`, `verification_logs/phase-21/
{p21-017,p21-018}/*`。

---

## 14. 全テスト対応表

lib(`src/war/tests.rs`、20件)+ `src/save/validate.rs`(5件)+
`src/ui/peace_panel.rs`(5件)+ `src/diplomacy/ai_claim_generation/
tests.rs`(2件)= **lib 32件**、integration
(`tests/p21_019_crisis_war_peace_resolution_e2e_test.rs`)= **2件**。
合計 **34件**。

| 要求テスト番号 | 対応するテスト名(モジュール) |
|---|---|
| 1,3,11 | `crisis_war_cede_settlement_consumes_claim_and_transfers_target_state_to_initiator` |
| 2,14 | `crisis_war_white_peace_consumes_claim_without_transferring_territory` |
| 4,15 | `crisis_war_attacker_concedes_consumes_claim_without_transferring_territory` |
| 5 | `crisis_war_capitulation_path_consumes_claim` |
| 6 | `normal_war_peace_does_not_touch_unrelated_claims` |
| 7,8 | `only_the_exact_lineage_claim_is_consumed_not_other_claims_by_the_same_claimant` |
| 9 | `already_consumed_claim_is_handled_idempotently` |
| 10 | `dangling_source_crisis_id_is_rejected`等lineage失敗系全テスト(Claim状態Active維持を確認) |
| 12,29 | `multilateral_crisis_war_transfers_territory_to_primary_attacker_not_supporters` |
| 13,30 | `supporter_insertion_order_does_not_affect_the_territory_recipient` |
| 16 | `unrelated_states_are_not_touched_by_crisis_settlement` |
| 17,18 | `target_state_owned_by_unrelated_third_party_is_safely_rejected` |
| 19 | `dangling_source_crisis_id_is_rejected` |
| 20 | `dangling_claim_reference_is_rejected` |
| 21 | `attacker_crisis_initiator_mismatch_is_rejected` |
| 22 | `defender_crisis_target_mismatch_is_rejected` |
| 23 | `claim_claimant_crisis_initiator_mismatch_is_rejected` |
| 24 | `claim_target_state_crisis_target_state_mismatch_is_rejected` |
| 25 | `crisis_target_state_missing_from_state_registry_is_rejected` |
| 26,27,28 | `lineage_failure_leaves_every_registry_completely_unchanged` |
| 31 | `peace_settlement_clears_hostility_for_all_participants_including_supporters`(既存P21-016テスト、regression) |
| 32 | `source_crisis_id_survives_the_peace_settlement` |
| 33 | `war::tests`全65件green(regression、§16参照) |
| 34,35,36 | E2E `peace_settlement_result_survives_save_load_round_trip` |
| 37 | `war_primary_attacker_crisis_initiator_mismatch_is_rejected`, `war_primary_defender_crisis_target_mismatch_is_rejected` |
| 38 | 既存`unknown_related_claim_reference_is_rejected`(Crisis→Claim dangling、再利用) |
| 39 | `war_crisis_claim_claimant_mismatch_is_rejected` |
| 40 | `war_crisis_claim_target_state_mismatch_is_rejected` |
| 41 | 既存`baseline_save_passes_validation`(source_crisis_id=None、無変更) |
| 42 | 既存`consumed_claim_is_not_a_candidate`(P21-017、ai_crisis_start/tests.rs、regression) |
| 43,44,46 | `post_peace_consumed_claim_cooldown_boundaries_and_active_block_release` |
| 45 | `mark_consumed_does_not_rewrite_created_date_and_cooldown_uses_the_original_date` |
| 47 | E2E `crisis_derived_war_peace_settlement_transfers_territory_consumes_claim_and_ends_the_war`(末尾のcooldown確認)+ 既存P21-017/018 E2E群(regression) |
| 48,49 | `crisis_war_goal_shows_claimant_target_and_state_names`, `crisis_war_goal_shows_claim_status` |
| 50 | `normal_war_does_not_show_crisis_war_goal_line` |
| 51 | `crisis_war_goal_text_follows_locale_switch` |
| 52 | `crisis_war_goal_line_preserves_scroll_and_peace_action_buttons` |
| 53-58 | E2E共有ヘルパー`build_crisis_derived_war_via_real_gameplay`(Claim生成→Crisis開始→拒否→AI正当化→宣戦布告、P21-018と同じ実データ経路) |
| 59,60 | E2Eヘルパー`settle_peace`(実際の`execute_peace_settlement`を実App World経由で呼ぶ) |
| 61,62,63 | E2E `crisis_derived_war_peace_settlement_transfers_territory_consumes_claim_and_ends_the_war` |
| 64 | E2E `peace_settlement_result_survives_save_load_round_trip` |
| 65 | E2E `crisis_derived_war_peace_settlement_transfers_territory_consumes_claim_and_ends_the_war`(講和直後の再生成ブロック確認)+ 既存P21-018決定論性テスト(regression) |

**注記**: 要求56/57(第三国支持・多国間War化)は、単体テスト
(`multilateral_crisis_war_transfers_territory_to_primary_attacker_not_supporters`)
では手構築フィクスチャで確実に検証しているが、E2Eテスト(実データ)では
選ばれるclaimant/targetの組次第でP21-015のGreat Power支持が自然発生するとは
限らないため、E2E側では意図的に主張していない(早期returnで誤魔化さず、
確実性のある単体テストで担保する方針)。

---

## 15. 開始時・終了時テスト数

| | lib | integration | 合計 |
|---|---|---|---|
| 開始時 | 908 | 186 | 1094 |
| 初回実装完了時 | 940 | 188 | 1128 |
| 3点修正後(最終) | 945 | 190 | 1135 |
| 差分(開始時→最終) | +37 | +4 | +41 |

---

## 16. 品質ゲート

### 16.1 初回実装完了時

| ゲート | 結果 |
|---|---|
| `cargo check --all-targets` | 成功 |
| `cargo test --lib` | 940 passed / 0 failed |
| `cargo test --tests` | 全29integrationバイナリでpassed / 0 failed(188件) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 警告0件 |
| `cargo build --release` | 成功 |
| `git diff --check` | エラーなし(既存のLF/CRLF警告のみ、内容差分なし) |
| `cargo fmt --all -- --check` | 41箇所(§17参照) |

### 16.2 3点修正後(最終、2026-08-21再実行)

| ゲート | 結果 |
|---|---|
| `cargo check --all-targets` | 成功 |
| `cargo test --lib` | 945 passed / 0 failed |
| `cargo test --tests` | 全30integrationバイナリでpassed / 0 failed(190件) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 警告0件 |
| `cargo build --release` | 成功 |
| `git diff --check` | エラーなし(既存のLF/CRLF警告のみ、内容差分なし) |
| `cargo fmt --all -- --check` | 41箇所(§17と同一、新規drift無し) |
| スクリーンショット差分(headlessテスト副作用) | `git checkout --`で復元済み、`git status`クリーン確認済み |

---

## 17. fmt基準

- 開始時基準: **44箇所**(11ファイル、P21-018完了時と同一)。
- P21-019変更ファイルへ`rustfmt --edition 2024 <file>`を個別実行:
  `src/war/peace.rs`, `src/war/tests.rs`, `src/war/update.rs`,
  `src/save/validate.rs`, `src/ui/peace_panel.rs`,
  `src/diplomacy/claims.rs`, `src/diplomacy/ai_claim_generation/tests.rs`,
  `tests/p21_019_crisis_war_peace_resolution_e2e_test.rs`。
- **副作用による既存ドリフトの解消**: `src/war/peace.rs`は開始時基準44箇所の
  内訳に含まれる**既存の無関係な3箇所のドリフトを持つファイルだった**
  (`relocate_hostile_territory_divisions`内の折り返し、P21-019が触れていない
  関数)。P21-019がこのファイルへ実質的な変更(`resolve_crisis_war_lineage`
  新設・シグネチャ拡張)を加えたため、ファイル単位でしか実行できない
  `rustfmt`により、この3箇所も同時に整形された。個別の行単位フォーマットは
  安定版rustfmtでは不可能なため、「無関係なdriftを勝手に整形しない」の
  対象は「触れていないファイル」であり、実質改変したファイル内の
  未着手コード部分の整形は不可避な副作用として許容し、正直に報告する。
- 終了時総ドリフト: **41箇所**(44 − 3[war/peace.rsの副次解消] + 0[新規drift]
  = 41)。内訳: `tests/land_war_combat_peace_test.rs`(15、P21-019は該当行の
  みEdit直接編集・rustfmt未実行で維持)、`tests/daily_system_integration_test.rs`
  (7)、`src/map/division_render.rs`(5)、`tests/p21_save_003_end_to_end_test.rs`
  (4)、`src/map/mod.rs`(3)、`src/save/runtime.rs`(2)、
  `src/military/movement.rs`(2)、`tests/profile_workload_correctness_test.rs`
  (1)、`src/military/supply.rs`(1)、`src/military/recruitment.rs`(1)。
  いずれもP21-019が触れていない既存ファイル。
- P21-019変更ファイル8件は`rustfmt --edition 2024 --check`で個別に
  ドリフト0を確認済み。

---

## 18. 性能測定方法・結果・生データ

一時プロファイラ(`src/war/peace_perf_temp.rs`、`#[ignore]`付き`#[test]`、
`src/war/mod.rs`へ一時的に`#[cfg(test)] pub mod peace_perf_temp;`を追加して
実行後、両方とも削除)を
`cargo test --release --lib war::peace_perf_temp -- --ignored --nocapture`
で実行。`resolve_crisis_war_lineage`はClaim ID/Crisis ID/State IDの
`HashMap::get`(O(1))のみで構成され、走査を一切含まないため、以下の
全軸で理論どおりほぼ横ばい(サブミリ秒)であることを実測した:

**12.1 Claim履歴スケーリング**(crises=100, participants=1/side、Crisis由来War):

| claims | ms |
|---|---|
| 100 | 0.0396 |
| 1,000 | 0.0019 |
| 10,000 | 0.0044 |
| 50,000 | 0.0078 |

**12.2 Crisis履歴スケーリング**(claims=100, participants=1/side):

| crises | ms |
|---|---|
| 100 | 0.0047 |
| 1,000 | 0.0019 |
| 10,000 | 0.0038 |

**12.3 War参加国スケーリング**(claims=100, crises=100):

| participants/side | ms |
|---|---|
| 1 | 0.0022 |
| 10 | 0.0049 |
| 100 | 0.0013 |
| 1,000 | 0.0010 |

**12.4 通常WarとCrisis由来Warの比較**(claims=50,000, crises=10,000,
participants=1,000/side):

| | ms |
|---|---|
| 通常War(source_crisis_id=None) | 0.0081 |
| Crisis由来War | 0.0078 |

Claim/Crisis履歴・War参加国数いずれの軸でも増加傾向は見られず(全て
0.001〜0.04ms台、ノイズの範囲)、`countries × states`/`wars × claims`等の
禁止パターンは発生していない。通常WarとCrisis由来Warのコストもほぼ同一
(lineage解決の追加コストは無視できる)。

**12.5 非講和フレーム**: `handle_daily_war_resolution`は
`evaluate_war_capitulation`が`CapitulationResult::None`を返す限り
`execute_peace_settlement`自体を一切呼ばない(`match`の`None`分岐は
空処理)。よってlineage解決コードは非講和日には実行されず、追加コストは
構造的に0(実測不要、既存コード構造から自明)。

生データ: `verification_logs/phase-21/p21-019/perf/results.csv`,
`verification_logs/phase-21/p21-019/perf/summary.txt`。

---

## 19. 一時ファイル削除確認

- `src/war/peace_perf_temp.rs`: 削除済み。
- `src/war/mod.rs`内の一時`pub mod peace_perf_temp;`登録: 削除済み。
- 一時bin登録・scratch Save・一時スクリーンショット: 追加していない。
- デバッグ出力(`eprintln!`等、UIテスト調査で一時的に追加したもの含む):
  全て削除済み(最終版のテストコードに残っていないことを確認)。
- headlessテスト実行が変更した既存スクリーンショット9件
  (`verification_logs/p20-007/`, `p20-009/`, `phase-21/p21-save-002e/`配下)は
  `git checkout --`で復元済み(`git status`でクリーンを確認)。

---

## 20. 発見した既存問題

- ~~`capital_state_id`が征服後に追従しない~~: **§21-Fix1で修正済み**。
  当初は「P21-019範囲外」と誤判定していたが、ユーザー指摘により
  `resolve_capital_reassignment`(`src/war/peace.rs`)を新設し、
  `CedeWarGoalRegion`講和が対象国の首都州を割譲する場合、残存する
  最小`StateId`の陸州へ決定論的に再割当する(残存州が無ければ部分適用前に
  安全に拒否する)よう対応した。詳細は§21参照。
- **`transfer_region_ownership`は現在の所有国を検証しない**(既存、
  P21-019範囲外): §5で述べた通り、Crisis由来Warの`CedeWarGoalRegion`には
  P21-019が事前検証を追加したが、通常Warの同じ経路にはこの検証が無い
  (仕様§6の回帰防止により意図的に変更していない)。
- **`peace_panel.rs`は`execute_peace_settlement`のErr戻り値を無視する**
  (既存、P21-019範囲外): `handle_peace_action_buttons`は
  `if result.is_ok() { 成功通知 }`のみで、Err時に何も通知しない
  (`can_accept_peace_offer`のErrだけがプレイヤーへ通知される)。
  P21-019の新しいlineage系エラーもこの経路では無言で失敗する
  (正規のゲームプレイでは発生し得ない、Save改変等の異常系のみに
  影響するため、今回は対応範囲外とした)。

---

## 21. ユーザー指摘対応(2026-08-21): 3点修正

初回実装完了報告に対し、ユーザーから以下3点の指摘を受け、いずれも修正した
(§16.2の品質ゲート再実行で全件green確認済み)。

### Fix 1: 首都割譲でSaveが壊れる問題(P21-019対象内として修正)

**問題**: P21-018はP21-018専用の首都禁止ルールを追加しないと明示しており、
首都州も実際にClaim候補として許可されている。その正規経路(Claim→Crisis→
War→`CedeWarGoalRegion`講和)で対象州がたまたま相手国の首都だった場合、
`transfer_region_ownership`は無条件に所有権を移すが`capital_state_id`が
追従しないため、Save validationが`OwnershipMismatch`で失敗する。初回実装は
これを「E2Eで首都を除外して回避」するだけで済ませており、ユーザーから
「その正規経路で講和するとSave validationが失敗するため不十分」と指摘された。

**修正**: `resolve_capital_reassignment`(`src/war/peace.rs`、`execute_peace_settlement`
直前の純粋関数)を新設。`CedeWarGoalRegion`講和で対象州が対象国の
`capital_state_id`と一致する場合、割譲後にその国が所有する陸州の中から
**決定論的に最小`StateId`**を新首都として選ぶ。残存する陸州が無ければ
(既存コードベースに「国家消滅」概念が存在しないことをgrep確認済み、
ユーザー提示の代替案どおり)部分適用前に`Err`で安全に拒否する
(`war_error.peace.cede_would_leave_country_without_a_capital`翻訳キー、
EN/JA追加)。検証は`execute_peace_settlement`の既存の「全ステップ完了前に
`?`で早期return」パターンに従い、他のlineage検証と同じ原子性を持つ
(検証失敗時はState/War/Claim/Country Registryのいずれも変更されない)。

`execute_peace_settlement`のシグネチャへ`country_registry: &mut CountryRegistry`
を追加(13番目の引数)。呼び出し元3箇所
(`ui/peace_panel.rs::handle_peace_action_buttons`、
`war/update.rs::handle_daily_war_resolution`内2箇所)を更新。
`tests/land_war_combat_peace_test.rs`(保護対象ファイル)の呼び出し1箇所も
該当行のみ手動編集。この修正はCrisis由来Warに限定していない
——`transfer_region_ownership`の首都非追従は`CedeWarGoalRegion`講和全般の
既存ギャップだったため、一般レベルで修正した(通常Warの首都割譲講和にも
同じ再割当ロジックが適用される)。

新規テスト:
- `src/war/tests.rs`: `cede_capital_reassigns_to_lowest_remaining_state_id_and_succeeds`
  (成功時の再割当確認)、`cede_capital_with_no_remaining_states_is_safely_rejected`
  (残存州無し時の拒否+全Registry不変を確認)、
  `cede_non_capital_state_does_not_touch_capital_state_id`(非首都割譲では
  `capital_state_id`が一切変更されないことの回帰防止)。
- 既存`tests/land_war_combat_peace_test.rs`は実データ(Oceanic Magic Empire、
  `CountryId(3)`、実際の首都`StateId(8)`、6州所有)に対して無変更のまま
  再割当ロジックを実際にexerciseし、regressionなくpassすることを確認済み
  ——単体テストだけでなく実マップデータでも動作することの追加証拠。
- E2E `tests/p21_019_crisis_war_peace_resolution_e2e_test.rs::
  cede_capital_claim_generates_crisis_war_cession_and_survives_save_validation_and_load`:
  `pick_adjacent_border_pair_targeting_a_capital`で対象州が相手国の首都で
  ある組を意図的に選び、実際のAI Claim生成→Crisis開始→拒否→多国間War開始
  →`CedeWarGoalRegion`講和→**Save validation通過**→Save/Load往復後も
  領土移転・Claim消費・再割当済み首都のいずれも維持されることを確認
  (要求どおり「capital Claim → Crisis → War → 割譲 → Save validation →
  Save/Load」を1本のE2Eで通した)。

### Fix 2: Save validationにWar goalとの三者一致検証が不足していた問題

**問題**: `save/validate.rs::validate_wars`のCrisis↔Claim整合性検証が
`crisis.war_goals.first()`(先頭要素のみ)に依存していた。War自身の
`war_goals`との一致は検証していなかった。ユーザー指摘どおり、これでは
「先頭要素だけ一致し、2番目以降が食い違う」または「War側のwar_goalsだけが
Claim/Crisisと食い違う」壊れたSaveがvalidationを通過し、ロード後の
`resolve_crisis_war_lineage`(`claim.target_state == crisis`自身のtarget_state
の一致検証、§6の9ステップ目)で永久に失敗し得る。

**修正**: `related_claim_id`のClaimを正として、以下3者の一致を明示的に
検証するよう`validate_wars`を書き換えた:

1. **Claim target state**(`related_claim_id`から引くClaimの`target_state`)
2. **Crisis war goal**(`crisis.war_goals`の全要素の`target_states`)
3. **War war goal**(`war.war_goals`の全要素の`target_states`、新規追加)

いずれも`.first()`だけでなく`Vec`全体を`vec![claim.target_state]`と
完全一致比較する(`crisis_target_states != vec![claim.target_state]`
形式)。これにより「1 Claim = 1対象州」という現在の不変条件を、
先頭要素だけでなく明示的に検証する。

新規テスト(`src/save/validate.rs`):
- `war_own_war_goals_target_state_mismatch_with_claim_is_rejected`:
  War側の`war_goals`だけがClaim/Crisisと食い違う(Crisis側は正しいまま)
  壊れたSaveを`.first()`依存の旧検証では見逃していたはずのケースとして
  設計し、新検証が確実に`ParticipantMismatch`で拒否することを確認。
- `crisis_war_goals_with_extra_target_state_beyond_the_claim_is_rejected`:
  Crisisの`war_goals`に、先頭要素はClaimと一致するが2番目以降に余分な
  target_stateが追加された壊れたSaveを、旧`.first()`依存の検証では
  通過してしまっていたはずのケースとして設計し、新検証が拒否することを確認。

### Fix 3: P21-015/016の完全E2E未達だった問題

**問題**: 初回実装のE2E報告(§14注記)は「第三国支持・多国間War化は
手構築の単体テストだけで、E2E(実データ)では意図的に主張していない」と
明記していた。ユーザーから、要求53-65は実経路による完全E2Eであるべきで、
`pledge_support`を直接呼ぶような近道ではなく、実データを読み込んだ上で
PowerTier・同盟または関係値を決定論的に調整し、AI Claim→Crisis→**実際の**
AI支持判断→pledge_support→拒否→declare_war→supporter参戦→
execute_peace_settlementを早期returnなしで1本通すよう指摘された。

**修正**: `tests/p21_019_crisis_war_peace_resolution_e2e_test.rs`へ新規E2E
`ai_crisis_support_via_real_decision_path_joins_war_but_does_not_receive_ceded_territory`
を追加。以下の実経路を`pledge_support`の直接呼び出しなし・早期returnなしで
1本通す:

1. 実マップ(6か国28州)から隣接ペア(claimant/target)を選択(既存ヘルパー)。
2. 第三国`supporter`を`CountryId`昇順で1つ選択。
3. `force_power_advantage_with_ally`(新規ヘルパー、既存`force_power_advantage`
   と同じ人口操作手法の拡張)で、claimant・supporterの人口を大きく引き上げ、
   targetの人口を最小化。6か国では`PowerTier::GreatPower`は上位2枠
   (`compute_tier_counts(6) == (2, _)`)のみのため、claimant・supporterが
   その2枠を占め、targetは確実に非GreatPowerになるよう決定論的に調整。
4. `set_alliance`(新規ヘルパー)でsupporter⇔claimant間にAlliance条約を
   締結し、`decide_ai_crisis_support`の`ally_of_initiator`分岐を無条件に
   発火させる(関係値の大小に依存しない)。
5. 実際のAI Claim生成→Crisis開始まで日次シミュレーションで進行(既存ヘルパー)。
6. `advance_until_supporter_pledges_initiator`(新規ヘルパー)で、
   `pledge_support`を直接呼ばず、**日次ループから自動的に呼ばれる
   `evaluate_ai_crisis_support`/`decide_ai_crisis_support`**が
   `crisis.third_party_reactions[supporter] == SupportsInitiator`を
   実際に記録するまで日を進める(記録されなければpanic、早期returnによる
   スキップ無し)。
7. 拒否(既存の期限切れ自動拒否、`apply_rejection`が
   `build_support_snapshot`でこの時点の`third_party_reactions`を
   スナップショットし、`committed_attackers`へsupporterを含める)→
   AI正当化→`declare_war`まで日次進行。
8. `war.attackers.contains(&supporter)`を確認(third_party_reactionsの
   スナップショットだけでなく、実際にWarへ反映されていることの確認)。
   `war.primary_attacker_id() == claimant`(supporterではない)も確認。
9. `execute_peace_settlement`(`CedeWarGoalRegion`)を実行し、割譲州が
   **claimant**へ渡り、**supporterへは渡らない**ことを確認
   (`assert_ne!(new_owner, supporter)`)。
10. 元Claimが`Consumed`になることを確認。

これにより、要求53-65(第三国支持・多国間War化を含む完全E2E)を、
手構築フィクスチャではなく実際のAI判断経路で確認できた。既存の単体テスト
(`multilateral_crisis_war_transfers_territory_to_primary_attacker_not_supporters`
等)はこの新E2Eと役割が重複しないよう維持——単体テストは支持側の
組み合わせ網羅性(挿入順序等)を、新E2Eは「実データ・実AI判断経路での
到達可能性」をそれぞれ担保する。

---

## 22. GUI手動確認チェックリスト

実施していない。実ウィンドウでの目視確認は行っていないため、
**COMPLETE WITH MANUAL VERIFICATION PENDING**とする。以下、確認すべき項目:

1. Crisis由来Warで、要求州がCrisis initiatorへ実際に移転して見えるか
   (地図・州所有色の変化)
2. 講和成立後、外交パネルで元Claimの状態が「消費済み」表示になるか
   (P21-018の外国Claim一覧セクション経由)
3. 講和パネルにCrisis由来戦争目標行(claimant/対象国/州名/Claim状態)が
   実際に表示されるか、JA/EN切替で文言が変わるか
4. 通常War(Claim/Crisisと無関係)の講和パネルにCrisis戦争目標行が
   出ないことの目視確認
5. 講和成立後、cooldown(12か月)満了までAIが同じ対象へ新規Claimを
   生成しないことの目視確認
6. cooldown満了後、AIが新しいClaimを生成し、通常どおりP21-017/018の
   サイクルへ戻ることの目視確認
7. Save→Load後も、地図上の州所有・Claim状態・講和済みWarの表示が
   正しく復元されるか
8. 白紙講和・防御側勝利(AttackerConcedes)でも、領土は動かず
   Claimだけが消費済みになる表示になるか
9. 多国間Warで、支持国ではなく主要国(primary attacker)へ領土が渡る
   ことの目視確認
10. 講和パネルの既存スクロール・ボタン配置が崩れていないかの目視確認
