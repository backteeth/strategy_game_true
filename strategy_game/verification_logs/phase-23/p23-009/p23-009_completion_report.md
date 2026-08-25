# P23-009 完了報告: AI国家の月次自動募兵

## 1. 総合判定

**COMPLETE**

新しいUIは追加しておらず、既存UI表示は既存の更新経路がそのまま反映するため、UI未確認を
理由に判定を落とす条件には該当しない。自動テスト・検証コマンドはすべて成功した。

## 2. 基準状態

* 開始時HEAD: `6f35ba133db8cd22feb45de7d91373a01f5509ea`(P23-008完了報告書コミット)
* 実装コミットの完全なハッシュ: `69c6093255c649214ad4d05360e4de634a518a9c`
  (`feat: add monthly AI recruitment`)
* 報告書コミットの完全なハッシュ: 本報告コミット後に確定するため、このコミット自身の
  ハッシュはここへ自己参照的に記載しない(コミット後の最終回答で報告する)
* 最終HEAD: 報告書コミット後に確定
* `saves/savegame_v1.ron`はユーザーの既存未コミット変更(実プレイデータ)であり、
  本タスクを通じて一切参照・変更・ステージ・コミットしていない。

事前確認: `military::recruitment::request_recruitment`/`cancel_recruitment`/
`process_recruitment`・`RecruitmentQueueItem`・`DivisionDefinition`・New Game
初期Infantry Divisionが使用するDefinition ID(`app::loader::spawn_debug_divisions`)・
プレイヤーUIが使用するDefinition ID(`ui::military_panel::RECRUIT_DIVISION_ID`)・
`CountryData.available_manpower`/`mobilized_manpower`/`recruitment_queue`・
treasuryと`required_equipment`の既存の意味(装備費という名目でtreasuryから直接
差し引かれる、既存の経済モデルそのままであることを確認)・
`population::growth::compute_population_by_owner`/`compute_manpower_cap`・
`StateData::controller()`・国家の首都StateId・`PlayerCountry`Resource・AI Systemの
既存登録方法(`DailySimulationSet`、既存の`country::country_ai`/
`diplomacy::ai_claim_generation`の月次AI判断パターン)・`MonthChangedMessage`・
`DailySimulationSet`の全体順序(`Economy`→…→`CountryAi`→…→`UiUpdate`、
`configure_sets(...).chain()`で強制)・CountrySelection/Playing/Loaded Gameの
遷移・save/load時のイベントリセット・国家消滅時の募兵キュー処理
(`country::elimination`が消滅時に`recruitment_queue.clear()`する既存経路)を、
現在のHEADで再確認した。

重要な発見: New Game初期配置(`app::loader::spawn_debug_divisions`)とプレイヤーUI
募兵(`ui::military_panel::RECRUIT_DIVISION_ID`)は、どちらも`DivisionDefinitionId(0)`
を使っていたが、**共有された正規定数は存在せず、別々の場所でそれぞれハードコード**
されていた。本タスクの要求(§2「同じInfantry IDをAIコードへ別の文字列として重複
ハードコードしないでください」「存在しない場合は…正規ヘルパーまたは定数へ集約して
ください」)に従い、`military::data::STANDARD_INFANTRY_DEFINITION_ID`を新設し、
New Game初期配置・プレイヤーUI・AI月次募兵の3箇所すべてがこの1つの定数を参照する
よう統合した。また、`military::ai`モジュール自体が`process_military_ai`という
未使用(どこからも呼ばれていない)の空スタブとして既に予約されていたため、新規ファイル
を作らず、その場所へ本タスクの実装を配置した。プロンプトの前提と実コードに重大な
食い違いはなかった。

## 3. 実装内容

### 3.1 対象国家

`is_active()==true`かつ`PlayerCountry`と一致しないプレイヤー以外の国家を対象とする。
プレイヤーIDのハードコードはしておらず、`Res<PlayerCountry>`から取得した値と比較する。
消滅国家・プレイヤー国家は`evaluate_ai_monthly_recruitment`の候補集合構築時点で除外
される。

### 3.2 月次頻度

`MonthChangedMessage`駆動。`diplomacy::ai_claim_generation::handle_monthly_ai_
claim_generation`と同じ`evaluated_this_frame`フラグにより、同一フレーム内に複数の
`MonthChangedMessage`が届いても2回以上処理しない。

### 3.3 Infantry Definition

