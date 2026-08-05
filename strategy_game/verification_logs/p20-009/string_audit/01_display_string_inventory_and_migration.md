# P20-009: 表示文字列監査・翻訳キー移行対応表

## 1. 既存資料でのP20-009の定義(作業開始前調査)

`audit_report.md` / `walkthrough.md` にはP20-009の判定表エントリ(`| P20-009 | OPEN |`)のみが存在し、
詳細な受入条件テキストは記載されていなかった。ユーザー指示に基づき、本タスクのプロンプト本文
(セクション2〜13)を正式な受入条件として採用した。

## 2. 監査範囲

- `src/ui/*.rs` 全10ファイル(country_selection, state_panel, economy_panel, research_panel,
  politics_panel, military_panel, diplomacy_panel, peace_panel, top_bar, notification, mod.rs)
- 通知メッセージを構築する非UIモジュール: `economy/mod.rs`, `research/mod.rs`,
  `diplomacy/update.rs`, `politics/mod.rs`, `debug/mod.rs`
- enum表示名(`display_name()`)を持つ12ファイル: `building/data.rs`, `country/mod.rs`,
  `country/country_ai.rs`, `diplomacy/data.rs`, `economy/resources.rs`,
  `economy/economic_state.rs`, `military/data.rs`, `politics/values.rs`,
  `politics/interest_groups.rs`, `research/data.rs`, `war/peace.rs`, `war/frontline.rs`,
  `war/military_ai.rs`
- UI表示へ配線されたエラー文字列を返す3ファイル: `war/data.rs`, `war/justification.rs`,
  `war/peace.rs`

## 3. 監査結果サマリ(数値)

| 区分 | 件数 |
|---|---|
| 監査した表示文字列/箇所(概算、Text::new/format!/message呼び出し単位) | 約230箇所 |
| 翻訳キーへ移行した文字列 | 約210箇所(キー数336 × ja/en、共通キーの再利用を含む) |
| 正当な除外(カテゴリ2〜4) | 9箇所(以下参照)+ データ由来固有名詞多数 |
| 追加した翻訳キー総数(ja-JP = en-US) | 336(全キーがコード側から最低1箇所参照されていることを確認済み) |

## 4. カテゴリ別分類

### カテゴリ1: ローカライズキーへ移行
上記「監査範囲」の全ファイルに存在した表示用文字列(パネル見出し・ボタン・ラベル・
動的info文字列・通知・enum表示名・エラーメッセージ)を、`assets/localization/{ja-JP,en-US}.ron`
の翻訳キーへ移行した。詳細な対応表は本ディレクトリの `02_translation_key_usage_map.md` を参照。

### カテゴリ2: ゲームデータ由来の固有名詞(翻訳対象外、保護対象は不変)
- `CountryData.name`(`assets/data/countries.ron`由来)
- `StateData.name`(`assets/data/states.ron`由来、**保護対象・変更禁止**)
- `TechnologyDefinition.name` / `.description`(`assets/data/technologies.ron`由来)
- `WorldStageDefinition.display_name` / `.description`(`assets/data/world_stages.ron`由来)
- `BuildingDefinition.name`(`assets/data/buildings.ron`由来、UI側は`BuildingType::display_name()`
  という別の翻訳キー経由の表示を使用しており、こちらの`name`フィールド自体はデータ由来のまま)
- `DivisionDefinition.name`(`assets/data/divisions.ron`由来)
- `War.name`(国名・州名から`format!`で動的生成される識別名。"Conquest of"という接続語を含むが、
  `War`構造体のフィールド型を変更すると15箇所以上のテスト・フィクスチャに影響するため、
  データ由来の生成済み識別名として翻訳対象外とした)

これらは保護対象`states.ron`を変更せず、既存のRONデータ構造もそのまま維持している。

### カテゴリ3: ログ・デバッグ・テスト専用でユーザー非表示
- `println!!` / `info!` / `warn!` / `error!` 等のログ出力(`localization.rs`内の`translate()`の
  警告・エラーログ等も含む)
