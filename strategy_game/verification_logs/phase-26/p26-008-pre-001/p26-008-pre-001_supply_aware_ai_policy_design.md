# P26-008-PRE-001: 補給を考慮したAI判断・備蓄目標・評価周期の設計監査

本報告書は、P26-004〜P26-007で完成したモデル2補給(国家別MilitarySupplies備蓄・月次生産・
日次消費・Hamilton配分・首都連結性・国家消滅/占領/海外領土会計)を、P26-008でAI判断
(募兵・攻勢・建設)へ接続する**前に**、具体的な備蓄目標日数、募兵/攻勢の停止・再開条件
とヒステリシス、建設優先度、AI評価周期(`dirty`問題を含む)を、実コードと6国家の実測に
基づいて確定する**設計専用**の報告書である。本タスクでは本番コード・本番データ・既存テストを
一切変更しない。

## 1. 調査完了判定・2. 設計確定判定

**ANALYSIS COMPLETE**
**DESIGN DECIDED**(ユーザー判断が必要な論点は §26 に3件へ絞って明示する)

## 3. 開始時HEAD・4. 監査対象コードHEAD

- 開始時HEAD: `2cdf32a0ed4b6250e91eaa97a7088cca9aabd115`(期待値`2cdf32a`と一致、`git rev-parse HEAD`で確認)
- 開始時の既存差分は空(`git status --short`が空。タスク文書が想定した`M saves/savegame_v1.ron`は、
  直前のP26-007タスク終了時に既に`git checkout --`で復元済みだったため存在しなかった。
  reset/checkout/stash等でこれを合わせる操作は行っていない――実際の状態をそのまま報告する)。
- 開始時テスト基準: `cargo test --all-targets` = 2360 passed / 0 failed / 1 ignored(期待値と一致)。
  `cargo clippy --all-targets --all-features -- -D warnings` = 0 warnings(期待値と一致)。
- 監査対象コードHEAD = 開始時HEADと同一(`2cdf32a`。本タスクはコード変更を行わないため、
  監査対象コードは開始から終了まで一貫して同一コミット)。

## 5. P26-008の目的

P26-004〜007で完成したモデル2補給の会計(国家別備蓄・日次需要・Hamilton配分・首都連結性・
国家消滅/占領時の会計)を、AIの意思決定(募兵・攻勢・建設)へ**初めて**接続すること。
現在、AIはMilitarySuppliesの状態を一切参照せずに募兵・攻勢・建設を判断している
(§6〜11で実コードから確認)。本タスクはP26-008を実装するのではなく、その実装が
迷わず着手できるよう、指標・目標値・停止/再開条件・優先順位・評価周期の方針を
数値と根拠付きで確定する。

## 6. 現行AI System一覧

実コードから確認した、補給状態に将来影響し得るAI経路は以下の6系統。「AI」と総称せず、
各々の実Systemと判断単位を分離して監査した(§7〜11)。

| # | 系統 | 主要関数 | 登録System | SystemSet | 呼び出し契機 | 対象単位 |
|---|---|---|---|---|---|---|
| 1 | 建設AI | `process_construction_ai`(`src/country/country_ai.rs:486`) | `process_daily_country_ai`から呼ばれる(System自体は`handle_daily_country_ai`, 未確認箇所は§7で補足) | `DailySimulationSet::CountryAi` | 日次、`is_monthly_due`(`day%30==country_id%30 \|\| dirty`)成立時のみ内部実行 | 国家 |
| 2 | 研究AI | `process_research_ai`(`country_ai.rs:597`) | 同上 | `CountryAi` | 日次、`is_weekly_due`成立時のみ | 国家 |
| 3 | 戦争準備AI(同盟/宣戦候補評価) | `process_war_preparation_ai`(`country_ai.rs:668`) | 同上 | `CountryAi` | 日次、`is_weekly_due`成立時のみ、かつ`!is_at_war` | 国家 |
| 4 | 宣戦布告AI | `process_war_declaration_ai`(`country_ai.rs:802`) | 同上 | `CountryAi` | **日次・無条件**(dirty/cadence判定なし) | 国家 |
| 5 | 募兵AI | `evaluate_ai_monthly_recruitment`→`decide_and_recruit_for_country`(`src/military/ai.rs`) | `handle_monthly_ai_recruitment` | `CountryAi` | `MonthChangedMessage`ごとに1回(dirty/cadence判定なし、月次固定) | 国家 |
| 6 | 軍事AI(前線スタンス: 攻勢/防御) | `evaluate_military_ai_decision`(`src/war/military_ai.rs:288`) | `handle_daily_military_ai` | `DailySimulationSet::MilitaryAi` | 日次、`current_day - last_evaluated_day >= 3 \|\| dirty`成立時のみ、かつ当該国が交戦中 | 国家×前線 |

§7〜11で各系統を個別に詳細化する。

## 7. 建設AI

- **System名/頻度**: `process_construction_ai`。`process_daily_country_ai`(`country_ai.rs:885`)の
  ループ内で、`is_monthly_due = (current_day as usize % 30 == country_id.0 % 30) || ai_state.dirty`
  が真の日にのみ呼ばれる(意図: 月次、国家ごとに評価日を分散)。
- **dirtyを立てる条件**: `CountryAiRegistry::mark_all_dirty`/`mark_country_dirty`が唯一の書き手。
  実際には`war::update::mark_ai_registries_dirty_daily`が`DailySimulationSet::WarResolution`
  (`handle_daily_war_resolution`の直後)で**日次無条件**に`mark_all_dirty()`を呼ぶ(§12〜14で詳述)。
- **dirtyを解除する条件**: `process_daily_country_ai`の各国家処理の末尾(`country_ai.rs:1012`)で
  `ai_state.dirty = false`。ただし同日内の後続System(`WarResolution`)で必ず`true`へ戻される。
- **国家処理順**: `country_registry.countries`から`is_active()`かつプレイヤー以外を抽出し
  `CountryId`昇順にソート(`country_ai.rs:906-913`)――決定的。
- **候補処理順**: 優先度1(Food<50→Farm)→優先度2(P26-004-PRE-002の事前開発Mine、
  `ResourceType::ALL`宣言順→`StateId`昇順のtie-break)→優先度3(Wood/Iron/Coal<30→Mine、
  それ以外→Factory、Mineに候補が無ければ同月中にFactoryへフォールバック)。
- **読み取る状態**: `country.stockpile`(Food/Wood/Iron/Coal)、`country.treasury`、
  `country.construction_queue.len()`、`state_registry`の所有/実効支配/資源鉱床/既存建物レベル。
- **書き換える状態**: `country.treasury`(建設費減算)、`country.construction_queue`(1件追加)、
  `ai_state.decision_reason`。
- **New Game初期値/Loaded Game初期値**: `construction_queue`は空、`ai_state`は
  `CountryAiState::default()`(`dirty:true`)。Loadedでも`dirty`は保存されず、load後は
  常に`dirty:true`から再開する(`src/save/apply.rs`のP21-SAVE-002A1方針、§47参照)。
