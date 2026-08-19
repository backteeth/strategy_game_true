# P21-013 完了報告: プレイヤー第三国によるCrisis支持表明

最終検証日: 2026-08-18(初版) / 2026-08-18(二次レビュー修正版)
最終検証コマンド実行環境: Windows 11, cargo (release/debug両方), rustfmt (edition 2024)

**最終ステータス: COMPLETE WITH MANUAL VERIFICATION PENDING**
(GUIでの対話的手動確認は実施していない。理由は §15 参照。)

**2026-08-18 二次レビューで期限判定の抜けが指摘され、修正済み。詳細は §18。**
本レポート中の全ての数値・API記述は修正後の最終状態(lib 707 / 全体860)を反映している。

---

## 1. 事前調査結果 (12項目)

実装着手前に以下をコード読解で確認した(すべて完了):

1. `DiplomaticCrisis`の全フィールド (`src/diplomacy/crisis.rs`): `id, initiator, target, war_goals,
   start_date, current_phase, escalation, initiator_support, target_resistance, days_in_phase,
   deadline_date, international_concern, third_party_reactions: HashMap<CountryId,
   ThirdCountryReaction>, related_claim_id, related_justification_id, related_war_id`。
2. `third_party_reactions`/`ThirdCountryReaction`はP21-010から存在するが、それまで一切の
   読み書きコードが存在しない「死んだ」フィールドだったことをgrepで確認(`Neutral,
   SupportsInitiator, SupportsTarget, CondemnsInitiator`の4値)。
3. `calculate_demand_acceptance`(`src/diplomacy/ai.rs`)のシグネチャと採点式(base -20、
   power比±30/-40、relation補正、state価値ペナルティ、首都-200、`clamp(-100,100)`)を確認し、
   この関数自体は変更しないことを確認。
4. P21-012が追加した`compute_ally_power_by_country`(同盟ベースの`HashMap<CountryId, f32>`を
   日次1回だけ構築)と、`evaluate_ai_crisis_responses`内でのO(1)引き(`country_by_id`)を確認。
5. `DailySimulationSet`の順序定義(`src/app/plugin.rs`相当)を確認し、P21-013では列挙順を
   一切変更しないことを確認。
6. `handle_daily_diplomacy`とUI側コマンドハンドラ(`diplomacy_panel.rs`)の間に、これまで
   実行順序の制約が一切存在しないことを確認(P21-011のAccept/Reject/WithdrawボタンもP21-013の
   支持ボタンも同様に無防備だった、という潜在的なギャップ)。
7. `can_X`/`X`検証→実行パターン(`crisis_response.rs`の既存`accept_demand`/`reject_demand`/
   `withdraw_crisis`)を確認し、新APIも同じ形に合わせることを決定。
8. `CountryRegistry`が`Clone`を派生していないこと(`src/country/mod.rs`)を確認(後に
   借用エラーの原因として再浮上)。
9. 既存のセーブ検証(`src/save/validate.rs`)内に、`third_party_reactions`に対する
   ダングリング参照チェックが**P21-010時点からすでに実装済みだが、実データで一度も
   演習されていなかった**ことを確認。
10. `diplomacy_panel.rs`の既存クライシス一覧描画が`my_crises`(当事者のみ)でフィルタされて
    おり、第三国プレイヤーには何も見えない実装だったことを確認。
11. `assets/localization/{ja-JP,en-US}.ron`の既存キー命名規則(`diplomacy_panel.*`,
    `diplomacy_error.crisis.*`)を確認。
12. `tests/p21_011_crisis_ultimatum_e2e_test.rs` / `p21_012_ai_crisis_response_e2e_test.rs`の
    E2Eテストパターン(`setup_app_in_playing`/`snapshot_save`/`validation_context`/
    `round_trip_through_save`/`CloneForTest`)を確認し、同一パターンを新E2Eファイルでも踏襲。

## 2. 採用した支持データ構造

新規型は追加せず、P21-010から存在した`third_party_reactions: HashMap<CountryId,
ThirdCountryReaction>`をそのまま再利用した。理由:
- `HashMap<CountryId, _>`が「1国につき最大1レコード」「同時両陣営支持の禁止」を構造的に
  無償で保証する。
