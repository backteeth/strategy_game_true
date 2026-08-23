# P21-018 完了報告: AI国家による領土Claim生成

## 1. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

全品質ゲート(`cargo check/test/clippy/build --release/diff --check/fmt`)を通過し、
新規テスト67件(lib 62件・integration 5件、後述の対応表参照)が全てgreen。
実データ(6か国28州)によるE2Eテスト5件を含む。ユーザー自身の実プレイによる
非公式確認は一部行われたが(§27参照)、正式なGUIチェックリストの実施は
していないため、引き続き**COMPLETE WITH MANUAL VERIFICATION PENDING**とする。

**レビュー指摘(2026-08-20)を受け、以下2件を修正・再検証済み**:
(1) `build_at_war_pairs`(P21-017が持つ、Warごとに攻撃側×防御側の全ペアを展開する
関数)を初回実装がそのまま呼んでいたため、多国間War(P21-016)では
`O(wars)`ではなく`O(Σ_war |attackers|×|defenders|)`になり得た(1000 War×
攻撃側/防御側各100か国で1000万ペア、実測970ms)。P21-018はclaimant自身を
「現在いずれのWarにも参加していない」国だけに絞り込み済みのため、このペア集合は
常に空集合と同じ判定結果になる——ペア展開自体を行わず空集合を渡すよう修正し
(`collect_ai_claim_candidates`は`O(Σ_war (|attackers|+|defenders|))`の線形
`collect_war_participants`だけを使う)、同規模で再測定した(§12/§13/§24参照)。
(2) `ClaimSource`(既存5variant+`AiExpansion`)がDebug表示のまま(JA UIでも
"AiExpansion"等の生のRust識別子)だった点について、`display_name()`翻訳キーを
追加してJA/EN両方の自然な表示文へ変更した(§8/§16参照)。
また、§27のGUI手動確認12項目チェックリストが本報告書に存在せず参照先も
誤っていた指摘を受け、実際の12項目を§27へ追加し、参照箇所を修正した。

**さらに、ユーザーの実プレイ(2026-08-20)により新たな問題が見つかり、
その場で修正した**: Crisis要求(30日応答期限、P21-011由来・本タスクでは
未変更)がプレイヤー宛てに届いても自動一時停止する仕組みが無かったため、
ゲーム速度次第では応答する間もなく期限切れ→拒否→AI宣戦布告まで進んでしまう
ケースが実際に発生した。ユーザーの選択(3択のうち「自動一時停止を追加」)に
従い、新規`pause_on_new_crisis_targeting_player`Systemを追加した(§29参照)。
30日の期限自体は変更していない。

---

## 2. 開始時の実測基準

| 項目 | 仕様書§0の主張 | 実測値 | 差異 |
|---|---|---|---|
| lib テスト | 846 | 846 passed | なし |
| integration テスト | 181 | 181 passed(28バイナリ) | なし |
| 合計 | 1027 | 1027 passed | なし |
| fmtドリフト | 44箇所 | 44箇所(11ファイル) | なし(下記参照) |
| 実データ | 6か国・28州 | 6か国・28州 | なし |

fmtドリフトの44箇所は、実装完了後にP21-018が変更していないファイルだけを
`cargo fmt --all -- --check`のDiff出力から抽出して逆算した値(下記参照)。
セッション開始直後にfmtチェックのみを単独実行するステップを踏まなかったため、
「実装前」の生ログそのものは残していないが、値自体は仕様書の主張と完全一致し、
実装完了後の総ドリフト62箇所からP21-018変更ファイル分(18箇所)を引いた残数とも
一致する(§24)。

`git status`により、P21-017の実装成果物(`src/diplomacy/ai_crisis_start.rs`・
`src/diplomacy/crisis.rs`の一部・`src/ui/diplomacy_panel.rs`の一部等)は本セッション
開始時点で**未コミットのまま作業ツリーに残存**していることを確認した。P21-018は
この未コミット状態の上に実装している(§18で区別する)。

---

## 3. 実装前調査結果(§1の30項目)

1. **`TerritorialClaim`の全フィールド**(`src/diplomacy/claims.rs`):
   `id: ClaimId`, `claimant_country: CountryId`, `target_state: StateId`,
   `strength: f32`(コメント上0.0〜100.0), `created_date: String`,
   `is_permanent: bool`, `source: ClaimSource`, `status: ClaimStatus`。
   対象国(target)フィールドは存在せず、`state_registry.get(target_state).
   owner_country_id`から毎回導出する(P21-017と同じ規約)。
2. **`ClaimRegistry`の作成・追加・ID採番API**: `add_claim`(`next_id`を採番し
   上書き)、`get_claims_by_country`、`has_claim`、`can_create_claim`(検証のみ)、
   `create_claim`(検証込み作成)、`mark_consumed`。IDは`ClaimRegistry`内部の
   `next_id: usize`カウンタで一元管理され、Save/Loadは`from_saved_parts`経由。
3. **既存プレイヤーUI/ドメインAPI**: `ClaimRegistry::create_claim`
   (`src/ui/diplomacy_panel.rs::execute_claim_create`が直接呼ぶ、唯一のClaim作成
   経路)。P21-018はこの関数をそのまま再利用する(§8参照)。
4. **`ClaimSource`の全variant**: `Historical`, `BorderDispute`, `Cultural`,
   `Strategic`, `Debug`(+ 本タスクで追加した`AiExpansion`)。表示箇所は
   `diplomacy_panel.rs`の`claim_line`/新設`foreign_claim_line`ラベルの`{source}`
   プレースホルダのみで、**`format!("{:?}", claim.source)`によるDebug表示**
   (専用のJA/EN翻訳テーブルは存在しない)。
5. **Claim強度の標準値**: `create_claim`は呼び出し元から`strength`を一切
   受け取らず、内部で常に`50.0`を書き込む(既存プレイヤー作成分も含め全て同じ)。
   よって仕様書§8の`AI_GENERATED_CLAIM_STRENGTH = 50.0`は複製ではなく、
   既存の唯一の標準値をドキュメント目的で命名しただけの定数。
6. **`is_permanent`の意味**: `create_claim`は常に`false`を書き込む。P21-017の
   調査で確認済みの通り、Claimに有効期限フィールド自体が存在しないため、
   `is_permanent`を読んで分岐する既存ロジックは見つからなかった(フィールドの
   保持のみ)。
7. **`created_date`の形式とパースAPI**: `"YYYY/MM/DD"`または`"YYYY-MM-DD"`文字列。
   `GameDate::from_string(&str) -> Option<GameDate>`(不正形式は`None`、panicしない)。
8. **Active/Consumed遷移**: `ClaimStatus::Active`(デフォルト) → 
   `ClaimRegistry::mark_consumed(id)`で`Consumed`へ(冪等)。
