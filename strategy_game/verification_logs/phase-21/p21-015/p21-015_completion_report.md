# P21-015 完了報告: AI大国によるCrisis支持判断

最終検証日: 2026-08-19(初版) / 2026-08-19(二次レビュー修正版)
最終検証コマンド実行環境: Windows 11, cargo (release/debug両方), rustfmt (edition 2024)

## 1. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**
(GUIでの対話的手動確認は実施していない。理由は §22 参照。)

**2026-08-19 二次レビューで、同一フレームに複数の`DayChangedMessage`がある場合に
支持判断が2回実行されてしまう実装要件違反が指摘され、修正済み。詳細は §24。
本レポート中の記述・数値はすべて修正後の最終状態を反映している。**

---

## 2. 実装前調査結果

1. **Crisis本体・当事国・支持国の実型**: `DiplomaticCrisis`(`src/diplomacy/crisis.rs`)は
   `initiator: CountryId`, `target: CountryId`, `current_phase: CrisisPhase`,
   `deadline_date: Option<String>`, `third_party_reactions: HashMap<CountryId,
   ThirdCountryReaction>`を持つ。`ThirdCountryReaction`は
   `{Neutral, SupportsInitiator, SupportsTarget, CondemnsInitiator}`。支持国専用の
   別リストは存在せず、要求国側/対象国側どちらの支持者一覧も、この1つのHashMapを
   値でフィルタして得る。
2. **P21-013の公開API**: `crisis_response::can_pledge_support`/`pledge_support`/
   `can_withdraw_support`/`withdraw_support`は全て
   `(crisis_registry, [country_registry,] crisis_id, supporter, [side,]
   current_date: &GameDate) -> Result<(), &'static str>`の形。`pledge_support`は
   内部で`can_pledge_support`を呼び、失敗時はRegistryを一切変更しない。
3. **`crisis_is_awaiting_support_response`の現在の利用箇所**: `crisis_response.rs`内で
   `can_pledge_support`/`can_withdraw_support`の両方から呼ばれている(`pub(crate)`)。
   UI側では`ui/diplomacy_panel.rs`の支持ボタン表示条件が同じ関数を直接呼ぶ。
   P21-015もこれをそのまま再利用し(`crisis_registry`から取得した`&DiplomaticCrisis`
   と`&GameDate`を渡すだけ)、独自の期限判定を一切複製していない。
4. **P21-012 AI回答のSystem/関数/日付条件**: `diplomacy::update::
   evaluate_ai_crisis_responses`(`handle_daily_diplomacy`内、`DayChangedMessage`
   1件ごとに1回呼ばれる)。対象は`current_phase == DemandSent`かつ
   `player_country.0 != Some(c.target)`かつ`current_date < deadline_date`のCrisisのみ。
5. **期限切れ処理との実行順序**: 修正前の`handle_daily_diplomacy`は
   `0(正当化) → 3(days_in_phase増分) → 3a(P21-012回答) → 3b(P21-011期限切れ)`の
   逐次呼び出しだった。
6. **`DailySimulationSet::Diplomacy`/`CountryAi`周辺の順序**: `TimeUpdate → Economy →
   Research → Diplomacy → CountryAi → WarPreparation → MilitaryAi → FrontlineOrders →
   MilitaryAction → WarResolution → UiUpdate`の11Set・`.chain()`。P21-015の新規処理は
   `Diplomacy`Set内、`handle_daily_diplomacy`という単一関数の中に完全に収まるため、
   `DailySimulationSet`自体の列挙順は一切変更していない。
7. **外交関係値API・値域・中立値**: `DiplomacyRegistry::get_or_default(a, b) ->
   DiplomaticRelation`(`Option`ではなく必ず値を返す)。`opinion: f32`、範囲
   `-100.0〜100.0`、`DiplomaticRelation::default()`の`opinion`は`0.0`。未登録の
   国家ペアや`a==b`の場合も`get_or_default`が`DiplomaticRelation::default()`を返し、
   これが「既存の中立値」に相当する。
8. **同盟API・対称性の保証**: `DiplomaticRelation::has_treaty(TreatyType) -> bool`。
   `DiplomaticPairKey::new(a, b)`が`a.0 < b.0`になるよう常に正規化して格納するため、
   `get(a, b)`と`get(b, a)`は常に同じ`DiplomaticRelation`(=同じ同盟状態)を返す。
   新しい対称化ロジックは不要だった。
9. **プレイヤー国家Resource**: `country::PlayerCountry(pub Option<CountryId>)`。
10. **`CountryPowerRegistry`/`PowerTier`/世界順位順ID取得**: `country::power::
    CountryPowerRegistry::{get(CountryId) -> Option<&CountryPowerAssessment>,
    ordered_country_ids() -> &[CountryId], country_count() -> usize}`。
    `PowerTier::{GreatPower, RegionalPower, MinorPower}`(`Copy`・`PartialEq`)。
    `CountryPowerAssessment.power_tier`で判定できる。
