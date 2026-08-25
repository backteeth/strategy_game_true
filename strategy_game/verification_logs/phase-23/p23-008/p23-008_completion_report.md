# P23-008 完了報告: 日次自動補充と人的資源帳簿への接続

## 1. 総合判定

**COMPLETE**

新しいUI操作は追加しておらず、既存UIの表示は既存の更新経路がそのまま反映するため、
実ウィンドウ未確認を理由に判定を落とす条件(§9 UI変更が必要になった場合)に該当しない。
自動テスト・検証コマンドはすべて成功した。

## 2. 基準状態

* 開始時HEAD: `fb8492ba5b12746caee6002fb4e300bbb78a5548`(P23-007完了報告書コミット)
* 実装コミットの完全なハッシュ: `d71a97d5c4f9b53c05d33159e3dbb94aacf7b0d2`
  (`feat: add daily manpower reinforcement`)
* 報告書コミットの完全なハッシュ: 本報告コミット後に確定するため、このコミット自身の
  ハッシュはここへ自己参照的に記載しない(コミット後の最終回答で報告する)
* 最終HEAD: 報告書コミット後に確定
* `saves/savegame_v1.ron`はユーザーの既存未コミット変更(実プレイデータ)であり、
  本タスクを通じて一切参照・変更・ステージ・コミットしていない。

事前確認: `military::update::handle_daily_military`(募兵→検証→移動→戦闘→終了処理→
クリーンアップ→組織率回復の順の単一Systemであることを確認)・`process_recruitment`・
`process_daily_battles`/`resolve_finished_battles`・`Division.manpower`/
`max_manpower`/`status`・Divisionの現在州・owner・`StateData::controller()`
(法的所有国へフォールバックする既存API、`process_org_recovery`が既に同じ式で
自国支配地域判定に使用)・`BattleRegistry`・`Division.combat_id`・
`CountryData.available_manpower`/`mobilized_manpower`・`compute_manpower_ledger`・
P23-007の`demobilize_division`・月次民間労働力更新
(`population::mobilization::apportion_mobilized_manpower`、最大剰余法の既存実装)・
save/load V1・既存のDivisionId順序・決定性規約を、現在のHEADで再確認した。
プロンプトの前提と実コードに重大な食い違いはなかった。

## 3. 実装内容

### 3.1 日次補充上限

`ceil(max_manpower/100)`を`max_manpower/100 + (max_manpower%100!=0 ? 1 : 0)`
(整数除算と剰余のみ、加算オーバーフローなし)で計算する
(`military::reinforcement::calculate_daily_reinforcement_cap`)。

### 3.2 補充対象status

`DivisionStatus::Idle`・`Occupying`のみを対象とし、`Fighting`・`Moving`・
`Retreating`・`Destroyed`・`Disbanding`は対象外とした。さらに、`combat_id`が
`Some`の場合、および`BattleRegistry`の進行中戦闘に参加者として含まれている場合は、
statusだけを見て許可せず対象外とする(`military::demobilization::
demobilize_division`と同じ防御的検証)。`manpower>0`かつ`max_manpower>manpower`
も必須条件とし、manpower 0でレジストリに残るDivisionは復活させない。

### 3.3 支配地域条件

`StateData::controller()`(占領による`controller_country`があればそれ、なければ
`owner_country_id`にフォールバックする既存API、`process_org_recovery`と同一の式)が
Division ownerと一致する場合のみ補充可能とした。「支配国不明」という状態は
`controller()`の設計上存在しない(常に具体的な国家へ解決される)ため、実際に
検証したのは「敵国支配」「第三国(同盟国相当)支配」の2パターンである。

### 3.4 日次処理内の実行順序

`handle_daily_military`内で、既存の「1. 募兵キュー進行」の直後、「3. 経路の
事前検証」「4. 移動処理(侵攻・戦闘開始を含む)」より前に「2. 日次自動補充」を
挿入した。これにより、その日発生する戦闘の損耗は同日中に補充されない
(補充は前日までの状態のみを対象とする)。日次開始時に補充条件を満たしていた
Divisionがその後同日中に戦闘へ入る場合は、戦闘開始前の補充を受けられる仕様どおり
動作する(E2Eテストで確認)。

### 3.5 国家単位予算

対象Divisionの要求量合計(`u128`)、`available_manpower`、
`u64::MAX - mobilized_manpower`(残り負担容量)の最小値を予算とした。

### 3.6 最大剰余法

`population::mobilization::apportion_mobilized_manpower`と同じ設計
(`u128`中間計算、剰余降順→`DivisionId`昇順のタイブレーク)で
`military::reinforcement::apportion_reinforcements`を実装した。各配分が要求量を
超えないことは、`budget<=total_request`の下で`base_i<=r_i`が数学的に保証され
(`budget<total_request`かつ`r_i>0`なら`base_i<=r_i-1`と真に小さい)、剰余の+1でも
`r_i`を超えないことをコメントと単体テストの両方で示した。

### 3.7 `available`・`B`・`L`・`K`の遷移

