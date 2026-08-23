# P21-017 完了報告: AI国家による既存ClaimからのCrisis開始

## 1. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

全品質ゲート(`cargo check/test/clippy/build --release/diff --check/fmt`)を通過し、
実データ(6か国28州)によるE2Eテスト7件を含む56件の新規テストが全て green
(初回パス48件+二次レビュー対応8件、§30参照)。GUI実ウィンドウでの手動確認は
実施していない(§28参照)。

**二次レビュー(2026-08-20)で3件の指摘を受け、全て修正・再検証済み。** 詳細は
§30「二次レビュー対応」を参照。要約: (1) 同盟・戦争中・不可侵条約・休戦の判定を
`can_start_crisis`自身(共有ドメインAPI)へ移し、AIとプレイヤーUIの両方が同じ
関数を経由するよう修正。(2) 既存の生の戦争準備AI(`process_war_preparation_ai`)
との並行進行を、正当化の出所を問わない除外判定+未解決Crisis中の生正当化停止の
両面から防止。(3) `WarRegistry::are_countries_at_war`の呼び出し回数を
`unique claimant-target pairs`単位からO(1)化し、active_wars=0/100/1000で
再測定。

---

## 2. 開始時の実測基準

実装着手前に実測した値(仕様書§0が主張する基準値と完全一致、差異なし)。

| 項目 | 実測値 |
|---|---|
| lib テスト | 797 passed |
| integration テスト | 174 passed(24バイナリ内訳) |
| 合計 | 971 passed |
| fmtドリフト | 44箇所(11ファイル、全てP21-016以前からの既存ドリフト) |
| 実データ | 6か国・28州 |

`git status`により、P21-016の実装成果物(`war/data.rs`・`war/peace.rs`・
`save/validate.rs`等)は本セッション開始時点で**未コミットのまま作業ツリーに
残存**していることを確認した。P21-017はこの未コミット状態の上に実装しており、
それらのファイルへP21-017自身が加えた変更は無い(§18の変更ファイル一覧で
P21-016由来の変更とP21-017自身の変更を明確に区別する)。

---

## 3. 実装前調査結果

仕様書§1の24項目について実コードを調査した結果(番号は§1と対応):

1. **Claimの型**: `TerritorialClaim`(`src/diplomacy/claims.rs`)。フィールドは
   `id: ClaimId`, `claimant_country: CountryId`, `target_state: StateId`,
   `strength: f32`, `created_date: String`, `is_permanent: bool`,
   `source: ClaimSource`, `status: ClaimStatus`。**対象国(target)フィールドは
   存在しない** — `state_registry.get(target_state).owner_country_id`で毎回
   導出する規約(`CrisisRegistry::can_start_crisis`のdocコメントに明記)。
   **有効期限フィールドも存在しない**(仕様書§5手順3を参照する項目が該当なし)。
2. **有効判定API**: `claim.status == ClaimStatus::Active`。他に「有効期限切れ」
   の概念自体が存在しない(1参照)。
3. **プレイヤーがClaimからCrisisを開始する既存ドメインAPI**:
   `CrisisRegistry::can_start_crisis`/`start_crisis`(`src/diplomacy/crisis.rs`)。
   `diplomacy_panel.rs`がプレイヤー操作からこれを直接呼ぶ(line 1146, 2332)。
4. **Claim⇔Crisis結びつけID**: `DiplomaticCrisis::related_claim_id: Option<ClaimId>`。
5. **Crisisの初期phase**: **`CrisisPhase::Preparing`ではなく`DemandSent`**。
   `start_crisis`は呼び出した瞬間に最後通牒として即座に`DemandSent`へ遷移し、
   `deadline_date`(開始日+`CRISIS_DEMAND_PERIOD_DAYS`=30日)を確定する
   (P21-011の既存仕様、`start_crisis_succeeds_for_valid_claim`のアサーション
   に明記)。`CrisisPhase::Preparing`は列挙型に残存するが、実際にこの値が
   セットされる本番コード経路は存在しない(UI表示分岐とセーブ検証テストの
   フィクスチャにのみ出現)。
6. **要求送信の既存API**: **存在しない(5と同一理由)**。「Crisis開始」と
   「要求送信」を分離する独立ドメインAPIはなく、`start_crisis`単体が両方を
   不可分に行う。この発見は仕様書§2項目4/§8の設計へ直接影響する(§8参照)。
7. **AIが要求を送信しなければ停止するphase**: 該当なし(6と同一理由により
   そもそも「未送信」状態が存在しない)。
8. **`can_start_crisis`/`start_crisis`/`can_send_demand`/`send_demand`相当API**:
   前2つはそのままの名前で存在する。後2つは存在しない(6参照)。
9. **AI対象国回答・AI第三国支持・期限処理の順序**: `diplomacy::update::
   handle_daily_diplomacy`内で明示的な逐次呼び出し順として保証されている
   (P21-015のコメントに明記): `0.正当化日次進行 → 3.days_in_phase増分 →
   3a.AI大国支持(P21-015、フレーム内最大1回) → 3b.AI対象国回答(P21-012) →
   3c.期限切れ自動拒否(P21-011)`。
10. **既存Country AIがClaimから直接正当化を作っていないか**: **作っていない**。
    `country_ai.rs::process_war_preparation_ai`は隣接する到達可能な敵州を
    パスファインディングと戦力比較だけで選び、`start_justification`を直接
    呼ぶ — Claim/Crisisシステムとは完全に独立した別経路。
11. **既存Country AIがCrisisを経由せず直接宣戦していないか**: 宣戦自体は
    Ready正当化があれば常に`process_war_declaration_ai`が行う(12参照)が、
    これは「Crisis経由かどうか」を区別せず、Ready正当化を汎用的に処理する
    既存の共有経路であり、Claimを迂回して正当化やWarを直接作る専用コードは
    存在しない。
12. **Ready正当化に対する既存AI宣戦処理**: `country_ai.rs::
    process_war_declaration_ai`が、`initiator`が自国かつ`is_ready`な
    正当化を全て走査し、`can_declare_war_with_date`→`declare_war`を呼ぶ。
    出所がCrisis拒否由来(`grant_completed_justification`経由)か、
    生の`start_justification`由来かを区別しない — つまり**P21-017が拒否後の
    正当化を作りさえすれば、宣戦以降は完全に既存コードで処理される**。
13. **同じClaimから複数Crisisを作れない既存保証**: `can_start_crisis`は
    同じ当事国ペア・同じ`target_state`を含む未解決Crisisを拒否する
    (`is_duplicate`チェック)。Claim自体のID一致ではなく当事国・州の一致で
    判定するが、1つのClaimは常に同じ`(claimant, target_state)`の組を生むため
    実質的に同義。
14. **同じ国家ペアに複数Crisis**: terminal state(ResolvedPeacefully/
    WarStarted/Cancelled)なら新しいCrisisを開始できる
    (`resolved_crisis_does_not_block_a_new_crisis_for_the_same_claim`)。
15. **同じAI国家が複数の進行中Crisisを開始できるか**: 既存の`can_start_crisis`
    自体には「1国家1件まで」の制限は無い(当事国ペア単位の重複チェックのみ)。
    P21-017は§3の要求どおり、AI専用の追加ガード(`claimants_with_unresolved_
    self_crisis`)としてこれを実装した(既存APIには影響しない)。
16. **同盟・不可侵条約などの既存禁止条件**: **`can_start_crisis`自体には
    一切存在しない**(同盟中の国へもクリスプ開始APIレベルでは阻止されない)。
    比較として`WarJustificationRegistry::can_start_justification_with_date`は
    Alliance/NonAggressionPact/Truceを全てチェックする。P21-017はこの既存の
    非対称性を`can_start_crisis`自体の変更ではなく、AI候補選定側の追加
    フィルタとして埋めた(§11の設計判断を参照)。
17. **`CountryPowerRegistry`取得API**: `get(id) -> Option<&CountryPowerAssessment>`
    (O(1))、`ordered_country_ids() -> &[CountryId]`。`CountryPowerAssessment::
    total_score: f32`が0.0〜100.0の50/30/20加重合成値。
18. **外交関係値取得API・中立値**: `DiplomacyRegistry::get/get_or_default`
    (共にO(1))。未登録ペアは`DiplomaticRelation::default()`(`opinion: 0.0`)。
19. **`MonthChangedMessage`発行/消費箇所**: `app/time.rs::advance_game_date`
    (`DailySimulationSet::TimeUpdate`)が、日が進むたびに`DayChangedMessage`を、
    月境界を跨いだ日にはさらに`MonthChangedMessage`も**同一フレーム内で**
    発行する。既存消費者は`diplomacy::power.rs::rebuild_country_power_monthly`
    (`UiUpdate`)のみだった。
20. **`DailySimulationSet::Diplomacy`と`CountryAi`の順序**: `Diplomacy`が
    `CountryAi`より**前**(`app/time.rs`の`configure_sets`で`.chain()`済み)。