11. **P21-013支持者一覧UIとAI支持の互換性**: `ui/diplomacy_panel.rs`の支持者一覧
    描画は`crisis.third_party_reactions`をCountryId順にソートして国名解決するだけで、
    「誰が(プレイヤーかAIか)追加したか」を判別する仕組みが元から存在しない。
    AIが`pledge_support`を通して追加した支持は、プレイヤーが追加した支持と
    データ上完全に区別がつかないため、**UIコード変更なしでそのまま表示される**
    ことを実際にテストで確認した(§13、要求テスト45)。
12. **支持者情報がSave DTOに保存されていること**: `third_party_reactions`は
    P21-010以来`DiplomaticCrisis`の必須フィールドとしてそのままSave/Loadされる
    (専用のSave DTO変換は無く、Crisis自体のシリアライズに含まれる)。P21-015は
    Save DTOへ一切手を加えていない。
13. **同一国の重複支持を防ぐ既存保証**: `can_pledge_support`が
    `third_party_reactions.get(&supporter)`を見て、既存エントリと同じ側なら
    冪等成功、異なる側なら`support_side_conflict`で拒否する。P21-015は候補抽出時に
    「既に支持している国は再評価しない」という自前のスキップも追加しており(効率化)、
    最終防波堤は既存ドメインAPI自身に委ねている。
14. **両陣営支持を防ぐ既存保証**: 同上(13)の`support_side_conflict`が
    「反対側への直接切り替え」を拒否する仕組みそのものであり、同じ国が
    `third_party_reactions`へ2エントリ持つことはHashMapの構造上そもそも不可能。
15. **HashMap非決定的反復順への依存箇所**: `CrisisRegistry.crises: HashMap<
    DiplomaticCrisisId, DiplomaticCrisis>`、`CountryRegistry.countries: Vec<
    CountryData>`(順序保証あり)。P21-015自身の新規コードでは、Great Power候補を
    `CountryId.0`昇順、Crisis処理順を`DiplomaticCrisisId.0`昇順にそれぞれ明示的に
    `sort_by_key`しており、`HashMap`の反復順をそのまま使う箇所は無い。

既存APIの名前・配置は本仕様書の記述と完全に一致していたため、最小限の変更
(可視性の追加のみ)で適合できた。

---

## 3. AI支持資格

`diplomacy::update::evaluate_ai_crisis_support`の候補抽出フィルタが、仕様書の7条件を
すべてそのまま実装している:

```rust
power_registry.ordered_country_ids().iter().copied().filter(|&id| {
    Some(id) != player_country.0
        && country_registry.get(id).is_some()
        && power_registry.get(id).is_some_and(|a| a.power_tier == PowerTier::GreatPower)
})
```

「まだどちらの陣営も支持していない」(条件7)はCrisisごとに`third_party_reactions.
contains_key(&ai_id)`で判定し、既に支持している候補は該当Crisisについてのみ
スキップする(他のCrisisでは引き続き評価対象)。「外交行動不能」「国家消滅」
「降伏済み」に相当する既存の共通判定は見つからなかった(このゲームには該当する
汎用フラグが無い)ため、新しい存続ルールは発明せず、`country_registry.get(id).
is_some()`(国家データの現存確認)だけを流用している。

---

## 4. 支持判断式と定数

```rust
pub const AI_CRISIS_SUPPORT_RELATION_MARGIN: f32 = 25.0;

fn decide_ai_crisis_support(
    ally_of_initiator: bool,
    ally_of_target: bool,
    relation_with_initiator: f32,
    relation_with_target: f32,
) -> Option<CrisisSupportSide> {
    match (ally_of_initiator, ally_of_target) {
        (true, false) => Some(CrisisSupportSide::Initiator),
        (false, true) => Some(CrisisSupportSide::Target),
        _ => {
            let relation_delta = relation_with_initiator - relation_with_target;
            if relation_delta >= AI_CRISIS_SUPPORT_RELATION_MARGIN {
                Some(CrisisSupportSide::Initiator)
            } else if relation_delta <= -AI_CRISIS_SUPPORT_RELATION_MARGIN {
                Some(CrisisSupportSide::Target)
            } else {
                None
            }
        }
    }
}
```

Bevy非依存の純粋関数(乱数不使用)。「中立」を表す3値目は新しい列挙型を作らず、
既存の`CrisisSupportSide`(Initiator/Target)を`Option`で包んで表現した
(仕様書の`AiCrisisSupportDecision`はこの`Option<CrisisSupportSide>`へ最小変更で
適合させ、重複する2値型を新設していない)。

---

## 5. 同盟優先規則