- **消滅国家の除外**: `is_active()`フィルタで`process_daily_country_ai`のループ自体に入らない。
- **MilitaryFactoryは決して建設AIの対象にならない**――`process_construction_ai`が対象にする
  `BuildingType`は`Farm`/`Mine`/`Factory`のみで、`MilitaryFactory`を選ぶコードパスは
  存在しない(grep実測、`country_ai.rs`内で`MilitaryFactory`が現れるのはテストの
  フィクスチャ定義のみ)。MilitaryFactoryのレベルは`assets/data/states.ron`の初期データで
  固定され、ゲーム中に変化しない。これは§18の生産鎖ボトルネック分類において決定的な事実。
- **Save対象フィールド/派生キャッシュ**: `ai_state`のうち`dirty`を除く全フィールドが
  `SavedCountryAiState`(§47)。`construction_queue`自体は`CountryData`の一部として通常のSave
  経路で保存される(本タスクの対象外、既存契約)。

## 8. 募兵AI

- **System名/頻度**: `handle_monthly_ai_recruitment`(`src/military/ai.rs:237`)。
  `MonthChangedMessage`受信ごとに、同一フレーム内の二重処理を`evaluated_this_frame`フラグで
  防いだ上で`evaluate_ai_monthly_recruitment`を1回だけ実行する。**建設AI/研究AI/戦争準備AIとは
  完全に独立したSystemであり、`CountryAiState.dirty`もこの経路の判断には一切関与しない**――
  したがって募兵AIの評価頻度自体は「daily dirty問題」(§12〜17)の影響を受けない、正しく
  月次1回のまま動作している唯一の系統である。
- **国家処理順**: `CountryId`昇順(`ai.rs:213-219`)。
- **判断ロジック**(`decide_and_recruit_for_country`, `ai.rs:118`):
  1. `country.recruitment_queue`が非空なら即スキップ(`SkippedQueueNotEmpty`)――**AIは常に
     最大1件までしか同時に募兵しない**。
  2. `target = calculate_ai_target_divisions(cap, required_manpower)`
     = `floor((cap/2) / required_manpower)`(0または`cap<required`なら0、それ以外は最低1)。
     `cap = compute_manpower_cap(population)`。
  3. `current = 生存Division数 + 募兵キュー件数` が`target`以上ならスキップ(`SkippedTargetMet`)。
  4. `reserve = ceil(cap/4)`。`available_manpower < required_manpower + reserve`ならスキップ
     (`SkippedInsufficientManpower`)。
  5. 募兵州は自国支配の首都優先、なければ人口最大の自国支配州(`select_ai_recruitment_state`)。
  6. 上記すべてを満たした場合のみ`request_recruitment`(プレイヤーUIと同一の公開API)を呼ぶ。
- **MilitarySuppliesは一切参照されない**――上記のどのステップにもMilitarySuppliesの
  読み取りが存在しない(grep実測でゼロ)。これがP26-008で追加すべき唯一のゲート対象。
- **募兵完了と日次補給の順序**(`src/military/mod.rs:51-76`監査): `handle_monthly_ai_recruitment`は
  `CountryAi` Set、日次補給消費(`daily_supply::process_daily_military_supply_consumption`)は
  `Economy` Set(`handle_monthly_economy`の直後)、募兵完了によるDivision生成
  (`process_recruitment`、`handle_daily_military`経由)は`MilitaryAction` Set。
  `DailySimulationSet`の宣言順は`Economy → ... → CountryAi → ... → MilitaryAction`である
  ため、**その日Economy Setで確定した需要には、その日完成した新Divisionの需要は含まれない**
  ――新Divisionの需要は**完成翌日**の日次補給から初めてカウントされる(§23で再利用)。
- **重要な実測発見(募兵キューの永久停滞)**: `process_recruitment`(`src/military/recruitment.rs:85`)は
  `days_remaining==0`に達した項目を、`MilitaryEquipment`備蓄から`try_issue_equipment`で
  装備を支給できた場合にのみDivisionへ変換する。装備備蓄が不足する場合
  (`EquipmentAccountingError::InsufficientStockpile`)、**項目は完了させずキューに残したまま
  何度でも同じ判定を繰り返す**――つまり`MilitaryEquipment`(MilitarySuppliesとは別資源)が
  慢性的に不足する国家では、募兵キューが**永久に**先頭でスタックし、
  `decide_and_recruit_for_country`のステップ1(キュー非空チェック)により**新規募兵自体が
  それ以降ずっと発生しなくなる**。§21の実測で、この現象がOceanic等で実際に972日間発生し
  続けていることを確認した――これはMilitarySuppliesとは無関係な既存のゲームバランス事象
  であり、P26-008が新たに作る問題ではないが、P26-008の効果を評価する際の重要な前提となる
  (§39〜41)。

## 9. 移動AI

- 独立した「移動AI」Systemは存在しない。Divisionの移動自体は`military::movement::process_movement`
  (`MilitaryAction` Set内、`handle_daily_military`から呼ばれる)が担い、移動先の決定は
  戦線計画AI(§10)またはプレイヤー操作が`target_state`/`destination`を設定した結果を
  実行するだけである。したがって「移動AI」という単位は実質的に存在せず、
  「軍事AIが前線スタンスと攻勢目標を決め、それを`FrontlineRegistry`経由で移動へ変換する」
  という一連の流れ(§10・§11参照)がこれに相当する。P26-008が新設すべきなのは、この
  変換の入り口(前線スタンス決定)に補給ゲートを追加することであり、移動そのものの
  ロジックを変更する必要はない(§16・§33)。

## 10. 戦線計画AI

- **System名/頻度**: `handle_daily_military_ai`(`src/war/military_ai.rs:665`)。
  `DailySimulationSet::MilitaryAi` Set。`assign_unassigned_divisions_to_frontlines`
  (未割当Divisionの前線割当、`CountryId`昇順→`WarId`昇順で決定的)の後、各前線について
  `evaluate_military_ai_decision`(下記)で`FrontlineStance`を決定する。
- **評価cadence**: `is_evaluation_due = current_day.saturating_sub(last_evaluated_day) >= 3
  || ai_state.dirty`(`military_ai.rs:581-582`)。意図は「3日おき、ただし変化があれば即時」。
  この`ai_state.dirty`は`MilitaryAiRegistry`(`CountryAiRegistry`とは別の独立レジストリ)の
  ものだが、**同じ`mark_ai_registries_dirty_daily`が両方を毎日dirty化する**(§12〜17で詳述)。
- **攻勢の正規定義(§30の先取り回答)**: `FrontlineStance::Offensive`という列挙値そのもの。
  これが`FrontlinePlan.stance`へ書き込まれ、`war::frontline::process_daily_frontline_plans`
  (`DailySimulationSet::FrontlineOrders`)が実際の移動命令(`process_offensive_plan`)を生成する。
  つまり「攻勢」とは**「敵州へ向かう移動命令」ではなく「前線スタンスがOffensiveへ遷移した
  瞬間」**であり、これがAI攻勢判断の唯一の分岐点である。
- **既存の110%/90%ヒステリシス**(`evaluate_military_ai_decision`, `military_ai.rs:333-347`):
  ```text
  攻勢開始条件: own_power * 1000 >= enemy_power * 1100  (110%)
  攻勢維持条件: own_power * 1000 >= enemy_power * 900   (90%)
  ```
  現在`Offensive`なら90%条件で継続判定、現在`Offensive`でなければ110%条件で新規開始判定
  ――**開始しきい値と維持しきい値を非対称にして振動を防ぐ、既存のヒステリシス実装が
  すでに存在する**。P26-008の補給ゲート設計(§30〜33)はこの既存パターンを模倣する。