9. **Consumed化の経路**: `crisis_response::accept_demand`(Crisis平和的受諾)が
   `mark_consumed`を呼ぶ。戦争終結・講和経由でClaimがConsumedになる経路は
   見つからなかった(P21-018のスコープ外、既存仕様のまま)。
10. **State隣接関係API**: `StateData.neighbors: Vec<StateId>`(専用の`get_neighbors`
    メソッドは存在せず、フィールド直接参照+`StateRegistry::get`のO(1)ルックアップ
    で辿る)。
11. **海州/陸州判定**: `StateData.is_sea: bool`。
12. **法的所有国とcontroller**: `owner_country_id: CountryId`(法的所有)と
    `controller_country: Option<CountryId>`(占領国、`controller()`メソッドで
    `unwrap_or(owner_country_id)`)。P21-018は§20の指示通り一貫して
    `owner_country_id`を使う。
13. **国家ごとの所有州取得APIの計算量**: `StateRegistry::get_owned_states(country)`
    は`states`全件を`filter`するO(states)の線形走査であり、O(1)ではない。
    **P21-018はこのAPIを一切呼ばない**(全国家分呼べば`countries × states`に
    なるため、§14で述べる自前の単一走査`build_border_data`で代替)。
14. **全州1回走査での国境候補構築**: 既存APIには存在しなかったため、P21-018が
    新設(`build_border_data`、§10参照)。
15. **`capital_state_id`の扱い**: 単なる`CountryId`所有の`StateId`フィールド。
    `create_claim`・`can_create_claim`は首都かどうかを一切考慮しない。P21-018も
    首都専用の禁止ルールを追加しない(§5)。
16. **State人口・建物レベルのO(1)取得**: `StateData.population: u64`
    (直接フィールド)、`StateData.buildings: HashMap<BuildingType, u32>`
    (直接フィールド、`.values().sum()`で合計)。
17. **P21-017の共有外交条件判定**: `crisis::diplomatic_conditions_permit_crisis_start`
    (`pub(crate)`、同盟/NAP/休戦を判定)。
18. **P21-017のopinion/power ratio純粋関数**: `ai_crisis_start::
    ai_crisis_opinion_permits_start`/`compute_power_ratio`/
    `ai_crisis_power_ratio_permits_start`、定数`AI_CRISIS_MAX_OPINION=-25.0`/
    `AI_CRISIS_MIN_POWER_RATIO=0.75`。
19. **P21-017の月次System名/順序**: `handle_monthly_ai_crisis_start`
    (`DailySimulationSet::CountryAi`、`.before(handle_daily_country_ai)`)。
20. **通常戦争準備AIとAI宣戦の順序**: 両方とも単一System`handle_daily_country_ai`
    (`DayChangedMessage`駆動)の中で、国ごとに(1)`process_war_declaration_ai`
    (日次)→(2)`process_war_preparation_ai`(週次条件成立時)→(3)
    `process_construction_ai`(月次条件成立時)の順に呼ばれる。別System化されて
    いない。
21. **進行中Crisis/正当化/War確認の共有API**: Crisisは
    `CrisisRegistry::has_unresolved_crisis_initiated_by`(既存)。正当化・Warには
    「その国のIDを持つ全国家分」を1回で求める既存の集合構築APIが存在せず、
    `country_ai.rs`内で都度`.any()`する場当たり的パターンのみだったため、
    P21-018が`collect_justification_initiators`/`collect_war_participants`を
    新設した(§10)。
22. **同一国家の複数Active Claim**: 構造的に禁止されていない(`create_claim`は
    claimant単位の上限を設けない)。P21-018は自身が生成する分だけを
    「Active Claimを1件も持たない国のみ対象」に制限する(既存の他ソースからの
    複数保有自体は変更しない)。
23. **同一claimant・stateの重複防止**: `can_create_claim`の既存チェック
    (`claims.values().any(|c| c.claimant_country==claimant && c.target_state==
    target_state)`)がそのまま働く。新しい重複チェックは追加していない。
24. **既存Claim一覧UI**: `diplomacy_panel.rs`の「自国Claim」セクション
    (`claim_registry.get_claims_by_country(p_cid)`、claimant=プレイヤー限定、
    選択中の対象国が所有する州へ絞り込み表示)。
25. **外国AIのプレイヤー州へのClaim確認可否**: **既存UIでは不可能だった**
    (上記の通りclaimant=プレイヤーの行しか表示しない)。P21-018で新設した
    「外国Claim」セクションが担う(§13)。
26. **Save/Load・validation**: `SavedClaimRegistry{ claims:
    HashMap<ClaimId,TerritorialClaim>, next_id: usize }`(`save/dto.rs`)。
    `save/validate.rs`はClaim参照整合性(claimant/target国家の実在等)を検証する
    既存ロジックを持つ。
27. **新ClaimSource variantの旧Save互換性**: 既存variantへの列挙子追加であり、
    削除・改名を伴わないため、旧Save(このvariantを一切含まない)は影響を受けない
    (`ai_expansion_claim_source_round_trips_through_ron`で確認)。
28. **`MonthChangedMessage`の既存消費**: `MessageReader<MonthChangedMessage>`を
    受け取り、`for event in month_events.read() { ... }`で処理するBevy標準パターン
    (P21-017の`handle_monthly_ai_crisis_start`が先行例)。
29. **P21-017の同一フレーム一回ガード**: ループ内`evaluated_this_frame: bool`
    フラグ+任意の診断用Resource(`AiCrisisStartEvaluationCount`)への加算。
    P21-018は`AiClaimGenerationEvaluationCount`で全く同じパターンを踏襲。
30. **現在の国家数・州数・国境辺数**: 6か国・28州(実データ)。国境辺の正確な数は
    別途集計していないが、E2Eテスト(`pick_adjacent_border_pair`)が実際に
    陸上国境ペアを発見できることで非ゼロであることを確認済み。

既存の`ClaimRegistry::create_claim`が要求(項目1-9)を全て満たしていたため、
新しいClaim作成ドメインAPIは追加していない(§8)。

---

## 4. AI国家側の生成資格(§4)

`country_can_generate_claim`(`src/diplomacy/ai_claim_generation.rs`)が以下を
全て判定する純粋関数:

1. `Some(country_id) != player_country`
2. `owned_land_state_count`(1回の州走査で事前構築)が1以上
3. `claimants_with_active_claim`(1回のClaim走査で事前構築)に含まれない
4. `claimants_with_unresolved_self_crisis`
   (`CrisisRegistry::unresolved_crisis_initiators`、1回のCrisis走査)に含まれない
5. `justification_initiators`(1回の正当化走査、出所を問わず)に含まれない
6. `war_participants`(1回のWar走査、Active戦争の攻撃側/防御側どちらも)に
   含まれない
7. `CountryPowerRegistry::get(country_id)`が`Some`
8. cooldown充足(§6)

「Country Registryに存在する」(仕様書項目2)は、claimant候補自体を
`country_registry.countries`から列挙するため構造的に自明。

