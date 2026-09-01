# P26-002-REV-001: MilitarySuppliesローカライズ欠落の訂正

## 1. 総合判定

`COMPLETE`

## 2. P26-002の最終的な扱い

```text
P26-002:
  COMPLETE WITH CORRECTION APPLIED

P26-002-REV-001:
  COMPLETE
```

P26-002のfixed-point会計実装(`src/economy/resources.rs`)、公開API、新規テスト(`tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs`)、旧報告書、旧コミット(`4e633552d937b8f274882127e0ce55f7e8d22854`)はいずれも変更・revert・amendしていない。P26-002で報告されたHigh finding 1件(ローカライズキー欠落による確定的テスト失敗)は、本タスクで解消した。

## 3. 開始時HEAD

```text
4e633552d937b8f274882127e0ce55f7e8d22854
```

開始直後の記録:

```text
git rev-parse HEAD
  → 4e633552d937b8f274882127e0ce55f7e8d22854

git status --short
  →  M saves/savegame_v1.ron

git log --oneline --decorate -10
  4e63355 (HEAD -> main) feat: add military supplies accounting foundation
  b44619a docs: design military supply and logistics system
  150e2ae docs: finalize phase 25 equipment lifecycle audit
  0adcfcc feat: apply equipment readiness to combat power
  6023bd5 fix: account for equipment on forced division removal
  7b20ca4 feat: return surviving equipment on demobilization
  26a8629 feat: add daily equipment reinforcement
  0c57beb feat: apply combat equipment losses
  17f54ae feat: require equipment for division recruitment
  9342cdd fix: use fixed-point military equipment accounting
```

## 4. 訂正対象コードHEAD

`4e633552d937b8f274882127e0ce55f7e8d22854`(訂正実装前、本報告書コミット前の時点)。

## 5. P26-002コミットを保持したこと

`4e633552d937b8f274882127e0ce55f7e8d22854`は無変更のまま保持した。`git log`上に引き続き存在し、`git show --name-only --format= 4e63355`で確認できる変更ファイルは`src/economy/resources.rs`・`tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs`・`verification_logs/phase-26/p26-002/p26-002_military_supplies_accounting_foundation.md`の3件のみで、本タスクによる変更は含まれない。

## 6. P26-002報告書を保持したこと

`verification_logs/phase-26/p26-002/p26-002_military_supplies_accounting_foundation.md`は無変更。`git diff --stat`で当該パスの差分がないことを確認した(空出力)。同様に`verification_logs/phase-26/p26-001/`配下も無変更を確認した。

## 7. High findingの内容

P26-002報告書で報告された内容:

```text
Critical/High/Medium/Low: 0 / 1 High / 0 / 1 Low

High finding:
  ResourceType::ALLに追加されたMilitarySuppliesにより、
  src/ui/economy_panel.rsの備蓄表示ループが翻訳キー
  "resource.military_supplies"を要求するようになったが、
  en-US/ja-JPのいずれにも当該キーが存在せず、既存の
  ローカライズ回帰テストが確定的に失敗する。
```

## 8. 回帰失敗の再現結果

訂正実装前に対象テストを実行し、報告どおりの失敗を独立に再現した。

```text
$ cargo test --test p20_009_localization_headless_render_test

thread 'localization_headless_render_ja_en_switch_and_back' panicked at
tests\p20_009_localization_headless_render_test.rs:890:9:
[P20-009] missing keys after opening all panels (ja-JP):
  ["key='economy_panel.info' text='...
    軍需物資: 0
    ⟦MISSING:resource.military_supplies⟧: 0
    ...'"]

test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

## 9. 根本原因

```text
ResourceType::ALL (9 variant、MilitarySuppliesを含む)
→ src/ui/economy_panel.rs:200 の備蓄表示ループ
   for res in ResourceType::ALL {
       ... t(&catalog, locale.0, res.display_name()) ...
   }
→ res.display_name() が ResourceType::MilitarySupplies に対し
   "resource.military_supplies" を返す
   (src/economy/resources.rs:49、P26-002で追加済み)
→ assets/localization/en-US.ron / ja-JP.ron の
   どちらにも該当キーが存在しない
