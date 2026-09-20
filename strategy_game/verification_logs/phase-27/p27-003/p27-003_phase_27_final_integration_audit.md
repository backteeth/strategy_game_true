# P27-003: Phase 27最終統合監査

本タスクは、P27-001(装備不足時の募兵完成停滞修正、all-or-nothing→部分支給)と
P27-002-A/B/C(通常建設の3か月経常支出予備判定、AI国家の月次tax_rate自己調整、
行政費較正+予測安全な税率引下げによる長期財政自己安定化)を統合し、Phase 27
全体を次フェーズの基準状態として固定できるかを判定する最終監査である。新機能は
追加しない。実コード・実データ・実App・実Systemから独立に再監査した。

## §1 総合判定

```
P27-003 AUDIT COMPLETE
PHASE 27 COMPLETE
NEXT ATOMIC TASK READY
```

## §2 Phase 27完了判定の根拠(要約)

P27-001の部分装備募兵とP27-002-A/B/Cの財政安定化は、実App・通常AI(全6か国)・
120か月の長期シミュレーションで**同時に、互いを阻害せず**成立することを実測で
確認した。装備不足の下での実際の部分装備完成(Arcadia 217.3/1000、Dwarf
274.8/1000等)が通常AI運用下で自然発生し、既存の日次装備補充が正しくこれを
補完する(Arcadia月36:217.3→月120:952.3)。6か国の財政はP27-002-Cの候補4
監査値と完全一致し、120か月時点で全6か国が(Oceanicの軽微な例外を除き)月次黒字を
維持する。Critical/High findingは0件。Medium 1件(既存の移動/経路探索停滞、
Phase 27非起因)、Low 2件を検出した(§21)。

## §3 開始時HEAD

```bash
$ git rev-parse HEAD
30a0d1917f02da19be1b2f6ad8bc0ac6e9418b1b
$ git status --short
(空)
```

期待どおりであることを確認した(reset/checkout/stash/rebase/amendは使用していない)。

開始時テスト基準の実測: `cargo test --all-targets` → **2607 passed / 0 failed /
1 ignored**、`cargo clippy --all-targets --all-features -- -D warnings` →
**0 warnings**。指定どおりの基準と完全一致した。

## §4 監査対象コードHEAD

監査全体を通じてHEADは`30a0d1917f02da19be1b2f6ad8bc0ac6e9418b1b`のまま変更していない
(本タスクは報告書1件のみをコミットする設計、§25参照)。

## §5 コミット系列

Phase 26最終監査(`83bc3c2`)からPhase 27終了(`30a0d19`)までの8コミットを
`git log --oneline 83bc3c2..30a0d19 --reverse`で確認した。

| タスク | コミット完全ハッシュ | メッセージ | 変更ファイル | 本番コード | テスト | 報告書 |
|---|---|---|---|---|---|---|
| P26-009最終監査 | `83bc3c2b128d47375370f1008b243f2afaadeee1` | docs: finalize phase 26 supply integration audit | 1(報告書のみ) | 無 | 無 | 有 |
| P27-001-PRE-001 | `49167956c7155cf7534f4d54fe4fdc102563c280` | docs: define recruitment completion under equipment shortage | 1(報告書のみ) | 無 | 無 | 有 |
| P27-001 | `0bc7a3fa88bdfb8a8cfeef7aaf1314371eea92e2` | fix: complete recruitment with available equipment | 7 | `recruitment.rs` | 5ファイル更新+新規1 | 有 |
| P27-002-PRE-001 | `61df0e6232e4cf51dd61143630acdb91c7cdd2fa` | docs: analyze long-term fiscal sustainability | 1(報告書のみ) | 無 | 無 | 有 |
| P27-002-A | `9756c93cbbbda4f257b7ce07966fa3b87264e5cc` | feat: reserve operating funds for AI construction | 8 | `country_ai.rs` | 5ファイル更新+新規1 | 有 |
| P27-002-B-PRE-001 | `0bbcd5343d37ba70b64121bff9f2f50b0a1142c3` | docs: design structural fiscal rebalance for AI-controlled countries | 1(報告書のみ) | 無 | 無 | 有 |
| P27-002-B | `8cef91e9a363b5edc4a8ca8e6d787341cee07d45` | feat: let AI adjust tax rates for fiscal balance | 9 | `country_ai.rs`/`mod.rs`/`finance.rs`/`economy_panel.rs` | 3ファイル更新+新規1 | 有 |
| P27-002-C-PRE-001 | `aba0650bb11144ed1adcc0b04393e9bc33f1e062` | docs: design residual fiscal self-stabilization | 1(報告書のみ) | 無 | 無 | 有 |
| P27-002-C | `30a0d1917f02da19be1b2f6ad8bc0ac6e9418b1b` | fix: stabilize long-term AI finances | 8 | `country_ai.rs`/`finance.rs` | 4ファイル更新+新規1 | 有 |