- 新規ストレージを追加すると、セーブ/ロード・RONラウンドトリップ・既存の
  ダングリング参照チェック(§9)をすべて重複実装することになる。
- 唯一追加したのは`CrisisSupportSide`(`enum { Initiator, Target }`、`crisis_response.rs`)。
  UI/ドメイン層のAPI境界でのみ使う一時的な列挙で、`From<CrisisSupportSide> for
  ThirdCountryReaction`で内部表現に変換する。`CondemnsInitiator`はP21-013の対象外
  (要求仕様に明記された第三国支持のみを実装)。

## 3. 支持表明/撤回API

`src/diplomacy/crisis_response.rs`に4関数を追加(既存の`can_X`/`X`パターンに準拠):

```
can_pledge_support(crisis_registry, country_registry, crisis_id, supporter, side, current_date: &GameDate) -> Result<(), &'static str>
pledge_support(crisis_registry, country_registry, crisis_id, supporter, side, current_date: &GameDate) -> Result<(), &'static str>
can_withdraw_support(crisis_registry, crisis_id, supporter, current_date: &GameDate) -> Result<(), &'static str>
withdraw_support(crisis_registry, crisis_id, supporter, current_date: &GameDate) -> Result<(), &'static str>
```

`current_date`引数は初版に存在せず、二次レビュー(§18)で追加された。

検証ルール:
- crisis不存在 → エラー
- `supporter == initiator || supporter == target` → `"support_self"`
- `country_registry.get(supporter)`が存在しない → `"support_country_missing"`
- `current_phase != DemandSent` **または** `current_date >= deadline_date` →
  `"not_awaiting_response"`(両条件は共有関数`crisis_is_awaiting_support_response`が
  一括判定する。期限後・terminal相に加え、「phaseはまだDemandSentだが期限は既に
  到達済み」というロード直後特有の状態も含む — §18参照)
- 既存プレッジと異なる陣営への`pledge_support` → `"support_side_conflict"`(直接の
  陣営切り替えを禁止。撤回してから再表明する必要がある)
- 同一陣営への再表明は`insert`の冪等性によりエラーにならない(要求仕様どおり)
- `withdraw_support`でプレッジが存在しない → `"support_not_found"`

「要求元国」はP21-011/012同様、呼び出し元がプレイヤー自身のIDを渡すことを信頼する
既存の権限モデルに合わせた(セッション層は存在しない)。

## 4. 同盟との二重計上回避方式

`src/diplomacy/update.rs`の新関数`crisis_support_adjusted_power`が、P21-012が日次1回だけ
構築する`ally_power_by_country`(同盟ベース)と`country_by_id`(O(1)引き)をそのまま再利用し、
crisisごとに以下の「減算→加算」デルタを計算する:

1. crisisの`third_party_reactions`を走査(supporterがinitiator/target自身の場合はスキップ)。
2. supporterが既にtarget側の同盟国なら、そのpowerをまず`target_delta`から減算。
   initiator側の同盟国なら同様に`initiator_delta`から減算。
3. 明示的な支持陣営に応じて、そのpowerを対応する`_delta`へ加算。
4. `ally_power_by_country`のベース値に`_delta`を足し、`max(0.0)`でクランプ。

これにより:
- 支持ゼロのcrisisは`_delta`が常に0.0となり、**P21-012までの計算結果と完全に同一**
  (`no_support_matches_p21_012_baseline`テストで保証)。
- 同盟国が同じ陣営を明示支持しても、2重計上されない
  (`same_side_alliance_and_explicit_support_do_not_double_count`)。
- 同盟国が逆陣営を明示支持すると、その貢献は同盟側から明示支持側へ丸ごと移動する
  (`explicit_support_overrides_and_deduplicates_existing_alliance`)。
- `HashMap`の走査順に依存しないことを`iteration_order_does_not_affect_the_computed_power`で確認。
- `crises × countries`の総当たりは発生しない。既存のO(1)日次1回構築マップを再利用し、
  crisisごとに`third_party_reactions`(≤全国家数だが実運用では少数)のみを走査する。