---

## 5. 対象州の生成資格(§5)

境界候補の構築(`build_border_data`、§10)により、以下は**構造的に**満たされる
(候補として生成される時点で真):

- State Registryに存在する(隣接リストの参照解決に失敗した場合はスキップ)
- 海州ではない(海州はそもそも`owned_land_state_count`にも境界候補にも含めない)
- Claimantの法的所有陸州ではない(`owner_country_id`が異なる州のみを境界候補にする)
- Claimantの法的所有陸州と直接隣接する(境界候補自体が隣接から生成される)
- 対象州の法的所有国が存在する(`country_by_id.contains_key(&target)`で確認)
- 対象国がClaimantと異なる(異なるownerの州だけを境界候補にするため自明)
- 対象国と現在戦争中ではない(claimant自身が§4項目6でWar不参加を保証済みのため、
  targetとclaimantが戦争中というケースは構造的に発生しない。念のため
  `evaluate_pair_eligibility`内の`at_war_pairs`チェックも通す)
- 同じClaimant・州のActive Claimが存在しない(§4項目3の「Active Claim0件」ゲート
  がより強い条件としてこれを包含する)
- 同じ国家ペア・対象州の進行中Crisis/正当化が存在しない(§4項目4-5がより強い
  条件としてこれを包含する)

`(claimant, target)`ペア単位で判定するのは以下(`evaluate_pair_eligibility`、
P21-017と共有、§6参照):

- 同盟していない/不可侵条約なし/休戦中でない
  (`diplomatic_conditions_permit_crisis_start`)
- opinion(claimant→target) <= -25.0
- power ratio(claimant/target) >= 0.75

対象国の首都州も、`can_create_claim`が禁止していないため候補に含まれる
(専用の首都除外ルールは追加していない、§4項目35で確認)。

---

## 6. cooldownの正確な計算(§3)

```rust
fn month_index(year: i32, month: u8) -> i32 { year * 12 + (month as i32 - 1) }

pub fn claim_generation_cooldown_satisfied(
    latest_claim_month_index: Option<i32>,
    current_month_index: i32,
) -> bool {
    match latest_claim_month_index {
        None => true,
        Some(latest) => current_month_index - latest >= 12, // AI_CLAIM_GENERATION_COOLDOWN_MONTHS
    }
}
```

- 履歴なし(`None`) → 常に生成可能。
- 未来日付(`latest > current`)は`elapsed`が負になり自然に不成立(特別扱い不要)。
- 不正な日付文字列は`GameDate::from_string`が`None`を返すため、
  `claim_month_index_or_blocking`が**`current_month_index`自体を返す**
  (経過0か月=安全側でcooldown不成立)。panicしない
  (`invalid_created_date_does_not_panic_and_blocks_cooldown`で確認)。
- 対象Claimは`ClaimRegistry`全体を1回走査(`collect_claim_history`)し、
  Active/Consumed・ClaimSourceを問わず、claimantごとの最新月indexを求める。
- Save validationは`GameDate`の年/月/日の妥当性を(既存の`days_in_month`等の
  仕組みで)チェックする既存保証を持つが、`created_date`文字列自体の形式検証は
  Claim作成時(`GameDate::from_string`のOption化)に委ねられており、
  Save側で不正形式の`created_date`文字列そのものを拒否する専用チェックは
  見当たらなかった(cooldown計算側のフォールバックで安全に吸収する設計とした、
  新しいSave Resourceは追加していない)。

---

## 7. opinion・power ratio再利用(§6)

`AI_CRISIS_MAX_OPINION`/`AI_CRISIS_MIN_POWER_RATIO`定数、
`ai_crisis_opinion_permits_start`/`compute_power_ratio`/
`ai_crisis_power_ratio_permits_start`関数を**そのままインポートして使用**。
P21-018独自の閾値・純粋関数は一切定義していない。

同盟/NAP/休戦/戦争中の判定も、`ai_crisis_start::evaluate_pair_eligibility`
(P21-017が定義)を`pub(crate)`化してそのまま呼び出す形にした
(`diplomatic_conditions_permit_hostile_action`のような新しい汎化関数は作らず、
既存の合成関数を共有する方が「将来ズレるリスク」を最小化すると判断)。

**レビュー指摘対応**: 当初`WarPair`型も`pub(crate)`化し、P21-018自身が
`build_at_war_pairs(war_registry)`(P21-017が多国間War対応のため各Warの
`attackers × defenders`全ペアを展開する関数)を呼んで`evaluate_pair_eligibility`
へ渡していたが、これは多国間War規模で重大な計算量問題があった(§12/§13参照)。
修正後はP21-018から`build_at_war_pairs`を一切呼ばず、空の
`HashSet::new()`を渡す(claimantは`war_participants`ゲートで既に
「いずれのWarにも参加していない」ことが保証されているため、この判定は常に
`false`になる、という証明を伴う)。`build_at_war_pairs`自体は
P21-017が引き続き使用するため、関数・型定義は変更していない。

---

## 8. ClaimSource・strength・permanence(§8)

既存の`ClaimRegistry::create_claim`をそのまま使用(strength=50.0固定、
is_permanent=false固定、status=Active固定は全て既存の内部実装)。

`ClaimSource::AiExpansion`をP21-018で新規追加。serde・Save validationへの
影響がないことは`ai_expansion_claim_source_round_trips_through_ron`で確認。

**レビュー指摘対応**: 初回実装は既存の`format!("{:?}", claim.source)`
Debug表示規約(§4調査項目4)をそのまま踏襲していたため、JA UIでも
"AiExpansion"等の生のRust識別子がそのまま表示されていた。指摘を受け、
`ClaimSource::display_name()`(`PowerTier`/`CountryAiMode`と同じ
翻訳キー規約)を新設し、既存5variant+`AiExpansion`の全6variantをJA/EN両方の
自然な表示文へ変更した(`claim_source.historical`/`border_dispute`/
`cultural`/`strategic`/`debug`/`ai_expansion`、`diplomacy_panel.rs`の
`claim_line`/`foreign_claim_line`双方の呼び出し箇所を修正)。
`claim_source_text_follows_locale_switch`でロケール切替への追随を確認、
既存の`ja_jp_and_en_us_key_sets_match_exactly`等キー整合性テストも green。

---

## 9. Claim作成ドメインAPI(§8後段)

`try_create_claim_from_candidates`が、ソート済み候補リストを先頭から順に試し、
`ClaimRegistry::create_claim`(内部で`can_create_claim`により直前の再検証を行う)
に最初に成功したものだけを実際に作成する。全候補が失敗すれば何も作成しない。

- IDは`ClaimRegistry::add_claim`の`next_id`カウンタが保証(重複不可)。
- 重複Claim拒否は`can_create_claim`の既存チェックがそのまま働く。
- 不正CountryId/StateId拒否・部分書き込みなしは`can_create_claim`が`&self`のみ
  借用し、失敗時に`add_claim`へ到達し得ない既存の設計により保証される
  (P21-017と同じ設計)。