`military::data::STANDARD_INFANTRY_DEFINITION_ID`(新設、§2参照)のみを対象とする。
存在しない場合はその月の全AI国家分をまとめてスキップする(`SkippedMissingDefinition`)。

### 3.4 目標Division数

`calculate_ai_target_divisions(cap, required)`: `cap<required`または`required==0`
なら0。それ以外は`floor(cap/2)/required`(先にcapを2で割ることで`2*required`の
overflowを経由しない)。結果が0でも`cap>=required`なら最低1。7つの指定例すべてを
単体テストで固定した。

### 3.5 現在Division数

`count_living_divisions_by_owner`が`MilitaryRegistry`を1回走査し、owner別の生存
Division数(`manpower>0 && status!=Destroyed`)を集計する。これに
`recruitment_queue.len()`を加えたものを現在数とする。Army数は使用しない。

### 3.6 募兵キュー制限

`recruitment_queue.is_empty()`を必須条件とし、満たさない場合は
`SkippedQueueNotEmpty`。

### 3.7 人的資源予備25%

`calculate_ai_manpower_reserve(cap)`: `ceil(cap/4)`(整数除算と剰余のみ)。
`available_manpower >= required + reserve`(`checked_add`でoverflow検証)を満たさ
なければ`SkippedInsufficientManpower`。

### 3.8 資金条件

AI側は`treasury < def.required_equipment`を事前確認するのみで、新しい資金予備率は
追加していない。最終判定は必ず`request_recruitment`自身の検証に委ねる。

### 3.9 首都・代替州選択

`select_ai_recruitment_state`: 首都が自国所有かつ`StateData::controller()`が
自国ならそれを使用。使えない場合は自国所有・自国支配の州のうちpopulation最大
(同率ならStateId最小)を選ぶ。対象州が存在しなければ`SkippedNoControlledState`。
`StateRegistry.states`(`Vec`)を1回走査するのみで、`HashMap`走査順には依存しない。

### 3.10 月次System順序

`handle_monthly_ai_recruitment`を`MilitaryPlugin`から`DailySimulationSet::
CountryAi`へ登録した。`DailySimulationSet`の宣言順(`Economy`→`Research`→
`Diplomacy`→`CountryAi`→…→`UiUpdate`、`app::time`の`configure_sets(...).chain()`
が強制)により、月次人口増加・人的資源回復・雇用処理(いずれも`Economy`)が確定した
後、UI表示(`UiUpdate`)より前に実行されることが保証される。

### 3.11 決定性

対象国家は`CountryId`昇順で処理する(`evaluate_ai_monthly_recruitment`が
`sort_by_key`で明示的にソートしたVecを走査する)。乱数は一切使用しない。

### 3.12 `request_recruitment`の再利用

`available_manpower`・`mobilized_manpower`・`treasury`・`recruitment_queue`・
Division生成のいずれも、AIコード内で直接変更しない。すべて既存の
`request_recruitment`呼び出しに委ねる。

### 3.13 人的資源帳簿

募兵成功時は`request_recruitment`の既存動作どおり
`available -= required; mobilized_manpower(B) += required; treasury -= 
required_equipment`となり、募兵キュー由来の`L`が`required`分増える。`K`は不変。
`compute_manpower_ledger`で`B=L+K`・`unaccounted_living_excess=0`が維持される
ことを単体テストで確認した。

### 3.14 セーブV1を維持した方法

新しい保存フィールド・Resource・Componentは追加していない。AI判断のクールダウンは
保存せず、`MonthChangedMessage`駆動の毎月再評価だけで「月1回」を実現する。E2Eテスト
で、ロード直後には募兵されず、ロード後の次の正規`MonthChangedMessage`から判断が
再開することを確認した。

## 4. 変更ファイル一覧と役割

| ファイル | 役割 |
|---|---|
| `src/military/ai.rs`(変更、旧`process_military_ai`スタブを置換) | AI月次募兵の正規Domain API群、`AiRecruitmentOutcome`、月次System、Domain単体テスト28件 |
| `src/military/data.rs`(変更) | `STANDARD_INFANTRY_DEFINITION_ID`の新設 |
| `src/military/mod.rs`(変更) | `handle_monthly_ai_recruitment`を`DailySimulationSet::CountryAi`へ登録 |
| `src/app/loader.rs`(変更) | `spawn_debug_divisions`が新設の共有定数を参照するよう変更(値は変わらず`DivisionDefinitionId(0)`のまま) |
| `src/ui/military_panel.rs`(変更) | `RECRUIT_DIVISION_ID`が新設の共有定数を参照するよう変更(同上) |
| `tests/p23_002_mobilized_manpower_workforce_e2e_test.rs`(変更) | AI月次募兵導入による回帰1件の修正(§9参照) |
| `tests/p23_009_ai_recruitment_e2e_test.rs`(新規) | 実経路統合テスト22件 |
| 本完了報告書(新規、別コミット) | — |