`decide_ai_crisis_support`の`match`が仕様書の優先順位をそのまま実装している:
片方とのみ同盟していれば(`(true,false)`/`(false,true)`)、関係値差を一切計算せず
その陣営を返す。両陣営と同盟(`(true,true)`)・どちらとも非同盟(`(false,false)`)の
どちらも同じ`_`アームへ落ち、関係値差の計算へ進む。単体テスト
`single_side_alliance_overrides_a_large_opposite_relation_delta`で、
逆方向に極端な関係値差(-100.0 vs 100.0)があっても片側同盟が優先されることを
直接確認済み。

---

## 6. 関係値差の境界条件

`relation_delta = relation(ai, initiator) - relation(ai, target)`。
`>= +25.0`で要求国側、`<= -25.0`で対象国側(両境界とも「以上/以下」で支持する側に
含む)、`-25.0 < delta < 25.0`は中立。単体テスト`relation_delta_boundaries_without_
any_alliance`で`delta ∈ {30, 25, 24, 0, -24, -25, -30}`の全境界値を直接確認した。
値の型・値域は`DiplomaticRelation.opinion: f32`(-100.0〜100.0)をそのまま使い、
整数への丸めは一切行っていない。

---

## 7. 日次実行条件

- `evaluate_ai_crisis_support`は`handle_daily_diplomacy`の`for _event in
  day_events.read() { ... }`ループの**外側**で`let mut
  ai_support_evaluated_this_frame = false;`を宣言し、ループ内では
  `if !ai_support_evaluated_this_frame { evaluate_ai_crisis_support(...);
  ai_support_evaluated_this_frame = true; }`というガードを通してのみ呼ぶ。
  これにより、同一フレームに`DayChangedMessage`が複数件積まれていても(高速
  進行等で1フレームに複数日が進んだ場合)、`evaluate_ai_crisis_support`自体は
  そのフレームにつき最大1回しか実行されない。0(正当化進行)・3
  (`days_in_phase`増分)・3b(P21-012応答)・3c(P21-011期限処理)は、
  ガードの対象外として引き続きイベント1件ごとに実行され続ける(既存の
  日次処理ループの意味・実行回数は変更していない)。
- `GamePaused`中は`app::time::advance_game_date`が`DayChangedMessage`自体を
  発行しないため、Pause中はP21-015のコードへ到達すらしない(既存の保証を
  そのまま享受)。

**2026-08-19 二次レビューで判明した不具合(§24で修正済み)**: 初版はこの
ガードを持たず、`evaluate_ai_crisis_support`を`day_events.read()`ループの
本体に直接置いていたため、同一フレームに`DayChangedMessage`が2件あれば
支持判断も2回実行されていた。初版の単体テスト
(`multiple_day_changes_in_one_frame_do_not_duplicate_support`)は
`third_party_reactions`の**結果**(支持済みなら2回目はドメインAPI自身の
冪等性により実質no-op)だけを見ていたため、この重複実行を検知できなかった
(全候補が中立でnobody supportsのケースでは、この見逃しが特に深刻だった —
「1回評価して誰も支持しなかった」のか「2回評価して2回とも誰も支持
しなかった」のかを、結果状態だけからは原理的に区別できない)。

---

## 8. P21-012回答・期限処理との正確な順序

`handle_daily_diplomacy`内のコメント番号を振り直し、以下の順で**逐次関数呼び出し**
として明示した(System登録順・`.before()`/`.after()`ではなく、同一関数内の
コード上の記述順そのものによる保証 — 仕様書が求める「コード上確認できる方法」の
最も強い形):

```text
0. justification_registry.process_daily_justifications(...)
3. crisis.days_in_phase += 1 (P21-010、対象外の既存処理)
3a. evaluate_ai_crisis_support(...)      ← P21-015 新規
3b. evaluate_ai_crisis_responses(...)    ← P21-012(既存、番号のみ3a→3bへ変更)
3c. 期限切れDemandSentの自動拒否           ← P21-011(既存、番号のみ3b→3cへ変更)
```

3aが3bより必ず先に完了するため、同日中に追加された支持は、直後に実行される3bの
`crisis_support_adjusted_power`(P21-013由来、`third_party_reactions`を毎回
読み直す)から必ず見える。これはE2Eテスト
`ai_support_is_applied_before_p21_012_ai_response_on_the_same_day`で、
「支持なしなら拒否になる」ことを別Appで確認した上で、「AI大国が同日に支持を
追加すると受諾に転じる」ことを直接検証済み。`calculate_demand_acceptance`自体・
既存の`score > 0.0`閾値は一切変更していない。

deadline当日(`current_date == deadline_date`)・期限超過後
(`current_date > deadline_date`、phaseがまだ`DemandSent`)は、
`crisis_is_awaiting_support_response`が両方とも`false`を返すため3aの時点で
一切支持されず、3cの既存期限処理がそのまま結果を決める
(`deadline_day_takes_priority_over_ai_support`テストで確認)。

---

## 9. P21-013ドメインAPIの再利用方法