- NaN/値域外strength拒否(要求49-50)は、`create_claim`がそもそも呼び出し元
  からstrengthを受け取らない(常に50.0固定)ため、この経路からは**構造的に
  発生し得ない**。テストではこの不変条件そのもの
  (`create_claim_always_produces_finite_in_range_strength`)を固定する形で
  記録した(P21-017の「実際のAPIの形を正とする」方針を踏襲)。

---

## 10. 月次一回保証(§9)

`handle_monthly_ai_claim_generation`が`MessageReader<MonthChangedMessage>`を
読み、ループ内`evaluated_this_frame`フラグで実評価を最大1回に制限する
(P21-017の`handle_monthly_ai_crisis_start`と同一パターン)。診断用Resource
`AiClaimGenerationEvaluationCount`(`Option<ResMut<_>>`)で実際の呼び出し回数を
テストから直接検証できる。

Pause中・Load直後に評価が起きないことは、`MonthChangedMessage`自体が
`advance_game_date`(Pause中は早期return)からしか発行されず、Loadも合成メッセージ
を発行しないというBevy Messageの性質から構造的に保証される(P21-017と同じ理由、
専用のPause/Loadチェックは追加していない)。

---

## 11. System順序(§9, P21-017・通常戦争準備・宣戦との関係)

`DiplomacyPlugin`(`src/diplomacy/mod.rs`)に以下を明示的に登録:

```rust
handle_monthly_ai_claim_generation
    .in_set(DailySimulationSet::CountryAi)
    .after(handle_monthly_ai_crisis_start)
    .after(crate::country::country_ai::handle_daily_country_ai)
    .run_if(in_state(GameState::Playing))
```

`handle_monthly_ai_crisis_start`自身が既に`.before(handle_daily_country_ai)`を
持つため、上記2つの`.after()`はBevyのスケジューラが解決する推移的な順序制約
として重複しつつ働く: `P21-017 → handle_daily_country_ai(通常戦争準備AI+AI宣戦
を内包) → P21-018`という全順序が、System登録順ではなく明示的な依存辺だけで
保証される。

この順序が実際に意味のある結果を生むこと(要求63-64)は、`handle_daily_country_ai`
というSystemそのものではなく、それが呼ぶ純粋関数`process_daily_country_ai`を
直接呼んで正当化/宣戦を発生させ、その**同じ変異後のRegistry**を続けて
`evaluate_ai_claim_generation`へ渡すテスト
(`war_prep_ai_justification_started_this_frame_preempts_claim_generation_for_the_same_country`,
`ai_declares_war_this_frame_preempts_claim_generation_for_the_same_country`)で
データレベルの正しさを確認した(Bevyスケジューラの`.after()`遵守自体は
フレームワークの責務として別途再検証していない)。

---

## 12. 国境候補の構築方法(§10)

1. Country/Claim/Crisis/正当化/War/Stateを各1回ずつ走査
   (`collect_claim_history`/`collect_war_participants`/
   `collect_justification_initiators`/`CrisisRegistry::
   unresolved_crisis_initiators`/`build_border_data`)。
2. `build_border_data`が`state_registry.states`を1回走査し、陸州の
   `owned_land_state_count`と、隣接する他国所有陸州を集めた
   `candidate_targets: HashMap<CountryId, HashSet<StateId>>`を同時に構築する。
   `HashSet`を使うため、隣接リストが双方向に存在していても、また1国が複数の
   自国州から同じ対象州に隣接していても重複挿入されない。
3. 資格を満たした国だけを対象に、`(claimant, target)`ペア単位で
   `evaluate_pair_eligibility`の結果をメモ化キャッシュ
   (`HashMap<(CountryId,CountryId), Option<(f32,f32)>>`)。
4. claimantごとにグループ化した候補Vecを`compare_claim_candidates`でソート。

**レビュー指摘対応: War側は`build_at_war_pairs`のペア展開を使わない。**
初回実装では、P21-017が持つ`ai_crisis_start::build_at_war_pairs(war_registry)`
(各Warについて`attackers × defenders`の全組を`HashSet`へ展開する関数)をそのまま
呼び出していた。多国間War(P21-016)では1つのWarが複数の攻撃側・防御側を持ち得る
ため、この関数の計算量は`O(wars)`ではなく`O(Σ_war |attackers| × |defenders|)`
であり、1000 War×攻撃側/防御側各100か国のような規模では
`1000 × (100×100) = 10,000,000`組のペア展開が発生し得る(§24で実測)。

修正: P21-018は`country_can_generate_claim`の`war_participants`ゲート
(§4項目6、`collect_war_participants`が`O(Σ_war (|attackers|+|defenders|))`の
**線形**走査で構築する、参加国だけの集合)で、claimant自身が現在いずれのWarにも
参加していないことを既に保証している。claimantがどのWarの`attackers`/
`defenders`にも一切登場しない以上、`evaluate_pair_eligibility`(P21-017と共有)
内部の`at_war_pairs.contains(&normalize_pair(claimant, target))`判定は、
実際のペア集合を渡しても空集合を渡しても常に同じ結果(`false`)を返す。そこで
P21-018はペア展開を一切行わず、空の`HashSet::new()`を`evaluate_pair_eligibility`
へ渡すよう変更した(`src/diplomacy/ai_claim_generation.rs`の
`collect_ai_claim_candidates`、コード中に上記の証明をコメントとして明記)。
`build_at_war_pairs`関数自体はP21-017が引き続き使用するため削除していない
(P21-017はclaimantが第三国と交戦中でもtargetと非交戦なら候補になり得るため、
ペア単位の判定が本質的に必要)。新規回帰テスト
`unrelated_multilateral_war_does_not_affect_candidate_generation`で、
claimant/targetが一切参加していない多国間Warが存在しても候補生成が正常に
機能することを確認した。

---

## 13. 計算量(§10)

**修正後の正しい計算量**(初回報告の`wars_and_participants`は不正確だったため
訂正):

```
O(countries + states + border_edges + claims + crises + justifications
  + Σ_war (|attackers| + |defenders|)   // collect_war_participants、線形
  + candidate_sort)
```

`build_at_war_pairs`のペア展開(`O(Σ_war |attackers|×|defenders|)`)は
P21-018の経路からは呼ばれなくなったため、この項は計算量に含まれない。
`countries × states`/`countries × claims`/`states × claims`のいずれも発生しない
(15.1: states 100→10000でほぼ線形、15.2: countries=1000固定でclaims 100→50000
でも緩やかな増加、15.3: claims数に無関係にほぼ一定)。多国間War規模
(1000 War×攻撃側/防御側各100か国、Σ参加国スロット=200,000)でも
`collect_ai_claim_candidates`は約8.8msに留まり(§24参照)、修正前なら
`build_at_war_pairs`単体だけで約970msかかっていた規模であることを実測で確認した。