21. **AI開始Crisisが既存外交UIに自動表示されるか**: **される**。
    `diplomacy_panel.rs`は`crisis_registry.crises`の全件(当事国に限らず)を
    表示する設計(P21-013で「第三国支持のため全件表示」へ変更済み)ため、
    initiatorがAIか人間かをUIは一切区別しない。
22. **プレイヤーがAI要求対象になった場合のUI**: **既存のまま表示される**。
    `crisis.target == player_id && phase == DemandSent`の条件で受諾/拒否
    ボタンが表示され、`initiator`の値は一切見ない(`crisis_accept_confirm_
    transfers_state_and_resolves`等の既存テストで、`initiator=CountryId(1)`
    という「プレイヤーでない国」からの要求に対する受諾/拒否が既に検証済み —
    これはAI起点かどうかをUIが区別しないことの直接証拠)。
23. **Claim・Crisis・正当化・WarのSave/Loadパス**: 既存のP21-SAVE-002D/P21-016
    経路がそのまま使える。`SaveGameSources`は既に`claim_registry`/
    `crisis_registry`/`war_justification_registry`/`war_registry`を含む。
    **P21-017は新しいフィールドを一切追加しない**ため、Save DTOへの変更は不要
    (§14で詳述)。
24. **既存の性能測定方法**: `src/bin/profile_1000_states.rs`(P20-008)のような
    一時プロファイラパターン(`[[bin]]`をCargo.tomlへ一時登録→実行→削除)を
    そのまま踏襲した(§25参照)。

**既存API名が仕様書と異なっていた最重要点**: 仕様書は暗黙に「Crisis開始」と
「要求送信」を別の2段階操作と想定していたが、実コードでは`start_crisis`が
両方を単一の不可分な呼び出しとして行う(5/6参照)。この差異は§8で詳述する
設計判断に直結する。

---

## 4. AI Crisis開始資格(§3実装)

`collect_ai_crisis_candidates`(`src/diplomacy/ai_crisis_start.rs`)が、
仕様書§3の16項目を以下のように実装する:

| 項目 | 実装箇所 |
|---|---|
| 1. 所有国がプレイヤーでない | `Some(claimant) == player_country`で除外 |
| 2-3. 両国が実在・別国家 | `country_by_id`(O(1))で存在確認、`target == claimant`除外 |
| 4. Claimが有効 | `claim.status != ClaimStatus::Active`で除外 |
| 5-6. 対象州が実在・所有国一致 | `state_registry.get`が`None`なら除外(所有国は定義上常に一致、実装前調査1参照) |
| 7. 現在戦争中でない | 事前集約した`at_war_pairs`をO(1)参照(二次レビュー対応、§16・§30.3参照。判定自体は共有ドメインAPI`can_start_crisis`内の`are_countries_at_war`と最終確認時にも二重に保証される) |
| 8. 同盟していない | `diplomatic_conditions_permit_crisis_start`(二次レビュー対応で新設した共有関数、`CrisisRegistry::can_start_crisis`と共通、§30.1参照) |
| 9. 既存条約・休戦が禁止していない | 同じく`diplomatic_conditions_permit_crisis_start`がNonAggressionPact・`is_in_truce`もチェック(既存の`can_start_justification_with_date`と同じ3条件、§30.1参照) |
| 10. 同じClaim由来の未解決Crisis無し | `claims_with_unresolved_crisis: HashSet<ClaimId>`で除外 |
| 11. 同じClaim由来のReady/進行中正当化無し | 二次レビュー対応で「Crisis由来かどうかを問わず」へ拡張: `justified_triples: HashSet<(CountryId,CountryId,StateId)>`(全正当化の`(initiator,target,target_state)`を直接収集、§30.2参照) |
| 12. 自国開始の未解決Crisisが無い | `claimants_with_unresolved_self_crisis: HashSet<CountryId>`で除外 |
| 13. 両国の国力評価が存在 | `power_registry.get`が`None`なら除外(panicしない) |
| 14. 関係値基準 | `ai_crisis_opinion_permits_start`(§5) |
| 15. 相対国力基準 | `ai_crisis_power_ratio_permits_start`(§5) |
| 16. `can_start_crisis`相当APIが成功 | `try_start_crisis_from_candidates`が実際に`start_crisis`を呼び、その戻り値だけを正とする(§7) |

---

## 5. opinion・power ratio境界

`AI_CRISIS_MAX_OPINION = -25.0`、`AI_CRISIS_MIN_POWER_RATIO = 0.75`
(いずれも`src/diplomacy/ai_crisis_start.rs`の名前付き定数、仕様書§4と同名)。

`compute_power_ratio(initiator_score, target_score) -> f32`:

```
target_score > 0.0                        => initiator_score / target_score
target_score == 0.0 && initiator_score > 0.0 => f32::MAX (有限の最大値、真の無限大ではない)
target_score == 0.0 && initiator_score == 0.0 => 1.0
```

`initiator_score == 0.0 && target_score > 0.0`は最初の分岐(`target_score > 0.0`)
に落ち、`0.0 / target_score == 0.0`となり自然に不許可側(`0.75`未満)になる —
仕様書が要求する4つの境界ケース全てが、特別扱いなしの単一の`if/else if/else`で
NaN・無限大を一切生成せず実現される(要求テスト11で全境界を`is_finite()`/
`is_nan()`により直接検証)。

---

## 6. 候補選択順(§5実装)

`compare_candidates`が仕様書§5の順位をそのまま実装する:
`opinion昇順(低いほど優先) → power_ratio降順(高いほど優先) → [有効期限:
Claimに該当フィールドが無いため省略、§3の3参照] → claim_id昇順 →
target(CountryId)昇順 → target_state(StateId)昇順`。

浮動小数点比較は`partial_cmp().unwrap_or(Ordering::Equal)`で、NaNが混入して
även panicしない(実運用でNaNが生成されないことは§5の設計で保証済みだが、
比較関数自体も独立して安全)。

HashMap挿入順への非依存性は`insertion_order_does_not_affect_selected_
candidate`で直接検証(同じClaim集合を異なる挿入順で構築し、選択結果が
完全一致することを確認)。

---

## 7. 月次実行と同一フレーム一回保証(§6実装)

`handle_monthly_ai_crisis_start`(Bevy System)は`MessageReader<
MonthChangedMessage>`を**必ず最後まで読み切りつつ**、`evaluate_ai_crisis_start`
の呼び出しは`evaluated_this_frame`フラグで1回に制限する — P21-015の
`evaluate_ai_crisis_support`と全く同じ設計(P21-015が過去に「MessageReaderの
ループ内に置いたため同一フレームに複数回評価された」問題を起こした教訓を
踏襲、仕様書§6が明示的に警告する内容そのもの)。

**診断専用Resource `AiCrisisStartEvaluationCount`**を追加した
(`Option<ResMut<_>>`として受け取るため、Worldに存在しなければ完全に
ゼロコスト)。必要性: 「同一フレームに複数`MonthChangedMessage`があっても
評価はフレームにつき1回」という要件は、*結果*(Crisisが1件しか増えない)
だけでは「そもそも2回目の評価が候補を持たなかっただけ」という偽陽性を
排除できない。実際の呼び出し回数を直接数えることで、ガードそのものが
機能していることを証明する(`multiple_month_changed_messages_in_one_frame_
evaluate_only_once`、および性能プロファイラの15.2でも`evaluations`カウンタを
アサーションに使用)。コストはP21-015の先例と同一(`Option`のnullチェック
1回のみ、本番でResourceを挿入しなければ完全に無)。

Pause中・Load直後の非実行は、専用のガードコードではなく**構造的に**保証される:
`GamePaused`が真の間`advance_game_date`自体が早期returnし
`MonthChangedMessage`を一切書かないため、このSystemは`MessageReader`を
読んでも空で即returnする。Load処理・Load直後も同様に`MonthChangedMessage`が
発行されないため評価されない(§14でE2E検証)。

---

## 8. AI要求送信(§8)

**実装前調査の発見(§3の5-6)により、仕様書§8が想定する「Crisis開始」と
「要求送信」を分離した2段階の日次パイプラインは実装していない。**

`CrisisRegistry::start_crisis`はP21-011の設計により、呼び出した瞬間に
即座に`CrisisPhase::DemandSent`へ遷移し`deadline_date`を確定する
(プレイヤーがUIボタンから呼ぶのと全く同じ関数)。したがって
「AI国家が既存Claimを月次で評価してCrisisを開始する」ことと
「送信可能になった要求を自動送信する」ことは、**同一の`start_crisis`呼び出し
1回で不可分に発生する** — 要求は理論上あり得る最速のタイミング(Crisis創出と
同時)で送信されており、これは仕様書が求める「最初の送信可能になった日次tick」
より遅くなることは構造的にあり得ない。