- `#[cfg(test)]` モジュール内のテストフィクスチャ文字列・assertメッセージ
- `src/profiling.rs` / `src/bin/profile_1000_states.rs` のCSV/JSON/コンソールレポート文字列
  (P20-008のプロファイリング専用、ゲームUIには一切表示されない)

### カテゴリ4: その他の正当な除外(言語に依存しない記号・数値ラベル)
`tests/p20_009_hardcoded_string_scan_test.rs` の `EXEMPTIONS` に理由付きで記録:
1. `top_bar.rs` `"||"` — 一時停止の記号(ポーズ記号、言語非依存)
2. `top_bar.rs` `">{}"` (speed button) — 速度切替の記号+数値(言語非依存)
3. `top_bar.rs` `"1800/01/01"` — TopBarDateTextの初期placeholder値、Startup直後に
   `update_top_bar_date`で即座に上書きされる。実際の表示は`format_date_for_locale()`経由。
4. `economy_panel.rs` `"-1%"` / `"+1%"` — 税率調整ボタンの記号(言語非依存)
5. `Text::new("")` の各初期値(diplomacy_panel/top_bar/politics_panel/research_panel) —
   空文字列であり翻訳対象の文言を含まず、同フレーム後に各パネルの更新Systemが
   `LocalizedText`経由の内容へ即座に差し替える。

これらは`no_hardcoded_text_new_literals_outside_allowlist`テストにより、ファイル・完全一致文字列・
理由を明記した固定リストでのみ除外されており、ディレクトリ全体の除外や検査の無効化は一切行っていない。

## 5. enum由来の表示値の網羅性

以下のenumはすべて`display_name() -> &'static str`が翻訳キーの`match`を返す実装へ変更され、
enumに新しいvariantが追加された場合はRustの`match`網羅性チェックによりコンパイルエラーで
検出される(取りこぼし防止)。Debug表現(`{:?}`)を直接ユーザー表示に使っていた唯一の箇所
(`debug/mod.rs`のF1デバッグショートカット、`format!("[DEBUG] Advanced World Era to: {:?}", next_stage)`)
は`next_stage.display_name()`経由の翻訳キーへ修正した。

- `BuildingType`, `GovernmentType`, `EconomicSystem`, `TreatyType`, `DiplomaticActivityType`,
  `ResourceType`, `EconomicState`, `DivisionType`, `DivisionSize`, `ValueAxis`,
  `InterestGroupType`, `WorldStage`, `TechnologyField`, `PeaceTerm`, `FrontlineStance`,
  `CountryAiMode`, `CountryAiDecisionReason`, `MilitaryAiDecisionReason`

`WarStatus`・`ArmyStatus`はenumとして`display_name()`メソッドを持たず、UI側(`peace_panel.rs`/
`military_panel.rs`)で`match`により翻訳キーへ直接マッピングしている(`war_status.*` /
`army_status.*`キー)。`WarGoalType` / `CrisisPhase` / `ThirdCountryReaction`
(`diplomacy/crisis.rs`)は、監査の結果現在どのUIコードからも参照・表示されていないことを
確認したため、対応スコープ外とした(将来表示を追加する際に同じ`display_name()`パターンを
適用すること)。

## 6. 見つかった既存の懸念事項(参考記録)

- `src/politics/mod.rs`の`toggle_politics_panel_key`と`src/ui/peace_panel.rs`の
  `toggle_peace_panel_key`が共にキーボードショートカット`P`キーを使用している
  (P20-009より前から存在する既存の重複、本タスクでは変更しない)。
- `assets/fonts/JapaneseFont.ttc` は既存(初回コミット時から存在)の未使用ファイルで、
  実体解析の結果 Microsoft の "MS UI Gothic / MS Gothic / MS PGothic"(Windows同梱の
  プロプライエタリフォント)であることが判明した。再配布可能なライセンスではないため、
  本タスクではこのファイルを使用せず、新たにSIL Open Font License 1.1の"Noto Sans JP"を
  追加した。既存ファイル自体は削除していない(削除の是非はユーザー判断に委ねる)。