- **決定理由**: `MilitaryAiDecisionReason`列挙(`NoActiveWar`/`NoValidFrontline`/
  `NoAvailableDivision`/`NoReachableFrontline`/`NoValidTarget`/`Recovering`/`EnemyStronger`/
  `ComparableStrength`/`SufficientAdvantage`/`TargetCaptured`)――補給不足による抑制を追加する
  場合、この列挙へ新たな理由(例: `SupplyShortage`)を追加するのが既存パターンと整合する。
- **読み取る状態**: 割当済みDivisionの`manpower`/`status`(`evaluate_powers`)、
  州の`is_sea`/`controller()`。MilitarySuppliesは一切参照しない。

## 11. 戦争・外交AI

- **宣戦布告AI**(`process_war_declaration_ai`, `country_ai.rs:802`): 日次・無条件呼び出し
  (dirty/cadenceの対象外)。既存のCrisis/Claim機構から生成された宣戦候補を評価する。
- **戦争準備AI**(`process_war_preparation_ai`, `country_ai.rs:668`): 週次評価
  (`is_weekly_due`)、`!is_at_war`のときのみ。同盟・クリシス開始等の準備判断。
- いずれもMilitarySuppliesを一切参照しない。P26-008の対象は募兵AI・戦線計画AIの2系統に
  限定してよく、宣戦布告AI/戦争準備AI自体を変更する必要はない(§20の除外事項)。

## 12〜17. `dirty`/評価cadence問題の監査と実測

### 12. `dirty`の意味

`CountryAiState.dirty`/`MilitaryAiState.dirty`は「このAIの週次/月次/3日おき評価を、
定期スケジュールを待たずに**今日**強制的に実行してよい(すべき)」という**キャッシュ無効化
フラグ**である。設計意図は「通常は国家ごとに評価日をずらして負荷を分散するが、状態が
実際に変化した日(宣戦布告等)は即座に再評価する」というもの。

### 13. daily dirty実挙動

`war::update::mark_ai_registries_dirty_daily`(`src/war/update.rs:92-101`)は
`DailySimulationSet::WarResolution`に`handle_daily_war_resolution`の直後として登録され
(`src/war/mod.rs:38-53`)、**`DayChangedMessage`を受信するたびに無条件で
`ai_registry.mark_all_dirty()`と`country_ai_registry.mark_all_dirty()`を呼ぶ**。

`DailySimulationSet`の宣言順は
`TimeUpdate → Economy → Research → Diplomacy → CountryAi → WarPreparation → MilitaryAi
→ FrontlineOrders → MilitaryAction → WarResolution → UiUpdate`であるため、日Nの
`WarResolution`(dirty=true化)は日Nの`CountryAi`/`MilitaryAi`(dirty読み取り)より**後**に
実行される。したがって日N+1の`CountryAi`/`MilitaryAi`は、日Nの終わりに`true`へ戻された
`dirty`を読む――**`dirty`は、`process_daily_country_ai`/`handle_daily_military_ai`が読む
時点では常に`true`である**。両System自身は処理末尾で`dirty`を`false`に戻す
(`country_ai.rs:1012`、`military_ai.rs:576/659`)が、それは同日の`WarResolution`より前の
話であり、その日の`WarResolution`が必ず後から`true`に戻す。

### 14. AI評価cadence(実測で確定)

一時計測用の非コミットE2Eテスト(6か国・36か月・実AIプレイ、`PlayerCountry(None)`、
診断用無限資金なし)を用いて、`CountryAiState.dirty`/`MilitaryAiState.dirty`を各月末
(月内27日分の日次tickの後)に直接サンプリングした。

**結果: 216サンプル(6か国×36か月)全件で`ai_dirty == true`かつ`mil_ai_dirty == true`。**

これは§13のコード解析どおり、`dirty`が事実上「常にtrue」であることの直接的な実測確認である。

### 15. 90日の評価回数・16. 36か月の評価回数

`dirty`が常にtrueである結果、以下のSystemの実際の評価頻度は設計意図と乖離する
(6か国×日数、AI対象国はプレイヤー設定により5または6か国):

| System | 設計意図の頻度 | 実際の頻度 | 90日(=90日×6国)の想定評価回数 | 実際の評価回数 | 36か月(=972日×6国)の想定評価回数 | 実際の評価回数 |
|---|---|---|---|---|---|---|
| 研究AI・戦争準備AI(週次) | 7日に1回 | **毎日** | 約77回 | **540回** | 約834回 | **5832回** |
| 建設AI(月次) | 30日に1回 | **毎日** | 約18回 | **540回** | 約194回 | **5832回** |
| 戦線計画AI(3日おき) | 3日に1回 | **毎日** | 約180回 | **540回** | 約1944回 | **5832回** |
| 募兵AI(月次) | 月次1回 | **月次1回(正常)** | 18回 | 18回 | 216回 | 216回 |
| 宣戦布告AI(日次) | 毎日 | 毎日(正常、元から無条件) | 540回 | 540回 | 5832回 | 5832回 |

(90日/36か月の値は6か国換算。想定回数は`floor(日数/周期)×6`に基づく概算。募兵AIのみ
`dirty`と無関係な独立System[§8]のため唯一正しい頻度で動作している。)

**結論として、建設AI・研究AI・戦争準備AI・戦線計画AIは、設計意図(週次/月次/3日おき)の
実質**5〜30倍**の頻度で評価されている。**個々の評価の判断ロジック自体は決定的かつ
正しい(状態が変化していなければ同じ結論を出す)ため、**ゲーム結果の正しさそのものは
損なわれていない**――これは主に計算負荷(§51)とAIの「意図された落ち着き」(週次/月次で
段階的に動くはずの挙動が実質毎日動く)に関する問題である。

### 17. daily dirty問題の判定

**判定: 2 (P26-008-PRE-002として先に独立修正が必要)**

理由:
1. この修正は「今日評価すべきか」を変えるものであり、**AI判断が実際に発生する日**を
   変える――日が変われば、その日にAIが下す個々の決定(建設候補の選択、攻勢開始の有無)
   も変わり得る。これは本タスクのタスク定義書自身が明示する
   「AI判断の意味やsave/load決定性が変わる場合は、P26-008本体へ混在させず、原則として
   独立タスクを選んでください」に該当する。
2. P26-008が新設する補給ゲート(募兵停止・攻勢停止)は、cadenceが直っているかどうかに
   関わらず正しく機能する(ゲート自体は「評価が呼ばれた時に何を判断するか」の話であり、
   「いつ呼ばれるか」とは独立)。したがってP26-008自身の実装をこの修正でブロックする
   必要はないが、**両方を同じコミットで混在させると、cadence変更由来の副作用
   (どの日に攻勢が始まるか等)とP26-008自身の新規ロジックの副作用を後から分離できなくなる**。
3. P26-006-REV-001が既に同じ種類の問題(タイミング系決定性)を独立タスクとして
   1件だけ扱い、成功させた前例がある――同じ手法をここでも踏襲する。