---

## 14. Claim→Crisis→WarのE2E接続(§11)

P21-018はCrisis開始経路を一切持たない
(`evaluate_ai_claim_generation`は`CrisisRegistry`を`&CrisisRegistry`
(不変参照)としてしか受け取らない)。生成したClaimは次回以降の月変更で
P21-017(`handle_monthly_ai_crisis_start`)が通常の既存Claimとして評価する。
E2Eテスト(§21参照)でAI→AI・AI→playerの両方について
Claim生成→(翌月以降)Crisis開始→拒否→多国間War(`source_crisis_id`維持)
までを実データで確認した。

---

## 15. Save/Load(§12)

新しいcooldown用Resourceや「今月生成済み」フラグは追加していない。cooldownは
保存済み`ClaimRegistry`の`created_date`から`collect_claim_history`が毎回
再計算する。既存の`SavedClaimRegistry`/`ClaimRegistry::from_saved_parts`を
そのまま使用。E2Eテスト
`ai_generated_claim_round_trips_through_save_load_without_duplication_and_next_month_still_works`
でSave→Load→重複なし確認→次月Crisis開始まで確認済み。

---

## 16. UI変更(§13)

既存の「自国Claim」セクション(claimant=プレイヤー限定)はAI生成Claimを
自動的に表示できない設計だったため、`diplomacy_panel.rs`へ「外国Claim
(選択中の外国またはプレイヤーを対象とする、自国以外の請求)」セクションを
新設した(`diplomacy_panel.foreign_claims_header`/`_none`/
`foreign_claim_line`のJA/EN翻訳キーを追加)。claimant国名・対象州名・
ClaimSource・created_dateを表示する。既存の
Claim一覧・危機一覧・受諾/拒否・支持・国家ランク・スクロール構造・JA/EN切替
コードは変更していない。

**レビュー指摘対応**: ClaimSourceの表示は当初、既存の`claim_line`セクションと
同じ`format!("{:?}", claim.source)`Debug表示を踏襲していたが、指摘を受けて
`ClaimSource::display_name()`翻訳キー経由の自然なJA/EN表示へ変更した(§8参照)。
この変更は新設の`foreign_claim_line`だけでなく、既存の`claim_line`
(自国Claimセクション)の表示も含む——既存5variant(Historical/BorderDispute/
Cultural/Strategic/Debug)も含めて全variantが対象になったため、既存UIの
表示内容が変化する(Debug文字列→翻訳済み文言)。既存の`claim_command_flow_
selects_state_and_creates_claim`等の既存テストはClaimSourceの表示文字列を
検証していなかったため、この変更で既存テストが壊れることはなく、
lib 846→901全件がgreenであることで確認した(§21参照)。

---

## 17. 決定論性

- 浮動小数点比較は全て`partial_cmp().unwrap_or(Ordering::Equal)`でpanic回避
  (P21-017と同じ規約)。
- 候補構築は`HashMap`/`HashSet`を中間状態に使うが、`grouped`の各Vecを
  `compare_claim_candidates`で最終的にCountryId/StateId昇順までタイブレークする
  ため、挿入順・ハッシュ順に依存しない
  (`candidate_order_is_independent_of_state_insertion_order`で確認)。
- claimant処理順は`evaluate_ai_claim_generation`内で`CountryId`昇順に明示ソート
  してから処理する。

---

## 18. 変更ファイル一覧

**P21-018自身の変更(新規)**:
- `src/diplomacy/ai_claim_generation.rs`(新設、本体)
- `src/diplomacy/ai_claim_generation/tests.rs`(新設、単体テスト49件)
- `tests/p21_018_ai_claim_generation_e2e_test.rs`(新設、E2E5件)
- `src/diplomacy/player_demand_alert.rs`(新設、§29フォローアップ:
  Crisis要求の自動一時停止、単体テスト7件)

**P21-018自身の変更(既存ファイルへの追記)**:
- `src/diplomacy/claims.rs`: `ClaimSource::AiExpansion`追加 + テスト2件、
  レビュー指摘対応で`ClaimSource::display_name()`(JA/EN翻訳キー)を追加
- `src/diplomacy/crisis.rs`: `is_unresolved_phase`/`unresolved_crisis_initiators`追加
  (`has_unresolved_crisis_initiated_by`を内部でこれに委譲するようリファクタ、
  外部から見える挙動は不変)
- `src/diplomacy/mod.rs`: `ai_claim_generation`モジュール登録+System登録、
  §29フォローアップで`player_demand_alert`モジュール登録+System登録
- `src/diplomacy/ai_crisis_start.rs`(P21-017の未コミット新規ファイル):
  `evaluate_pair_eligibility`/`build_at_war_pairs`/`WarPair`を`pub(crate)`化、
  Crisis未解決判定を`is_unresolved_phase`委譲へリファクタ(挙動不変、
  P21-017自身のテスト全件green維持で確認)。`build_at_war_pairs`/`WarPair`自体は
  P21-017が引き続き使用するため、レビュー対応後もこのファイルへの変更内容は
  据え置き(可視性変更のみ)。
- `src/ui/diplomacy_panel.rs`: 外国Claimセクション追加 + テスト3件、
  レビュー指摘対応でClaimSource表示を`format!("{:?}", ...)`から
  `display_name()`翻訳キー経由へ変更(新設`foreign_claim_line`・既存
  `claim_line`の両方) + ロケール切替テスト1件追加
- `assets/localization/en-US.ron`/`ja-JP.ron`: 外国Claimセクション用キー3件ずつ +
  レビュー指摘対応で`claim_source.*`キー6件ずつ追加

**P21-018が触れていない、セッション開始時から未コミットのまま残る
P21-016/P21-017由来の変更**: `src/country/country_ai.rs`, `src/map/*`,
`src/military/invasion.rs`, `src/military/tests.rs`, `src/military/update.rs`,
`src/profiling.rs`, `src/save/apply.rs`, `src/save/export.rs`,
`src/save/validate.rs`, `src/ui/military_panel.rs`, `src/war/data.rs`,
`src/war/frontline.rs`, `src/war/military_ai.rs`, `src/war/peace.rs`,
`src/war/tests.rs`, `tests/p21_005/008/010/011/012/013/015/016_*`,
`verification_logs/phase-21/p21-016/*`。

---

## 19. 新規・更新テスト一覧 / 20. 要求番号との対応

lib(`src/diplomacy/ai_claim_generation/tests.rs`、49件)+
`src/diplomacy/claims.rs`(2件)+ `src/ui/diplomacy_panel.rs`(4件、レビュー指摘対応の
ロケール切替テスト1件を含む)+ `src/diplomacy/player_demand_alert.rs`
(§29フォローアップ、7件) = **lib 62件**、
integration(`tests/p21_018_ai_claim_generation_e2e_test.rs`) = **5件**。
合計 **67件**。