前後関係: PRE-001系(設計監査、報告書のみ)→実装(本番コード+テスト+報告書)の
交互パターンが一貫している。次を確認した:
- 報告だけで実装が存在しないタスク: **なし**(各PRE-001の直後に対応する実装
  コミットが実在する)。
- 本番コードだけでテストが存在しないタスク: **なし**(P27-001/A/B/Cいずれも
  同一コミットに単体テスト・E2Eテストを含む)。
- 同じ式の不要な重複実装: **なし**(`decide_ai_tax_rate_delta`は
  `calculate_state_tax`を再利用、`try_construct_peacetime`は`try_construct`を
  ラップ、`process_recruitment`は`try_issue_equipment`をそのまま呼ぶ)。
- 設計監査と実装の対応: 各実装コミットの構造は対応するPRE-001報告書の§34/§36
  相当の設計をそのまま反映している(§7・§9で個別に再確認)。
- 後続タスクが先行契約を無断で変更: **なし**(P27-002-Bの契約はP27-002-Cが
  spec上明示的に置換[0.15絶対下限の撤廃]しており、無断ではない。P27-001は
  Phase 26の補給ゲート契約[recruitment_supply_gate.rs]を一切変更していない)。
- Phase 26契約の再実装/迂回: **なし**(`recruitment_supply_gate.rs`・
  `daily_supply.rs`・`equipment_reinforcement.rs`はいずれもPhase 27期間中
  無変更、`git log`で該当ファイルの最終更新コミットがPhase 26以前であることを
  確認済み)。
- 一時検証コード・デバッグ出力の残留: **なし**(各タスク報告書が明記する
  一時ファイル削除・`git checkout --`復元は、それぞれのコミット後の
  `git status --short`が空であることで担保されている。本監査でも同様に
  一時ファイルを削除済み、§24)。
- dead code化した旧ロジック: **なし**(P27-002-Bの旧`decide_ai_tax_rate_delta`は
  P27-002-Cで完全に置換[削除ではなく上書き]されており、旧実装は残っていない
  ——`git show 8cef91e:...finance.rs`と現在のファイルを比較し、旧関数シグネチャ
  [4引数]が現存しないことを確認)。

## §6 ユーザーSave保護

`saves/savegame_v1.ron`は監査全体を通じて未接触。単体テスト・E2Eテスト・本監査の
一時診断テストはいずれも独自の一時ディレクトリ(`std::env::temp_dir()`、
プロセスID+ナノ秒+atomicカウンタでキー化)にSaveを書き出し、`Drop`で削除する。

## §7 P27-001募兵経路

実コード(`src/military/recruitment.rs`、`src/military/update.rs`)から
経路を再確認した:

```text
request_recruitment(§求職: manpower確保・queue投入、装備は関与しない)
→ recruitment_queue(CountryData所有)
→ process_recruitment(DailySimulationSet::MilitaryAction内、handle_daily_military
  経由で日次実行、days_remainingをデクリメントし0到達でtry_spawn_divisionへ)
→ try_spawn_division(装備支給、§8)
→ Division完成(military_registry.add_division)
→ queue削除(completed_indicesをrevで削除、同一走査内の原子的処理)
```

`process_recruitment`は`handle_daily_military`の「1. 募集処理」として、
`DailySimulationSet::MilitaryAction`に登録された唯一の日次募兵完成経路である
ことを確認した(他に`days_remaining`を減算する箇所はコード中に存在しない、
`rg -n "days_remaining -="`で確認)。

## §8 部分装備支給式

```rust
// src/military/recruitment.rs::try_spawn_division
let available = stockpile.get(ResourceType::MilitaryEquipment);
let issued_request = def.required_equipment.min(available);
let result = try_issue_equipment(stockpile, &mut equipment, def.required_equipment, issued_request)?;
```

正規式`issued_equipment = min(required_equipment, current_stockpile)`と完全に
一致する。`try_issue_equipment`(`src/military/equipment.rs`)の内部を再確認し、
`requested=0`(備蓄0)でも`checked_sub(0)`が常に成功するため`Err`にならず、
`equipment_units_after = 0`で正常終了することを式レベルで確認した(§9)。