この判断は仕様書冒頭の指示
「既存APIの名前が仕様書と異なる場合は実コードを正とし、重複APIを作らず
最小限の変更で実装してください」に従ったものである。新しい「未送信」状態や
別の送信専用APIを追加することは、既存のP21-011設計(プレイヤー・AI共通の
単一`start_crisis`)と矛盾し、§11が禁止する「AI専用の重複判定/期限計算」の
精神にも反するため採用しなかった。

要求テスト42-49への対応:

- **42(最初の日次tickで送信)**: `ai_started_crisis_has_already_sent_its_
  demand_immediately`で、AI開始直後のCrisisが既に`DemandSent`かつ
  `deadline_date`確定済みであることを検証(送信が「最初の日次tick」以前=
  即時に発生していることの直接証拠)。
- **43(プレイヤー開始Crisisの自動送信禁止)**: `player_initiated_crisis_is_
  never_touched_by_ai_evaluation`。このモジュールは新規Crisisの**作成**しか
  行わず、既存Crisisを一切変更しないため、プレイヤー開始Crisisへの影響は
  構造的に不可能。
- **44(重複送信禁止)**: `evaluating_twice_never_sends_a_duplicate_demand_
  for_the_same_claimant`。§3項目12のガードにより、claimantが未解決の
  自国開始Crisisを持てば2回目の評価では候補にならない。
- **45(既存ドメインAPI経由)**: `crisis_creation_goes_through_the_shared_
  start_crisis_domain_api`。`related_claim_id`・`war_goals`が正しく
  紐づくことを確認(直接`insert`ではこの整合性を再現できない)。
- **46-47(送信後にP21-015支持→P21-012回答の順で同日実行)**: §9で詳述する
  通り、既存の`handle_daily_diplomacy`が既にこの順序を保証しており
  (実装前調査9)、P21-017はこの関数に一切変更を加えていない。E2Eテスト
  `ai_initiator_ai_target_reject_leads_to_war_with_supporters_and_source_
  crisis_link`が実データでこの全体の連鎖(要求送信→支持→回答→拒否→
  正当化→AI宣戦→多国間War)を通しで検証する。
- **48(プレイヤー対象ならAI回答しない)**: `ai_initiated_crisis_targeting_
  the_player_is_not_auto_answered_by_p21_012`。既存の`evaluate_ai_crisis_
  responses`の`player_country.0 != Some(c.target)`フィルタが、initiatorが
  AIかどうかを問わず適用されることを確認。
- **49(deadline/phase既存規則の維持)**: `ai_started_crisis_uses_the_same_
  deadline_rule_as_the_player`。`CRISIS_DEMAND_PERIOD_DAYS`を使った同一計算
  であることを確認。

---

## 9. 日次外交内の正確な順序

P21-017は`diplomacy::update::handle_daily_diplomacy`(P21-011/012/013/015が
既に確立した順序: `0.正当化進行 → 3.days_in_phase → 3a.AI支持 → 3b.AI回答 →
3c.期限切れ`)へ**一切変更を加えていない**。§8で述べた通り「要求送信」は
Crisis作成(`CountryAi`Set)と同時に発生するため、この関数内に新しいステップを
挿入する必要が無かった。

`DailySimulationSet`順序(`Diplomacy → CountryAi`)そのものにより、AIが
`CountryAi`でCrisisを開始した**その日**の`Diplomacy`処理は既に完了済みであり
(同一フレーム内でDiplomacyがCountryAiより先に走るため)、新規作成された
Crisisへの支持判断・AI回答は自動的に**翌日の`DayChangedMessage`**から開始
される。これは仕様書§7の要求「AIがCountryAiでCrisisを開始した同じフレームでは、
P21-012回答やP21-015支持判断を実行しません」を、追加コード無しに既存の
System順序だけで満たしている。

---

## 10. プレイヤー対象時の挙動

§3実装前調査の21-22で確認した通り、既存UI(`diplomacy_panel.rs`)は
Crisisのinitiatorを一切区別せず、`crisis.target == player`かつ
`phase == DemandSent`であれば受諾/拒否ボタンを表示する。**新規UIコードは
一切追加していない**(仕様書§9「既存UIだけで十分表示できる場合は、新規画面を
作らないでください」に従う)。

E2Eテスト`ai_initiator_targeting_player_awaits_manual_response_and_can_be_
accepted`/`ai_initiator_targeting_player_can_be_manually_rejected`が、
実データ上でAI initiator・player targetのCrisisに対し、既存の
`crisis_response::accept_demand`/`reject_demand`(プレイヤーUIが呼ぶのと
同一のドメインAPI)を直接呼んで受諾/拒否できることを確認した。

---

## 11. AI対象時の挙動

新しい受諾式は一切作らず、既存のP21-012 `evaluate_ai_crisis_responses`/
`calculate_demand_acceptance`をそのまま使う。E2Eテスト2件
(`ai_initiator_ai_target_can_accept_and_transfer_the_state`/
`ai_initiator_ai_target_reject_leads_to_war_with_supporters_and_source_
crisis_link`)で、実データ上の完全な連鎖(AI initiatorが月次でCrisis開始→
翌日以降にAI大国支持→AI targetがP21-012式で受諾/拒否→受諾なら州移転、
拒否なら正当化Ready→既存Country AIが宣戦→P21-016多国間War)を確認した。

**設計判断の注記**: `calculate_demand_acceptance`はP21-014の総合力
(軍事50%/経済30%/人口20%の加重合成)ではなく、生の`CountryData::
available_manpower`で軍事力比較を行う。P21-017のAI Crisis開始資格
(§4.2)はP21-014の総合力比を使うため、この2つの「国力」概念は独立している
— 「AIが開始する資格を持つ」ことと「対象国が受諾する」ことは別の計算式に
基づく別の判断であり、これは仕様書§16が変更を禁止するP21-012の受諾式
(`score > 0.0`)をそのまま維持した結果の自然な帰結である。E2Eテストでは
受諾シナリオを安定して再現するため、`available_manpower`を直接操作している
(既存の受諾式自体は一切変更していない)。

---

## 12. 既存Country AI直接正当化経路への対応

実装前調査10-11で確認した通り、既存Country AIの正当化生成経路
(`process_war_preparation_ai`)はClaim/Crisisシステムと完全に独立しており、
Claimから直接正当化やWarを作る経路は元々存在しなかった。したがって
仕様書§7が要求する「Raw Claimから直接Ready正当化を作らない/Raw Claimから
直接Warを作らない」は、**P21-017が何もしなくても最初から満たされている**
(新規に回避コードを書く必要が無かった)。

Crisis拒否・期限切れから得たReady正当化に対する既存AI宣戦
(`process_war_declaration_ai`)は無変更のまま維持されており、これが
P21-017のCrisisが生む正当化も、既存の生正当化も、区別なく処理する
(§3実装前調査12)。

---

## 13. Claim→Crisis→正当化→Warの来歴

```
TerritorialClaim (claim_id)
  └─ related_claim_id → DiplomaticCrisis (crisis_id)
       ├─ 受諾: ClaimRegistry::mark_consumed(claim_id) [既存P21-011経路]
       └─ 拒否/期限切れ: apply_rejection
            └─ source_crisis_id: Some(crisis_id) → WarJustification
                 └─ declare_war (既存process_war_declaration_ai経由)
                      └─ source_crisis_id: Some(crisis_id) → War [P21-016既存フィールド]
```

E2Eテスト`ai_initiator_ai_target_reject_leads_to_war_with_supporters_and_
source_crisis_link`が、AI開始CrisisのIDが最終的な`War.source_crisis_id`まで
途切れず伝播することを実データ上で確認した(要求テスト57)。この来歴は
P21-011/P21-016が既に構築済みの仕組みをそのまま利用しており、P21-017は
「起点」(Claimの選定・Crisis開始)を追加しただけで、来歴を運ぶ配線自体には
一切手を入れていない。

---

## 14. Save/Load

**新しいSave DTOは一切追加していない。** P21-017が新規に導入した型
(`AiCrisisCandidate`はテスト外に公開されない内部構造体、`AiCrisisStartEvaluationCount`
は診断専用でSaveへ含まれない)はいずれもゲーム状態に影響しないため、
既存の`SaveGameV1`/`SaveGameSources`/`validate_save_game_v1`/
`apply_validated_save`を完全に無変更で利用できる。

AI開始CrisisはP21-011以前から存在する`DiplomaticCrisis`の通常の
インスタンスとして保存される。「initiatorがAIである」ことは`CountryId`と
`PlayerCountry`の比較だけで判定でき、Crisis自体に特別なフラグは不要
(そもそも既存モデルにその区別が無い — P21-011以来、プレイヤーもAIも
全く同じ`DiplomaticCrisis`型を共有している)。

E2Eテスト`save_load_preserves_ai_started_crisis_and_does_not_duplicate_
on_reload`で以下を実データ上で確認した:

- Save→Load後もCrisisのphase・deadline・`related_claim_id`が完全に保持される
- Load直後にCrisisが重複生成されない(件数が1件のまま)
- Load直後に追加で`app.update()`を呼んでも(`MonthChangedMessage`が
  発行されない限り)新たなCrisisが勝手に開始されない