## 5. 追加・更新したテスト

### 5.1 純粋関数(`src/military/ai.rs`、4件)

目標Division数の7例と`u64::MAX`境界(2件)、人的資源予備の5例と`u64::MAX`境界
(2件、まとめて2テストに整理)。目標計算関数のシグネチャ自体が`(cap, required_
manpower)`のみを受け取ることで、現在Division数・treasury・戦争状態を混ぜないこと
を型で保証している。

### 5.2 募兵州選択(同ファイル、8件)

首都優先・首都敵支配時の代替州・首都State不在でもpanicしない・代替州のpopulation
最大選択・同人口時のStateId最小・敵/他国/同盟国相当支配州の除外・募兵可能州なしで
`None`・挿入順非依存を確認した。

### 5.3 AI国家単位判断(同ファイル、16件)

目標未満での募兵・目標到達での非募兵・目標超過での非復員・キュー非空での非募兵・
人的資源予備境界(割込む場合/ちょうどの場合)・資金不足・消滅国家・帳簿計算不能
(未解決定義入りキューは`SkippedQueueNotEmpty`が先に成立し`SkippedLedgerInvalid`
にAI経路単独では到達しないことを判明・文書化)・`L>B`・Definition不明・募兵州なし・
成功時のavailable/B/treasury/queue変化・K不変・1国家の失敗が他国を妨げないこと・
プレイヤー国家が対象外であることを確認した。

### 5.4 月次System実経路(`tests/p23_009_ai_recruitment_e2e_test.rs`、14件)

各AI国家が月1件だけ判断・`MonthChanged`なしでは募兵しない・同一フレーム内の重複
イベント非処理・プレイヤー非対象・月次人的資源回復後のavailableを使用・同月の雇用
計算を先行実行しない・翌月の民間労働力反映・既存日次経路での完成(ownerとtarget
stateが正しい、完成時に人的資源を二重消費しない)・損耗のみでは重複募兵しない
(P23-008補充が担当)・New Game直後は募兵しない・Loaded Game直後は募兵しない・
長期間でもqueueが1件を超えない・目標到達後は追加募兵しないことを確認した。

### 5.5 Save/Load(同ファイル、3件)

AI募兵途中の国家の保存・ロード一致(available/B/帳簿の再導出含む)、ロード後の
queue保持中は重複募兵しないこと、完成後まだ目標未満なら翌月に次の1件を募兵する
ことを確認した。

### 5.6 回帰(同ファイル、5件)

プレイヤーの手動募兵(既存`request_recruitment`経路)が引き続き動作すること、消滅
国家がAI募兵しないこと、AI募兵成功後もP23-006帳簿(`B=L+K`)が複数フレーム後も
一貫すること、AI募兵がGameNotificationを追加発行しないこと(対照App比較)、
AI募兵後も軍事パネルが未翻訳キーを露出せずpanicしないことを確認した。募兵キャン
セル・P23-007復員・P23-008自動補充は§6のとおり既存テストファイルの再実行でも
回帰していないことを確認済み。