`evaluate_ai_crisis_support`は判断結果(`Option<CrisisSupportSide>`)を得た後、
`Some(side)`の場合のみ`crisis_response::pledge_support(crisis_registry,
country_registry, crisis_id, ai_id, side, date)`を呼ぶ。戻り値の`Result`は
無視する(通常は失敗しない設計だが、万一失敗してもRegistryは
`pledge_support`自身の保証により無変更のまま、panicもしない)。
`third_party_reactions`への直接書き込みは一切行っていない
(`crisis.third_party_reactions.insert(...)`のような行はP21-015のコードに存在しない)。

---

## 10. Great Power候補抽出方法

`CountryPowerRegistry::ordered_country_ids()`(P21-014が既に世界順位順で
保持している`Vec<CountryId>`)を1回だけ`.iter()`し、`power_tier ==
PowerTier::GreatPower`かつプレイヤーでないかつ国家データが現存するものだけを
`filter`で残し、`CountryId.0`で`sort_by_key`する。`state_registry`/
`military_registry`/`building_registry`をP21-015から直接読むコードは一切無い
(`CountryPowerRegistry`が既に集約済みの`power_tier`だけを参照する)。

---

## 11. 決定論性の保証

- Great Power候補: `CountryId.0`昇順に`sort_by_key`(§10)。
- Crisis処理順: `crisis_registry.crises.keys().copied().collect()`を
  `DiplomaticCrisisId.0`で`sort_by_key`。
- 浮動小数点の同点判定: `decide_ai_crisis_support`は`>=`/`<=`の明示的な閾値比較
  のみで、不定な`partial_cmp`等は使っていない。
- 1つのAI大国が同じCrisisの両陣営に入らない: `third_party_reactions`は
  `HashMap<CountryId, ThirdCountryReaction>`であり、1国につき1エントリしか
  持てない構造上の制約(§1事前調査項目14)。
- 単体テスト`crisis_processing_order_does_not_affect_individual_outcomes`
  (Crisis挿入順を変えても個々の結果が同じ)、
  `multiple_great_powers_are_processed_in_country_id_order_deterministically`
  (複数大国が両方正しく追加される)、E2Eテスト
  `ai_support_survives_save_round_trip_and_is_deterministic`(実データで
  同一シナリオを2回最初から実行して同じ結果になる)で確認済み。

---

## 12. Save／Loadへの影響

**Save DTOの変更なし。** `src/save/dto.rs`・`export.rs`・`validate.rs`は
P21-015で一切変更していない。支持済みAIは既存のP21-013 Save形式
(`DiplomaticCrisis.third_party_reactions`)にそのまま含まれてロードされ、専用の
AI判断履歴・追加フィールドは存在しない。ロード自体はP21-014が既に確立した
`apply_validated_save`内の`CountryPowerRegistry`再構築フックをそのまま利用する
(P21-015は変更していない)。ロード直後の最初の日次tickで既存支持が重複追加され
ないこと、未支持の有効なCrisisは次のtickで通常どおり評価されること、期限切れの
`DemandSent` Crisisはロード後も支持されないことを、いずれもE2Eテストで直接確認した
(§13、要求テスト37-40)。Saveバージョンは変更していない。

---

## 13. UI変更の有無

**UIコードの変更なし。** 既存のP21-013支持者一覧・支持/撤回ボタン・deadline時の
ボタン非表示・外交パネルのスクロール・P21-014の国家ランク/総合力表示は、いずれも
1行も変更していない。AI支持がそのまま表示されることを、2つの新規テスト
(`ai_pledged_support_is_rendered_by_the_existing_p21_013_supporter_list_ui`,
`scroll_and_power_ui_survive_alongside_ai_pledged_support`)で直接確認した。

---

## 14. 変更ファイル一覧

**P21-015で新規に変更/追加したファイル:**
- `src/diplomacy/update.rs`(`decide_ai_crisis_support`/`evaluate_ai_crisis_support`
  追加、`handle_daily_diplomacy`への配線、+18テスト。二次レビュー修正で
  `ai_support_evaluated_this_frame`ガードと診断用`AiSupportEvaluationCount`
  Resourceを追加— §24)
- `src/ui/diplomacy_panel.rs`(コード変更なし、確認用+2テストのみ追加)
- `tests/p21_015_ai_crisis_support_e2e_test.rs`(新規、11テスト。二次レビュー修正で
  実際の呼び出し回数を直接検証するテストを追加 — §24)
- `tests/p21_010_claim_crisis_e2e_test.rs`/`p21_011_crisis_ultimatum_e2e_test.rs`/
  `p21_012_ai_crisis_response_e2e_test.rs`/`p21_013_crisis_support_e2e_test.rs`
  (各ファイルの軽量Appビルダーへ`CountryPowerRegistry::default()`の`insert_resource`
  を1行追加しただけ — `handle_daily_diplomacy`が新たに`Res<CountryPowerRegistry>`を
  要求するようになったため。ロジック変更は無い)