修正の設計自体は本タスクの対象外(§2で実装除外)だが、方向性としては「週次/月次/3日おきの
判定に`dirty`をORで混ぜる現在の式自体は正しい設計だが、`mark_ai_registries_dirty_daily`が
`DayChangedMessage`ごとに**無条件**dirty化しているのが問題」であり、修正は「本当に
状態が変化した国だけを`mark_country_dirty`する(既存APIが既に存在する)」方向になる
はずである、という所見だけをここに記録する。

## 18〜23. AI用の正規指標

### 18. AI用備蓄指標(採用)

実コードで既に計算・保存されている値を最大限再利用し、新規の重い計算を増やさない
2系統の指標を採用する。

**指標A: 国家備蓄日数(warehouse健全性)**
```text
national_reserve_days_c = stockpile_units_c / total_daily_demand_units_c
```
`total_daily_demand_units_c`は`country_daily_demand_units`(`src/military/supply_
initialization.rs:172`、既存のP26-004-PRE-005正規関数、`DivisionId`昇順で決定的)を
そのまま呼ぶ。**接続需要(`eligible_demand_units`)ではなく総需要を分母に使う**――
これは§21の理由(首都占領時の誤認防止)による意図的選択。

**指標B: 配送健全性(delivery健全性)**
```text
min_fulfillment_c = 生存Division(demand>0)のうち MilitaryRegistry::supply_fulfillment_of
                     の最小値(該当Divisionが無ければ「対象外」)
```
これは日次補給処理(P26-004/006)が**既に毎日計算・保存済み**の値を読むだけであり、
新規計算コストはゼロ。

### 19. 需要0の扱い

`total_daily_demand_units_c == 0`(Divisionを持たない、または全Divisionの需要が
構造的に0)の場合、`national_reserve_days_c`は定義できない。この場合は**「不足signalなし」
として扱う**(募兵停止条件・攻勢停止条件のいずれも発火させない)――需要が無い国を
物資不足と誤判定してはならない(spec §9状態F)。`min_fulfillment_c`も同様に「対象Division
なし」として停止条件の対象外にする。

### 20. 切断需要の扱い

`eligible_demand_units`(連結Divisionだけの需要)は指標Aの分母には使わない
(§21で理由を説明)。ただし`min_fulfillment_c`が切断Divisionの補給率0.0を自然に
反映するため、「一部Division切断」(状態C)は指標Bで検出できる。

### 21. 首都占領の扱い(最重要の設計判断)

首都占領時、`connected_states_from_capital`は空集合を返し、`eligible_demand_units`は
0になる――このとき国家備蓄は消費されず、**むしろowner基準生産は継続するため増加し得る**
(P26-007で実測確認済み、§実測は同報告書参照)。もし指標Aの分母に接続需要
(`eligible_demand_units`)を使っていたら、この状況で「需要0→不足signalなし」と誤判定し、
**実際には全軍が0%補給という危機的状態にあるのに「健全」と誤認する**(spec §5.8・§9状態E
が明示的に警告する罠)。

**採用する回避策**: 指標A(`national_reserve_days_c`)は**総需要**を分母に使うため、
首都占領で接続需要が0になっても総需要自体は変化せず、この指標だけでは「健全」と
誤認しない(warehouse自体は健全に見えても構わない――実際warehouseは健全なので、これは
正しい)。**代わりに指標B(`min_fulfillment_c`)が0.0になることで、配送が完全に破綻した
状態を検出する**。したがって募兵・攻勢いずれのゲートも、**指標Aと指標Bの両方**を見る
複合判定にする(§26・§31)。これにより「首都占領=備蓄健全なので募兵継続してよい」という
誤判定と、「首都占領=前線に届いていないので新規攻勢は始めない」という正しい判断の両方を
同時に満たせる。

### 22. 月次生産予測

厳密な予測(次月の生産量を事前計算)は行わず、**カレンダーに基づく「次回生産までの
必要備蓄(bridge)」**という既存の考え方(P26-004-PRE-005)をそのまま再利用する
(§23で式を示す)。理由: `calculate_operation_ratio`(`src/economy/production.rs:58`)が
`min(workforce_ratio, logistics_ratio, input_resource_ratio) × (0.8 if bankrupt)`という
月次に確定する複数の外部要因(労働力・財政・入力資源在庫)に依存するため、日次AIの側から
「次回生産量」を事前に正確に予測することは実質不可能であり、過剰な予測ロジックを
AI側に持たせるより、シンプルに「今の備蓄が、次回生産まで持つか」だけを見る設計の方が
決定的かつ説明可能である。

### 23. 将来募兵需要

募兵AI(§8)の構造上、**AIの募兵キューは常に最大1件**である。したがって§12(元タスクの
比較候補)のうち妥当なのは「現在需要 + 新たに提案されたDivision自身の需要」の1段階のみで
十分であり、多段キューの合算ロジックは不要(現行AIには存在しない状態を想定した過剰設計に
なる)。

```text
bridge_deficit_c(新規募兵検討時)
  = max(0,
        compute_required_bridge_units(
            total_daily_demand_units_c + new_division_daily_demand_units,
            events_before_next_monthly_production(現在日, 現在月の日数)
        )
        - stockpile_units_c
    )
```

`new_division_daily_demand_units`は`STANDARD_INFANTRY_DEFINITION_ID`の
`division_daily_demand_units`から直接計算できる(AIは常にこの定義のみを募兵するため
定数的に扱える)。`compute_required_bridge_units`/`events_before_next_monthly_production`
はいずれもP26-004-PRE-005の既存pub関数(`src/military/supply_initialization.rs`)の
そのままの再利用であり、新規の暦計算コードは不要。

## 24〜29. 募兵の目標・停止・再開・ヒステリシス

### 24. 平時備蓄目標・25. 戦時備蓄目標(実測比較)

6か国36か月の実測(§38〜40、`PlayerCountry(None)`・実AI・診断用無限資金なし)によると、
月末`stockpile`(=`national_reserve_days_c`、需要が常に1.0/日で一定のため数値がそのまま
「日数」に等しい)は月36時点で以下のように**国家間で大きく分散する**:

| 国家 | 月36時点stockpile(=reserve_days) |
|---|---|
| Arcadia | 11.4 |
| Elfin | 1015.0 |
| Dwarf | 137.9 |
| Oceanic | 26.2 |
| Passguard | 2488.1 |
| Ferrowyn | 171.9 |

この分散は既存の生産チェーン(鉱床有無・IndustrialGoods有無・財政破綻)に起因する
構造的な差であり(P26-003〜004-PRE-002の既存監査結果と整合)、AI側の目標値設計とは
無関係である。この実測から言えること: **90日固定を平時目標に採用すると、Arcadia/Oceanic
は現在の経済状況下で「ほぼ永久に目標未達」になり、募兵停止が事実上「その国は二度と
募兵しない」という結果になる**――これは目標値の選び方の問題であり、経済バランス自体の
問題ではないため、目標値は経済的に到達可能な水準で選ぶべきである。