→ 翻訳解決が失敗し ⟦MISSING:...⟧ マーカーが描画される
→ p20_009のheadless renderテストがmissing key検出により失敗
```

## 10. ResourceType::ALLとUI表示の関係

`src/economy/resources.rs`内で`ResourceType::ALL`を反復する経路は、`CountryStockpile::new()`(全キー0初期化)・`economy_panel.rs`の備蓄表示ループ・`profiling.rs`(ベンチマーク用)の3種類。このうちローカライズ解決を伴うのは`economy_panel.rs`のみであり、これが今回のUI表示経路である。P26-002はResourceType追加とfixed-point会計APIの実装に限定されていたため、`ResourceType::ALL`へ1件追加するだけで、既存UIコードを一切変更せずに新たな翻訳キー要求が発生した。これはP26-001の設計監査(§36)で「未検証のリスク」として明示的に指摘されていた事象そのものである。

## 11. 追加した英語キー

`assets/localization/en-US.ron`(`resource.*`ブロック末尾、`resource.raw_magic_crystal`の直後):

```ron
("resource.military_supplies", "Military Supplies"),
```

## 12. 追加した日本語キー

`assets/localization/ja-JP.ron`(`resource.*`ブロック末尾、`resource.raw_magic_crystal`の直後):

```ron
("resource.military_supplies", "軍用補給物資"),
```

## 13. 日本語訳を`軍用補給物資`とした理由

ユーザー指定どおり「軍需物資」ではなく「軍用補給物資」を採用した。

* 既存の`resource.military_equipment`(耐久装備)の訳語「軍需物資」との混同を避け、両者が別資源であることを明確にする。
* 「補給」の語を含めることで、casualty比例でのみ破壊される耐久ストック(MilitaryEquipment)ではなく、日次で消費されるフロー資源(MilitarySupplies)であるという意味を伝える。
* P26-001設計報告書が定義した「弾薬・燃料・軍用食料・医療整備物資等を抽象化した単一の消費補給物資」という意味と一致する。
* 「軍需物資」だけでは装備・弾薬双方を包含するように読め、Phase 25で確立した装備語彙との区別が曖昧になる可能性がある。

## 14. キーの一意性

```text
$ rg -c '"resource\.military_supplies"' assets/localization/en-US.ron
1

$ rg -c '"resource\.military_supplies"' assets/localization/ja-JP.ron
1
```

各ファイルで正確に1件のみ存在することを確認した。

## 15. RON構文検証

* `git diff`で確認した実差分は各ファイル1行の追加のみ(既存行の変更・削除なし、末尾カンマ・括弧の破壊なし)。
* `cargo test --test p20_009_localization_resource_test`の`no_duplicate_keys_in_either_locale_file`・`ja_jp_and_en_us_key_sets_match_exactly`・`placeholder_sets_match_between_locales_for_every_key`が全て合格し、RON構文・キー重複なし・EN/JAキー集合の完全一致・プレースホルダ整合を確認した(8 passed / 0 failed)。
* 両ファイルとも正常にロードでき、既存のResourceType(Food・Wood・Iron・Coal・MagicCrystal・IndustrialGoods・MilitaryEquipment・RawMagicCrystal)の表示は変更前と同一である。

## 16. JA→EN→JA切り替え結果

```text
$ cargo test --test p20_009_localization_headless_render_test

test load_failure_outcome_is_displayed_in_full_app ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

`localization_headless_render_ja_en_switch_and_back`を含む4件全てが合格。JA→EN→JAの切り替え往復でmissing key検出パニックが発生しないことを確認した。

## 17. missing marker解消

修正前に確認された`⟦MISSING:resource.military_supplies⟧`表示は、修正後の同テスト実行では一切出力されなかった(テスト自体がmissing key検出時にpanicする設計のため、テスト合格自体がmissing markerの完全消滅を意味する)。

## 18. 他ローカライズへの非影響

`p20_009_localization_resource_test`の`ja_jp_and_en_us_key_sets_match_exactly`・`placeholder_sets_match_between_locales_for_every_key`・`fallback_to_en_us_works_for_synthetic_missing_ja_key`・`required_key_categories_are_present`が全て合格(8 passed / 0 failed)。`git diff`でも変更が各ファイル1行の追加のみであることを確認しており、既存キーの値・順序・意味への影響はない。

## 19. Rustコード無変更