## 5. AIスコアとの接続

`evaluate_ai_crisis_responses`内の直接`ally_power_by_country.get(...)`呼び出しを、
`crisis_support_adjusted_power(...)`の呼び出しに置き換えた。`calculate_demand_acceptance`
自体のシグネチャ・内部式は一切変更していない(§1-3で確認した既存関数のまま)。

## 6. 同一フレーム内の競合裁定

`DiplomacyPluginUI::build`内で、新設した`handle_crisis_support_command_buttons`システムのみに
`.after(crate::diplomacy::update::handle_daily_diplomacy)`を追加した。既存の
`DailySimulationSet`の列挙順序、およびP21-011の`handle_crisis_command_buttons`(Accept/
Reject/Withdraw)は一切変更していない。

効果: 同一フレームでAIの日次応答とプレイヤーの支持ボタン操作が重なった場合、AI応答が
先に確定し、その後に評価される支持コマンドは`can_pledge_support`/`can_withdraw_support`が
`current_phase != DemandSent`で弾く(`stale_support_pledge_after_ai_response_is_rejected`
E2Eテストで確認)。

**二次レビューで判明した抜け(§18で修正済み)**: この`.after()`順序だけに依存すると、
セーブロード直後など「`deadline_date`は既に過去だが`current_phase`はまだ`DemandSent`
のまま(`handle_daily_diplomacy`が一度も走っていない)」状態を防げなかった。修正後は
`current_date`をドメインAPI自身に渡し、`crisis_is_awaiting_support_response`が
phase・期限の両方を都度再検証する。

## 7. UI表示/確認フロー

`src/ui/diplomacy_panel.rs`:
- クライシス一覧のフィルタを`my_crises`(当事者限定)から`visible_crises`(全crisis)に変更。
  全プレイヤーが全crisisの支持者リストを閲覧できる(terminal相も含む)。
- 支持者リストは`third_party_reactions`をCountryId順にソートして描画し、国名解決で
  `CountryId`をそのまま出さない。
- 支持/撤回ボタンは「非当事者 (`!is_party`) かつ`current_phase == DemandSent`」の場合のみ表示。
- 支持表明(`RequestSupportInitiator`/`Target`)は2段階確認
  (`ConfirmSupport`/`CancelSupport`を挟む)。撤回(`WithdrawSupport`)は1クリックで即時実行
  (要求仕様どおり、表明のみ確認必須)。
- `DiplomacyPanelState::pending_support_pledge`はパネルを閉じる/`GameState`を離れる際に
  `clear_transient_selection()`でリセットされ、セーブへは一切書き込まれない
  (UI一時状態のみ)。
- JA/EN即時反映、既存スクロール(`ScrollPosition`/`RelativeCursorPosition`)の破壊がないことを
  それぞれ専用テストで確認済み。

## 8. セーブ後方互換性

`third_party_reactions`はP21-010から必須(非Option)フィールドであり、新規フィールドの
追加は不要だったため、**バージョン番号の変更は不要**と判断した(`#[serde(default)]`も
追加していない)。RONラウンドトリップは`third_party_support_data_round_trips_through_ron`/
`support_history_round_trips_after_terminal_resolution`で確認。

`src/save/validate.rs`の既存(P21-010由来、実データ未演習だった)検証ループを拡張し、以下を
アトミックに(1件でも違反があればセーブ全体をreject)検出する:
- 存在しない支持者国 (`DanglingReference`)
- 当事者自身による自己支持 (`SetOverlap`)
- `SupportsInitiator`/`SupportsTarget`以外の値(`Neutral`/`CondemnsInitiator`)が
  紛れ込んだ場合(`InvalidRange`。P21-013の仕様上、正規の支持レコードとしては
  この2値のみを許可)

## 9. 変更ファイル一覧