## 6. 実行した全コマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | PASS |
| `cargo test --lib military::` | PASS、177 passed |
| `cargo test --lib population::` | PASS、46 passed |
| `cargo test --lib save::` | PASS、300 passed |
| `cargo test --lib ui::military_panel::` | PASS、54 passed |
| `cargo test --test p23_001_population_manpower_e2e_test` | PASS、3 passed |
| `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test` | 初回1件FAILED(§9)→修正後PASS、8 passed |
| `cargo test --test p23_003_population_manpower_ui_e2e_test` | PASS、8 passed |
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test` | PASS、7 passed |
| `cargo test --test p23_006_manpower_ledger_e2e_test` | PASS、22 passed |
| `cargo test --test p23_006_fix_001_initial_manpower_ledger_e2e_test` | PASS、19 passed |
| `cargo test --test p23_007_division_demobilization_e2e_test` | PASS、24 passed |
| `cargo test --test p23_008_daily_reinforcement_e2e_test` | PASS、21 passed |
| `cargo test --test p23_009_ai_recruitment_e2e_test` | PASS、**22 passed** |
| `cargo clippy --all-targets --all-features -- -D warnings` | 初回2件(`drop_non_drop`・`identity_op`)→修正後PASS、**警告0** |
| `rustfmt --edition 2024 --check`(変更・新規ファイル個別指定) | 初回に整形差分あり→`rustfmt --edition 2024`適用後PASS(いずれも非`mod.rs`のリーフファイル。`src/military/mod.rs`は追記のみで整形は実行していない) |
| `cargo test --all-targets`(全体、最終実行) | PASS、**全件PASS**(**1575 passed**、**0 failed**、1 ignored[既存の`profile_p21_021_elimination_scaling_test`、本タスク無関係]) |

1575 = 直前の基準1525 + Domain単体テスト新規28件 + E2E新規22件、と正確に一致する
ことを確認した。`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない。
既知の非決定的PNG9枚は`cargo test --all-targets`実行後に想定どおり再生成され、
個別ファイル単位で`git checkout --`により復元した(ディレクトリ単位の復元はしていない)。

## 7. 対象外を先行実装していないこと

プレイヤー国家の自動募兵、AIによる同月複数募兵、複数募兵キュー、Infantry以外の
兵種選択、戦争・脅威・国境・敵戦力を考慮した目標補正、国力区分による目標補正、
AIによる復員・募兵キャンセル・補充優先度、装備備蓄・補給を考慮した募兵、AI専用の
人的資源・資金割引、同盟国領・敵地での募兵、即時Division spawn、初期Division数
変更、戦闘式・補充式の変更、population減少、`available + mobilized <= cap`の
強制、人的資源上限・回復式の変更、UI・ローカライズ変更、新規セーブフィールド、
セーブV2、P23-010以降の機能は、いずれも変更ファイル一覧(§4)に含まれておらず実装
していないことを確認した。

## 8. 残課題または後続候補

* 目標50%・予備25%・月1件という定数は「平時・戦時共通の保守的な軍備水準」という
  意図のみで、戦争状態・国力区分・イデオロギー・政治体制による補正は一切ない。
  将来これらを反映したい場合は別タスクとして目標式自体を再設計する必要がある。
* AIによる復員・補充優先度・Infantry以外の兵種選択は依然として未実装。
* P23-005〜008が既に指摘した残課題(Attrition、population↔恒久軍事損失の統合、
  `available + mobilized <= cap`の不変条件)は引き続き未着手。
* `military::ai::AiRecruitmentOutcome::SkippedCountryNotFound`は、
  `evaluate_ai_monthly_recruitment`(唯一の呼び出し経路)が`CountryRegistry`から
  直接構築した`CountryId`だけを渡すため、実際には到達しない防御的なバリアントで
  ある(P23-007/008の同種の防御的コードと同じ位置づけ)。

## 9. 実装コミット直後と報告書コミット後の`git status --short`

### 実装コミット(`69c6093255c649214ad4d05360e4de634a518a9c`)直後

```
 M saves/savegame_v1.ron
```

`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、実装コミットには
含めていない。それ以外の全変更(§4の実装・テスト・回帰修正ファイル)はコミット済み。

回帰修正の詳細: `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test`
の`one_countrys_mobilization_does_not_affect_another_countrys_employment`が
実装後に1件FAILEDした。原因は、このテストが検証対象国家(`CountryId(1)`)の
`available_manpower`を制御していなかったため、`MilitaryPlugin`へ新たに追加された
AI月次募兵が同国家で独立に発生し、そのAI自身の募兵による`mobilized_manpower`
増加(=プレイヤーの動員とは無関係の正当な要因)が、このテストが検証したかった
「プレイヤーの動員が他国へ波及しない」という主張と混線したことによる。テストの
assertを弱めるのではなく、対象国家の`available_manpower`を0にしてAI募兵の人的
資源予備条件(§3.7)を必ず満たせないようにし、テストの対象をプレイヤーの動員だけへ
明示的に限定する形で修正した(修正後は元の主張を保ったままPASSする)。

### 報告書コミット後

報告書コミット作成後の最終`git status --short`は、コミット完了後の最終回答で報告する。