| 要求テスト番号 | 対応するテスト名(モジュール) |
|---|---|
| 1 | `no_history_permits_generation` |
| 2-4 | `cooldown_boundaries_11_12_13_months` |
| 5 | `future_dated_claim_blocks_generation` |
| 6 | `invalid_created_date_does_not_panic_and_blocks_cooldown` |
| 7-8 | `cooldown_history_covers_active_and_consumed_regardless_of_source` |
| 9 | `ai_country_without_blockers_is_eligible` |
| 10 | `player_country_is_never_eligible`, `player_country_is_not_evaluated_via_system` |
| 11 | `country_without_owned_land_states_is_not_eligible` |
| 12 | `country_with_active_claim_is_not_eligible` |
| 13 | `country_with_unresolved_self_crisis_is_not_eligible` |
| 14-15 | `country_with_any_justification_status_is_not_eligible` |
| 16 | `country_at_war_is_not_eligible` |
| 17 | `missing_power_registry_entry_does_not_panic` |
| 18 | `adjacent_foreign_land_state_is_a_candidate`, E2E `ai_generates_claim_on_adjacent_foreign_land_state` |
| 19 | `non_adjacent_state_is_not_a_candidate` |
| 20 | `sea_state_is_not_a_candidate` |
| 21 | `own_state_is_not_a_candidate` |
| 22 | `owner_not_controller_determines_candidate_target` |
| 23 | `nonexistent_owner_country_is_not_a_candidate` |
| 24 | `evaluate_pair_eligibility_rejects_pair_at_war` |
| 25 | `allied_target_state_is_not_a_candidate` |
| 26 | `non_aggression_pact_target_state_is_not_a_candidate` |
| 27 | `truce_target_state_is_not_a_candidate` |
| 28-29 | `opinion_boundary` |
| 30-31 | `power_ratio_boundary` |
| 32 | `country_holding_any_active_claim_gets_no_new_candidates` |
| 33 | `unresolved_self_crisis_blocks_all_candidates_for_claimant` |
| 34 | `justification_blocks_all_candidates_for_claimant` |
| 35 | `capital_state_can_be_a_candidate` |
| 36 | `lowest_opinion_candidate_is_selected_first` |
| 37 | `tied_opinion_breaks_by_higher_power_ratio` |
| 38 | `tied_through_power_ratio_breaks_by_higher_population` |
| 39 | `tied_through_population_breaks_by_building_level_sum` |
| 40-41 | `fully_tied_candidates_break_by_country_id_then_state_id` |
| 42 | `candidate_order_is_independent_of_state_insertion_order` |
| 43 | `evaluate_ai_claim_generation_creates_at_most_one_claim_per_country_per_call` |
| 44 | `multiple_ai_countries_are_each_processed` |
| 45 | `first_candidate_domain_failure_falls_through_without_partial_writes`(ID非重複) |
| 46 | `duplicate_active_claim_is_never_created_via_this_module` |
| 47-51 | `create_claim_rejects_*`(claims.rs既存)+`create_claim_always_produces_finite_in_range_strength`(strength構造的制約) |
| 52 | `first_candidate_domain_failure_falls_through_without_partial_writes` |
| 53 | `ai_and_player_style_claim_creation_share_the_same_domain_api` |
| 54 | `no_month_changed_message_means_no_evaluation` |
| 55 | `month_changed_message_triggers_evaluation` |
| 56-57 | `multiple_month_changed_messages_in_one_frame_evaluate_only_once` |
| 58 | `no_message_means_no_generation_across_multiple_frames` |
| 59 | `load_without_a_month_changed_message_does_not_generate` |
| 60 | System登録順(§11)+E2E `claim_generated_this_month_is_not_immediately_a_crisis_but_becomes_one_next_month` |
| 61-62 | E2E `claim_generated_this_month_is_not_immediately_a_crisis_but_becomes_one_next_month` |
| 63 | `war_prep_ai_justification_started_this_frame_preempts_claim_generation_for_the_same_country` |
| 64 | `ai_declares_war_this_frame_preempts_claim_generation_for_the_same_country` |
| 65,74 | E2E `ai_generates_claim_on_adjacent_foreign_land_state` |
| 66 | E2E `claim_generated_this_month_is_not_immediately_a_crisis_but_becomes_one_next_month` |
| 67,69,70 | E2E `rejected_ai_generated_claim_crisis_leads_to_war_with_source_crisis_id_preserved` |
| 68,83 | E2E `ai_generates_claim_against_player_state_and_it_becomes_a_crisis_the_player_can_see` |
| 71-73 | E2E `ai_generated_claim_round_trips_through_save_load_without_duplication_and_next_month_still_works` |
| 75-77 | `foreign_ai_claim_against_selected_country_is_rendered`, `foreign_ai_claim_against_player_is_rendered_regardless_of_selected_country` |
| 78 | `foreign_claims_section_preserves_diplomacy_panel_scroll` |
| 79-84 | 既存E2Eテスト28ファイル全green(回帰なし、§22参照) |
| (レビュー対応、要求番号なし) | `unrelated_multilateral_war_does_not_affect_candidate_generation`(§12/§13の`at_war_pairs`最適化の正しさ)、`claim_source_text_follows_locale_switch`(ClaimSourceローカライズ、§8/§16) |

---

## 21. 開始時/終了時テスト数

| | lib | integration | 合計 |
|---|---|---|---|
| 開始時 | 846 | 181 | 1027 |
| 終了時(§29フォローアップ込み) | 908 | 186 | 1094 |
| 差分 | +62 | +5 | +67 |

---

## 22. 品質ゲート結果

| ゲート | 結果 |
|---|---|
| `cargo check --all-targets` | 成功 |
| `cargo test --lib` | 908 passed / 0 failed |
| `cargo test --tests` | 全28integrationバイナリでpassed / 0 failed(186件) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 警告0件 |
| `cargo build --release` | 成功 |
| `git diff --check` | エラーなし(既存のLF/CRLF警告のみ、内容差分なし) |
| `cargo fmt --all -- --check` | 44箇所(下記参照、P21-018変更ファイルのドリフト0) |

---

## 23. fmt基準との差分

- 実装完了直後(P21-018ファイルを未整形の状態)の総ドリフト: **62箇所**
  (11の既存ファイルで44箇所+P21-018ファイル4件で18箇所)。
- `rustfmt --edition 2024 <file>`を以下4ファイルへ**1件ずつ**適用
  (`cargo fmt`へ複数パスを渡さず、無関係な既存driftには触れない):
  `src/diplomacy/ai_crisis_start.rs`(1箇所)、
  `src/diplomacy/ai_claim_generation.rs`(1箇所)、
  `src/diplomacy/ai_claim_generation/tests.rs`(12箇所)、
  `tests/p21_018_ai_claim_generation_e2e_test.rs`(4箇所)。