**P21-013で新規に変更/追加したファイル:**
- `src/diplomacy/crisis.rs` (M, +2 tests)
- `src/diplomacy/crisis_response.rs` (新規, +12 tests)
- `src/diplomacy/update.rs` (M, +9 tests)
- `src/ui/diplomacy_panel.rs` (M, +11 tests)
- `src/save/validate.rs` (M, +5 tests)
- `src/save/apply.rs` (M, 既存fixtureの新フィールド対応。テスト新規追加なし)
- `assets/localization/ja-JP.ron` / `en-US.ron` (M, 各16+4キー追加)
- `tests/p21_013_crisis_support_e2e_test.rs` (新規, 11 tests)

**同一セッション内で以前完了したが、まだ未コミットの別タスクの変更(P21-013では
再変更していない):**
`src/country/country_ai.rs`, `src/diplomacy/claims.rs`, `src/diplomacy/mod.rs`,
`src/diplomacy/tests.rs`, `src/map/camera.rs`, `src/save/dto.rs`, `src/save/export.rs`,
`src/ui/{economy,military,peace,politics,research,state,top_bar}_panel.rs`(P21-013直前の
UIクリックスルー修正), `src/ui/mod.rs`, `src/ui/scroll.rs`, `src/ui/scrollbar.rs`,
`src/ui/tab_bar.rs`, `src/war/justification.rs`, `src/war/tests.rs`,
`tests/p21_010_claim_crisis_e2e_test.rs`, `tests/p21_011_crisis_ultimatum_e2e_test.rs`,
`tests/p21_012_ai_crisis_response_e2e_test.rs`。

**説明のつかない差分:** `src/ui/country_selection.rs`が、内容的に純粋なrustfmt整形
(長い1行を複数行へ折り返しただけ、意味変更なし)のみの差分で変更されていた。P21-013の
どの手順でもこのファイルを明示的に編集・整形しておらず、原因を特定できていない
(VSCode拡張環境でのファイル表示に伴う自動フォーマットの可能性はあるが未確認)。
差分は`git diff -- src/ui/country_selection.rs`で確認可能で、意味変更が無いことは
目視確認済み。§16に既知の未解明事項として記録する。

## 10. 純増テスト数と合計

計測済み合計 (2026-08-18、二次レビュー修正後の最終値):
- `cargo test --lib`: **707 passed** (0 failed)
- `cargo test --tests` (23 test binary、デフォルト並列、headless GPU含む全件): **153
  passed** (0 failed)
- 合計: **860 passed, 0 failed**

(初版報告時点は lib 705 / tests 152 / 合計857。§18の修正で
`crisis_response.rs`に+2、`tests/p21_013_crisis_support_e2e_test.rs`に+1、
計+3件のテストを追加した。)

軌跡(セッションメモリの記録との突き合わせ):
- 仕様書に明記されたP21-012完了時点のベースライン: lib=656 / tests=797
- P21-013着手前、直近のUIクリックスルー修正(+9 lib tests)適用後の実測値: lib=665 / tests=806
  (このズレはP21-013仕様書執筆後に別タスクを挟んだことによるもので、セッション内で
  既に把握・記録済み)
- 今回のP21-013作業(初版)で追加した新規テストの内訳(ファイル単位の集計、+50件):
  crisis.rs +2, crisis_response.rs +12, update.rs +9, diplomacy_panel.rs +11,
  save/validate.rs +5, 新規E2Eファイル +11
- 初版の実測合計は806→857 (+51)。上記+50の積み上げ集計と1件差があったが、原因を
  1件単位まで再特定するより、**実測値(857)を正とする**方針を優先した
  (仕様書の指示「数字が異なる場合は推測で合わせず差異を報告してください」に従い、
  ここに正直に報告する)。この1件差は§18の修正作業でも特定できておらず、未解決の
  ままである(機能上の影響はない)。
- §18の二次レビュー修正で`crisis_response.rs`に+2、新規E2Eファイルに+1の計+3件を追加し、
  857→860となった。

## 11. 品質ゲート結果

- `cargo check --all-targets`: 成功
- `cargo test --lib`: 707 passed, 0 failed
- `cargo test --tests` (デフォルト並列、headless GPU含む23バイナリ全実行): 153 passed,
  0 failed。個別内訳は本レポートと同ディレクトリの実行ログ参照。