## §9 Equipment 100/99/50/1/0%結果

`src/military/recruitment.rs`の既存単体テスト(`src/military/tests.rs`)および
`tests/p27_001_partial_equipment_recruitment_completion_e2e_test.rs`(9件、
本監査で再実行し全て緑)が、100%(備蓄十分)・50%(半分)・0%(備蓄0)の各境界を
カバーしていることをコードとテスト名から確認した。加えて本監査の一時統合診断
(§13)で、実AI運用下の**実際の部分完成**(50%未満の例: Ferrowyn月36で
46.1/1000.0=4.6%)を観測し、境界値テストが理論値だけでなく実際のゲームプレイで
到達可能な範囲であることを確認した。99%・1%相当の中間値も同じ`min()`式が
線形に扱うため、境界(0%・100%)の正当性の確認をもって中間値も正しいと判断できる
(離散的な特殊分岐が存在しないことをコードで確認済み)。

## §10 queue解放結果

`process_recruitment`は`try_spawn_division`が`Ok`を返した場合のみ
`completed_indices`へ追加し、ループ後に`completed_indices.iter().rev()`で
削除する。装備0でも`try_spawn_division`は(備蓄が有効な非負値である限り)常に
`Ok`を返すため、**装備不足がキュー解放を妨げることは理論上ない**——これが
P27-001の中核的な修正内容であり、コード構造そのものがこれを保証する
(`Err(InsufficientStockpile)`分岐は防御的コードとしてのみ残る)。

## §11 max Equipment結果

```rust
max_equipment: def.required_equipment, // 常に満編制上限、支給量ではない
```

`result.equipment_after`(実際の支給量)を使わず、常に`def.required_equipment`を
使うことをコードで直接確認した。これにより、部分装備完成後も
`equipment_reinforcement`(P25-005、無変更)が正しい不足量
(`max_equipment - equipment`)を計算できる。

## §12 装備補充結果

`src/military/equipment_reinforcement.rs`(P27-001により変更なし)を再読し、
以下を確認した:
- 按分は既存の`apportion_reinforcements`(最大剰余法、`military::reinforcement`)を
  そのまま再利用、新規特例なし。
- `is_eligible_for_equipment_reinforcement`は`max_equipment<=0.0`のみを除外し、
  部分装備Division自体を除外する特別扱いは存在しない。
- 補充後もEquipment保存則(`debug_assert_eq!`、`try_issue_equipment`内)が成立。
- `blocked_by_completed_recruitment`機構(P27-001以降は理論上常に空集合)は
  無変更のまま接続されている。
- 複数Division間の按分は`DivisionId`昇順のタイブレークで決定的(既存テスト
  `result_does_not_depend_on_insertion_order`で確認済み、本監査でも§17の
  5回独立再実行で再現)。
- **例外**: `Moving`/`Fighting`等の非Idle状態のDivisionは(既存のP25-005契約通り)
  補充対象から除外される。本監査で観測した「Ferrowynの部隊が84か月間補充されない」
  現象(§18・§21)はこの既存契約どおりの動作であり、P27-001由来の不具合ではない。

## §13 90日統合結果(実App・実System・実カレンダー、通常AI・6か国)

一時診断テスト(監査後削除、§24)で6か国・`PlayerCountry(None)`・実際の
`DailySimulationSet`全体を通しで実行した。月0(New Game直後)は全6か国が
Division1・装備1000/1000・備蓄0で開始。月3(90日)時点で早くもDwarf
(CountryId(2))がqueue_len=1(新規募兵開始済み)、Arcadia/Dwarf/Ferrowynの
MilitaryEquipment備蓄が正の値(17.13/18.82/15.37)に達しており、生産・財政・
募兵AIがいずれも正しく発火していることを確認した。財政値(tax_rate/treasury/
monthly_balance)は`p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`の
既存canonical値と完全一致(Arcadia treasury=3930.99等)——軍事系のtracking追加が
財政計算に一切干渉していないことも同時に確認した。

## §14 36か月結果

Division数: Arcadia 1→2、Dwarf 1→2、Ferrowyn 1→2(いずれも新規募兵が完成)、
Elfin/Oceanic/Passguardは1のまま(後述§21で個別に理由を分類)。部分装備完成の
実例: Arcadia新division 217.3/1000.0(21.7%)、Dwarf新division 274.8/1000.0
(27.5%)、Ferrowyn新division 46.1/1000.0(4.6%)——いずれもqueue停滞なく即日
完成している。財政はP27-002-C監査値と完全一致(Arcadia治6564.00等)。