- 整形後の総ドリフト: **44箇所**(開始時の基準値と完全一致、P21-018変更ファイルの
  ドリフトは0)。この後、要求63-64対応として`ai_claim_generation/tests.rs`へ
  テストを2件追記したが、`rustfmt --edition 2024 --check`で追加ドリフト0を確認
  (再整形不要)。内訳: `tests/land_war_combat_peace_test.rs`(15)、
  `tests/daily_system_integration_test.rs`(7)、`src/map/division_render.rs`(5)、
  `tests/p21_save_003_end_to_end_test.rs`(4)、`src/war/peace.rs`(3)、
  `src/map/mod.rs`(3)、`src/save/runtime.rs`(2)、`src/military/movement.rs`(2)、
  `tests/profile_workload_correctness_test.rs`(1)、`src/military/supply.rs`(1)、
  `src/military/recruitment.rs`(1)。いずれもP21-018が触れていない既存ファイル。
- **レビュー対応の追加編集後も再確認**: `at_war_pairs`最適化(`ai_claim_generation.rs`
  + 新規回帰テスト1件)とClaimSourceローカライズ(`claims.rs`/`ai_claim_generation/
  tests.rs`/`ui/diplomacy_panel.rs`/`en-US.ron`/`ja-JP.ron`)の編集後、
  `rustfmt --edition 2024 --check`を編集した各ファイルへ個別に実行し追加ドリフト0を
  確認(`diplomacy_panel.rs`は元々このセッションの前回パスで既にfmt-clean)、
  `cargo fmt --all -- --check`の総ドリフトも44箇所のまま変化なしを再確認した。

---

## 24. 性能測定方法・生データ・結果

一時プロファイラ(`src/diplomacy/ai_claim_generation/perf_temp.rs`、
`#[ignore]`付き`#[test]`)を実行後に削除する方式(P21-017と同じ方式)。
`cargo test --release --lib diplomacy::ai_claim_generation::perf_temp -- --ignored --nocapture`
で実行。レビュー指摘対応の多国間War軸(下記)は`perf_temp2.rs`として別途
実行・削除した(`... perf_temp2 -- --ignored --nocapture`)。

**15.1 State/border scaling**(claims=0, wars=0):

| states | countries | border_edges | ms |
|---|---|---|---|
| 100 | 8 | 198 | 0.093 |
| 500 | 25 | 998 | 0.221 |
| 1000 | 50 | 1998 | 0.350 |
| 2000 | 100 | 3998 | 0.708 |
| 10000 | 500 | 19998 | 4.381 |

states/border_edgesが100倍(198→19998)になっても実行時間は約47倍
(0.093ms→4.381ms)に留まり、`countries × states`のような二次的な爆発は
見られない。隣接リストの双方向重複(`HashSet`使用)による候補水増しも
`candidates`列(border_edges相当)と一致しており発生していない。

**15.2 Claim履歴スケーリング**(countries=1000, states=2000, 全Claimは
Consumed/十分過去日付=cooldown満了扱い):

| claims | ms |
|---|---|
| 100 | 1.329 |
| 1000 | 1.325 |
| 10000 | 2.009 |
| 50000 | 5.338 |

claimsが500倍(100→50000)でも実行時間は約4倍にしか増えず、
固定費(countries=1000+states/border_edges分)が支配的でclaims項が
線形かつ小さいことを示す。`countries × claims`の爆発はない。

**15.3 非月変更日**(System-level、`MessageReader`のみのコスト、
countries=1000/states=2000固定):

| claims | ms/frame(200フレーム平均) | evaluations |
|---|---|---|
| 100 | 0.00450 | 0 |
| 1000 | 0.00462 | 0 |
| 10000 | 0.00435 | 0 |
| 50000 | 0.00446 | 0 |

claims数に依存せずほぼ一定(0.0044〜0.0046ms)、評価回数は常に0を確認。

**15.1補足(レビュー指摘対応): 多国間War規模スケーリング**
(claim_country_count=1000, states=2000。War参加国はclaimant候補と完全に
別IDレンジへ分離し、claimant自身は一切参加しない):

| wars | 参加国/side | `build_at_war_pairs`単体(修正前の経路が払っていたはずのコスト、参考測定) | `collect_ai_claim_candidates`(修正後にP21-018が実際に使う経路) |
|---|---|---|---|
| 0 | 0 | 0.004 ms(pairs=0) | 1.274 ms |
| 100 | 10 | 0.459 ms(pairs=10,000) | 1.254 ms |
| 1000 | 10 | 4.242 ms(pairs=100,000) | 1.976 ms |
| 100 | 100 | 65.332 ms(pairs=1,000,000) | 2.084 ms |
| 1000 | 100 | **970.564 ms**(pairs=10,000,000) | **8.773 ms** |

1000 War×参加国100/side(要求された測定規模)で、`build_at_war_pairs`の
ペア展開そのものは**約970ms**かかる——これが初回実装の脆弱性の実際の大きさ
(この関数はP21-018から呼ばれなくなったが、参考測定として直接計測した)。
一方、修正後にP21-018が実際に使う`collect_ai_claim_candidates`
(`war_participants`の線形集合のみを使う経路)は同じ規模で**約8.8ms**に留まり、
War規模に対してほぼ横ばい(wars=0の1.27msから大きく乖離しない)。
`build_at_war_pairs`自体はP21-017が引き続き使用するコードとして変更していない
(削除していない)。

**15.4 完全パイプライン**: マイクロベンチではなくE2Eテスト
(`ai_generated_claim_round_trips_through_save_load_without_duplication_and_next_month_still_works`
等)として実データ上で正しさを確認済み(§14参照)。実データ規模(6か国・28州)では
全E2Eテストが1バイナリ合計0.04秒以内に完了しており、実運用上のパフォーマンス
懸念はない。

生データ: `verification_logs/phase-21/p21-018/perf/results.csv` /
`summary.txt`。

---

## 25. 一時ファイル削除確認

- `src/diplomacy/ai_claim_generation/perf_temp.rs`: 削除済み。
- `src/diplomacy/ai_claim_generation/perf_temp2.rs`(レビュー指摘対応の
  多国間War軸測定用): 削除済み。
- `ai_claim_generation.rs`内の一時`mod perf_temp;`/`mod perf_temp2;`登録: 削除済み。
- 一時bin登録・scratch Save・一時スクリーンショット: 追加していない。
- デバッグ出力(`eprintln!`等): 追加していない。
- headlessテスト実行が変更した既存スクリーンショット9件
  (`verification_logs/p20-007/`, `p20-009/`, `phase-21/p21-save-002e/`配下)は
  `git checkout --`で復元済み(`git status`でクリーンを確認)。

---

## 26. 発見した既存不具合・仕様差