`src/save/dto.rs`・`export.rs`・`validate.rs`・`crisis_response.rs`・
`country/power.rs`にはP21-015として一切触れていない。

---

## 15. 新規テスト内訳

- `src/diplomacy/update.rs`: 18テスト
  - 判断関数(`decide_ai_crisis_support`、要求テスト1-12): 6テスト
    (`alliance_with_only_one_side_determines_the_side`,
    `single_side_alliance_overrides_a_large_opposite_relation_delta`,
    `alliance_with_both_sides_falls_through_to_relation_delta`,
    `relation_delta_boundaries_without_any_alliance`[6条件を統合],
    `decide_ai_crisis_support_is_deterministic`, および§13の
    `unregistered_relation_is_treated_as_the_existing_neutral_default`で要求
    テスト11を担当)
  - 資格・接続・順序(`evaluate_ai_crisis_support`、要求テスト13-27,34-36): 12テスト
- `src/ui/diplomacy_panel.rs`: 2テスト(要求テスト45-46)
- `tests/p21_015_ai_crisis_support_e2e_test.rs`: 11テスト
  - 軽量App(要求テスト28-33): 7テスト
    (§24の修正で、`multiple_day_changes_in_one_frame_do_not_duplicate_support`を
    診断用`AiSupportEvaluationCount`による実呼び出し回数の直接検証へ強化し、
    「全候補が中立(Abstain)」版
    `multiple_day_changes_in_one_frame_do_not_duplicate_evaluation_even_when_
    everyone_abstains`を新規追加した — 結果状態だけでは区別できない、
    最も見逃しやすいケースを直接カバーする)
  - 実データE2E(要求テスト37,38-39,40,41-42,47,48): 4テスト
    (複数の要求テスト項目を1テストへ統合したものを含む — 例:
    `ai_support_survives_save_round_trip_and_is_deterministic`が37・40・48を、
    `load_does_not_duplicate_existing_support_and_still_evaluates_unsupported_crises`
    が38・39を、それぞれ担当)

要求テスト43(P21-012回帰)・44(P21-014回帰)は、既存のP21-012/P21-014
テストスイート(いずれも無改造のまま全件green — §17)がそのまま裏付けている。

合計: **31テスト新規追加**(既存テストの削除・弱体化は無し)。

---

## 16. 開始時／終了時テスト数

| | lib | integration | 合計 |
|---|---:|---:|---:|
| 開始時(実測、仕様書記載の基準どおり) | 740 | 162 | 902 |
| 終了時(実測、二次レビュー修正後) | 760 | 173 | 933 |
| 差分 | +20 | +11 | +31 |

開始時の実測値は仕様書記載の基準(lib 740・全体902)と完全に一致しており、
差異は無かった。終了時の差分(+31)は§15の内訳(lib: 18+2=20、
integration: 11)と完全に一致している(初版報告時点はintegration 10・合計932
だったが、§24の修正で1件追加した)。

---

## 17. 品質ゲート結果

- `cargo check --all-targets`: 成功
- `cargo test --lib`: 760 passed, 0 failed
- `cargo test --tests`(デフォルト並列、headless GPU含む全24バイナリ): 173 passed,
  0 failed
- `cargo clippy --all-targets --all-features -- -D warnings`: **0警告**(修正不要)
- `cargo build --release`: 成功
- `git diff --check`: 実質エラー0件(既存のLF→CRLF警告のみ)
- headless render系テストが書き換えたスクリーンショットは`git checkout --`で
  都度復元済み。
- 二次レビュー修正の適用後、上記すべてを再実行して確認済み。

---

## 18. fmt基準との差分

実装開始前(P21-014完了直後)の状態を記録: `cargo fmt --all -- --check`は
**57箇所**の`Diff in`ハンク(P21-015と無関係な既存drift、`division_render.rs`/
`map/mod.rs`/`movement.rs`/`recruitment.rs`/`supply.rs`/`military/tests.rs`/
`profiling.rs`/`save/runtime.rs`/`capitulation.rs`/`military_ai.rs`/`peace.rs`/
`daily_system_integration_test.rs`/`land_war_combat_peace_test.rs`[保護対象]/
`p21_save_003_end_to_end_test.rs`/`profile_workload_correctness_test.rs`、
計15ファイル)。`wc -l`で実測して記録した数値であり、P21-014完了報告記載の
「51」とは一致しない — P21-014完了報告の当該数値が実測に基づかない誤記
だった可能性が高い(本タスクでは実測値をそのまま採用し、推測で合わせて
いない)。

P21-015で変更した7つのファイル(`src/diplomacy/update.rs`,
`src/ui/diplomacy_panel.rs`, `tests/p21_015_ai_crisis_support_e2e_test.rs`,
`tests/p21_{010,011,012,013}_*_e2e_test.rs`)へ個別に`rustfmt --edition 2024`を
実行した(§24の修正で再度変更した`update.rs`/`p21_015_ai_crisis_support_e2e_test.rs`
の2ファイルへも、修正後に改めて実行済み)。