- `cargo clippy --all-targets -- -D warnings`: 初回`src/diplomacy/update.rs`の
  `country_by_id_of`関数で`needless_lifetimes`警告1件を検出・修正
  (`fn country_by_id_of<'a>(countries: &'a [CountryData]) -> HashMap<CountryId, &'a
  CountryData>` → ライフタイム省略形へ)。修正後は**0警告**。二次レビュー修正
  (§18)適用後も再実行し、引き続き0警告を確認。
- `cargo build --release`: 成功
- `git diff --check`: 実質エラー0件(既存のLF→CRLF警告のみ、内容は無害)
- headless render系テスト(`p20-007`/`p20-009`/`p21-save-002e`)が書き換えたスクリーンショット
  9枚は`git checkout --`でコミット済み版に復元済み。

## 12. 性能計測

`src/bin/profile_crisis_support_scaling.rs`(一時バイナリ、計測後に削除・`Cargo.toml`の
登録も削除済み)で、`state_count=2000`固定、crisis件数{0,1,100,1000}×支持有無
{なし,あり(1crisisあたり2名の第三国支持者を固定付与)}の7ケースを計測。
`evaluate_ai_crisis_responses`は評価当日にDemandSentを離脱させるため、計測日ごとに
CrisisRegistryを同一N件へ再投入して定常負荷を維持(再投入自体は計測区間外)。

生データ: `verification_logs/phase-21/p21-013/perf/{summary.txt,results.csv}`

主要な結果(Diplomacy SystemSet平均, 60日計測):
| crisis_count | support | overall mean(ms) | Diplomacy mean(ms) |
|---|---|---|---|
| 0    | -   | 0.328 | 0.048 |
| 1    | なし | 0.428 | 0.052 |
| 1    | あり | 0.431 | 0.054 |
| 100  | なし | 6.321 | 0.188 |
| 100  | あり | 6.225 | 0.191 |
| 1000 | なし | 6.602 | 0.554 |
| 1000 | あり | 6.906 | 0.724 |

観測: Diplomacy SystemSet単体のコストはcrisis件数に対しほぼ線形に増加し、支持ありは
支持なしに対して1000件時で約+0.17ms(support 1件あたり数百ナノ秒オーダー)の一定増分に
留まり、crisis件数の増加に伴う非線形な悪化は見られない。`crisis×country`の総当たりを
避けたO(1)引き(日次1回構築の`country_by_id`/`ally_power_by_country`の再利用)の効果を
裏付ける結果。overall全体は他のSystemSet(Economy等、2000州規模で支配的)の影響が大きく
100→1000でほぼ横ばいだが、Diplomacy単体では明確な増加傾向が見える。

## 13. フォーマット比較

`cargo fmt --all -- --check`はP21-013着手前から**ワークスペース全体では0/0ではなかった**
(military名称リファクタ・マップ拡張など、P21-013と無関係な既存コミット由来の未整形箇所が
`src/app/loader.rs`, `src/map/division_render.rs`, `src/map/mod.rs`,
`src/military/{movement,recruitment,supply,tests}.rs`, `src/profiling.rs`,
`src/save/runtime.rs`, `src/war/{capitulation,military_ai,peace}.rs`,
`tests/{daily_system_integration_test,land_war_combat_peace_test,p21_save_003_end_to_end_test,
profile_workload_correctness_test}.rs`に残存)。このうち`tests/land_war_combat_peace_test.rs`
は保護対象ファイルであり、P21-013では一切手を加えていない。

P21-013で変更・新規追加した全ファイル(§9の1グループ目)に対してのみ`rustfmt --edition
2024 <file>`を個別実行し、それらのファイルは現在すべて`cargo fmt --check`の差分から
消えていることを確認した(上記の残存差分リストにP21-013変更ファイルは1件も含まれない)。

## 14. verification_logsディレクトリの差分

新規追加: `verification_logs/phase-21/p21-013/completion_report.md`(本ファイル)、
`verification_logs/phase-21/p21-013/perf/{summary.txt,results.csv}`。
既存のP20-007/P20-009/P21-save-002eスクリーンショットはheadless renderテストにより
一時的に書き換わったが、`git checkout --`で復元済み(§11)。

