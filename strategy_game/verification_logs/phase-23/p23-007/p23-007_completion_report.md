# P23-007 完了報告: 個別Divisionの復員と生存兵員の人的資源返還

## 1. 総合判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

自動テスト・検証コマンドはすべて成功したが、UIを変更したタスクであるにもかかわらず
実ウィンドウでの目視確認(`cargo run`)は本セッションでは実施していない(§7参照)。

## 2. 基準状態

* 開始時HEAD: `90ea597908281c59a00f0084c9506d22bc044adf`(P23-006-FIX-001完了報告書コミット)
* 実装コミットの完全なハッシュ: `e18c6158f7d75ae3b9ce2d8643468af27abdff7a`
  (`feat: add division demobilization`)
* 報告書コミットの完全なハッシュ: 本報告コミット後に確定するため、このコミット自身の
  ハッシュはここへ自己参照的に記載しない(コミット後の最終回答で報告する)
* 最終HEAD: 報告書コミット後に確定
* `saves/savegame_v1.ron`はユーザーの既存未コミット変更(実プレイデータ)であり、
  本タスクを通じて一切参照・変更・ステージ・コミットしていない。

事前確認: `military::manpower::compute_manpower_ledger`・`ManpowerLedger`・
`ManpowerLedgerError`・`MilitaryRegistry::remove_division`・`DivisionStatus`・
`ArmyRegistry`(memberの管理・`sanitize_references`)・前線/攻勢/防御命令と
Army/Divisionの参照関係・`BattleRegistry`・`Division.combat_id`・移動/占領処理が
参照するDivision ID・UIの選択中Army/Division状態・国家消滅時のDivision削除経路
(`country::elimination`)・save validationの既存拒否経路・New Game初期Division
(P23-006-FIX-001)・P23-006/FIX-001のUI更新経路を、現在のHEADで再確認した。
特に重要な発見として、`FrontlineRegistry`にはArmy解散時の前線参照整理を専用に行う
`sanitize_army_references`(`sanitize_references`とは別の既存関数)と、それを
`ArmyRegistry`変化を検知して自動実行する既存の反応的System
`war::frontline::sync_army_frontline_references`が既に存在し、
`map::division_selection::prune_selected_division`と同じ設計思想
(`is_changed()`ガード)で運用されていることを確認した。これにより、本タスクの
Domain API自身が前線のArmy参照整理を直接行う必要はなく、既存の仕組みをそのまま
再利用できることが分かった。プロンプトの前提と実コードに重大な食い違いはなかった。

## 3. 実装内容

### 3.1 正規Domain API

`src/military/demobilization.rs`に新設した`military::demobilization::
demobilize_division(country_registry, military_registry, army_registry,
frontline_registry, war_registry, battle_registry, country_id, division_id)
-> Result<DemobilizationOutcome, DemobilizationError>`。UI・テスト・将来のAIは
すべてこの1つの関数を経由し、人的資源の返還とDivision削除を個別に再実装しない。

### 3.2 復員可能条件

* 要求国家が`CountryRegistry`に存在し、`is_active()`であること
* 対象Divisionが存在し、所有国が要求国家と一致すること
* `manpower > 0`かつ`status != Destroyed`(生存していること)
* `status == DivisionStatus::Idle`であること。`Fighting`・`Moving`・
  `Retreating`はそれぞれ専用のエラー(`DivisionFighting`/`DivisionMoving`/
  `DivisionRetreating`)を返す。`Occupying`・(未使用の)`Disbanding`は、
  州占領処理等との相互作用を個別に安全と確認できていないため、共通の
  `DivisionNotIdle`で一律拒否する(推測で許可しないという方針どおり)
* `status`がIdleに見えても、`BattleRegistry`の進行中(`Ongoing`)戦闘が当該
  Divisionを攻撃側・防御側いずれかの参加者として参照している場合、または
  `Division.combat_id`が`Some`の場合は`DivisionReferencedByOngoingBattle`で
  拒否する(statusだけを信用しない防御的検証)
* 人的資源帳簿が`compute_manpower_ledger`で計算できること(未解決の募兵定義が
  あれば`LedgerUnavailable`)、かつ`unaccounted_living_excess == 0`
  (`LedgerInconsistent`でなければ)であること

### 3.3 返還人数の定義

返還する人数`R`は、復員実行時点のDivisionの実際の`manpower`であり、
`max_manpower`や当初の募兵時人数ではない。Domain単体テストで、損耗済み
Divisionの復員が`max_manpower`ではなく現在manpowerを返すことを固定した。

### 3.4 `available`・`B`・`L`・`K`の遷移