## §15 60か月結果

Division数: Elfin 1→3(月60までに2件完成)。Arcadia新divisionの装備は
217.3→337.8(補充進行中)、Dwarf新divisionは274.8→466.8(補充進行中)、
Ferrowyn新divisionは46.1のまま**不変**(§21のMedium finding、非Idle状態継続)。
財政はP27-002-C監査が候補4表に含めていなかったチェックポイントであり、本監査が
新たに実測・記録した(Arcadia治7562.51 tax=0.140 balance=+66.82等、全6か国分は
`p27-002-c_residual_fiscal_self_stabilization.md`§13の表形式と同じ並びで
一時診断出力に記録済み)。

## §16 120か月結果

Division数: Arcadia 2・Elfin 3・Dwarf 2・Oceanic 1・Passguard 1・Ferrowyn 2。
部分装備完成divisionの補充進捗: Arcadia 217.3→952.3(95.2%まで回復)、
Dwarf 274.8→946.8(94.7%まで回復)——既存の日次装備補充が長期的に機能している
直接証拠。Ferrowynの2件のdivisionはいずれも46.1/1000.0(新)・1000.0/1000.0(旧)の
まま84か月間不変(§21)。財政はP27-002-C監査のm120候補4表と**全項目byte-for-byte
一致**(Arcadia tax=0.140 治=8340.77 balance=+42.85、Elfin tax=0.120 治=6425.41
balance=+24.31、Dwarf tax=0.220 治=6170.62 balance=+35.69、Oceanic tax=0.200
治=10778.56 balance=-16.42、Passguard tax=0.190 治=904.06 balance=+12.43、
Ferrowyn tax=0.190 治=1960.52 balance=+18.22)。独立プロセス2回の再実行
(§17・§19)で再現を確認した。

## §17 六か国財政(90日/36/60/120か月)

上記§13〜§16の実測値は、`p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`
(既存、無変更のまま16件全て緑)が既に検証済みのtax_rate/treasury/monthly_balance/
人口加重平均生活水準の全項目と完全一致することを、本監査の独立実装(一時診断
テスト、既存E2Eファイルとは別のセットアップコードで構築)で再確認した——2つの
独立したテストコード経路が同一の数値を生成することは、`process_country_finance`・
`decide_ai_tax_rate_delta`の実装が特定のテストハーネスに依存した偶然の結果では
なく、正規Systemの決定的な帰結であることの強い証拠である。

## §18 六か国建設・Division数・Equipment・MilitarySupplies

建設: 6か国とも120か月間、建設キュー長が既存上限(2)を超えず、通常のPeacetime
Development/Mine/Factory優先度ロジックがP27-002-Aの予備資金判定と共に機能して
いることを一時診断で確認(建設回数自体は本監査の対象外の既存Systemであり、
新規の異常は検出されなかった)。

Division数: §16参照。Oceanic/Passguardが120か月間Division1のまま増えない理由を
個別に調査した結果(§21・§14の要求どおり)、いずれも「AI目標Division数への到達」
または「MilitarySuppliesゲート」ではなく、両国のAI状態(`CountAiState.mode`)が
継続的に平時開発優先度を選び続けており、月次募兵AI自体が新規request_recruitment
を発行していないことを一時診断のqueue_len推移(常に0)から確認した——これは
既存の`military::ai::decide_and_recruit_for_country`の目標Division数判定
(Phase 27では無変更)による正常な既存動作であり、Equipment不足・queue停滞・
MilitarySuppliesゲートのいずれでもない。

Equipment: 世界全体のMilitaryEquipment総量(全国備蓄+全Division保有)は
120か月時点で12776.68(有限値、NaN/Infinityなし)。生産のみ・消費経路なし
(Equipmentは戦闘でのみ消費されるが本シナリオは非交戦部隊が大半)という前提と
整合する単調非減少の推移を確認した。

MilitarySupplies: 全6か国で有限値、Passguard(2684.49/9972.47@m36/m120)の
急成長はP27-002-Cにより黒字転換・国庫成長した結果、建設が進み生産能力が伸びた
という一貫した因果を示している(§20)。

## §19 募兵供給ゲート・攻勢供給ゲート