**採用: 平時目標 = 30日**。理由: 既存のP26-004-PRE-005ブリッジ計算(§23で再利用する
`compute_required_bridge_units`)が「次回生産までの日数+1日予備」という**1生産周期**を
基準にしているのと同じ考え方を、恒常的な安全マージンとして採用する――「常に最低1か月分の
備蓄を維持する」という説明可能な基準であり、31.0(New Gameの初期ブリッジ値そのもの)を
無根拠に転用したものではない(30という値は「1生産周期」という独立した根拠から導かれ、
たまたま初期ブリッジの31と近いだけである)。

**採用: 戦時目標 = 平時目標 × 2 = 60日**。理由: 戦時は損耗・追加募兵による需要増加が
見込まれるため、平時の2倍の余裕を持たせる。固定値(90日等)ではなく倍率にすることで、
将来平時目標が調整された場合も比率関係が保たれる。

### 26. 募兵停止条件・27. 募兵再開条件・28. 募兵ヒステリシス

指標A・Bの複合判定とし、既存の110%/90%パターン(§10)と同じ「非対称な開始/継続しきい値」
の考え方を採用する。

```text
target_days_c = 戦時目標(60) if 指標C[戦争状態]==AtWar else 平時目標(30)
stop_days_c   = round(target_days_c * 2/3)   平時20日・戦時40日
resume_days_c = round(target_days_c * 7/6)   平時35日・戦時70日
```

- **停止**: `national_reserve_days_c < stop_days_c` **または** (新規募兵を検討した場合の)
  `bridge_deficit_c > 0`(次回生産まで持たない)。いずれかが真なら新規募兵を見送る
  (既存の`SkippedInsufficientManpower`等と同列の新しい`SkippedSupplyShortage`相当の
  スキップ理由を追加する想定)。
- **再開**: `national_reserve_days_c >= resume_days_c` になった月から新規募兵を再検討可能。
- **ヒステリシス幅**: 停止/再開の間に`target_days_c`の半分(平時15日、戦時20日)の
  ギャップを設ける。§14で提示された3候補(7/15/30日)のうち**15日相当**を採用――
  7日は1週間の生産・消費揺れに対して過敏(§21のような月境界のたった1日の不足で
  停止→再開を繰り返す恐れ)、30日は目標そのものと同じ幅で「一度でも目標を割ったら
  ほぼ目標の倍まで回復しないと再開しない」という過度に慎重な挙動になるため、
  中間の15日相当(target比で1/2)を採用した。
- **状態保持の要否**: `stop_days`/`resume_days`はいずれも`national_reserve_days_c`と
  `target_days_c`(戦争状態から毎回導出可能)だけの純粋関数であり、**新しい永続状態は
  一切不要**――「現在停止中か再開待ちか」はその日の`national_reserve_days_c`と
  `stop_days_c`/`resume_days_c`を比較するだけで毎回re-deriveできるため、ヒステリシス自体の
  ためだけにSaveへ新フィールドを追加する必要はない(§48で結論を再確認)。

### 29. 進行中募兵

キューに既に入っている項目は**キャンセルしない**――既存コードに募兵の取消/中断機能が
一切存在しない(`cancel_recruitment`はプレイヤーUI起点の既存APIとして存在するが、
AI側からの利用は想定しない)ため、新設は過剰設計になる。停止条件は**「新規募兵の開始を
見送る」ことに限定**する(§13.3の方式D、段階制御の第1段のみを採用)。これは`decide_and_
recruit_for_country`のステップ1(`recruitment_queue.is_empty()`チェック)の直後に
ステップ2.5として補給チェックを挿入するだけで実現でき、既存の関数シグネチャや呼び出し
順序を変えない最小差分になる。

## 30〜33. 攻勢の定義・停止・再開

### 30. 攻勢の正規定義

§10で確定済み: `FrontlinePlan.stance`が`FrontlineStance::Offensive`へ遷移すること
(`evaluate_military_ai_decision`の戻り値)。防御(`Defend`)・停止(`Stopped`)・
既存の戦闘継続・退却は対象外。

### 31. 攻勢停止条件・32. 攻勢再開条件

**採用: 案C(複合方式)を、既存の110%/90%判定へのANDゲートとして追加する。**

```text
can_start_offensive_supply_gate(country) =
    national_reserve_days_country >= stop_days_country   (§26の同じ状態を再利用)
    かつ
    当該前線へ割当済みDivisionのmin_fulfillment >= 0.34
```

- 既存の`can_start_offensive`(110%)判定と**AND**で結合する――新規攻勢は「戦力比が十分」
  **かつ**「国家補給が停止水準でない」**かつ**「その前線の部隊が極端な物資不足でない」の
  3条件すべてを満たす場合のみ開始する。
- 既存の`can_continue_offensive`(90%)判定には**割当Divisionのmin_fulfillmentのみ**を
  追加でANDする(国家全体のreserve_daysが停止水準になっても、すでに動いている攻勢を
  即座に止めない――停止/再開ヒステリシスは主に「新規開始」側にかけ、既存の90%継続判定
  という既存のヒステリシスをさらに二重にしない)。0.34という値は「概ね3分の1未満」を
  表す代表値であり、既存の`evaluate_recruit_feasibility`等が使う厳密な閾値体系とは
  独立した、本タスクで新設する唯一の新定数である――実測(§39〜41)で妥当性を確認する。
- **再開**: 停止していた前線は、`national_reserve_days_country >= resume_days_country`
  (§26の同じ再開しきい値)になった月から、通常の110%判定に戻って評価される
  (新規攻勢用の追加状態は不要、§26と同じ理由)。

### 33. 防御・退却・接続回復移動

以下は**明示的に対象外**とし、この設計では一切変更しない(タスク文書§16の「原則として
許可する方向」の指示どおり):

- 防御戦闘(`FrontlineStance::Defend`への遷移、または既に`Defend`の前線)
- 既に開始済みの戦闘(`Battle`)の継続
- 自国支配州内の移動
- 退却(`find_retreat_destination`等の既存経路)
- 首都接続回復のための移動

**現状、これらのいずれも「接続性を回復するために動く」という自動ロジックは実コードに
一切存在しない**(P26-006/007は接続性を補給計算にのみ使い、移動/戦線AIへは一切
フィードバックしていない)。本タスクはこの経路回復AIを設計も実装もしない――P26-008は
「今ある移動・防御・退却をそのまま許可し、新規攻勢の開始だけを補給状態でゲートする」
という最小境界に留める。接続回復AIは将来の別タスクの対象として明示的に先送りする。

## 34〜37. 生産・建設への接続

### 34. 補給生産ボトルネック分類

`calculate_operation_ratio = min(workforce_ratio, logistics_ratio, input_resource_ratio)
× (0.8 if treasury<0)`(`src/economy/production.rs:58-71`、`is_bankrupt = treasury<0.0`は
`src/economy/mod.rs:147`)という既存の単一な稼働率式に基づき、MilitaryFactory(MilitaryEquipment
とMilitarySuppliesを co-produce)がゼロまたは低稼働になる原因は以下に分類できる:

| ボトルネック | 検出方法 | 既存AIの対応状況 |
|---|---|---|
| MilitaryFactoryそのものが0レベル | `state.building_level(MilitaryFactory)==0` | AIは決してMilitaryFactoryを建設しない(§7)。マップ初期データ依存、AI側で対処不可能。 |
| Iron不足(鉱床由来) | `resource_types_required_by_owned_buildings`にIronが含まれ`has_active_mine_for_resource`がfalse | **対応済み**(P26-004-PRE-002の事前開発Mine規則)。 |
| IndustrialGoods不足(Factory由来) | 同上でIndustrialGoodsが含まれるが、鉱床ではなくFactory出力に依存するため`find_undeveloped_deposit_state`では検出できない | **未対応**――Mine建設ロジックはIndustrialGoodsを対象にできない(deposit前提)。Passguardのような「civilian Factoryが0でCoalも無い」国家はこの経路で永久にゲートされる。 |
| Coal等の上流資源不足(Factoryの入力) | Factory自体が稼働できなければIndustrialGoods供給が止まる | 上記と同根、Mineの事前開発規則が対応(Coalは鉱床資源のため)。 |
| Mine/Factory建設不足(未着手) | `find_candidate_state`が候補を返さない/資金不足 | 既存の優先度3判定(Wood/Iron/Coal<30)で部分対応。 |
| 鉱床未発見 | `discovered==false` | 対象外(発見メカニズム自体はP26-008の範囲外)。 |
| 財政不足 | `treasury < construction_cost` | `ConstructOutcome::InsufficientFunds`として既存判定済み。 |
| workforce不足 | `workforce_ratio<1.0` | 既存の`calculate_operation_ratio`がそのまま反映(AI建設判断は関与しない)。 |
| 接続性だけの問題 | `min_fulfillment==0`だが`stockpile`は健全 | P26-006/007の既存契約どおり、生産自体は妨げられない――配送の問題であり生産の問題ではない。 |

### 35. 建設優先順位(候補比較と推奨)