```text
available_manpower += R  (checked_add、overflow時はAvailableManpowerOverflowで全体拒否)
mobilized_manpower(B) -= R  (事前にB>=Rを検証済みのため通常は失敗しない)
Divisionを削除 → L(現役軍事人的資源)がR分減る
K(恒久軍事損失) = B - L は自然に不変(BとLが同数減るため)
```

`K`を直接書き換える保存フィールドは追加していない。`K`は引き続き
`compute_manpower_ledger`が毎回`workforce_burden.saturating_sub(
living_military_manpower)`として導出する。

### 3.5 失敗時の原子性

`demobilize_division`は、国家・Division・所有権・生存・status・戦闘参照・
帳簿の全検証を先に完了し、`available_manpower + R`のoverflow検証・
`mobilized_manpower - R`の検証まで済ませた後で初めて、`CountryData`の
2フィールド書き換え・`MilitaryRegistry::remove_division`・
`ArmyRegistry::sanitize_references`・`FrontlineRegistry::sanitize_references`
を実行する。途中の検証が1つでも失敗すれば、これらのいずれも一切変更しない
(Domain単体テストの「失敗時の原子性」カテゴリで、国家・Division・Army・
戦闘参加状態がすべて変化しないことを確認済み)。

### 3.6 Army・前線・選択状態の整理

* Army member一覧からの除外・空Armyの自動解散は、既存の
  `ArmyRegistry::sanitize_references`をそのまま呼び出す(複製しない)
* 前線Plan側の`assigned_division_ids`の整理・非Active戦争の前線除去は、
  既存の`FrontlineRegistry::sanitize_references`をそのまま呼び出す
* Army解散によって空になったArmyへの前線側`assigned_army_ids`/
  `army_frontline_map`参照は、§2で確認した既存の反応的System
  `war::frontline::sync_army_frontline_references`(`ArmyRegistry`の変化を
  検知して`sanitize_army_references`を自動実行)が次フレームで処理する。
  本タスクのDomain API自身はこれを直接呼び出していない(既存の仕組みの
  再利用であり、重複実装ではない)
* UI選択状態(`SelectedDivision`)についても同様に、既存の
  `map::division_selection::prune_selected_division`
  (`MilitaryRegistry`の変化を検知して削除済みIDを毎フレーム除去する
  既存System)がそのまま機能する。これは`src/map/division_selection.rs`の
  `prune_selected_division_removes_destroyed_division_ids`で既に固定されて
  おり、本タスクの新規テストで重複して検証していない(理由はE2Eテストの
  対応コメントに明記した)

### 3.7 民間労働力の更新タイミング

`demobilize_division`は`StateData`の雇用フィールド(`employed_workforce`/
`unemployed_workforce`)を一切触れない。返還によって`mobilized_manpower`
(=B)が減っても、次の正規月次処理(`MonthChangedMessage`駆動の
`economy::handle_monthly_economy`)が新しいBを州へ人口比按分するまでは
反映されない。E2Eテストで、復員直後の同一フレームでは雇用フィールドが
不変であること、次月の更新で民間労働力が返還人数Rの分だけ回復すること
(損耗済みDivisionではK相当分は回復しないこと)を確認した。

### 3.8 UI

`src/ui/military_panel.rs`に、Army編成解散(`military_panel.army_disband_
button`)とは別のコンポーネント型・ラベルを持つ「師団を復員」ボタン
(`DemobilizeDivisionButton`/`DemobilizeDivisionInfoText`)を追加した。
既存の`StopMovementButton`と同じ構造(選択中Divisionに対して動作し、
背景色・件数表示をUIボタン活性判定専用の軽量なフィルタで更新し、実際の
実行可否は`demobilize_division`自身が再検証する)を踏襲した。UIコード内で
帳簿計算やArmy/前線整理ロジックを複製していない。成功時・失敗時とも
`GameNotification`で結果を通知し、失敗理由は戦闘中/移動中/撤退中/帳簿計算
不能/その他の5種類のローカライズキーへ写像する(生のDivisionDefinitionId
やエラー型名を画面へ出さない)。

### 3.9 セーブV1を維持した方法

新しい保存フィールド・新しいResource・新しいComponentは追加していない。
復員はDivisionの削除と`CountryData`の既存2フィールド(`available_manpower`/
`mobilized_manpower`)の変更だけで完結するため、既存のV1シリアライズ形式が
そのまま復員後の状態を保存・復元できる。save/loadのE2Eテストで、復員後の
帳簿(B/L/K)が保存・ロード後も一致すること、削除済みDivision・空Armyが
ロード後に復活しないことを確認した。

## 4. 変更ファイル一覧と役割