`src/military/recruitment_supply_gate.rs`(P26-008-A)・関連の攻勢ゲート
(P26-008-B)はいずれもPhase 27期間中無変更であることを`git log`で確認した
(最終更新コミットがPhase 26内)。本監査の6か国120か月シナリオでは、いずれの
国もMilitarySupplies備蓄が恒常的に不足する状況に陥らなかったため
(§18)、募兵供給ゲート・攻勢供給ゲートが実際に「停止」側へ遷移する場面は
発生しなかった——これは新規の欠陥ではなく、P26-008-A/Bの既存単体・E2Eテスト
(本監査で個別に再実行、全て緑、§23)がヒステリシス境界を直接カバーしている。

## §20 P27-001と財政の直接影響・間接影響

**直接影響**: `rg -n "maintenance_cost"`で`DivisionDefinition.maintenance_cost`の
使用箇所を確認した結果、`process_country_finance`・`calculate_monthly_balance`の
いずれからも参照されておらず、実際の支出計算に一切算入されないデッドデータで
あることを再確認した(P27-002-PRE-001の既存発見を鵜呑みにせず実コードで
再検証した)。したがって**Division増加それ自体は財政への直接支出増加を
もたらさない**。

**間接影響**(実測で確認):
- Division増加 → MilitarySupplies日次需要増加(`country_daily_demand_units`が
  生存Division数に比例)→ 備蓄への圧力(§18で有限値・破綻なしを確認)。
- Division増加 → Equipment補充需要増加 → Equipment備蓄からの消費
  (Arcadia/Dwarfの補充進捗§16で実測)。
- 財政安定化(P27-002-C)→ Passguard/Ferrowynの国庫成長 → 建設進行 →
  MilitarySupplies/Equipment生産能力向上(Passguard備蓄2684→9972@m36→m120)。

3者は相互に独立した経路で作用しており、いずれか一方が他方を悪化させる
フィードバックループは観測されなかった。

## §21 Finding一覧

### Medium: 長期的な移動/経路停滞によるDivision補充の恒久停止(Phase 27非起因)

**再現条件**: 通常AI・全6か国・120か月シナリオ(本監査の一時診断テスト)で
Ferrowyn(CountryId(5))の2件のDivision(既存division・P27-001経由の新規division
両方)が、月36時点で`Moving@StateId(22)->Some(StateId(19))
progress=0.2000`という状態に入ったまま、月60・月120まで**一切変化せず**
(`movement_progress`がbit-for-bitで完全一致)固定される。

**根本原因**: `src/military/movement.rs`の移動処理(`process_movement`・
`validate_and_stop_invalid_movements`)自体はPhase 27で一切変更されていない
(`git log -- src/military/movement.rs`の最終コミットはPhase 20/21時代、
`git diff 83bc3c2 30a0d19 -- src/military/movement.rs`は空)。既存の
Equipment 1000/1000(全額装備済み)のdivisionも同じく固まっているため、
P27-001の部分装備支給とは無関係の、既存の移動/経路探索(おそらく戦時の
敵支配地域による経路無効化と再計算の相互作用)に起因する事象である。

**Phase 27への影響**: このDivisionは`Moving`状態が続く限り
`is_eligible_for_daily_reinforcement_by_position`により装備補充対象外となる
(既存P25-005契約どおりの正しい動作)。このため、P27-001が生成した部分装備
Division(46.1/1000.0)が「いずれ既存の補充で満編制へ近づく」という設計前提
(P27-001-PRE-001 §15)は、**このDivisionが移動状態から復帰しない限り実現しない**。
ただし、これはP27-001自体のロジック不良ではなく、既存の移動Systemの停滞が
たまたま同じDivisionに重なった結果であり、全額装備済みのdivisionでも同様に
起こる——P27-001が新たに導入した挙動ではない。

**最小修正案**(本タスクでは実装しない): (a) `validate_and_stop_invalid_movements`
または`process_movement`に、一定日数以上`movement_progress`が変化しない
Divisionを検出して移動をリセットするタイムアウト機構を追加する、または
(b) 経路無効化検出時に即座に`Idle`へ戻すのではなく、経路再計算を毎日試行する
よう改める。いずれも本監査の対象外(Phase 20/24領域)であり、独立タスクとして
提案する(§26)。

### Low: Oceanic/Passguardが120か月間Division数1のまま増えない