## 15. GUI検証ステータス

**手動GUI検証は実施していない。** 対話的なゲーム画面での支持/撤回ボタン操作・
2段階確認フロー・支持者リスト表示・JA/EN切り替えの目視確認は、このセッションでは
一度も行っていない。以下12項目はいずれも未実施であり、必要であればユーザー自身または
別セッションでの手動確認を推奨する:
1. 第三国プレイヤーとしてDemandSent中のcrisisを開き、支持/撤回ボタンが表示されることの確認
2. 当事国プレイヤーとして同じcrisisを開き、支持ボタンが表示されないことの確認
3. 支持表明ボタン押下→2段階確認ダイアログの表示→確定操作の一連の流れ
4. 確認ダイアログのキャンセル操作
5. 撤回ボタンが1クリックで即座に反映されることの確認
6. 支持者リストに国名が正しく表示されることの確認(terminal相のcrisisも含む)
7. 日本語/英語切り替え時の即時反映
8. 既存のパネルスクロールが支持UI追加後も機能することの確認
9. セーブ→ロード後に支持データが復元されることの確認
10. AIの日次応答と支持操作が同一フレームで衝突した場合の実挙動確認
11. 支持ありでAIのAccept/Rejectが実際に反転する場面のスクリーンショット取得
12. 期限切れ(タイムアウト)による自動拒否が支持影響のAccept判定より優先されることの
    実機確認

## 16. 発見したバグ・曖昧点

- **テスト総数の1件差** (§10): ファイル単位の積み上げ集計(+50)と実測差分(+51)が
  1件不一致。原因未特定(実測値857を正とする)。
- **`src/ui/country_selection.rs`の出所不明な整形差分** (§9): 意味変更のない
  rustfmt整形のみで、P21-013の作業手順のどこにも該当する明示的な編集記録がない。
- P21-011/P21-012時点から存在していた「UIコマンドハンドラとAI日次応答の間に実行順序の
  保証が無い」という潜在的なギャップ(§6)は、P21-013の支持ボタンについてのみ`.after()`で
  対処した。P21-011のAccept/Reject/Withdrawボタン自体は要求仕様の保護対象であり、
  同種の潜在的な競合が理論上残っている可能性がある(本タスクでは意図的に対象外とした)。
- `save/validate.rs`の新チェックは`Neutral`/`CondemnsInitiator`を「P21-013としては
  不正な値」として一律rejectする実装にした。`CondemnsInitiator`は将来的に
  正規の値として使われる可能性があるenumバリアントだが、現時点でこれを生成する経路が
  コード上どこにも存在しないため、セーブファイルに紛れ込んだ場合は改ざん/バグの兆候として
  安全側でrejectする判断とした。

## 17. 次段階(多国間戦争等)への示唆

- 本タスクは意図的に「勝敗への直接関与なし」の支持表明に限定した。次段階で
  多国間参戦を実装する場合、`third_party_reactions`(または新設の`CrisisSupportSide`)を
  そのまま「参戦意思表明」の初期状態として再利用できる可能性がある。
- 現在の`crisis_support_adjusted_power`はcrisis単位でのpower集計に閉じているため、
  複数crisisにまたがる同盟網・大国ランク等を扱う場合は別途の集約層が必要になる。
- UIの支持者リスト表示(全プレイヤー・全crisis・terminal相含む)は、将来の外交ログ/
  戦争参加履歴表示の土台としてそのまま転用できる設計にしてある。

## 18. 二次レビュー対応(2026-08-18): 期限判定の修正

### 18.1 指摘内容

初版の`can_pledge_support`/`can_withdraw_support`は`current_phase == DemandSent`のみを
検証しており、`current_date`を一切受け取っていなかった。そのため、以下の状態
(セーブロード直後など、`handle_daily_diplomacy`の日次タイムアウト処理がまだ一度も
走っていない状況)では、期限を過ぎたCrisisへの支持表明・撤回が一時的に可能になり
得るという指摘を受けた:

```
current_phase == DemandSent
current_date >= deadline_date
```