| ファイル | 役割 |
|---|---|
| `src/military/demobilization.rs`(新規) | 正規Domain API `demobilize_division`、`DemobilizationOutcome`/`DemobilizationError`、Domain単体テスト22件 |
| `src/military/mod.rs`(変更) | `pub mod demobilization;`の追加のみ |
| `src/ui/military_panel.rs`(変更) | 「師団を復員」ボタン・情報テキストの追加、押下ハンドラ、失敗理由のローカライズキー写像 |
| `assets/localization/en-US.ron`・`ja-JP.ron`(変更) | 復員ボタン・状態表示・成功/失敗メッセージのJA/ENキーを追加 |
| `tests/p23_007_division_demobilization_e2e_test.rs`(新規) | 実経路統合テスト24件(募兵完成・実戦闘・Army/前線・民間労働力・UI・save/load) |
| 本完了報告書(新規、別コミット) | — |

## 5. 追加・更新したテスト

### 5.1 Domain単体テスト(`src/military/demobilization.rs`、22件)

無損耗Division全額返還・損耗済みDivisionの生存者のみ返還・`max_manpower`を
使わないこと・他Division/他国への非波及・Army member除外/空Army自動解散・
同一Divisionの2回目復員の安全な失敗・treasury/装備を返還しないこと、
および失敗時の原子性(存在しない国家/消滅国家/存在しないDivision/所有者不一致/
manpower 0/Destroyed/Fighting/Retreating/Moving/BattleRegistry参加中/
未知募兵定義/帳簿不整合/overflow)を固定した。

このうち`InsufficientBurden`(B<R)は、数式上`unaccounted_living_excess==0`
(`LedgerInconsistent`を通過)を満たす限り対象DivisionのRは必ずB以下になる
ため、正常経路からは到達し得ないことを`burden_smaller_than_division_
manpower_is_unreachable_once_the_ledger_is_consistent`で明示した。
`checked_sub`による防御的な安全網としては維持しているが、実際に返る理由は
常に`LedgerInconsistent`である(§9でも後述)。

### 5.2 実ライフサイクル統合テスト(§13.3相当、9件)

募兵開始→完成→無損耗復員、実戦闘(`handle_daily_military`)で実際に損耗させた
生存者の復員とK維持、実戦闘で全滅したDivisionの復員拒否(`DivisionNotFound`)、
Armyの一部member復員と他member維持、Army最後のmember復員による空Army自動解散
と前線参照整理(`sync_army_frontline_references`経由)、同一国家の複数
Divisionを順に復員した際の帳簿一貫性、他国への非波及、New Game初期Division
復員後の`B=L+K`維持、復員後複数フレーム継続してもpanicしないことを確認した。

### 5.3 民間労働力統合テスト(§13.4相当、4件)

復員直後は雇用フィールドを先行更新しないこと、次月の更新で民間労働力が
返還人数分回復すること、恒久軍事損失分は構造的に回復対象から除外される
こと(`apportion_mobilized_manpower`の入力がB[累積動員負担]のみでKを含まない
ため)、他国の民間労働力へ波及しないことを確認した。

### 5.4 UI・ローカライズ統合テスト(§13.5相当、6件)

ボタンの活性状態(選択中の復員可能Division数表示)、戦闘中Divisionを対象に
した押下が実際には何も変更せず失敗通知のみ発行すること(panicしないこと)、
Army解散と復員のラベルが異なること、成功後にパネル表示(現役/累積負担)が
更新されること、ボタン・情報テキストが複数回のUpdateでも重複spawnされない
こと、JA/ENいずれも未翻訳キーマーカーを露出しないことを確認した。

### 5.5 Save/Load統合テスト(§13.6相当、5件)

無損耗復員後の帳簿の保存・ロード一致、損耗済みDivision復員後もKが維持される
こと、削除済みDivision・空Armyがロード後に復活しないこと、`available`と
`B`がロード前後で一致すること、`L`と`K`がロード後に同一値として再導出される
ことを確認した。

## 6. 実行した全コマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | PASS |
| `cargo test --lib military::` | PASS、109 passed |
| `cargo test --lib population::` | PASS、46 passed |
| `cargo test --lib country::elimination::` | PASS、17 passed |
| `cargo test --lib save::` | PASS、300 passed |
| `cargo test --test p23_001_population_manpower_e2e_test` | PASS、3 passed |
| `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test` | PASS、8 passed |
| `cargo test --test p23_003_population_manpower_ui_e2e_test` | PASS、8 passed |
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test` | PASS、7 passed |
| `cargo test --test p23_006_manpower_ledger_e2e_test` | PASS、22 passed |
| `cargo test --test p23_006_fix_001_initial_manpower_ledger_e2e_test` | PASS、19 passed |
| `cargo test --test p23_007_division_demobilization_e2e_test` | PASS、**24 passed** |
| `cargo test --test p20_009_localization_resource_test` | PASS、8 passed |
| `cargo test --test p20_009_hardcoded_string_scan_test` | PASS、4 passed |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、**警告0**(`too_many_arguments`2件へ既存規約どおり`#[allow]`、テストファイルの`SystemState`複合タプル型へ`type_complexity`を1件`#![allow]`) |
| `rustfmt --edition 2024 --check`(変更・新規ファイル個別指定) | 初回に整形差分あり→`rustfmt --edition 2024`適用後PASS(いずれも非`mod.rs`のリーフファイル。`src/military/mod.rs`は1行追加のみで整形は実行していない) |
| `cargo test --all-targets`(全体、最終実行) | PASS、**全件PASS**(**1464 passed**、**0 failed**、1 ignored[既存の`profile_p21_021_elimination_scaling_test`、本タスク無関係]) |