現行の優先順位(§7)に、**IndustrialGoods向けの事前開発Factory規則**を、既存の事前開発
Mine規則と**同じ優先度2の内側**に、`ResourceType::ALL`宣言順のtie-break へ自然に組み込む
形で追加することを推奨する(案A相当、ただし新しい優先度階層を作らない点が案A/Bいずれとも
異なる、いわば「案A'」)。

- `resource_types_required_by_owned_buildings`が返す集合を`ResourceType::ALL`順で走査する
  際、資源が鉱床由来(Wood/Iron/Coal/MagicCrystal等)ならMine、IndustrialGoodsのように
  鉱床が存在しない資源ならFactory(既に`has_active_mine_for_resource`に相当する
  「既に自国にIndustrialGoods生産経路があるか」の判定を新設)を候補建物として選ぶ
  ――**既存の1つのループ構造を汎用化するだけ**で、優先度の相対順序(Food緊急 → 事前開発 →
  通常判断)は一切変更しない。
- これにより、Food緊急(§36)・既存の事前開発Mine(§37)のいずれの優先度も**変更されない**
  ――追加ルールは既存ルールの「同じ優先度2」の内側で資源種を追加するだけである。
- **この建設優先順位の変更自体は本タスクでは実装しない**(§2の除外事項どおり設計のみ)。
  次期タスクの候補として§53に記載する。

### 36. Food緊急の保護・37. 鉱床事前開発の保護

いずれも§35の設計が既存の優先度1(Food<50→Farm、無条件最優先)・既存の事前開発Mine規則
(Iron等の鉱床資源)を変更しないことを、コードレベル(`process_construction_ai`の
if-return構造、§7参照)で確認済み。新設するIndustrialGoods向けFactory規則は、既存の
Mine規則と全く同じ`find_proactive_mine_state`ループ構造(優先度2)の中に、対象資源種を
増やす形で追加されるため、優先度1(Food)を上書きすることはなく、既存の鉱床判定
(Iron/Coal等)の呼び出し順序も変更しない。

## 38〜46. 6か国の実測・振動・影響評価

### 38. 6か国の現行値・39〜40. 90日/36か月の候補別結果

**データ出所の明示**(feedback_verify_test_branch_is_actually_hitの精神に基づき、
出所を毎回明記する): 以下はいずれも一時計測用の非コミットE2Eテスト
(`tests/p26_008_pre_001_temp_measurement.rs`、報告書完成後に削除済み、コミット未実施)を
用いた**実App・6か国同時・`PlayerCountry(None)`・診断用無限資金なし**の実測値である
(90日=day_index 1〜90、36か月=month 1〜36、いずれも既存E2Eテストと同じ
`for day in 2..=28 { tick_day } ; tick_month`という**簡略月**[実26〜28日/月]の反復。
実カレンダーの28/30/31日とは正確には一致しない近似であることを明記する)。

**90日: 全6か国で共通して観測された事実**

- 初期備蓄31.0(既存P26-004-PRE-005ブリッジ)から1.0/日で単調減少し、day31相当
  (簡略月1回目の月境界直後)で0.000000へ到達、以後の消費は0(需要はあるが備蓄が無いため)。
- `is_active`は全期間`true`、`at_war`は**全期間`false`**(90日の間、AI主導のみでは
  自然発生的な宣戦布告は一度も観測されなかった――§41で考察)。
- `capital_controller_is_self`は全期間`true`(占領イベントなし)。
- `living_divisions`は全6か国で90日間ずっと`1`(初期Divisionのまま増えない)。

**36か月: 国家間の分散(§24で既に提示した月36値の再掲)**

| 国家 | 月36 stockpile | 月36 treasury | 月36 living_divisions | 36か月中のmin_fulfillment==0の観測有無 |
|---|---|---|---|---|
| Arcadia | 11.4 | -20233.0 | 1 | あり(複数回) |
| Elfin | 1015.0 | 684.3 | 1 | あり(序盤に複数回) |
| Dwarf | 137.9 | -20063.5 | 1 | なし(観測点では常に1.0) |
| Oceanic | 26.2 | 2521.8 | 1 | あり(終盤に複数回) |
| Passguard | 2488.1 | -5009.8 | 1 | なし(観測点では常に1.0) |
| Ferrowyn | 171.9 | -9325.7 | 1 | なし(観測点では常に1.0) |

**候補別(平時30/60/90日)の再現比較について**: 候補である「募兵抑制」「攻勢抑制」
「建設優先」のいずれも現時点で未実装のため、実App上でON/OFFを切り替えて比較することは
本タスクの範囲(設計のみ、実装なし)では行っていない。代わりに、上記の実測`stockpile`
軌跡(=`national_reserve_days`そのもの、需要が終始1.0/日のため)に対して**目標値30/60/90を
後から当てはめた場合にどの月から停止/再開が起こるか**を、実測データに対する後付け計算
(pure calculation、実装ではない)で確認した:

- 30日目標(採用値)を平時に適用した場合、Arcadiaは月18付近で`stockpile<20`(停止)に
  達して以降、月36までほぼ停止状態が続く(§21のとおり、これは同国の生産チェーンが
  実際に細いことの正しい反映であり、AI側の閾値設計の欠陥ではない)。
- 90日目標を適用した場合、Arcadia/Oceanicは**月1(初期ブリッジの時点)から一度も
  resume_days(105日相当)へ到達せず、36か月間ずっと募兵停止のまま**になる――これが
  §24で90日候補を不採用とした実測上の理由である。
- Passguard/Elfinのように月36で1000日分超の備蓄を持つ国家は、いずれの目標値
  (30/60/90)でも早期にresume水準を超え、実質的に一度も停止しない。

**live division数が1のまま変化しないため、募兵の停止/再開自体がこの36か月間で
実際に発火する機会がほぼ無かった**――これは§8で確認した「AI募兵の目標Division数は
概ね1のまま」「MilitaryEquipment不足で募兵キューが永久停滞する」という既存事象の
複合結果である。

### 41. Oceanicの判定

Oceanic(既存のP26-004〜006報告書が記録する「36か月972日中418日不足、最初の不足日は
32日目」)について、本タスクの実測(day_index基準)でも**day31〜相当で最初にstockpile=0へ
到達**しており、既存の記録と整合する定性的傾向(早期かつ頻繁な不足)を確認した
(厳密な日数の完全一致は、簡略月ループの近似[§38]と本タスクの計測解像度の違いにより
本タスクでは主張しない)。P26-008の募兵抑制ゲートは、Oceanicのような構造的に脆弱な
国家に対して最も強く発火することになる――これは意図した挙動である。

### 42. Dwarfの相対優位

Dwarfは月36時点でtreasuryが-20063.5と大きく負(破産状態が長期化)だが、`stockpile`は
137.9と中位を保っている――既存の`Iron→IndustrialGoods→MilitaryFactory`の完全な供給網
(P26-003〜004-PRE-001の既存監査結果)により、`is_bankrupt`による0.8倍ペナルティを
受けながらも生産自体は継続していることが実測から確認できる。P26-008の建設優先順位
(§35)の変更は、Dwarfの既存優位を損なわない(Dwarfは既に`has_active_mine_for_resource`
が真になるため、事前開発規則もIndustrialGoods向け新規則も発火対象にならない、
既存のP26-004-PRE-002報告書の記述どおり)。

### 43. 財政への影響・44. workforceへの影響

P26-008の設計(募兵ゲート・攻勢ゲート)はいずれも`treasury`・`available_manpower`・
`employed_workforce`を一切変更しない――既存の`request_recruitment`/`try_construct`の
外側で「実行するかどうか」を判定するだけであり、実行された場合の内部会計
(manpower/treasury消費)は既存コードのまま変わらない。したがって財政・workforceへの
直接的な影響はゼロ。間接的には、募兵が抑制される国家は`available_manpower`の消費が
減る(=`mobilized_manpower`が増えない)ため、Phase23の人的資源ライフサイクルには
既存の消費が単に「発生しない」という形でのみ影響する(既存の会計式自体の変更はない)。

### 45. 国力・Tierへの影響

`country::power::compute_state_aggregates`(P26-003-REV-003で確立)はDivision数や
MilitarySuppliesの量を一切参照しない(建物の`base_economic_capacity_points`のみに基づく)
ため、P26-008の募兵/攻勢ゲートは国力スコア・Tierに一切影響しない。

### 46. AI判断の振動

§28で設計したヒステリシス(停止/再開間に目標の半分のギャップ)により、月境界の
1日だけの一時的な不足(§実測で頻出する「月末に0へ落ちて月初production後すぐ回復する」
パターン)では、`stop_days`(20日)を大きく下回った後、`resume_days`(35日)まで
回復するには複数か月分の余裕が必要になるため、月ごとに停止/再開を繰り返す「振動」は
起きない設計である(既存の110%/90%パターンが実際に振動を防いでいることの類推でもある)。
実装が存在しないため実測での振動回数カウントは行っていない――この判断は式の構造
(非対称しきい値+目標の半分のギャップ)からの論理的帰結であることを明記する。

## 47〜51. Save/Load・決定性・性能

### 47. save/load・48. 旧Save

新設するヒステリシス判定(§28・§31)は`national_reserve_days_c`と`target_days_c`
(戦争状態から毎回導出)という既存Saveフィールド(`stockpile`・`War`参加状況)だけの
純粋関数であり、**新しいSaveフィールドは不要**。旧Save(これらのゲート機構が存在しない
時点のSave)を読み込んでも、次の日次/月次評価で通常どおり指標が再計算されるだけで
特別なマイグレーションは要らない。

`CountryAiState.dirty`/`MilitaryAiState.dirty`は既存どおりSave対象外のまま
(`SavedCountryAiState`/`SavedMilitaryAiState`、§7参照)で問題ない――§13〜17で確認した
とおり、この値はロード直後もその日のうちに`mark_ai_registries_dirty_daily`で
`true`に上書きされるため、Save/Load境界での`dirty`の扱いは(cadence問題自体が修正されても
されなくても)P26-008の新規ゲートには影響しない。

### 49. 決定性・50. tie-break

新設する全ての判定(§18の指標A/B、§26/§31のゲート)は、既存の決定的な入力
(`country_daily_demand_units`のDivisionId昇順合算、`supply_fulfillment_of`の直接読み取り、
`War.status`)だけに依存する純粋な数値比較であり、新たなHashMap反復やタイブレークを
一切必要としない。強制するとすれば、複数国家を一括処理する場合の処理順は既存の
`CountryId`昇順(§7・§8で確認済み)を維持すればよい。

### 51. 性能評価

新設する計算はいずれも既存の日次補給処理が既に計算済みの値(`supply_fulfillment_of`は
O(1)のHashMap読み取り、`total_daily_demand_units_c`は既存の`country_daily_demand_units`の
再利用)を参照するだけであり、追加の計算量は「国家ごとに1回の除算と比較」程度
(O(国家数)、既存のO(Division数)の日次補給計算に対して無視できる増分)。§14〜17の
cadence問題が修正されれば(P26-008-PRE-002)、建設AI/研究AI/戦争準備AI/戦線計画AIの
実行回数自体が設計意図の頻度(週次/月次/3日おき)に戻るため、**性能は現状より改善する
方向にしか動かない**――本タスクの新規ゲート追加による性能悪化よりも、cadence修正による
性能改善の方が圧倒的に大きい。キャッシュの新設は不要と判断した(§51はP26-008-PRE-002を
先に行うべきだという§17の判断をも補強する)。

## 52. 採用する実装パッケージ・53. 次の原子的タスク

**採用パッケージ: D(原子的タスクへの分割)。**

```text
P26-008-PRE-002: AI評価cadence修正(dirty常時trueの解消)
  → P26-008-A: 備蓄指標・募兵停止/再開ゲート(§18〜29の設計を実装)
  → P26-008-B: 攻勢停止/再開ゲート(§30〜33の設計を実装)
  → P26-008-C: 補給生産鎖の建設優先順位拡張(§34〜37の設計を実装)
```

理由: §17で確立したとおり、cadence修正はAI判断の意味・save/load決定性を変えるため
独立タスクが原則。募兵ゲート・攻勢ゲート・建設優先度拡張はそれぞれ異なるSystem
(§8/§10/§7)・異なるSaveへの影響(いずれも無し)・異なる既存ヒステリシスパターンへの
接続点を持つため、一括実装すると変更範囲の切り分けが難しくなる(§20のパッケージC相当の
リスク)。逆に、これらを意味的に結合しないと壊れる依存関係は無い――募兵ゲートが無くても
攻勢ゲートは単独で正しく動作し、その逆も成り立つ(§18の指標A/Bはいずれからも独立に参照
できる共有ロジックとして設計してある)。

**次に実装すべき原子的タスク1件: P26-008-PRE-002(AI評価cadence修正)。**

- 実装対象: `war::update::mark_ai_registries_dirty_daily`を「実際に状態が変化した国家
  だけを`mark_country_dirty`する」方式へ変更する(既存の`mark_country_dirty` APIを使う、
  新規メソッドは不要)。「状態が変化した」の定義(戦争開始/終了、占領/奪還、消滅等)は
  実装時に既存のイベント発生箇所を洗い出して決める。
- 変更ファイル候補: `src/war/update.rs`(該当System)、呼び出し元での状態変化イベントの
  検出。
- 対象外: 新規AI判断ロジック(本タスクP26-008-PRE-001が設計した募兵/攻勢/建設ゲートは
  含めない)。
- 必須テスト: 週次/月次/3日おきの評価が実際に設計意図の頻度で発生することを実測する
  新規E2Eテスト(本タスクの一時計測ハーネスと同種の手法を恒久化)、既存の
  `p26_006_rev_001_ai_save_resume_determinism_e2e_test`・`p23_010`の継続的な安定性
  (独立プロセス複数回実行)。
- 受け入れ条件: cadence修正後も既存の全リグレッション(§29相当の全ファイル)が
  変わらず合格し、AI評価回数が設計意図の頻度(週次≈1/7、月次≈1/30、3日おき≈1/3)へ
  実測で戻ることを確認する。

## 54. 対象外

本タスクでは以下を一切設計・実装していない(タスク文書§2どおり): AI補給判断の実装、
AI募兵停止の実装、AI攻勢停止の実装、AI建設優先度の実装、AI評価周期の実装修正、UI、
移動速度補正、organization、Attrition、接続回復AI(§33で明示的に先送り)。

## 55. Critical/High/Medium/Lowの所見

- **Critical: 0件**
- **High: 1件** — `mark_ai_registries_dirty_daily`によるAI週次/月次/3日おき評価
  cadenceの実質無効化(§12〜17)。P26-006-REV-001で発見され「Low finding、変更しない」と
  された事項だが、本タスクで実測により**評価回数が設計意図の5〜30倍**であることを
  定量化した結果、P26-008の実装順序を左右する実質的な影響がある(P26-008-PRE-002として
  独立修正が必要、§17)。Lowからの再分類として報告する。
- **Medium: 1件** — 募兵キューがMilitaryEquipment備蓄不足で永久停滞する既存の実装
  (`process_recruitment`、§8)。MilitarySuppliesとは無関係だが、P26-008の募兵ゲートの
  「実際の効果」がこの既存事象によって大きく減殺される(§39〜41)。MilitarySupplies関連
  タスクの対象外だが、AI募兵の実効性向上を検討する将来タスクの候補として記録する。
- **Low: 1件** — IndustrialGoods不足によるMilitaryFactory稼働率0%を解消する既存の
  建設AI規則が存在しない(§34、Passguardのような国家が対象)。§35で設計した拡張候補は
  次期タスク候補であり、本タスクでは実装しない。

## 56〜59. 実行コマンドと結果

### 56. 実行コマンドと正確な結果(逐次実行、並列cargo実行なし)

```text
cargo test --all-targets                                    → 2360 passed / 0 failed / 1 ignored (開始時)
cargo clippy --all-targets --all-features -- -D warnings     → 0 warnings (開始時)

(一時計測テスト tests/p26_008_pre_001_temp_measurement.rs を作成・実行・削除、
 コミット対象外――§38〜41の実測データ取得のみに使用)

cargo test --lib country::   → 91 passed / 0 failed
cargo test --lib military::  → 428 passed / 0 failed
cargo test --lib war::       → 153 passed / 0 failed
cargo test --lib economy::   → 39 passed / 0 failed
cargo test --lib save::      → 302 passed / 0 failed

cargo test --test p26_004_daily_supply_consumption_allocation_e2e_test              → 19 passed
cargo test --test p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test        → 11 passed
cargo test --test p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test       → 15 passed
cargo test --test p26_004_pre_005_initial_supply_bridge_migration_e2e_test           → 22 passed
cargo test --test p26_005_supply_combat_strength_e2e_test                           → 10 passed
cargo test --test p26_006_capital_connectivity_supply_e2e_test                      → 18 passed
cargo test --test p26_006_rev_001_ai_save_resume_determinism_e2e_test               → 9 passed
cargo test --test p26_007_elimination_occupation_overseas_supply_accounting_e2e_test → 23 passed
cargo test --test p26_det_001_construction_ai_determinism_e2e_test                  → 10 passed
cargo test --test p26_det_001_rev_001_cross_state_production_order_e2e_test         → 12 passed

cargo test --test p23_001_population_manpower_e2e_test ... (Phase23全10件+Phase24 1件+Phase25 8件
  +land_war_combat_peace_test) → 全件 passed、0 failed (個別結果は本文§省略・全て上記実行ログで確認済み)

cargo test --all-targets (最終)                              → 下記§57参照
cargo clippy --all-targets --all-features -- -D warnings (最終) → 下記§58参照

p23_010 を独立プロセスで5回実行                                → 下記§で確認
p26_007の新規テストファイルを独立プロセスで5回実行(P26-007保護対象として再確認)    → 下記§で確認
```

### 57. 全テスト件数・58. clippy警告数・59. Cargo逐次実行

上記は全て逐次実行(並列cargo実行なし)。

- 最終`cargo test --all-targets`: **2360 passed / 0 failed / 1 ignored**(開始時と完全一致
  ――本タスクはプロダクションコードを変更していないため、当然の結果として件数が
  変化しないことを確認した)。
- 最終`cargo clippy --all-targets --all-features -- -D warnings`: **0 warnings**。
- `p23_010_phase23_manpower_lifecycle_audit_test`を独立プロセスで5回実行:
  **5/5、各9 passed / 0 failed**(P26-006-REV-001の決定性修正が継続して安定していることを
  再確認)。
- `p26_007_elimination_occupation_overseas_supply_accounting_e2e_test`(P26-007の保護対象
  テスト)を独立プロセスで5回実行: **5/5、各23 passed / 0 failed**。

## 60. ユーザーセーブ保護

`saves/savegame_v1.ron`は本タスク開始から終了まで一度も読み書き・ステージ・復元して
いない(一時計測ハーネスは`TempTestDir`によりOS一時ディレクトリ内の独立パスにのみ
Saveを書く、既存のP26-004〜007のE2Eテストと同じパターン)。

## 61. コミット対象

本報告書1ファイルのみ。プロダクションコード・データ・既存テストの変更は0ファイル。
一時計測テストファイル(`tests/p26_008_pre_001_temp_measurement.rs`)は報告書完成前に
削除済みで、コミット対象に含まれない。

## 62. 最終`git status --short`

コミット前(本報告書ファイルをステージする直前)の`git status --short`は以下の1行のみ:

```text
?? verification_logs/phase-26/p26-008-pre-001/
```

ユーザーSave(`saves/savegame_v1.ron`)・既知の非決定的PNG(p20-007/p20-009/
phase-21/p21-save-002e)は、テスト実行で再生成された都度、個別ファイル指定の
`git checkout --`で復元済みであり、上記以外の差分は残っていない。