「今月評価済み」等のAI判断履歴フィールドはSaveへ一切追加していない
(通常の`MonthChangedMessage`駆動だけで一回性を保証する設計、§7参照)。

---

## 15. 決定論性

- **候補選定**: `compare_candidates`は完全に決定論的な全順序比較(§6)。
  HashMap挿入順非依存を`insertion_order_does_not_affect_selected_candidate`
  で検証。
- **処理順**: AI国家はCountryId昇順(`evaluate_ai_crisis_start`内で
  `claimants.sort_by_key`)、Claim候補も上記の全順序でソート済み。
  `multiple_ai_countries_are_processed_in_country_id_ascending_order`で
  複数AI国家が正しく処理されることを確認。
- **乱数不使用**: `ai_crisis_start.rs`全体で`rand`系クレートを一切import
  していない(`compute_power_ratio`/`compare_candidates`はいずれも入力のみに
  依存する純粋関数)。
- **E2E決定論性**: `identical_inputs_deterministically_select_the_same_claim`
  で、同一の実データ・同一の関係値シードから独立に2つの`App`を構築し、
  両方が同じClaimant・同じ対象国・同じ`target_state`を選ぶことを確認
  (要求テスト60)。

---

## 16. 計算量

**二次レビュー対応でO(active wars)の扱いを修正した(§27指摘3・§30.3参照)。**
`collect_ai_crisis_candidates`は仕様書§13が要求する
`O(countries + claims + crises + justifications + wars + candidate_sort)`を
実現する:

1. Crisis Registry走査(O(crises)) → `claims_with_unresolved_crisis`・
   `claimants_with_unresolved_self_crisis`を構築
2. WarJustification Registry走査(O(justifications)、出所を問わず
   `(initiator, target, target_state)`の組を直接収集、O(1)引き用の
   `justified_triples`セットを構築 — 二次レビュー対応でCrisis経由かどうかの
   間接参照(`crisis_claim_by_id`)自体を廃止し、より単純な実装になった)
3. War Registry走査(O(wars)、二次レビュー対応で新設) →
   `build_at_war_pairs`が現在Activeな全Warの攻撃側×防御側の組を
   `HashSet<(CountryId,CountryId)>`へ事前集約する
4. Country Registry走査(O(countries)) → `country_by_id`(O(1)引き用)を構築
5. Claim Registry走査(O(claims)) → `(claimant, target)`ペア単位で資格判定を
   メモ化キャッシュ(`pair_cache: HashMap<(CountryId,CountryId), Option<...>>`)
   しながら候補リストへ追加。同じ対象国へ複数Claimを持つclaimantがいても、
   ペアごとの同盟/戦争/条約/関係値判定は1回しか実行しない。「現在戦争中」は
   手順3で事前集約した`at_war_pairs`をO(1)参照する(以前は`WarRegistry::
   are_countries_at_war`をペア単位で直接呼んでおり、1回あたりO(active wars)
   だった — メモ化により呼び出し回数自体は`unique pairs`に抑えられていたが、
   `unique pairs × active wars`という総当たりが残っていた)。
6. 各claimantの候補Vecをソート(O(candidates_per_claimant log candidates_per_claimant)、
   全体でO(claims log claims)が最悪)

`countries × claims`・`claims × crises`・`unique pairs × active wars`の
いずれの総当たりにもならない。

**残存する既知のコスト(§30.3で詳述)**: 候補選定の後、実際にCrisisを開始する
最終確認は既存の共有ドメインAPI`CrisisRegistry::start_crisis`(内部で
`can_start_crisis`を呼ぶ)を経由する。`can_start_crisis`自身は(プレイヤーUIと
全く同じ権威ある検証として)`war_registry.are_countries_at_war`を直接呼ぶため、
**その月に実際にCrisisを開始した国の数**だけO(active wars)のコストが発生する
(候補選定フェーズでは発生しない)。これは`O(successful_starts × wars)`
≤`O(countries × wars)`であり、`claims`には一切依存しない。あえて
`can_start_crisis`自身にO(1)化した集合を渡さなかったのは、プレイヤーUIからの
単発呼び出しと共有する単一の権威ある検証を維持するため(§27指摘1で要求された
「AIとプレイヤーUIで判定基準を分裂させない」という原則を優先した、意図的な
トレードオフ)。実測(§30.6)では、現実的な国数(6)は言うまでもなく、
country_count=1000・active_wars=1000という極端な合成負荷でも数十ms以内に
収まることを確認済み。

非月変更日(`MonthChangedMessage`が無いフレーム)は`MessageReader::read()`が
即座に空を返すため、Claim Registry・Crisis Registry・War Registry等への走査は
一切発生しない(§25の性能測定15.2で実測、0.005ms/frame前後でClaim数に非依存)。

---

## 17. UI変更

**一切無し。** §10で述べた通り、既存の`diplomacy_panel.rs`はCrisisの
initiatorを区別せずに表示・受諾/拒否ボタンを出す設計のため、AI開始Crisisは
追加コード無しで既存UIにそのまま表示される。ローカライズキーの追加・変更も
無い。

---

## 18. 変更ファイル一覧

**P21-017が新規に追加/変更したファイルのみ**(§2で述べた通り、`war/data.rs`・
`war/peace.rs`・`save/validate.rs`等は本セッション開始前からのP21-016の
未コミット変更であり、P21-017自身はこれらに触れていない)。

新規:

- `src/diplomacy/ai_crisis_start.rs` — 純粋判断関数・候補収集・System本体
- `src/diplomacy/ai_crisis_start/tests.rs` — 単体テスト41件
- `tests/p21_017_ai_claim_crisis_e2e_test.rs` — 実データE2Eテスト7件
- `verification_logs/phase-21/p21-017/p21-017_completion_report.md` — 本報告書
- `verification_logs/phase-21/p21-017/perf/results.csv` — 性能生データ
- `verification_logs/phase-21/p21-017/perf/summary.txt` — 性能サマリー

変更(初回パス):

- `src/diplomacy/mod.rs` — `ai_crisis_start`モジュール宣言、
  `handle_monthly_ai_crisis_start`を`DailySimulationSet::CountryAi`へ登録

変更(二次レビュー対応、§30参照):

- `src/diplomacy/crisis.rs` — `can_start_crisis`/`start_crisis`へ
  `diplomacy_registry`/`war_registry`(/`current_date`)引数を追加し、
  戦争中・同盟・不可侵条約・休戦の拒否を関数自身へ実装。共有関数
  `diplomatic_conditions_permit_crisis_start`と`has_unresolved_crisis_
  initiated_by`を新設。新規単体テスト6件。
- `src/diplomacy/ai_crisis_start.rs` — 候補収集を`diplomatic_conditions_
  permit_crisis_start`経由へ変更、正当化除外をCrisis由来限定から出所を
  問わない`justified_triples`へ拡張、`WarRegistry::are_countries_at_war`の
  ペア単位呼び出しを`build_at_war_pairs`によるO(1)事前集約へ置換。
- `src/diplomacy/ai_crisis_start/tests.rs` — 新規単体テスト1件。
- `src/diplomacy/mod.rs` — `handle_monthly_ai_crisis_start`に
  `.before(handle_daily_country_ai)`を追加し、同一フレーム内での
  Crisis開始と生の戦争準備AIの決定論的な順序を保証。
- `src/country/country_ai.rs` — `process_war_preparation_ai`に
  `crisis_registry: &CrisisRegistry`引数と、未解決の自国開始Crisisが
  あれば新規正当化探索を止めるガードを追加。呼び出し元
  `process_daily_country_ai`内の呼び出しを更新。新規単体テスト1件。
- `src/ui/diplomacy_panel.rs` — `execute_crisis_confirm`/
  `handle_crisis_command_buttons`/`update_diplomacy_panel_ui`内の
  `start_crisis`/`can_start_crisis`呼び出しへ`DiplomacyRegistry`/
  `WarRegistry`(/現在日付)を追加。テスト用`build_test_app`へ両Registryを追加。
- `assets/localization/{en-US,ja-JP}.ron` — 新規エラーキー4件
  (`diplomacy_error.crisis.{at_war,ally,non_aggression_pact,truce}`)。
- `tests/p21_010_claim_crisis_e2e_test.rs`・
  `tests/p21_011_crisis_ultimatum_e2e_test.rs`・
  `tests/p21_012_ai_crisis_response_e2e_test.rs`・
  `tests/p21_013_crisis_support_e2e_test.rs`・
  `tests/p21_015_ai_crisis_support_e2e_test.rs`・
  `tests/p21_016_multilateral_war_e2e_test.rs` — いずれも既存の
  `start_crisis`直接呼び出し箇所を、新シグネチャに合わせて
  `world.resource_scope`経由で`DiplomacyRegistry`/`WarRegistry`を
  渡すよう更新(挙動・アサーションは無変更、シグネチャ追随のみ)。