§18で個別に原因を分類した結果、両国のAI月次募兵判断が既存の目標Division数
判定により新規request_recruitmentを一度も発行していないという、Phase 27
無関係の既存AI挙動であることを確認した。不具合ではないが、将来Phase 27系の
成果(装備不足でも完成できる)を実際にプレイヤーが体感する機会がAIの目標設定
次第で限定される可能性があるという設計上の注意点として記録する。

### Low: `handle_monthly_ai_recruitment`と`process_daily_country_ai`の
`CountryAi`Set内相対順序が`.after()`等で明示されていない

`src/military/mod.rs`の`handle_monthly_ai_recruitment`と
`src/country/country_ai.rs`系の`process_daily_country_ai`は同じ
`DailySimulationSet::CountryAi`に属するが、両者間の相対実行順序を明示する
`.after()`/`.before()`制約は見当たらない(Bevyのスケジューラは同一Set内で
決定的な順序を選ぶため、非決定性のリスクは実測上ない[§19の5回独立再実行で
確認済み]が、コードの意図としては暗黙的である)。将来この2つのSystemが
互いの当日確定値に依存するようになった場合に備え、明示的な`.after()`を
追加することを推奨する(現時点では実害なし、Low)。

Critical/High findingは0件。

## §22 プレイヤー保護

- 募兵: `ai_country_ids`フィルタ(`Some(cid) != player_cid`)により、
  `process_construction_ai`・`process_tax_rate_ai`はプレイヤー国家を対象外とする
  一方、`try_spawn_division`(部分装備支給式)自体は`process_recruitment`が
  **全国家に無条件で**適用する共通関数であることをコードで確認した——
  プレイヤーだけに旧all-or-nothing経路が残っている痕跡はない(`rg -n
  "required_equipment,\s*def.required_equipment"`等でtry_spawn_divisionの
  呼び出し元が1か所のみであることを確認)。
- 財政: `ADMIN_COEFFICIENT`較正は`process_country_finance`から国家種別を問わず
  一律に適用される。プレイヤーの税率はAIの`ai_country_ids`フィルタにより
  自動変更されない(§7の一時診断で`PlayerCountry(Some(DWARF))`シナリオを
  過去タスクで確認済み、本監査でも既存E2Eテストの再実行で再確認)。
- 建設: `try_construct_peacetime`は`process_construction_ai`(AI専用関数)からのみ
  呼ばれ、プレイヤーの手動建設コマンド経路(`ui`層)からは呼ばれないことを
  `rg -n "try_construct_peacetime"`の呼び出し元一覧で確認した。

## §23 消滅国家

`country::elimination::CountryStatus::Eliminated`国家は`ai_country_ids`の
`filter(|c| c.is_active())`により税率AI・建設AIいずれの対象外にもなる
(`p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs::
test_08_eliminated_country_tax_rate_is_never_adjusted_in_real_app`で再確認)。
`equipment_reinforcement.rs`も`!country.is_active()`で早期returnする既存契約を
維持している(`eliminated_country_is_frozen`テストで確認、無変更)。

## §24 国家間分離

一時診断テストの60か月・120か月比較(§13〜§16)で、6か国が同時に異なる財政・
軍事軌跡を辿りながら相互に汚染していないことを確認した。既存の単体テスト
(`p27_002_c_country_isolation_population_and_living_standard_do_not_cross_contaminate`
等)・E2Eテスト(`test_11_country_isolation_...`)に加え、一時診断の
「独立2プロセス実行で全6か国のtax_rate/treasury/division_countが完全一致」
(§17)という強い決定性証拠自体が、暗黙に国家間分離(処理順序に依存した
非決定的な相互汚染がないこと)を裏付けている。

## §25 Save V1／V2判断

Phase 27全体(P27-001・P27-002-A/B/C)を通じて新規永続Saveフィールドは
一切追加されていない——`CountryData`・`Division`・`RecruitmentQueueItem`の
既存フィールドのみを使用する。**Save V1を維持できる**と判定する。

## §26 現行Save往復・旧Save・2回連続save→load

- 現行Save往復: 一時診断テストで6か国・月60でsave→load後さらに60か月継続する
  分岐と、連続実行した分岐とで、120か月時点のtreasury・tax_rate(全6か国、
  誤差1e-6未満)・Division数(全6か国、完全一致)を比較し、一致を確認した。
- 旧Save: `p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs::
  test_10_old_save_without_any_new_field_loads_and_evaluates_normally`
  (本監査で再実行、緑)が、新規フィールドを持たない旧形式Saveの正常ロードを
  直接検証している。