終了時点: `cargo fmt --all -- --check`は**同じ57箇所**のまま(1箇所も増減なし、
ファイル一覧も完全に同一)。P21-015の変更ファイルはこの一覧に1件も含まれておらず、
リポジトリ全体の既存driftへは一切手を加えていない。

---

## 19. 性能測定方法と結果

一時バイナリ`src/bin/profile_ai_crisis_support.rs`(計測後に削除・`Cargo.toml`の
登録も削除済み)で、`evaluate_ai_crisis_support`を直接呼び出して計測した
(`handle_daily_diplomacy`全体ではなく対象関数だけを分離して計測)。

**最悪経路の確保**: `DiplomacyRegistry`を意図的に空のまま(同盟なし、
未登録関係は中立値opinion=0.0)にし、Crisisへも支持を一切追加しない構成にした。
これにより`decide_ai_crisis_support`は毎回「同盟チェック2回→関係値取得2回→
関係値差計算」までフル評価して中立(Abstain)に到達し、`already_supports`による
早期スキップも同盟による早期確定も一切発生しない(=「全候補が既に支持済みで
即終了するだけ」という偏った測定を避けている)。各ケースの最後に
`third_party_reactions`の総数が0であることをassertし、この前提が実際に
成立していたことを検証している。

生データ: `verification_logs/phase-21/p21-015/perf/{summary.txt,results.csv}`

**軸1: Crisis数スケーリング(国家数1000固定、Great Power=8)**

| active_crises | mean | max |
|---:|---:|---:|
| 1 | 0.14723ms | 0.15350ms |
| 10 | 0.15785ms | 0.23140ms |
| 100 | 0.23643ms | 0.38100ms |
| 1000 | 0.93899ms | 1.03030ms |

crisis=1→1000(1000倍)でmeanは約6.4倍にしか増えておらず、候補抽出の固定コスト
(約0.14ms)を差し引いた増分はcrisis数にほぼ比例している
(10→100: +0.0788ms/90crisis≈0.00088ms/crisis、100→1000: +0.7026ms/900crisis
≈0.00078ms/crisis — 傾きがほぼ一定であり線形)。

**軸2: 国家数スケーリング(Crisis数100固定)**

| country_count | great_powers(実測) | mean | max |
|---:|---:|---:|---:|
| 8 | 2 | 0.03692ms | 0.05810ms |
| 100 | 8 | 0.09248ms | 0.24300ms |
| 500 | 8 | 0.12244ms | 0.15250ms |
| 1000 | 8 | 0.23210ms | 0.30640ms |

country_count=8では`compute_tier_counts(8)==(2,2,4)`により大国は2か国のみ
(仕様書の「Great Powerは最大8」は上限であり、小規模国家数では自然に8未満になる —
実測どおり正直に報告する)。100→1000(10倍)でmeanは約2.5倍の増加に留まり、
Great Power数が8で飽和した後の増加は候補抽出(`ordered_country_ids()`の
`O(countries)`フィルタ)自体のコストによるもので、`active_crises ×
all_countries`のような二次的スケーリングではない
(軸1で確認したとおりCrisis数あたりのコストは国家数に関わらずほぼ一定)。

---

## 20. 一時ファイルの削除確認

- `src/bin/profile_ai_crisis_support.rs`: 削除済み
- `Cargo.toml`の`[[bin]] name = "profile_ai_crisis_support"`registration: 削除済み
  (`git diff --stat -- Cargo.toml`で差分ゼロを確認)
- 一時スクリーンショット: headless renderテストが書き換えた既存スクリーンショット
  9枚は毎回`git checkout --`で復元、新規の一時スクリーンショットは作成していない
- scratch用Save: 作成していない(全テストがインメモリの`App`内で完結)
- デバッグログ: 恒久的なデバッグ用`println!`/`eprintln!`等はコードに残していない

---

## 21. 発見した既存不具合・仕様差

- P21-015の実装範囲では、既存コードとの不整合や仕様書との齟齬は見つからなかった
  (P21-014完了報告で既に報告済みの「実6か国 vs 仕様書の想定7か国」はP21-015側でも
  影響を受けるが、これはP21-014側の既知差異としてP21-014完了報告に記録済みであり、
  本タスクでの新規発見ではない。実測では`real_map_has_at_most_two_great_powers`で
  「大国は2か国以下」であることを確認しており、これは6か国構成での
  `compute_tier_counts(6)==(2,2,2)`と整合する)。
- 既存の`handle_daily_diplomacy`のコメント番号(3a/3b)がP21-015の新規挿入により
  3a/3b/3cへ振り直しになった(意味変更ではなくコメントの整理のみ)。
- **P21-015自身の実装不具合(二次レビュー指摘、§24で修正)**: 同一フレームに
  複数の`DayChangedMessage`があると支持判断が重複実行される要件違反があった。