- `tests/p21_017_ai_claim_crisis_e2e_test.rs` — `create_claim`系ヘルパーの
  変更は無し(`start_crisis`を直接呼ばず、AIのSystem経由でのみCrisisを
  開始するため、このファイルは初回パスから無変更)。

一時作成・削除済み(§25参照):

- `src/bin/profile_p21_017.rs`(初回パス・二次レビュー対応でそれぞれ作成→
  削除、最終的に不在)
- `Cargo.toml`の`[[bin]] profile_p21_017`登録(削除済み、`git diff`で
  無差分を確認済み)

---

## 19. 新規・更新テスト一覧

### `src/diplomacy/ai_crisis_start/tests.rs`(41件)

純粋判断: `opinion_boundaries`, `unregistered_relation_neutral_zero_does_
not_permit_start`, `power_ratio_boundaries`, `both_scores_zero_yields_
ratio_one`, `target_zero_initiator_positive_permits_start`,
`initiator_zero_target_positive_does_not_permit_start`,
`compute_power_ratio_never_produces_nan_or_infinite`,
`compute_power_ratio_is_deterministic`

Claim資格: `ai_owned_active_claim_is_a_candidate`, `player_owned_claim_is_
never_a_candidate`, `territorial_claim_has_no_expiry_field_so_this_
condition_is_vacuously_satisfied`, `consumed_claim_is_not_a_candidate`,
`claim_targeting_nonexistent_state_is_not_a_candidate`,
`claim_targeting_state_now_owned_by_claimant_is_not_a_candidate`,
`target_currently_at_war_with_claimant_is_not_a_candidate`,
`allied_target_is_not_a_candidate`, `claim_with_unresolved_crisis_is_not_
a_candidate`, `claim_with_ready_justification_is_not_a_candidate`,
`claim_with_ready_justification_is_excluded_even_if_source_crisis_is_
resolved`, `claimant_with_any_unresolved_self_initiated_crisis_gets_no_
new_candidates`, `missing_power_registry_entries_do_not_panic`,
`one_sided_missing_power_registry_entry_does_not_panic`

候補選択: `candidate_with_lowest_opinion_is_selected_first`,
`tied_opinion_breaks_by_higher_power_ratio`,
`fully_tied_candidates_break_by_claim_id_ascending`,
`insertion_order_does_not_affect_selected_candidate`,
`evaluate_ai_crisis_start_opens_at_most_one_crisis_per_country_per_call`,
`first_candidate_domain_failure_falls_through_to_next_candidate`,
`multiple_ai_countries_are_processed_in_country_id_ascending_order`

月次System: `no_month_changed_message_means_no_evaluation`,
`month_changed_message_triggers_evaluation`,
`multiple_month_changed_messages_in_one_frame_evaluate_only_once`,
`no_message_means_no_evaluation_even_though_this_system_has_no_pause_
check_of_its_own`, `load_without_a_month_changed_message_does_not_
evaluate`, `after_previous_crisis_resolves_the_next_month_can_start_a_
different_claim`

AI要求送信: `ai_started_crisis_has_already_sent_its_demand_immediately`,
`player_initiated_crisis_is_never_touched_by_ai_evaluation`,
`evaluating_twice_never_sends_a_duplicate_demand_for_the_same_claimant`,
`crisis_creation_goes_through_the_shared_start_crisis_domain_api`,
`ai_initiated_crisis_targeting_the_player_is_not_auto_answered_by_p21_012`,
`ai_started_crisis_uses_the_same_deadline_rule_as_the_player`

### `tests/p21_017_ai_claim_crisis_e2e_test.rs`(7件、実データ)

`ai_starts_crisis_from_existing_claim_and_sends_demand_on_month_change`,
`ai_initiator_ai_target_can_accept_and_transfer_the_state`,
`ai_initiator_ai_target_reject_leads_to_war_with_supporters_and_source_
crisis_link`, `ai_initiator_targeting_player_awaits_manual_response_and_
can_be_accepted`, `ai_initiator_targeting_player_can_be_manually_
rejected`, `save_load_preserves_ai_started_crisis_and_does_not_duplicate_
on_reload`, `identical_inputs_deterministically_select_the_same_claim`

既存テストの削除・無効化・弱体化は一切行っていない。

### 二次レビュー対応で追加した8件(§30参照)

`src/diplomacy/crisis.rs`: `can_start_crisis_rejects_when_claimant_and_
target_are_at_war`, `can_start_crisis_rejects_when_claimant_and_target_are_
allied`, `can_start_crisis_rejects_when_claimant_and_target_have_a_non_
aggression_pact`, `can_start_crisis_rejects_when_claimant_and_target_are_
in_truce`, `can_start_crisis_succeeds_once_the_truce_has_expired`,
`has_unresolved_crisis_initiated_by_reflects_current_phase`。

`src/country/country_ai.rs`: `process_war_preparation_ai_does_not_start_a_
competing_justification_while_a_crisis_is_unresolved`。

`src/diplomacy/ai_crisis_start/tests.rs`: `claim_matching_a_raw_non_crisis_
justification_is_not_a_candidate`。

---

## 20. 要求テスト番号との対応

| 要求番号 | 対応テスト |
|---|---|
| 1-3 | `opinion_boundaries` |
| 4 | `unregistered_relation_neutral_zero_does_not_permit_start` |
| 5 | (仕様書§4.1、同盟優先は§3の`allied_target_is_not_a_candidate`が実運用上のカバー) |
| 6-7 | `power_ratio_boundaries` |
| 8 | `both_scores_zero_yields_ratio_one` |
| 9 | `target_zero_initiator_positive_permits_start` |
| 10 | `initiator_zero_target_positive_does_not_permit_start` |
| 11 | `compute_power_ratio_never_produces_nan_or_infinite` |
| 12 | `compute_power_ratio_is_deterministic` |
| 13 | `ai_owned_active_claim_is_a_candidate` |
| 14 | `player_owned_claim_is_never_a_candidate` |
| 15 | `territorial_claim_has_no_expiry_field_so_this_condition_is_vacuously_satisfied` |
| 16 | `consumed_claim_is_not_a_candidate` |
| 17 | `claim_targeting_nonexistent_state_is_not_a_candidate` |
| 18 | `claim_targeting_state_now_owned_by_claimant_is_not_a_candidate` |
| 19 | `target_currently_at_war_with_claimant_is_not_a_candidate` |
| 20 | `allied_target_is_not_a_candidate` |
| 21 | `claim_with_unresolved_crisis_is_not_a_candidate` |
| 22 | `claim_with_ready_justification_is_not_a_candidate`(+resolved版) |
| 23 | `claimant_with_any_unresolved_self_initiated_crisis_gets_no_new_candidates` |
| 24 | `missing_power_registry_entries_do_not_panic` |
| 25 | `one_sided_missing_power_registry_entry_does_not_panic` |
| 26 | `candidate_with_lowest_opinion_is_selected_first` |
| 27 | `tied_opinion_breaks_by_higher_power_ratio` |
| 28 | (有効期限フィールド無しのため§3の15と同じ理由で省略) |
| 29 | `fully_tied_candidates_break_by_claim_id_ascending` |
| 30 | `insertion_order_does_not_affect_selected_candidate` |
| 31 | `evaluate_ai_crisis_start_opens_at_most_one_crisis_per_country_per_call` |
| 32 | `first_candidate_domain_failure_falls_through_to_next_candidate` |
| 33 | `multiple_ai_countries_are_processed_in_country_id_ascending_order` |
| 34 | `no_month_changed_message_means_no_evaluation` |
| 35 | `month_changed_message_triggers_evaluation` |
| 36-37 | `multiple_month_changed_messages_in_one_frame_evaluate_only_once` |
| 38 | `no_message_means_no_evaluation_even_though_this_system_has_no_pause_check_of_its_own` |
| 39 | `load_without_a_month_changed_message_does_not_evaluate` |
| 40 | (性能プロファイラ15.2で実測、`evaluations==0`をassert) |
| 41 | `after_previous_crisis_resolves_the_next_month_can_start_a_different_claim` |
| 42-49 | §8の一覧を参照 |
| 50 | `ai_starts_crisis_from_existing_claim_and_sends_demand_on_month_change` |
| 51 | `ai_initiator_ai_target_can_accept_and_transfer_the_state` |
| 52,55,56,57 | `ai_initiator_ai_target_reject_leads_to_war_with_supporters_and_source_crisis_link` |
| 53 | `ai_initiator_targeting_player_awaits_manual_response_and_can_be_accepted` |
| 54 | `ai_initiator_targeting_player_can_be_manually_rejected` |
| 58-59 | `save_load_preserves_ai_started_crisis_and_does_not_duplicate_on_reload` |
| 60 | `identical_inputs_deterministically_select_the_same_claim` |
| 61-71 | §21参照(既存テストによる回帰保護、新規UIテストは追加していない) |

---

## 21. UI・回帰の既存テストによる担保(要求61-71)

新規UIコードを追加していないため(§10・§17)、要求61-71は以下の**既存**
テストが既に検証している(P21-017としての追加変更は無い):