補充量`X`につき`available_manpower -= X`、`mobilized_manpower(B) += X`、
`Division.manpower(L) += X`。`K`は保存フィールドではなく引き続き
`compute_manpower_ledger`が`B.saturating_sub(L)`として導出するため、`B`と`L`が
同数増える補充では自然に不変となる。

### 3.8 帳簿不整合時の扱い

国家単位で処理する前に`compute_manpower_ledger`を評価し、`Err`
(未解決の募兵定義)または`unaccounted_living_excess>0`(`L>B`)の場合は、その国家の
当日の補充を全件スキップする。一部Divisionだけ補充するような部分適用は行わない。
毎日同じ理由でスキップされてもプレイヤー通知は発行せず、`bevy::log::debug!`
(既定のログレベルでは出力されない)へのみ記録する。

### 3.9 民間労働力への反映時期

補充処理自体は`StateData`の雇用フィールドを一切変更しない。`mobilized_manpower`
(B)が増えても、次の正規月次処理(`MonthChangedMessage`駆動)が新しいBを州へ
人口比按分するまでは民間労働力へ反映されない。E2Eテストで、補充直後の同一フレームで
雇用フィールドが不変であること、次月の更新で民間労働力が減少すること、他国へ
波及しないこと、恒久軍事損失`K`が補充の前後で不変であることを確認した。

### 3.10 セーブV1を維持した方法

新しい保存フィールド・Resource・Componentは追加していない。補充は
`CountryData`の既存2フィールドと`Division.manpower`の変更だけで完結するため、
既存のV1シリアライズ形式がそのまま補充後の状態を保存・復元できる。日次補充速度・
補充要求そのものは保存しない(`calculate_daily_reinforcement_cap`等の純粋関数から
毎回再計算される)。E2Eテストで、ロード直後には補充が実行されず、ロード後の
最初の正規`DayChanged`で1日分だけ補充されることを確認した。

## 4. 変更ファイル一覧と役割

| ファイル | 役割 |
|---|---|
| `src/military/reinforcement.rs`(新規) | 正規Domain API群(`calculate_daily_reinforcement_cap`/`calculate_reinforcement_request`/`apportion_reinforcements`/`apply_daily_reinforcement`)、Domain単体テスト40件 |
| `src/military/mod.rs`(変更) | `pub mod reinforcement;`の追加のみ |
| `src/military/update.rs`(変更) | `handle_daily_military`内、募兵処理の直後・移動処理より前に`apply_daily_reinforcement`呼び出しを挿入 |
| `tests/p23_008_daily_reinforcement_e2e_test.rs`(新規) | 実経路統合テスト21件(日次System・実New Game・民間労働力・save/load・UI回帰) |
| 本完了報告書(新規、別コミット) | — |

## 5. 追加・更新したテスト

### 5.1 純粋関数(`src/military/reinforcement.rs`、16件)

日次上限2件(境界値・`u64::MAX`)、要求量5件、最大剰余法9件(満額配分・予算0・
要求0除外・合計一致・要求超過なし・比例配分・同率端数のDivisionId順・挿入順非依存・
`u64::MAX`近傍のoverflow安全性)を固定した。

### 5.2 補充対象条件(同ファイル、10件)

Idle/Occupying(自国支配)の対象化、Occupyingでも敵国支配中なら対象外、
Fighting/Moving/Retreating/Destroyed/Disbandingの対象外、manpower 0・満員
Divisionの対象外、`combat_id`・進行中Battle参加者の対象外、存在しない州の対象外、
第三国(同盟国相当)支配の対象外を確認した。

### 5.3 国家単位補充(同ファイル、14件)

available十分時の全対象日次上限までの補充、available不足時の比例配分、
available/B/Lそれぞれが補充合計分だけ変化しKは不変、Division.manpowerがmaxを
超えない、available枯渇時のunderflow防止、負担容量(`u64::MAX`近傍)による予算
制限、帳簿計算不能・`L>B`国家の全件スキップ、ある国家の失敗が他国を妨げないこと、
treasury/装備/organization/experience/populationを変更しないこと、HashMap挿入順
非依存を確認した。

### 5.4 日次System実経路(`tests/p23_008_daily_reinforcement_e2e_test.rs`、10件)

`DayChanged`で1回だけ実行・`DayChanged`なしでは実行しない・消費済みイベントの
再処理なし・その日の戦闘より前に補充が行われ同日の戦闘損失は同日中に補充されない
こと・戦闘後翌日からの補充再開・複数日での1%ずつの回復・shortage未満の最終日の
過不足ない補充・available枯渇での停止と再開・他国非波及・P23-007復員後の帳簿一致
を確認した。

### 5.5 戦闘順序・実New Game統合(同ファイル、2件)

実データ(6か国)でNew Game開始直後の満員初期Divisionが補充対象外であること、
AI国家(プレイヤー以外)にも同じDomain経路が適用されることを確認した。

### 5.6 民間労働力・経済(同ファイル、4件)

補充直後の雇用フィールド不変、次月の民間労働力減少、他国非波及、恒久軍事損失`K`が
補充前後で不変であることを確認した。