1464 = 直前の基準1418 + Domain単体テスト新規22件 + E2E新規24件、と正確に一致する
ことを確認した。`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない。
既知の非決定的PNG9枚は`cargo test --all-targets`実行後に想定どおり再生成され、
個別ファイル単位で`git checkout --`により復元した(ディレクトリ単位の復元はしていない)。

## 7. 手動GUI確認

**実施していない。** 本セッションには実ウィンドウを操作する手段が無く、
`cargo run`によるスクリーンショット撮影・実際の目視確認は一切行っていない。

未確認項目:

* 実際の画面上で「編成解散」ボタンと「師団を復員」ボタンが混同しにくい配置・
  文言になっているか
* 復員可能なDivisionだけがボタン活性表示になり、戦闘中/移動中/撤退中の
  Divisionを選択した際に実際の見た目上も無効化されるか(現状の実装は
  StopMovementButtonと同じく背景色変化のみで、`disabled`属性自体は使用して
  いない——これは既存のRecruitButton等も同じ既存パターンである)
* 復員成功時の通知表示のタイミング・見た目・他の通知との重なりの有無
* 日本語・英語いずれでもボタン文言・情報テキストが長さにより崩れないか
* 実際のSave/Load操作(UIボタン経由)後の表示・空Army一覧表示が残らないか

これらは自動テスト(§5)でSystem経由の実値・ローカライズキー解決・
Army/前線参照整理・selection pruneへの依拠は検証済みだが、実際のレイアウト・
視認性はテストでは検出できない。

## 8. 対象外を先行実装していないこと

Army解散の意味変更、国家全体の一括動員解除、募兵キューの新しいキャンセル
方式、補充、Attrition、AIによる復員判断、AI募兵、自動戦後復員、装備・資金・
経験値の返還、戦死者・負傷者・捕虜の内訳、戦闘損失によるpopulation減少、
恒久軍事損失の州別履歴、`available + mobilized <= cap`の強制、初期
available/人的資源上限の再調整、徴兵法・動員法、`war_exhaustion`、戦闘式・
移動式・占領式の変更、新規セーブフィールド、セーブV2、P23-008以降の機能は、
いずれも変更ファイル一覧(§4)に含まれておらず実装していないことを確認した。
戦闘コード側での`mobilized_manpower`変更・`available_manpower`の戦闘関連
返還も同様に行っていない(復員経路のみが人的資源を動かす)。

## 9. 残課題または後続候補

* `DemobilizationError::InsufficientBurden`は§5.1で述べたとおり正常経路
  からは到達し得ない(`LedgerInconsistent`が先に成立するため)。API契約上の
  防御的な安全網として維持する価値はあるが、将来この帳簿検証の順序や
  `compute_manpower_ledger`自体の不変条件が変わる場合は、この到達不能性が
  崩れないか再検討が必要。
* `DivisionStatus::Occupying`は今回「復員可能条件を個別に安全と確認できて
  いない」として一律拒否した(`DivisionNotIdle`)。州占領処理
  (`war::occupation`)との相互作用を精査すれば、将来的にOccupying状態の
  Divisionも復員可能にできる可能性がある。
* `DivisionStatus::Disbanding`は本タスクを含め、コードベース全体で実際に
  Divisionへ設定される経路が存在しない未使用の列挙子である(P23-005監査の
  対象外だった軽微な既存の死んだコード)。復員条件の判定には含めたが、
  本タスクではこの列挙子自体を削除・活用するリファクタリングは行っていない。
* P23-005/P23-006が既に指摘した残課題(個別Division以外の一括動員解除、
  補充、Attrition、AI募兵、population↔恒久軍事損失の統合)は本タスクの
  スコープ外のまま未着手。

## 10. 実装コミット直後と報告書コミット後の`git status --short`

### 実装コミット(`e18c6158f7d75ae3b9ce2d8643468af27abdff7a`)直後

```
 M saves/savegame_v1.ron
```

`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、実装コミットには
含めていない。それ以外の全変更(§4の実装・テスト・ローカライズファイル)はコミット済み。

### 報告書コミット後

報告書コミット作成後の最終`git status --short`は、コミット完了後の最終回答で報告する。