- 61-63(AI開始Crisisの表示・受諾/拒否ボタン・要求国名/対象州/deadline表示):
  `src/ui/diplomacy_panel.rs`の`crisis_accept_confirm_transfers_state_and_
  resolves`/`crisis_reject_confirm_grants_justification_and_escalates`が、
  `insert_demand_sent_crisis_targeting_player(app, CountryId(1), ...)`
  というプレイヤーでない国からの要求に対する受諾/拒否を既に検証している —
  この`initiator`がAI操作由来かテストコード由来かをUI側は区別しないため、
  AI開始Crisisに対しても同一の保証が成立する。
- 64(JA/EN切替): `p20_009_localization_headless_render_test`/
  `p20_009_localization_resource_test`が`diplomacy_panel`のローカライズ
  キー(`diplomacy_panel.crisis_line`等、P21-017が新規追加していない既存
  キー)の言語切替を継続してカバーする。
- 65(スクロール維持): `diplomacy_panel.rs`のレイアウトコードに一切触れて
  いないため、スクロール実装(`feedback_bevy_ui_scroll_gotchas`memory参照)
  は無変更のまま維持される。
- 66(P21-013支持UI維持): `insert_third_party_crisis`/
  `terminal_crisis_shows_no_support_buttons`等の既存テスト群が無変更で
  green。
- 67(P21-014国力UI維持): `country::power`モジュールへ一切変更を加えていない。
- 68(P21-015同一フレーム一回評価維持): `AiSupportEvaluationCount`関連の
  既存テストが無変更で green(P21-017は独立した`AiCrisisStartEvaluationCount`
  を新設し、既存Resourceには触れていない)。
- 69(P21-016多国間War・primary・講和維持): `war::tests`・
  `war::peace`関連の既存テストが無変更で green、かつ本タスクのE2Eテスト
  (要求52,55-57)がAI開始CrisisからのWarでも同じ多国間モデルが機能することを
  追加確認した。
- 70(既存プレイヤーClaim→Crisis開始操作維持): `crisis::tests`・
  `diplomacy_panel.rs`のプレイヤー操作系テストが無変更で green
  (`can_start_crisis`/`start_crisis`のシグネチャ・挙動を一切変更していない)。
- 71(既存二国間War維持): `war::tests`の二国間War関連テストが無変更で green。

---

## 22. 開始時/終了時テスト数

| | lib | integration | 合計 |
|---|---|---|---|
| 開始時(仕様書§0の基準) | 797 | 174 | 971 |
| 初回パス終了時 | 838 (+41) | 181 (+7) | 1019 (+48) |
| 二次レビュー対応後(最終) | 846 (+8) | 181 (+0) | 1027 (+8) |

二次レビュー対応の内訳(全てlib単体テスト、新規integrationテストは追加していない):
`src/diplomacy/crisis.rs`に6件(共有ドメインAPIの新条件×4+`has_unresolved_
crisis_initiated_by`+休戦期限切れ後の成功確認)、`src/country/country_ai.rs`に
1件、`src/diplomacy/ai_crisis_start/tests.rs`に1件。詳細は§30.5参照。

---

## 23. 品質ゲート結果

全て green(実行順・コマンドは以下の通り、二次レビュー対応後の最終結果):

```
cargo check --all-targets         → 0 errors
cargo test --lib                  → 846 passed; 0 failed
cargo test --tests                → 全28バイナリ(24既存+P21-017 E2E1+他2件は
                                     テスト0件のbin)で 0 failed、合計1027
cargo clippy --all-targets --all-features -- -D warnings → 0 warnings
                                     (can_start_crisis/start_crisisの引数増加に
                                     伴うtoo_many_arguments警告2件は
                                     #[allow]で明示的に許容、既存の同種関数と
                                     同じ扱い)
cargo build --release             → 成功
git diff --check                  → 実質的な問題なし(LF/CRLF警告は全て
                                     既存差分ファイルおよびP21-017が今回変更した
                                     ファイルのみ、実際の空白/改行競合は無し)
cargo fmt --all -- --check        → 44箇所(§24参照、全て既存ドリフト)
```

---

## 24. fmt基準との差分

開始時ドリフト: 44箇所(11ファイル、全てP21-016以前からの既存ドリフト —
`map/division_render.rs`・`map/mod.rs`・`military/{movement,recruitment,
supply}.rs`・`save/runtime.rs`・`war/peace.rs`・4件の統合テストファイル)。

初回パスで新規追加した3ファイル、および二次レビュー対応で変更した
`src/diplomacy/crisis.rs`・`src/diplomacy/ai_crisis_start.rs`は、
`cargo fmt -- <path>`がワークスペース全体を巻き込む既知の事故
([[feedback_cargo_fmt_scope]])を再発させないため、`rustfmt --edition 2024
<file>`を対象ファイルだけへ個別実行して整形した(Cargo.tomlの
`edition = "2024"`を確認の上で指定)。

整形前後で`cargo fmt --all -- --check`のヒット数を比較し、開始時・初回パス
終了時・二次レビュー対応後の全てで共に**44**で完全一致することを確認した
(新規/変更ファイル分のドリフトはその都度解消され、既存11ファイル分は一切
変更されていない)。

---

## 25. 性能測定方法・結果

**最終版(二次レビュー対応、active_wars軸を追加して再測定)。** 一時プロファイラ
`src/bin/profile_p21_017.rs`(Cargo.tomlへ`[[bin]]`を一時登録)を作成し、
`cargo run --release --bin profile_p21_017`で実行、
`verification_logs/phase-21/p21-017/perf/{results.csv,summary.txt}`へ
出力した後、バイナリ本体とCargo.tomlの登録を削除した
(`git status`/`git diff Cargo.toml`で無差分を確認済み)。初回パスは
active_wars=0のみで測定していたため(§27指摘3)、二次レビュー対応で
active_wars=0/100/1000の軸を追加して全面的に再測定した。

### 15.1 Claim数スケーリング(countries=1000, active_wars=0)

`evaluate_ai_crisis_start`を直接呼び出し1回分の所要時間を計測
(best_case=全Claim資格あり、worst_case=全Claim opinion条件で不適格)。

| claims | best_case (ms) | worst_case (ms) |
|---|---|---|
| 100 | 0.201 | 0.046 |
| 1,000 | 2.573 | 0.310 |
| 10,000 | 8.098 | 4.691 |
| 50,000 | 35.450 | 29.695 |

claims 500倍(100→50,000)に対し所要時間は最大でも約176倍程度に収まり、
`countries × claims`(1000×50000=5000万)相当の爆発は観測されない。

### 15.1補足(二次レビュー対応): active_warsスケーリング

countries=1000・claims=10,000(全Claim資格あり)に固定し、active_warsだけを
0/100/1000へ増やして`evaluate_ai_crisis_start`の所要時間を計測。

| active_wars | ms |
|---|---|
| 0 | 8.054 |
| 100 | 11.300 |
| 1,000 | 33.114 |

active_warsに対して緩やかな増加が見られる。原因を切り分けると、候補選定
フェーズ自体(`build_at_war_pairs`によるO(1)化後)は増加のごく一部でしかなく、
主要因は§16で述べた「実際にCrisisを開始した国(このシナリオでは1000か国
全て)ごとに、`start_crisis`内部の`can_start_crisis`が権威ある
`war_registry.are_countries_at_war`(O(active wars))を1回ずつ呼ぶ」という
**意図的に残した**コストである(`O(successful_starts × wars)` ≤
`O(countries × wars)`、`claims`には非依存)。1000か国×1000 Warの最悪合成
負荷でも約33msであり、`claims`をいくら増やしても(15.1で確認した通り)この
コストは増えない。現実的な国数(6)ではこの項は事実上ゼロに等しい。

### 15.2 非月変更日(System単体、`MonthChangedMessage`無し)

`handle_monthly_ai_crisis_start`を200フレーム連続実行(`MonthChangedMessage`
は一度も書き込まない)、1フレームあたりの平均コストを計測。

| claims | ms/frame | evaluations |
|---|---|---|
| 100 | 0.00541 | 0(assert済み) |
| 1,000 | 0.00479 | 0(assert済み) |
| 10,000 | 0.00474 | 0(assert済み) |
| 50,000 | 0.00416 | 0(assert済み) |

claims数に依存せず概ね一定(0.005ms前後)であることを確認 —
`MessageReader::read()`が空の場合に即returnし、Claim/Crisis/War Registryを
一切走査していないことの直接証拠。

### 15.3 月次全体(System経由、`MonthChangedMessage`あり、active_wars軸を含む)