`git diff --stat HEAD`(訂正前HEAD基準)で`src/`配下に一切の差分がないことを確認した。変更は`assets/localization/en-US.ron`・`assets/localization/ja-JP.ron`の2ファイルのみ。

## 20. 既存テスト無変更

`tests/`配下のいずれのファイルにも変更を加えていない。新規テストも追加していない(既存の`p20_009_localization_headless_render_test`・`p20_009_localization_resource_test`が今回の訂正の直接的な証拠として機能する)。

## 21. P26-002 fixed-point会計無変更

`src/economy/resources.rs`は無変更。`cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test`を再実行し、45 passed / 0 failed / 0 ignoredで変更前と同一件数であることを確認した。

## 22. Save V1維持

Save関連コード(`src/save/`)・Save format versionともに無変更。本タスクはローカライズリソースファイルのみの変更であり、Save構造・意味論に一切影響しない。

## 23. 本番Systemへ未接続

`MilitarySupplies`の会計APIは引き続きいかなる生産・消費Systemからも呼び出されていない。本タスクは表示用ローカライズキーの追加のみであり、`add_military_supplies_produced`・`try_consume_military_supplies`の呼び出し箇所を追加していない。

## 24. Phase 25会計の保護

P25関連E2Eテスト8ファイルを個別に逐次実行し、全て変更前と同一件数で合格した:

| テストファイル | 結果 |
| --- | --- |
| p25_002_equipment_accounting_foundation_e2e_test | 15 passed |
| p25_002_rev_001_fixed_point_equipment_accounting_e2e_test | 6 passed |
| p25_003_recruitment_equipment_issue_e2e_test | 22 passed |
| p25_004_combat_equipment_loss_e2e_test | 5 passed |
| p25_005_daily_equipment_reinforcement_e2e_test | 10 passed |
| p25_006_demobilization_equipment_return_e2e_test | 29 passed |
| p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test | 8 passed |
| p25_007_equipment_combat_effectiveness_e2e_test | 21 passed |

全て0 failed。

## 25. Phase 23・24会計の保護

対象8ファイルを個別に逐次実行し、全て合格した:

| テストファイル | 結果 |
| --- | --- |
| p23_002_mobilized_manpower_workforce_e2e_test | 8 passed |
| p23_004_economic_workforce_ratio_e2e_test | 7 passed |
| p23_006_manpower_ledger_e2e_test | 22 passed |
| p23_007_division_demobilization_e2e_test | 24 passed |
| p23_008_daily_reinforcement_e2e_test | 21 passed |
| p23_009_ai_recruitment_e2e_test | 22 passed |
| p23_010_phase23_manpower_lifecycle_audit_test | 9 passed |
| p24_002_workforce_rounding_unification_e2e_test | 17 passed |

全て0 failed。

## 26. P26-003を先行実装していないこと

生産System・Division需要・日次配分・戦闘力反映・移動・AI・UIロジックへの接続は一切追加していない。変更はローカライズリソースファイル2件と本報告書のみ。

## 27. Critical／High／Medium／Low

```text
Critical: 0
High:     0  (P26-002報告のHigh finding 1件は本タスクで解消)
Medium:   0
Low:      0  (P26-002報告のLow finding「MAX_UNITS暫定値の再監査」はP26-003側の課題として引き続き有効、本タスクの対象外)
```

## 28. 変更したファイル

```text
assets/localization/en-US.ron   (+1行)
assets/localization/ja-JP.ron   (+1行)
verification_logs/phase-26/p26-002-rev-001/p26-002-rev-001_military_supplies_localization_correction.md  (新規)
```

## 29. 実行した全コマンドと正確な結果

全てのCargoコマンドは逐次実行した(並列実行なし)。