初版レポート§6の「stale化したCrisisへの支持操作は`can_pledge_support`/
`can_withdraw_support`のphaseチェックにより自動的に拒否される」という記述は、この
ケースを見落としており不正確だった。

### 18.2 根本原因

`current_phase`の更新は`handle_daily_diplomacy`(3b: 期限切れタイムアウト処理)が
`DayChangedMessage`を受け取って初めて行われる。ロード直後・あるいは同一日内で
複数フレームが経過する間は、`deadline_date`が既に過去でも`current_phase`は
`DemandSent`のまま据え置かれる。初版の実装はこの「フェーズ更新の遅延」を
考慮しておらず、`.after(handle_daily_diplomacy)`という同一フレーム内の順序制約だけで
安全性を担保しようとしていたが、これは「次のタイムアウト処理が一度でも走った後」の
フレームにしか効かず、「タイムアウト処理が一度も走っていない」フレーム(ロード直後の
最初の数フレーム等)を保護できていなかった。

### 18.3 修正内容

1. **ドメインAPIへの`current_date: &GameDate`引数追加**: `can_pledge_support`/
   `pledge_support`/`can_withdraw_support`/`withdraw_support`(すべて
   `src/diplomacy/crisis_response.rs`)の末尾に追加。
2. **判定基準の一本化**: 新規`pub(crate) fn crisis_is_awaiting_support_response(crisis,
   current_date) -> bool`が`current_phase == DemandSent && !期限到達`を1箇所で判定し、
   ドメインAPI・UIの両方がこの関数を直接呼ぶ。期限到達判定自体は既存の
   `evaluate_ai_crisis_responses`(`diplomacy::update`)の期限フィルタと同じ規約
   (`deadline_date`が存在せず/解析不能なら「期限なし」として扱う)を踏襲する新規
   `crisis_demand_has_expired`ヘルパーが行う。
3. **UI側の表示条件も同じ関数に置き換え**: `src/ui/diplomacy_panel.rs`の支持ボタン
   表示条件を`crisis.current_phase == CrisisPhase::DemandSent`から
   `crisis_response::crisis_is_awaiting_support_response(crisis, &date)`へ変更
   (`update_diplomacy_panel_ui`に`Res<GameDate>`を追加)。期限到達済みだが
   phase未更新のCrisisでは、ボタン自体が表示されなくなる。
4. **`handle_crisis_support_command_buttons`にも`Res<GameDate>`を追加**し、
   `execute_support_confirm`/`execute_support_withdraw`経由でドメインAPIへ渡す。
5. **既存呼び出し元(15箇所)の更新**: `src/ui/diplomacy_panel.rs`(本体2箇所+テスト
   1箇所)、`tests/p21_013_crisis_support_e2e_test.rs`(9箇所)、
   `src/diplomacy/crisis_response.rs`のテストモジュール内の全呼び出し
   (PowerShell正規表現による機械的挿入、その後全箇所を目視確認)。
6. **タイムアウト優先テスト
   (`timeout_takes_priority_over_support_influenced_acceptance_on_deadline_day`)の
   再設計**: 元のテストは`deadline_date == start_date`(初日で即期限到達)という
   設定で、修正後の`current_date < deadline_date`要件のもとでは支持表明の登録自体が
   最初から拒否されてしまい、「一度は正当に登録された支持が、翌日のタイムアウト処理で
   上書きされる」という本来のテスト意図(要求テスト項目27)を検証できなくなっていた。
   `deadline_date`を開始日の翌日にずらし、支持表明は期限前(開始日当日、日次進行が
   一度も走っていない時点)に行い、その後1日進めて期限日当日のタイムアウト処理を
   発生させる設計に変更した。これにより「登録は成功するが、期限到達後は無視される」
   という元の検証意図を保ったまま、修正後のAPIとも整合させた。

### 18.4 追加した回帰テスト