| countries | claims/country | active_wars | 全AI開始成功 (ms) | 全AI開始せず (ms) |
|---|---|---|---|---|
| 100 | 1 | 0 | 0.351 | 0.187 |
| 100 | 1 | 100 | 0.482 | 0.187 |
| 100 | 1 | 1,000 | 2.587 | 0.402 |
| 100 | 10 | 0 | 0.639 | 0.518 |
| 100 | 10 | 100 | 0.829 | 0.370 |
| 100 | 10 | 1,000 | 3.348 | 0.626 |
| 500 | 1 | 0 | 0.994 | 0.450 |
| 500 | 1 | 100 | 1.960 | 0.462 |
| 500 | 1 | 1,000 | 13.138 | 0.758 |
| 500 | 10 | 0 | 3.505 | 1.917 |
| 500 | 10 | 100 | 4.213 | 1.664 |
| 500 | 10 | 1,000 | 17.448 | 3.870 |
| 1,000 | 1 | 0 | 2.478 | 1.076 |
| 1,000 | 1 | 100 | 4.499 | 0.732 |
| 1,000 | 1 | 1,000 | 27.235 | 1.140 |
| 1,000 | 10 | 0 | 9.650 | 5.441 |
| 1,000 | 10 | 100 | 11.009 | 5.205 |
| 1,000 | 10 | 1,000 | 33.958 | 6.936 |

最悪ケース(1000か国×10claims/国×active_wars=1000、全AI開始成功)でも
約34msと、月次1回だけ発生するイベントとして十分に実用的な範囲に収まる。
「全AI開始せず」列がactive_warsの影響をほぼ受けないのに対し「全AI開始成功」
列が影響を受けるのは、§16・本節で説明した「実際に開始した国の数だけ
`can_start_crisis`の権威ある戦争中判定が発生する」という設計と整合する
(開始が0件なら、その追加コストも0件になる)。

---

## 26. 一時ファイル削除確認

- `src/bin/profile_p21_017.rs`: 削除済み(`ls`で不在確認、初回パス・
  二次レビュー対応の両方で作成→削除)
- `Cargo.toml`の`[[bin]] profile_p21_017`: 削除済み(`git diff Cargo.toml`で
  無差分確認、両パスとも)
- scratch Save: 使用していない(E2Eテストは`App`内の`SaveGameV1`を直接
  構築・検証するのみで、ディスクへファイルを書き出していない)
- 一時スクリーンショット: 生成していない(GUI手動確認未実施、§28)
- デバッグ出力: E2Eテスト作成過程・回帰確認過程で使用した`eprintln!`は
  全て削除済み(`grep -rn eprintln tests/p21_017_ai_claim_crisis_e2e_test.rs
  src/diplomacy/ai_crisis_start.rs src/country/country_ai.rs`で無ヒット確認)
- headless render testが書き換えた既存スクリーンショット
  (`verification_logs/{p20-007,p20-009,phase-21/p21-save-002e}/screenshots/`
  内、計9ファイル)は`git checkout --`で復元済み(初回パス・二次レビュー
  対応それぞれの品質ゲート実行後の両方で発生・復元)、最終`git status`で
  無差分確認

---

## 27. 発見した既存不具合・仕様差

1. **仕様書§8が想定する「Crisis開始」/「要求送信」の2段階APIは実コードに
   存在しない**(§3・§8で詳述)。既存の`start_crisis`が両方を不可分に行う
   P21-011設計と一貫させるため、新しい送信専用APIは追加しなかった。
2. **(二次レビューで修正済み)`can_start_crisis`自体には同盟・戦争中・条約に
   関する検証が一切無かった**(§3実装前調査16)。初回パスではAI候補選定側
   だけに同盟/戦争/条約/休戦フィルタを追加し、プレイヤー側フローは対象外
   として`can_start_crisis`自体を変更しなかったが、二次レビューで
   「AIとプレイヤーUIとで判定基準が分裂するリスクがある」との指摘を受け、
   §30.1の対応で`can_start_crisis`自身へこれらの検証を実装した(共有関数
   `diplomatic_conditions_permit_crisis_start`を新設し、AI候補選定からも
   同じ関数を呼ぶ)。現在はプレイヤーもこれらの制約無しにCrisisを開始する
   ことはできない。
3. **`calculate_demand_acceptance`(P21-012)はP21-014の国家総合力ではなく
   生の`available_manpower`を使う**(§11で詳述) — これはP21-012時点からの
   既存仕様であり、P21-017が新たに発見した設計上の非対称性ではあるが、
   仕様書§16がP21-012の受諾式変更を明示的に禁止しているため修正対象外。
4. **(二次レビューで発見)既存の生の戦争準備AI(`process_war_preparation_ai`)
   は、Claim/Crisisシステムと完全に独立して正当化を開始する設計のため、
   P21-017がAIをCrisis起点者にした結果、同じAI・対象国・対象州へ
   Crisis経由の正当化と生の正当化が並行して進行し得る抜けが新たに
   発生していた**(§30.2で詳述)。P21-016以前は「AIがCrisisを開始する」
   こと自体が無かったため顕在化しなかった、P21-017が導入した機能同士の
   相互作用による新規の不具合であり、修正済み。

---

## 28. GUI手動確認の実施有無

**実施していない。** 実ウィンドウでの対話操作による確認は行わず、
`cargo test`(headless、`MinimalPlugins`+`ScheduleRunnerPlugin::run_once()`)
による自動テストのみで検証した。仕様書§19が列挙する12項目の手動確認は
全て未実施のまま正直に記載する。

---

## 29. 次タスクへ残す対象外項目

仕様書§2「実装しない内容」に列挙された項目(AIによる新規Claim生成、
Claim捏造、AIによるCrisis撤回、ランダムな外交性格、威信・悪評・世界緊張度、
Call to Arms、個別講和、戦争貢献度、新しい同盟システム・外交条約・
領土価値計算、プレイヤーへの特別な保護、難易度補正)は全て未着手のまま。

追加で、初回パスでは「`can_start_crisis`自体への同盟/戦争/条約検証の追加」を
プレイヤー操作フローも含めた共有ドメインAPIの強化として見送っていたが、
二次レビューでの指摘を受けて§30.1の対応で実装済み(§27項目2参照)。

二次レビュー対応(§30)でも、以下は引き続き対象外のまま:

- `can_start_crisis`の「現在戦争中」判定を候補選定と同じO(1)集合で統一する
  こと(§16・§25で述べた通り、意図的なトレードオフとして権威ある
  `are_countries_at_war`呼び出しを維持している)。国数が現実的な規模(6)で
  ある限り実務上の影響は無いが、将来国数が大幅に増える場合は再検討の
  余地がある。
- `calculate_demand_acceptance`がP21-014の国家総合力ではなく生の
  `available_manpower`を使う非対称性(§27項目3、修正対象外として維持)。

---

## 30. 二次レビュー対応(2026-08-20)

初回パス完了報告に対し、ユーザーから3件の指摘を受けた。全て修正・再検証済み。

### 30.1 指摘1: 共有ドメインAPIに外交制限がない

**指摘**: `can_start_crisis`は戦争中・同盟・不可侵条約・休戦中の相手を拒否
しない。AI候補側だけに同じ判定を持たせると、AIとプレイヤーで規則が分裂し、
将来また判定がズレる可能性がある。制限を`can_start_crisis`/共有述語へ移し、
AIとプレイヤーUIの両方から再利用すること。各条件のドメインAPI単体テストを
用意すること。

**対応**:

- `CrisisRegistry::can_start_crisis`/`start_crisis`(`src/diplomacy/crisis.rs`)
  へ`diplomacy_registry: &DiplomacyRegistry`・`war_registry: &WarRegistry`
  (`can_start_crisis`のみ`current_date: &str`も)引数を追加。
- 同盟・不可侵条約・休戦の判定を、新設した`pub(crate)`自由関数
  `diplomatic_conditions_permit_crisis_start(claimant, target,
  diplomacy_registry, current_date)`へ切り出した。戦争中判定
  (`war_registry.are_countries_at_war`)は`can_start_crisis`自身に直接実装
  (呼び出し元ごとに効率が異なる手段を使うため、この関数には含めない設計、
  §16参照)。
- `diplomacy::ai_crisis_start::evaluate_pair_eligibility`は、この同じ
  `diplomatic_conditions_permit_crisis_start`を呼ぶよう書き換えた(以前は
  `TreatyType::Alliance`/`NonAggressionPact`/`is_in_truce`を独自に
  再実装していた)。これにより、AIとプレイヤーUIの外交条件判定は
  **物理的に同一の関数**を経由するようになり、片方だけを直して他方を
  忘れるという将来のズレが構造的に起こり得なくなった。
- プレイヤーUI(`src/ui/diplomacy_panel.rs`)の2箇所の呼び出し元
  (`execute_crisis_confirm`が呼ぶ`start_crisis`、`update_diplomacy_panel_ui`
  内のボタン表示判定が呼ぶ`can_start_crisis`)を新シグネチャへ追随させた。
  前者は呼び出し元System(`handle_crisis_command_buttons`)に
  `DiplomacyRegistry`/`WarRegistry`パラメータを追加して伝播、後者は
  そのSystem(`update_diplomacy_panel_ui`)が既に両Registryを持っていたため
  そのまま渡すだけで済んだ。