```text
$ cargo test --test p20_009_localization_headless_render_test  (修正前)
  → FAILED (3 passed; 1 failed) — 既知の失敗を再現

$ cargo test --test p20_009_localization_headless_render_test  (修正後)
  → ok. 4 passed; 0 failed; 0 ignored

$ cargo test --test p20_009_localization_resource_test
  → ok. 8 passed; 0 failed; 0 ignored

$ cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test
  → ok. 45 passed; 0 failed; 0 ignored

$ cargo test --test p25_002_equipment_accounting_foundation_e2e_test
  → ok. 15 passed; 0 failed; 0 ignored

$ cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test
  → ok. 6 passed; 0 failed; 0 ignored

$ cargo test --test p25_003_recruitment_equipment_issue_e2e_test
  → ok. 22 passed; 0 failed; 0 ignored

$ cargo test --test p25_004_combat_equipment_loss_e2e_test
  → ok. 5 passed; 0 failed; 0 ignored

$ cargo test --test p25_005_daily_equipment_reinforcement_e2e_test
  → ok. 10 passed; 0 failed; 0 ignored

$ cargo test --test p25_006_demobilization_equipment_return_e2e_test
  → ok. 29 passed; 0 failed; 0 ignored

$ cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test
  → ok. 8 passed; 0 failed; 0 ignored

$ cargo test --test p25_007_equipment_combat_effectiveness_e2e_test
  → ok. 21 passed; 0 failed; 0 ignored

$ cargo test --test p23_002_mobilized_manpower_workforce_e2e_test
  → ok. 8 passed; 0 failed; 0 ignored

$ cargo test --test p23_004_economic_workforce_ratio_e2e_test
  → ok. 7 passed; 0 failed; 0 ignored

$ cargo test --test p23_006_manpower_ledger_e2e_test
  → ok. 22 passed; 0 failed; 0 ignored

$ cargo test --test p23_007_division_demobilization_e2e_test
  → ok. 24 passed; 0 failed; 0 ignored

$ cargo test --test p23_008_daily_reinforcement_e2e_test
  → ok. 21 passed; 0 failed; 0 ignored

$ cargo test --test p23_009_ai_recruitment_e2e_test
  → ok. 22 passed; 0 failed; 0 ignored

$ cargo test --test p23_010_phase23_manpower_lifecycle_audit_test
  → ok. 9 passed; 0 failed; 0 ignored

$ cargo test --test p24_002_workforce_rounding_unification_e2e_test
  → ok. 17 passed; 0 failed; 0 ignored

$ cargo test --all-targets
  → 全バイナリ合計: 1898 passed; 0 failed; 1 ignored

$ cargo clippy --all-targets --all-features -- -D warnings
  → 警告0、正常終了
```

`cargo clean`は実行していない(キャッシュ破損は発生しなかった)。

## 30. 全テストpassed／failed／ignored件数

```text
開始時: 1897 passed / 1 failed / 1 ignored
最終:   1898 passed / 0 failed / 1 ignored
```

新規テストは追加していないため、`cargo test --all-targets`の総件数(1898)は開始時の1897から1件失敗が解消し合格へ転じた結果である(1897 passed + 1 failed→passed = 1898 passed)。実測ログから`test result:`行を全バイナリ分合算して算出した。

## 31. clippy警告数

```text
0
```

## 32. Cargo逐次実行

全Cargoコマンドを1つずつ順番に実行し、並列実行は一切行っていない。

## 33. PNG差分の処理

`cargo test --test p20_009_localization_headless_render_test`および`cargo test --all-targets`の実行により、既知の非決定的PNG(最大9枚)が都度再生成された。いずれも既知の対象ファイルであることを確認したうえで、個別パス指定の`git checkout --`で復元した(広範なcheckoutは未使用)。最終的にコミット対象PNGは0件。

## 34. ユーザーセーブの保護

`saves/savegame_v1.ron`は全工程を通じて未読み書き・未復元・未ステージ・未コミット。開始時・終了時とも既存の`M`差分のみで、本タスクによる変更は一切ない。

## 35. 訂正対象コードの`git status --short`

訂正実装完了・テスト完了直後(コミット前)の状態:

```text
 M assets/localization/en-US.ron
 M assets/localization/ja-JP.ron
 M saves/savegame_v1.ron
```

## 36. P26-003へ進める状態か

進める状態である。P26-002報告のHigh finding(ローカライズキー欠落)は本タスクで解消され、`cargo test --all-targets`は0 failedへ復帰した。P26-002報告のLow finding(`MAX_MILITARY_SUPPLIES_UNITS`が暫定値であり、実生産式確定後に再監査が必要)は本タスクの対象外のまま引き続き有効であり、P26-003(生産配線)着手時に扱うべき事項として残る。