- **本レポート自身の記載ミス**: §18のfmt基準ハンク数を、実測せずP21-014完了報告の
  数値(51)をそのまま転記していた。§24修正作業中に再測して**57**が正しい実測値
  であることを確認し、§18を訂正した(P21-015自体の変更が原因ではなく、レポート
  作成時に実測を怠ったこと自体が問題だった)。

---

## 22. GUI手動確認の実施有無

**手動GUI確認は実施していない。** 対話的なゲーム画面でのAI大国自動支持の目視確認・
支持先の同盟/関係値規則との一致確認・支持者一覧UIでのAI支持表示・セーブ→ロード後の
維持確認・JA/EN切替・外交パネルスクロールの確認は、このセッションでは一度も
行っていない。要求されている12項目のGUI手動確認チェックリストはいずれも未実施で
あり、必要であればユーザー自身または別セッションでの手動確認を推奨する:

1. AI大国が有効なCrisisへ自動的に支持を表明する
2. 支持先が同盟・関係値規則と一致する
3. 地域大国・小国が自動支持しない
4. プレイヤー国家が勝手に自動支持しない
5. AI支持国がP21-013の支持者一覧へ表示される
6. 同じAI国が重複表示されない
7. 同じAI国が両陣営へ表示されない
8. deadline到達後に新しいAI支持が増えない
9. プレイヤーの支持・撤回ボタンが引き続き動く
10. セーブ→ロード後もAI支持者が維持される
11. P21-014の国家ランク・世界順位表示が維持される
12. JA/EN切替と外交パネルスクロールが正常

---

## 23. P21-016へ引き継ぐ公開APIと制約

- `strategy_game::diplomacy::update::evaluate_ai_crisis_support(crisis_registry,
  diplomacy_registry, country_registry, power_registry, player_country, date)` —
  日次1回、`handle_daily_diplomacy`から呼ばれる。テスト・将来のプロファイラからも
  直接呼べるよう`pub`にしてある。
- `strategy_game::diplomacy::update::decide_ai_crisis_support(ally_of_initiator,
  ally_of_target, relation_with_initiator, relation_with_target) ->
  Option<CrisisSupportSide>` — 純粋関数。同盟・関係値だけに基づく支持側判定を
  再利用したい場合に使える(ファイル内`fn`、`pub`ではないため、P21-016で外部から
  再利用する場合は可視性を広げる最小限の変更が必要になる)。
- `AI_CRISIS_SUPPORT_RELATION_MARGIN: f32 = 25.0` — 関係値差のしきい値定数
  (`pub`)。
- **制約**: 支持は既存の外交コミットメントの域を出ない
  ([対象外]多国間War参戦・威圧・自主撤回はP21-015に一切実装していない)。
  P21-016で「支持を実際の参戦へ接続する」場合は、`third_party_reactions`
  (誰がどちらを支持しているか)を読み取るだけで足り、P21-015のAI判断ロジック
  自体に変更は不要なはず。ただし、AIは一度支持したら自動撤回しない設計
  (大国から降格しても支持は残る)なので、P21-016側で「現在も大国かどうか」を
  参戦条件に含めたい場合は、支持時点のスナップショットではなく`CountryPowerRegistry`
  を都度再照会する必要がある。

---

## 24. 二次レビュー対応(2026-08-19): 同一フレーム複数DayChangedMessageの重複実行修正

### 24.1 指摘内容

初版の`handle_daily_diplomacy`は`evaluate_ai_crisis_support(...)`を
`for _event in day_events.read() { ... }`ループの本体に直接置いていた。
同一フレームに`DayChangedMessage`が2件以上積まれる状況(ゲーム速度が速く、
1フレームの実時間経過が1日分のaccumulatorしきい値を複数回超える場合)では、
ループが複数回反復し、`evaluate_ai_crisis_support`もその回数だけ実行されて
しまう。これは要求仕様6「同一フレームで複数の`DayChangedMessage`が存在しても、
全Crisis評価は最大1回にしてください」への違反である。

初版の単体テスト`multiple_day_changes_in_one_frame_do_not_duplicate_support`は
`third_party_reactions`の中身(支持済みなら2回目は`can_pledge_support`の
冪等性により実質no-op)だけを確認しており、実際の呼び出し回数を検証していな
かった。特に、全候補が中立(Abstain)で誰も支持しないケースでは、
「1回評価して誰も支持しなかった」結果と「2回評価して2回とも誰も支持
しなかった」結果が状態上完全に同一になるため、この見逃しが最も顕著になる。

### 24.2 修正内容

1. **フレーム単位のガード追加**: `handle_daily_diplomacy`内、
   `day_events.read()`ループの**外側**に`let mut ai_support_evaluated_this_frame
   = false;`を宣言し、ループ内の呼び出しを`if !ai_support_evaluated_this_frame {
   evaluate_ai_crisis_support(...); ai_support_evaluated_this_frame = true; }`
   で囲んだ。既存の0/3/3b/3c(正当化進行・`days_in_phase`増分・P21-012応答・
   P21-011期限処理)はガードの対象外とし、従来どおりイベント1件ごとに実行され
   続ける(既存の日次処理ループの構造・実行回数自体は変更していない — ユーザー
   提示の修正案どおり)。