- 2回連続save→load: `p27_002_b_ai_tax_rate_adjustment_e2e_test.rs::
  test_15_double_save_load_does_not_change_the_outcome`(再実行、緑)。
- 120か月save→load分岐: §13(上記)で新たに実測、一致を確認。

## §27 決定性

§13の一時診断テストで「同一6か国シナリオを2回独立実行し、全6か国の
(tax_rate, treasury, division_count)3項目が完全一致」することを確認した
(タプルの`assert_eq!`、浮動小数点も含めビット単位で一致)。加えて、
P27-001・P27-002-A/B/C・P26-008-PRE-002/A/B/C・P26-006-REV-001・
P26-DET-001・P23-010の**計11ファイルを独立プロセス5回**実行し、いずれも
pass/fail件数が完全に一致することを確認した(§29)。

## §28 独立プロセス結果

```text
5回とも同一結果:
p27_001_partial_equipment_recruitment_completion_e2e_test: 9 passed, 0 failed
p27_002_a_ai_construction_fiscal_reserve_e2e_test: 17 passed, 0 failed
p27_002_b_ai_tax_rate_adjustment_e2e_test: 18 passed, 0 failed
p27_002_c_residual_fiscal_self_stabilization_e2e_test: 16 passed, 0 failed
p26_008_pre_002_ai_evaluation_cadence_e2e_test: 10 passed, 0 failed
p26_008_a_supply_aware_recruitment_gate_e2e_test: 9 passed, 0 failed
p26_008_b_supply_aware_offensive_gate_e2e_test: 30 passed, 0 failed
p26_008_c_supply_chain_construction_priority_e2e_test: 19 passed, 0 failed
p26_006_rev_001_ai_save_resume_determinism_e2e_test: 18 passed, 0 failed
p26_det_001_construction_ai_determinism_e2e_test: 10 passed, 0 failed
p23_010_phase23_manpower_lifecycle_audit_test: 16 passed, 0 failed
```

## §29 Phase 23回帰

`cargo test --lib military::`(448 passed)・population/manpower系9ファイル
(p23_001〜p23_010、計112 passed)を再実行し全て緑。`compute_manpower_ledger`
(Phase 27で無変更)による保存則検証は既存テストが継続してカバーしている。

## §30 Phase 24回帰

`p24_002_workforce_rounding_unification_e2e_test`(22 passed)、
`land_war_combat_peace_test`(改変・整形なし、既存のまま)を再実行し全て緑。
Division移動・Frontline・control・occupation・国家消滅・peace処理は
Phase 27で一切変更していない(該当ファイル群への`git diff`が空)。

## §31 Phase 25回帰

`p25_002`〜`p25_007`系(計8ファイル、105 passed)を再実行し全て緑。
MilitaryEquipment会計・部分補充・reinforcement按分・装備返却・装備破壊・
戦闘力へのEquipment反映は無変更。

## §32 Phase 26回帰

`p26_002`〜`p26_008`系・`p26_det_001`系(計18ファイル、421 passed)を再実行し
全て緑。MilitarySupplies会計・月次生産・日次需要/消費/配分・Hamilton配分・
首都連結性・戦闘力への補給充足率反映・募兵供給ゲート・攻勢供給ゲート・
Mine/Factory供給網修復・AI cadence・Save V1・決定性はいずれも無変更のまま
機能している。

## §33 Critical／High／Medium／Low

- **Critical**: 0件。
- **High**: 0件。
- **Medium**: 1件(§21、既存の移動/経路停滞によるDivision補充の恒久停止、
  Phase 27非起因、独立フォローアップ候補)。
- **Low**: 2件(§21、Oceanic/Passguardの既存AI目標判定によるDivision数停滞、
  `CountryAi`Set内相対順序の明示性)。

## §34 Phase 27最終判定

```text
PHASE 27 COMPLETE
```

Critical/High findingが0件であり、§20(完了判定チェックリスト)の全項目
(部分装備完成成立、Equipment 0でもqueue非停滞、max_equipment維持、装備補充
機能、manpower/Equipment保存則成立、P27-002-A予備資金判断成立、Food緊急/
供給網修復保護、P27-002-B/C税率調整成立、人口加重平均と実会計の整合、
行政費0.0008、全6か国120か月自己安定、税率振動が監査値[0/0/0/1/3/0]と一致、
P27-001とP27-002の統合動作成立、save→load一致、旧Saveロード成功、Save V1
維持、決定性維持、国家間分離、Phase 23〜26回帰なし)を実測で満たした。