### 5.7 P23-007復員回帰

日次実経路の1件(`reinforcement_after_demobilization_still_yields_a_consistent_
ledger`、§5.4に計上)とsave/loadの1件(`reinforcement_then_demobilization_round_
trips_through_save_and_load`、§5.8に計上)で、補充後のDivisionをP23-007の
`demobilize_division`で復員しても帳簿(B/L/K)が一致することを確認した。

### 5.8 Save/Load(同ファイル、3件)

補充途中のDivisionの保存・ロード一致、ロード単体では補充されずロード後の最初の
正規`DayChanged`で1日分だけ補充されること、補充→復員→保存・ロードの帳簿一致を
確認した。

### 5.9 UI回帰(同ファイル、2件)

新しいUIを追加していないことを前提に、日次補充後に既存の軍事パネル表示
(available/living/burden/Division現在manpower)が更新されること、表示Entityが
複数回の日次更新でも重複spawnされないことを確認した。

## 6. 実行した全コマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | PASS |
| `cargo test --lib military::` | PASS、149 passed |
| `cargo test --lib population::` | PASS、46 passed |
| `cargo test --lib save::` | PASS、300 passed |
| `cargo test --test p23_001_population_manpower_e2e_test` | PASS、3 passed |
| `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test` | PASS、8 passed |
| `cargo test --test p23_003_population_manpower_ui_e2e_test` | PASS、8 passed |
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test` | PASS、7 passed |
| `cargo test --test p23_006_manpower_ledger_e2e_test` | PASS、22 passed |
| `cargo test --test p23_006_fix_001_initial_manpower_ledger_e2e_test` | PASS、19 passed |
| `cargo test --test p23_007_division_demobilization_e2e_test` | PASS、24 passed |
| `cargo test --test p23_008_daily_reinforcement_e2e_test` | PASS、**21 passed** |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、**警告0**(`clippy::manual_is_multiple_of`1件を`is_multiple_of`使用へ修正) |
| `rustfmt --edition 2024 --check`(変更・新規ファイル個別指定) | 初回に整形差分あり→`rustfmt --edition 2024`適用後PASS(いずれも非`mod.rs`のリーフファイル。`src/military/mod.rs`は1行追加のみで整形は実行していない) |
| `cargo test --all-targets`(全体、最終実行) | PASS、**全件PASS**(**1525 passed**、**0 failed**、1 ignored[既存の`profile_p21_021_elimination_scaling_test`、本タスク無関係]) |

1525 = 直前の基準1464 + Domain単体テスト新規40件 + E2E新規21件、と正確に一致する
ことを確認した。`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない。
既知の非決定的PNG9枚は`cargo test --all-targets`実行後に想定どおり再生成され、
個別ファイル単位で`git checkout --`により復元した(ディレクトリ単位の復元はしていない)。

## 7. 対象外を先行実装していないこと

手動補充ボタン、補充優先度、補充停止設定、AI固有の補充判断、装備・資金・物資消費、
`supply_ratio`/`logistics_ratio`による補充速度補正・制限、同盟国領・敵支配地域での
補充、戦闘中・移動中・撤退中の補充、manpower 0 Divisionの復活、Divisionの自動
新規作成、自動復員、Attrition、population減少、恒久損失の減少、
`available + mobilized <= cap`の強制、人的資源上限・回復式の変更、新規セーブ
フィールド、セーブV2、P23-009以降の機能は、いずれも変更ファイル一覧(§4)に含まれて
おらず実装していないことを確認した。手動補充ボタン等のUI要素も追加していない。

## 8. 残課題または後続候補

* 補充に装備・補給(`supply_ratio`/`logistics_ratio`)を関連付ける仕組みは、
  プロンプト自身が「後続候補」と明示したとおり未着手。
* 同盟国領・軍事通行権・補給協定による補充(他国支配地域での自国部隊の補充)は
  今回のスコープ外のまま。
* P23-005/006/007が既に指摘した残課題(AI募兵、Attrition、population↔恒久軍事
  損失の統合、`available + mobilized <= cap`の不変条件)は引き続き未着手。
* `military::reinforcement::apportion_reinforcements`は`budget`が`total_request`
  を上回る場合を`budget.min(total_request)`で防御的にクランプするが、実際の
  呼び出し元(`apply_reinforcement_for_country`)は既に`total_request`以下に
  制限した`budget`しか渡さないため、このクランプ自体は現状到達しない防御的
  コードである(`military::demobilization::DemobilizationError::
  InsufficientBurden`と同種の、契約違反に備えた安全網)。

## 9. 実装コミット直後と報告書コミット後の`git status --short`

### 実装コミット(`d71a97d5c4f9b53c05d33159e3dbb94aacf7b0d2`)直後

```
 M saves/savegame_v1.ron
```

`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、実装コミットには
含めていない。それ以外の全変更(§4の実装・テストファイル)はコミット済み。

### 報告書コミット後

報告書コミット作成後の最終`git status --short`は、コミット完了後の最終回答で報告する。