- ドメインAPI単体テストを`src/diplomacy/crisis.rs`のテストモジュールへ
  6件追加: `can_start_crisis_rejects_when_claimant_and_target_are_at_war`、
  `can_start_crisis_rejects_when_claimant_and_target_are_allied`、
  `can_start_crisis_rejects_when_claimant_and_target_have_a_non_aggression_
  pact`、`can_start_crisis_rejects_when_claimant_and_target_are_in_truce`、
  `can_start_crisis_succeeds_once_the_truce_has_expired`(休戦期限切れ後は
  成功することの確認)、`has_unresolved_crisis_initiated_by_reflects_
  current_phase`。
- 新規エラーキー4件(`diplomacy_error.crisis.{at_war,ally,
  non_aggression_pact,truce}`)を`assets/localization/{en-US,ja-JP}.ron`へ
  追加。
- この変更に伴い、`can_start_crisis`/`start_crisis`を直接呼んでいた
  P21-010〜P21-016の既存E2Eテストファイル6件(`p21_010_claim_crisis_
  e2e_test.rs`ほか)の呼び出し箇所を新シグネチャへ更新した(挙動・
  アサーションは無変更、`World::resource_scope`経由で実際の
  `DiplomacyRegistry`/`WarRegistry`を渡すだけ)。

### 30.2 指摘2: 既存AI正当化との並行進行

**指摘**: `process_war_preparation_ai`はClaimとは独立に直接正当化を開始する。
`claims_with_justification`はCrisis由来の正当化しか除外しておらず、同じ
AI・対象国・対象州について既存AIの通常正当化とP21-017のCrisisが並行して
作られる可能性がある。

**対応(2方向とも実装)**:

1. **候補選定側**: `collect_ai_crisis_candidates`の正当化除外を、Crisis由来
   限定(`source_crisis_id`→`related_claim_id`の連鎖)から、出所を問わない
   `justified_triples: HashSet<(CountryId,CountryId,StateId)>`(全正当化の
   `(initiator,target,target_state)`を直接収集)へ拡張した。これは
   ユーザー提案の選択肢(a)に相当し、既存の生正当化が**先に**存在する
   ケースを塞ぐ。副次効果として、`crisis_claim_by_id`という間接参照が
   不要になり実装も単純化した。
2. **生の戦争準備AI側**: `process_war_preparation_ai`
   (`src/country/country_ai.rs`)に`crisis_registry: &CrisisRegistry`引数を
   追加し、`is_justifying`チェックの直後に「自国開始の未解決Crisisが
   あれば新規正当化探索を始めない」ガードを追加した(新設した
   `CrisisRegistry::has_unresolved_crisis_initiated_by`を使用)。これは
   選択肢(b)に相当し、Crisisが**先に**(まだ正当化が生まれていない
   `DemandSent`の間から)存在するケースを塞ぐ — 1だけでは、Crisisがまだ
   正当化を持たない期間に生AIが独立に正当化を開始してしまう抜けが
   残っていたため、両方が必要だった。
3. **決定論性**: 上記2つのSystem(`handle_monthly_ai_crisis_start`と
   `handle_daily_country_ai`)は同じ`DailySimulationSet::CountryAi`内の
   兄弟Systemであり、明示的な順序制約が無いとBevyのスケジューラ内部詳細に
   順序が依存し得る(結果自体はどちらの順でも「矛盾は起きない」よう
   設計したが、同一フレーム内でどちらが「勝つ」かが不定だと決定論の要件を
   満たさない)。`src/diplomacy/mod.rs`で`handle_monthly_ai_crisis_start`
   に`.before(handle_daily_country_ai)`を明示し、Crisis開始が常に先に
   確定するようにした(ユーザー提案の選択肢(c)の精神を、`.before()`という
   形で取り入れた)。
4. **テスト**: 「Crisisがある状態で同じ対象への通常正当化が作られない」
   ことを検証するE2E相当のテストとして、`src/country/country_ai.rs`へ
   `process_war_preparation_ai_does_not_start_a_competing_justification_
   while_a_crisis_is_unresolved`を追加した。既存の
   `test_war_preparation_ai_starts_justification_via_optimized_path`
   (戦力優位・隣接という条件で正当化が**開始される**ことを検証する既存
   テスト)と全く同じ環境を使い、唯一の違いとしてStrong(0)自身の未解決
   Crisisを事前に挿入し、正当化が**開始されない**ことを確認する対比テスト
   にした。本番の公開エントリーポイント`process_daily_country_ai`を直接
   呼ぶため、実際のSystemが呼ぶのと同じ経路を通る。修正コードを一時的に
   無効化して本テストが実際に落ちることを確認してから元に戻し(誤って
   何も検証しない「見かけ上のテスト」になっていないことの直接確認)、
   `src/diplomacy/ai_crisis_start/tests.rs`にも
   `claim_matching_a_raw_non_crisis_justification_is_not_a_candidate`を
   追加して候補選定側単体でも検証した。
   実データ(6か国/28州)を使ったE2E化は見送った: `process_war_preparation_
   ai`が要求する地理的隣接性は、P21-017のAI選定(国力比が高い順)とは
   独立に決まるため、実データ上で両者が同じ国ペアを選ぶ保証がなく、
   仮に選んでも隣接していなければテストが偽陽性(何も検証せず素通り)に
   なるリスクがあった。[[feedback_verify_test_branch_is_actually_hit]]
   の教訓に従い、手製フィクスチャで隣接性を確定させた上記の直接テストの
   方が確実な検証になると判断した。

### 30.3 指摘3: 計算量の説明と測定条件が不十分

**指摘**: 候補ごとの`WarRegistry::are_countries_at_war`が全Warを走査するため、
実際の最悪計算量は`unique claimant-target pairs × active wars`。性能測定では
`active wars = 0`だったため、この経路を検証できていない。月次評価の冒頭で
戦争中国家ペアを一度だけSetへ集約しClaim判定ではO(1)参照すること、
active_warsを0・100・1000に増やした性能測定を追加すること。

**対応**:

- `diplomacy::ai_crisis_start`へ`build_at_war_pairs(war_registry) ->
  HashSet<(CountryId,CountryId)>`を新設。`collect_ai_crisis_candidates`の
  冒頭で1回だけ呼び、以後`evaluate_pair_eligibility`は
  `war_registry.are_countries_at_war`を直接呼ばずこの事前集約した集合を
  O(1)参照する(`are_countries_at_war(a,b)`の定義— `(attackers.contains(a)
  && defenders.contains(b)) || 逆` — と完全に同値な結果になることを、
  正規化した`(CountryId,CountryId)`ペアをAttacker×Defenderの直積で
  挿入する実装で保証)。
- 性能プロファイラへactive_wars軸(0/100/1000)を追加し、15.1補足・15.3の
  両方で再測定した(§25参照)。結果、**候補選定フェーズ自体は
  active_warsに対してほぼ影響を受けない**ことを確認した一方、**実際に
  Crisisを開始した国の数だけ、`start_crisis`内部の`can_start_crisis`が
  権威ある`are_countries_at_war`(O(active wars))を1回ずつ呼ぶ**という
  残存コストが測定で可視化された(country_count=1000・active_wars=1000の
  極端な合成負荷で最大約33ms、§16・§25で詳述)。この残存コストは
  `claims`には一切依存せず`O(successful_starts × wars)` ≤
  `O(countries × wars)`に留まるため、`unique claimant-target pairs ×
  active wars`という指摘された総当たりそのものは解消されている。
  `can_start_crisis`自身をO(1)化しなかったのは、指摘1で要求された
  「AIとプレイヤーUIで判定基準を分裂させない」を優先し、単一の権威ある
  検証(プレイヤーUIからの単発呼び出しと共有)を維持するための意図的な
  トレードオフである — 現実的な国数(6)ではこの残存コストは事実上ゼロに
  等しい。

### 30.4 ユーザーが問題無しとした点(変更していない)

- 要求送信専用APIを重複して作らなかった判断(§8の設計、実コード優先の
  方針)。
- 月次一回ガード(`AiCrisisStartEvaluationCount`、P21-015と同じパターン)。
- AI→AI/AI→プレイヤー/多国間Warまでの一連のE2E接続。

### 30.5 最終テスト数・変更ファイル一覧

§18(変更ファイル一覧)・§19(新規テスト一覧、§30分は本節に集約)・§22
(開始時/終了時テスト数)を参照。要約: lib 838→846(+8: `crisis.rs`+6、
`country_ai.rs`+1、`ai_crisis_start/tests.rs`+1)、integration 181のまま
不変(新規integrationテストファイルは追加せず、既存6ファイルのシグネチャ
追随のみ)、合計1019→1027(+8)。

### 30.6 品質ゲート・fmt基準・性能の最終確認

§23(品質ゲート結果)・§24(fmt基準との差分)・§25(性能測定方法・結果)に
最終値を反映済み。全て green、fmtドリフトは開始時と変わらず44箇所、
性能は現実的な国数は言うまでもなく極端な合成負荷でも実用範囲内。