- **`crisis_response.rs`(ドメインAPI単体、+2件)**:
  - `pledge_and_withdraw_support_are_rejected_exactly_on_the_deadline_day_while_phase_is_still_demand_sent`:
    phaseは`DemandSent`のまま、`current_date`が期限日当日に達している状態で
    支持・撤回とも拒否されることを確認(指摘された抜けそのものの再現・再発防止)。
  - `pledge_and_withdraw_support_are_rejected_after_deadline_while_phase_is_still_demand_sent_like_right_after_a_stale_load`:
    期限を超過した日付(セーブロード直後を模擬)でも、phaseがまだ`DemandSent`のままなら
    拒否されることを確認。既存プレッジが変更されないことも合わせて検証。
- **`tests/p21_013_crisis_support_e2e_test.rs::real_map_e2e`(実データ・save往復込み、
  +1件)**:
  - `stale_deadline_crisis_rejects_support_and_withdrawal_immediately_after_load`:
    実7か国28州マップ・実際の`build_save_game_v1`/`validate_save_game_v1`/
    `apply_validated_save`を経由したセーブ往復で、期限超過済み(`deadline_date`を
    ロード前に過去日へ書き換え)だがphaseは`DemandSent`のままのCrisisをロードした
    直後、`DayChangedMessage`が一度も発火していない時点で支持・撤回とも拒否される
    ことを確認する、指摘された「期限超過Saveロード直後」ケースそのもの。

いずれも**ドメインAPI単体**(UIを経由しない直接呼び出し)で拒否を検証しており、
UI側の表示条件変更(項目3)と独立に、ドメイン層自身が安全であることを保証している。

### 18.5 再実行した品質ゲート(修正後)

`cargo check --all-targets`成功、`cargo test --lib` 707 passed / 0 failed、
`cargo test --tests`(23バイナリ、デフォルト並列、headless GPU含む)153 passed /
0 failed(合計860、0失敗)、`cargo clippy --all-targets -- -D warnings` 0警告、
`cargo build --release`成功、`git diff --check`実質エラー0件、
`cargo fmt --all -- --check`は修正で触れた3ファイル
(`crisis_response.rs`/`diplomacy_panel.rs`/`p21_013_crisis_support_e2e_test.rs`)
いずれも差分から消えており、残存する差分一覧は§13記載の初版から完全に不変
(この修正で新たに未整形箇所を作っていない)。headless renderテストが書き換えた
スクリーンショットは`git checkout --`で復元済み。

### 18.6 その他の指摘事項(機能上の阻害要因ではないが確認)

- **`src/ui/country_selection.rs`の出所不明な整形差分**(初版§9/§16で報告済み):
  再確認したところ、この差分は依然として純粋なrustfmt整形のみ(意味変更なし)であり、
  かつ**修正後の内容は`cargo fmt --check`に対して既に適合済み**(このファイルに対する
  差分は現在の`cargo fmt --check`出力に一切含まれない)。つまりこの差分を元に戻すと
  逆にrustfmt非準拠の状態へ後退することになるため、**そのまま維持するのが正しい**と
  判断した。原因(どの操作でこの整形が入ったか)は依然として特定できていないが、
  意味変更が皆無であることと、既にrustfmt準拠であることの2点は再確認済みであり、
  コミット前に追加の手当ては不要と判断する。
- **§10のテスト数1件差**: 上記の通り未解決(機能上の影響なし)。

### 18.7 結論

指摘された期限判定の抜けは実在するバグであり、修正した。修正はドメインAPI
(`crisis_response.rs`)とUI(`diplomacy_panel.rs`)の両方に及び、判定基準を単一の
共有関数に統一することで、今後同様の判定基準のズレが再発しにくい構造にした。
3件の新規回帰テスト(ドメインAPI単体2件 + 実データsave往復E2E1件)がこの修正を
直接裏付けている。品質ゲートはすべて再実行し、初版からの追加のfmt汚染・regressionは
ない。最終ステータスは引き続き**COMPLETE WITH MANUAL VERIFICATION PENDING**
(GUI手動確認は今回も実施していない)。

---

*本レポートはP21-013仕様書の要求に基づき、実測値をそのまま記載した。数値の不一致
(§10, §16)は推測で埋め合わせず、判明している事実のみを記載している。§18は
2026-08-18の二次レビュー指摘に対する修正の記録である。*