- **Claim強度・是恒性フィールドは実質未使用**: `create_claim`は`strength`を
  常に50.0固定、`is_permanent`を常に`false`固定で書き込み、呼び出し元から
  カスタマイズする経路が存在しない。仕様書§8/§14が想定する
  「NaN/値域外strengthを拒否する」テストは、この構造により該当経路が
  そもそも存在しないため、拒否テストではなく不変条件の記録テストとして
  実装した(§9参照、P21-017の前例を踏襲)。
- **`get_owned_states`はO(states)の線形API**: O(1)ではない
  (§3調査項目13)。全国家分呼ぶと`countries × states`になるため使用を避けた。
- **Claimが戦争終結・講和経由でConsumedになる経路は見当たらなかった**
  (§3調査項目9)。P21-011以来の既存仕様のままで、P21-018のスコープ外。

---

## 27. GUI手動確認の実施有無

正式なチェックリスト形式の実施はしていないが、**ユーザー自身の実プレイ
(2026-08-20)により以下が非公式に確認された**。全項目を実施済みとは言えない
ため、引き続き**COMPLETE WITH MANUAL VERIFICATION PENDING**とする。以下、
仕様書§19の12項目チェックリストと、実プレイでの確認状況:

1. Claimを持たないAIが月変更時に国境州へClaimを生成する — **確認済み**
   (Elfin Republicがプレイヤー国Oceanic Magic Empireの隣接州へClaim生成)
2. プレイヤー国家へは自動Claimを生成しない — 未確認(プレイヤーが意図的に
   関係悪化させた後の観測のため、悪化させなくても発生しないことまでは未検証)
3. 同盟・不可侵条約・休戦国にはClaimを生成しない — 未確認
4. 一つのAIが複数Claimを同時生成しない — 未確認
5. 同じ月に生成したClaimが即座にCrisisにならない — **確認済み**
   (ログ上、Claim生成とCrisis開始(要求送信)が別タイミングで発生)
6. 次回以降の月変更でP21-017によりCrisisになる — **確認済み**
7. AIがプレイヤー州へ作ったClaimから要求が届く — **確認済み**
8. プレイヤーが受諾・拒否できる — **未確認、むしろ問題が見つかった**
   (30日の応答期限がゲーム速度次第で一瞬で過ぎ、応答する間もなく期限切れ→
   拒否扱いになった。原因調査の上、§29で自動一時停止を追加対応した)
9. AI→AI Crisisも既存式で進行する — 未確認(このセッションはAI→player)
10. 拒否後、支持国を含む多国間Warへ進む — **部分的に確認**(拒否後の宣戦布告は
    確認。支持国を伴う多国間War化は今回のログからは確認できていない)
11. Save→Load後もClaim・cooldownを維持する — 未確認
12. Claim・Crisis・国家ランク・War表示とJA/EN・スクロールが正常 — **部分的に
    確認**(通知ログの日本語表示は正常に読めている。パネルUIそのものの
    目視確認は未実施)

---

## 28. 次タスクへ残す対象外項目

- 本報告書§27のGUI手動確認12項目のうち未確認分(2, 3, 4, 9, 11、および
  12・10の残り部分)。
- Claimが戦争終結・講和経由でConsumedになる経路の追加(既存仕様のギャップ、
  今回のスコープ外)。
- 国境辺の正確な総数の実データ上での明示的な集計(E2Eテストで非ゼロなことは
  確認済みだが、専用のログ出力は行っていない)。

---

## 29. フォローアップ: Crisis要求の自動一時停止(2026-08-20、ユーザー実プレイ後)

ユーザーの実プレイで§27項目8が実際に失敗する場面が観測された:
`CRISIS_DEMAND_PERIOD_DAYS`(30日、P21-011由来、本タスクでは変更していない)の
応答猶予が、ゲーム速度(最大30日/実秒)次第では実時間で約1秒しかなく、
プレイヤーが要求に気づいて受諾/拒否を選択する前に期限切れ→拒否扱い→
AI宣戦布告まで進んでしまうケースが確認された。

**調査結果**: `GamePaused`はCrisis関連コードから一切参照されておらず、
新しい要求がプレイヤー宛てに届いても自動一時停止する仕組みが存在しなかった
(既存の`NotificationHistory`は最大5件保持の地味なログのみで、ポップアップ等の
強い演出も無い)。この30日期限自体はP21-011以来の既存仕様でP21-018は
変更していないが、AIが自発的にプレイヤーへ要求を送る実用的な経路を
初めて作ったのがP21-018であるため、このタイミングで問題が顕在化した。

**ユーザーの選択**: 3つの選択肢(自動一時停止を追加/通知だけ強化/仕様として
許容)を提示し、「自動一時停止を追加」を選択(30日の期限自体は変更しない)。

**実装**: 新規`src/diplomacy/player_demand_alert.rs`。
`pause_on_new_crisis_targeting_player`Systemが`CrisisRegistry`を走査し、
`current_phase == DemandSent`かつ`target == プレイヤー国`の**新規**Crisisを
見つけたら`GamePaused`を`true`にする。「新規」の判定は
`SeenPlayerTargetedCrisisIds`(`HashSet<DiplomaticCrisisId>`、Save対象外の
セッション内限定リソース)への挿入可否で行うため、同じCrisisで繰り返し
一時停止することはない(プレイヤーが手動で再開した後、勝手に再度停止しない)。
`DiplomacyPlugin`へ`GameState::Playing`中のみ動作するSystemとして登録
(`DailySimulationSet`の日次シミュレーション連鎖には含めない — 監視System
であり、今フレーム/次フレームどちらで新規Crisisを検出しても実害が無いため)。

**回帰確認**: この変更後、`DiplomacyPlugin`(実際の`GamePaused`参照)を使う
既存E2Eテスト(P21-011/012/013/015/017/018、いずれもプレイヤーがCrisis当事者に
なり得るシナリオを含む)を含む全1094件(lib 908・integration 186)が
引き続きgreenであることを確認した — 既存テストのいずれも「プレイヤーが
要求対象になった直後にさらに日を進め続ける」パターンを持たないため、
この自動一時停止によって既存テストの日次進行がブロックされる事態は
発生していない。

新規テスト7件(`src/diplomacy/player_demand_alert.rs`内): 新規要求で一時停止、
プレイヤーが開始者側では停止しない、無関係なCrisisでは停止しない、
DemandSent以外のフェーズでは停止しない、同じCrisisでは一度しか停止しない、
プレイヤー国家未設定でもpanicしない、複数Crisisが同一フレームにあっても
走査順に依存せず停止する。

最終テスト数(この追加込み): lib 901→908(+7)、integration 186(変更なし)、
合計1087→1094。品質ゲート再確認: `cargo test --lib/--tests`全green、
`cargo clippy --all-targets --all-features -- -D warnings`警告0件、
`cargo build --release`成功、`cargo fmt --all -- --check`総ドリフト44箇所
(基準どおり、新規ファイル`player_demand_alert.rs`・`mod.rs`ともドリフト0)。
headlessテストが変更した既存スクリーンショット9件は`git checkout --`で復元済み。