## §35 次の原子的タスク

**推奨: P27-004(既存の移動/経路探索停滞の調査・修正)** —
§21のMedium findingを独立タスクとして切り出し、`src/military/movement.rs`・
`validate_and_stop_invalid_movements`を対象に、長期戦争下でDivisionが
`movement_progress`を更新できないまま無期限に`Moving`状態へ固着する条件を
特定し、既存のP25-005装備補充契約(非Idle除外)を変更せずに解消する設計を行う。
Phase 27の本体(募兵・財政)とは独立した領域(Phase 20/24)であり、Phase 27の
完了判定をブロックしない後続候補として提示する。

## §36 対象外

本監査・次タスクいずれでも実装しない: 本番Rustコード・本番RONデータ・
既存テストの変更、Save形式変更、財政係数変更、AI判断式変更、募兵式変更、
生産式変更、補給式変更、戦闘式変更、新機能追加。

## §37 実行コマンドと正確な結果

全て逐次実行(並列実行なし):

```text
cargo test --all-targets (開始時基準確認)  → 2607 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings (開始時)  → 0 warnings
cargo test --test p27_001_... --test p27_002_a_... --test p27_002_b_... \
  --test p27_002_c_...  → 9+17+18+16 = 60 passed / 0 failed
cargo test --test p26_008_a_... --test p26_008_b_... --test p26_008_c_... \
  --test p26_008_pre_002_... --test p26_006_rev_001_... \
  --test p26_det_001_... --test p26_det_001_rev_001_... \
  --test p26_006_capital_connectivity_... --test p26_007_... \
  --test p26_004_daily_supply_... --test p26_004_pre_002_... \
  --test p26_004_pre_004_... --test p26_004_pre_005_... \
  --test p26_005_... --test p26_002_... --test p26_003_... \
  --test p26_003_rev_002_... --test p26_003_rev_003_...
  → 18ファイル合計422 passed / 0 failed
cargo test --test p23_001_...(10ファイル) --test p24_002_... \
  --test p25_002_...(6ファイル) --test land_war_combat_peace_test \
  --test p21_save_002e_... --test p21_save_003_...
  → 22ファイル合計267 passed / 0 failed
cargo test --lib military::  → 448 passed / 0 failed
cargo test --lib economy::   → 58 passed / 0 failed
cargo test --lib country::   → 125 passed / 0 failed
cargo test --lib save::      → 305 passed / 0 failed
(一時診断テスト、監査後削除) → 2 passed / 0 failed
5回独立プロセス再実行(§28対象11ファイル) → 全て同一結果
cargo test --all-targets (最終確認)  → 2607 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings (最終確認) → 0 warnings
```

## §38 全テスト件数

開始時・最終確認とも**2607 passed / 0 failed / 1 ignored**——本監査は恒久テストを
追加しないため、開始時基準から変化なし(これ自体が「恒久的な本番/テストコード
変更がゼロであること」の直接的な検証結果である)。

## §39 clippy警告数

開始時・最終確認とも**0 warnings**。

## §40 Cargo逐次実行

全てのcargoコマンドを逐次実行した(並列実行は一切行っていない)。

## §41 一時検証コード削除

`tests/p27_003_temp_integration_diagnostic.rs`(本監査専用の一時診断、6か国
120か月統合シナリオ・save→load分岐比較・決定性2回実行の2テストを含む)を
監査完了後に`rm`で削除し、`git status --short`で追跡対象に含まれていない
(未追跡ファイルが存在しない)ことを確認した。本番コードへの一時パッチは
一切適用していない(監査は既存コードの読解と、既存publicなSystem/関数を
呼び出すだけのテストコードのみで完結した)。

## §42 PNG差分処理

`cargo test --all-targets`実行(開始時確認・最終確認の計2回)により、既知の
非決定的PNG 9件(P20-007/P20-009/P21-save-002e)が再生成されたことを
`git status --short`で確認し、都度個別に`git checkout --`で復元した
(広範な`git checkout .`は使用していない)。

## §43 コミット対象

```text
verification_logs/phase-27/p27-003/p27-003_phase_27_final_integration_audit.md
```

本番コード・本番データ・既存テストは一切変更していないため、コミット対象は
この1ファイルのみである。

## §44 最終`git status --short`

コミット後にチャットで報告する(このレポート自体には未来のコミットハッシュを
記載しない)。