2. **順序保証は維持**: ガードにより実際に支持判断が実行されるのはフレーム内の
   最初のイベント反復時だけだが、これはそのフレーム内で行われる**全ての**
   P21-012応答評価(3b、イベントごとに毎回実行される)よりも必ず先に完了する
   (「最初のP21-012回答より前」という要求よりも強く、「そのフレームの全ての
   P21-012回答より前」を満たす)。
3. **診断用Resourceの追加**: `AiSupportEvaluationCount(pub usize)`
   (`#[derive(Resource, Default)]`)を新設し、`handle_daily_diplomacy`が
   `Option<ResMut<AiSupportEvaluationCount>>`として受け取るようにした。本番の
   `World`にこのResourceが挿入されることは無いため(`Option`が`None`のまま
   静かに無視される)、本番動作・既存テストへの影響は一切無い。テストが明示的に
   `insert_resource`した場合にのみ、支持判断を実際に実行するたびに1ずつ
   増分される — これにより「結果の重複が無いこと」ではなく「呼び出し回数が
   本当に1回であること」をテストで直接検証できるようにした。

### 24.3 追加・強化した回帰テスト

- `multiple_day_changes_in_one_frame_do_not_duplicate_support`
  (`tests/p21_015_ai_crisis_support_e2e_test.rs`)を強化: `AiSupportEvaluationCount`
  を挿入し、2件の`DayChangedMessage`を同一フレームで発生させた後、
  `.0 == 1`であることを直接assertするよう変更(結果状態の確認は従来どおり維持)。
- 新規`multiple_day_changes_in_one_frame_do_not_duplicate_evaluation_even_when_
  everyone_abstains`: 同盟なし・関係値未登録(中立)の状況で、2件の
  `DayChangedMessage`が同一フレームにあっても呼び出し回数が1回であることを
  直接確認する。全候補が中立で「支持結果が常に空のまま」という、状態だけでは
  最も見逃しやすいケースをピンポイントでカバーする。
- **サニティチェック**: 修正の妥当性を確認するため、ガード条件を一時的に
  `if true`へ書き換えて(修正前と同じ「毎回無条件で呼ぶ」動作を再現し)上記
  2テストを実行したところ、両方とも`left: 2, right: 1`で明確に失敗することを
  確認した。その後、直ちに正しい修正内容へ復元し、`diff`でファイルが復元前と
  完全に一致することを確認してから本来の品質ゲートへ進んだ。

### 24.4 再実行した品質ゲート

`cargo check --all-targets`成功、`cargo test --lib` 760 passed / 0 failed、
`cargo test --tests`(24バイナリ、デフォルト並列、headless GPU含む)173 passed /
0 failed(合計933、0失敗)、`cargo clippy --all-targets --all-features -- -D
warnings` 0警告、`cargo build --release`成功、`git diff --check`実質エラー0件、
`cargo fmt --all -- --check`は修正で触れた2ファイル(`update.rs`/
`p21_015_ai_crisis_support_e2e_test.rs`)いずれも差分から消えており、残存する
57ハンクの一覧は§18記載のとおり初版から完全に不変(この修正で新たに未整形箇所を
作っていない)。headless renderテストが書き換えたスクリーンショットは
`git checkout --`で復元済み。

### 24.5 結論

指摘された「同一フレーム内の重複実行」は実在するバグであり、ユーザー提示の
修正案(フレーム単位のガード変数)をそのまま採用して修正した。修正の検証には、
状態ベースの確認だけでは原理的に不十分である(全候補中立のケースで特に顕著)
という指摘を踏まえ、本番コードに実害の無い診断専用Resourceを追加し、実際の
呼び出し回数を直接検証するテストを新設・強化した。ガードを一時的に無効化した
状態でこれらのテストが実際に失敗することも確認し、テスト自体がこのバグに
対して真に有効であることを裏付けた。§7の説明も、初版の誤った前提
(「複数件あってもループの1反復につき1回呼ばれるだけ」)から、実際の修正内容
(フレーム単位の明示的ガード)へ訂正した。あわせて、本レポート自身の§18に
あった実測に基づかない記載ミス(fmtハンク数)も本修正作業中に発見・訂正した。
最終ステータスは引き続き**COMPLETE WITH MANUAL VERIFICATION PENDING**
(GUI手動確認は今回も実施していない)。

---

*本レポートはP21-015仕様書の要求に基づき、実測値をそのまま記載した。数値の
不一致は今回発生しなかったため、その旨も正直に記録している。§24は2026-08-19の
二次レビュー指摘に対する修正の記録である。*
