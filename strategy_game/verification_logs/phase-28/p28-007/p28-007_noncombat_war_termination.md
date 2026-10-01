# P28-007：国境を失った戦争の非戦闘終結（実装）

## 0. 概要

P28-007-PRE-001で確定したE2契約（進行中Warの`border_region_pairs`が空になった場合、既存の白紙講和経路で終結させる。war6・war9型の補給・戦力不足による停止は対象外、無戦闘期間だけを理由にしない）を恒久実装した。実装自体はPRE-001の一時パッチと等価だが、Phase 23〜28の既存回帰テスト全体を実行した結果、PRE-001の600か月実行では一度も顕在化しなかった2件の未検証境界条件を新たに発見し、契約を黙って拡張せず、原因を特定したうえで最小限の対処（1件は本番コードの契約追加、1件はテスト側の隔離）を行った。詳細は本報告書§3・§8で報告する。

## 1. 基準状態

- 開始時 `git rev-parse HEAD`: `ecab2fd4ae1afc76dfbdfeab36e7b2ab83ec89f3`（期待値と一致）
- 開始時 `git status --short`: 出力なし（クリーン）
- 前タスク最終実測基準（今回転記のみ）: 2752 passed / 0 failed / 1 ignored, clippy warnings 0

## 2. 必読資料とコードでの再確認

`verification_logs/phase-28/p28-007-pre-001/p28-007-pre-001_noncombat_war_termination_design.md`を読了し、採用契約（E2）と反実仮想の観測値を確認した。そのうえで`rg`により以下を実コードで再確認した。

- `src/war/update.rs::handle_daily_war_resolution`（130行目〜）: ステップ(1)戦闘結果集計 (2)戦勝点更新 (3)降伏判定・自動講和 (4)前線再計算、の順。設計どおり(3)と(4)の間に新設ステップを挿入できることを確認。
- `src/war/frontline.rs::calculate_frontline_border`（712行目〜）: `border_region_pairs`は、攻撃側がcontrollerの州と、defenderがcontrollerの隣接州の組だけを集めた、日次で完全に再計算される純粋な地理的関数。補給・前線割当Division数には一切依存しないことをコードで再確認した。
- `src/war/capitulation.rs::check_defender_capitulation`/`check_attacker_capitulation`: 占領（支配権変更）を前提とする既存の唯一の自動終結条件であること、`war.war_goals.first()`が`None`なら即`false`を返すこと（本タスクで発見した境界条件の根拠、§3参照）を再確認した。
- 複数Warが同日に解決される際の処理順: `handle_daily_war_resolution`はActive War一覧を`.values().filter().map().collect()`で都度取得し直す（既存ステップ3と同一パターン）。E2による白紙講和(`PeaceTerm::WhitePeace`)は`transferred_state`を発生させないため`invalidate_other_wars_targeting_state`（既存E1）を連鎖的に起動しない。したがってE2はWar間で副作用を持たず、取得順（HashMap反復順）に依存しないことを確認した。
- `src/war/peace.rs::execute_peace_settlement`の「占領状態の正常化」ステップ（589行目付近）: `state.war_id == Some(war_id) || state.controller_country.is_some()`という条件で、終結したWarの当事国が所有する**現在占領中の全州**（`war_id`で終結対象Warに限定されない）の支配権を所有者へ戻す既存仕様であることを再確認した。これが本タスクで発見した境界条件の直接の原因である（§3参照）。
- `src/war/data.rs`・`tests`：既存のSave関連テスト（`p21_save_003`, `p26_006_rev_001_ai_save_resume_determinism_e2e_test`）で、War関連状態（`WarRegistry`・`FrontlineRegistry`）が特別な永続フィールドなしにSave/Load可能であることを再確認した。

## 3. 実装契約

### 3.1 本体実装

`src/war/update.rs::handle_daily_war_resolution`に、ステップ3（降伏判定）とステップ4（前線再計算）の間に「ステップ3.5」を追加した。

```
let frontline_stalled_war_ids: Vec<WarId> = war_registry
    .wars
    .values()
    .filter(|w| w.status == WarStatus::Active && !w.war_goals.is_empty())
    .map(|w| w.id)
    .collect();

for war_id in frontline_stalled_war_ids {
    if !war_has_no_valid_frontline(war_id, &frontline_registry) {
        continue;
    }
    let outcome = settle_capitulation(
        war_id, PeaceTerm::WhitePeace, FRONTLINE_UNREACHABLE_REASON, ...
    );
    report_settlement_outcome(outcome, war_id, "FrontlineUnreachable", &mut notif_writer, &loc);
}
```

- **判定式**: `war_has_no_valid_frontline(war_id, &frontline_registry)` = `frontline_registry.get_frontline_for_war(war_id).is_some_and(|fl| fl.border_region_pairs.is_empty())`。`Frontline`が存在しない（宣戦当日などまだ生成されていない）場合は`None`となり自然にスキップされる。
- **処理順**: ステップ3（降伏判定、既存E1を内包）→ **ステップ3.5（新設E2）** → ステップ4（前線再計算）。ステップ3.5はActive War一覧を**改めて取得し直す**ため、ステップ3で既に終結したWar（降伏・既存E1連鎖）は対象から自然に除外される。
- **終戦処理の再利用**: 既存の`settle_capitulation`（`execute_peace_settlement`のラッパー、プレイヤー通知・重複抑止込み）をそのまま呼ぶ。新しい清算経路は作っていない。終結理由だけを`FRONTLINE_UNREACHABLE_REASON = "Frontline Unreachable"`という新規の文字列にし、既存の`"War Goal No Longer Achievable"`（E1）と区別できるようにした。
- **二重実行防止**: `execute_peace_settlement`自体の冪等性（`war.status != Active`なら即`Ok(())`）に加え、ステップ3.5は自分自身の対象リストも１日に１回しか評価しないため、同一War・同日内での二重実行は起きない。
- **リファクタ**: ステップ3とステップ3.5で同じ`match outcome { ... }`通知処理が重複するため、`report_settlement_outcome`という小さな共通関数へ抽出した（最小限の変更の一部として許容されるレベルの重複排除）。
- **保護**: N日待機・永続カウンター・新しいSaveフィールドは追加していない。`border_region_pairs`は`update_all_frontlines`により毎日完全再計算されるため、ロード後も同一条件を再導出できる（Save V1のまま、migration不要）。

### 3.2 一時パッチで未検証だった境界条件の発見と対処

Phase 23〜28の既存回帰テスト（`cargo test --all-targets`）を実行したところ、2件のテストが新規に失敗した。いずれもP28-007-PRE-001の600か月間の標準シナリオ実行では一度も顕在化しなかった境界条件であり、契約を黙って拡張せず、原因を特定したうえで対処した。

**境界条件A（本番コードの契約を1条件追加）**: `tests/p27_005_offensive_frontline_supply_bridge_e2e_test.rs::test_01_arcadia_baseline_bridge_matches_the_audited_canonical_timeline`が失敗。原因を調査したところ、このテストの`War`は`war_goals: vec![]`（戦争目標を持たない、前線補給ブリッジ機構だけを検証するための人為的fixture）であった。攻撃側が防御側の唯一の州を完全占領すると、占領によっても`border_region_pairs`が空になり得るが、`war_goals`が空のWarは`check_defender_capitulation`の前提（`war.war_goals.first()`）が成立せず、降伏で終わる経路が最初から存在しない。この状態でE2が発火すると、「構造的停止」ではなく「占領による実質的勝利」を白紙講和に変換し、`execute_peace_settlement`の占領正常化ステップが攻撃側の占領を解除してしまう。**自然発生するAI戦争は必ず単一の`WarGoalType::ConquerState`を持つ**（`src/diplomacy/crisis.rs`等、宣戦5箇所を`rg`で再確認）ため、`war_goals.is_empty()`のWarをE2の対象から除外する契約の明確化（拡張ではなく、対象外原因を正確化する収縮）を行った。この追加条件は、現行の対象原因（国境・戦争目標喪失）の挙動には一切影響しない（§7の600か月反実仮想で確認）。

**境界条件B（テスト側の隔離、本番コード無変更）**: `tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs::test_90_day_controlled_war_offensive_supply_gate_real_app_verification`が失敗。調査の結果、このテストは実データの標準シナリオ上に手動でWarを1件追加するが、**実データには別途、CountryId(3)対CountryId(1)(Elfin)の自然発生Warが存在し、これ自体は国境を失ったまま構造的に停止したWarで、E2により1800/03/14に正しく白紙講和となる**（E2固有の誤判定ではない）ことを確認した。しかし`execute_peace_settlement`の占領状態正常化ステップは、終結したWarの当事国が所有する**現在占領中の全州**（`war_id`での絞り込みはOR条件のフォールバックで迂回される既存の仕様）を基準へ戻すため、Elfinが所有する州をこのテストの制御Warが占領していると、無関係なそのWarの終結に巻き込まれて占領が解除されてしまう。この集団処理は**E2固有の問題ではなく、降伏・既存E1を含むあらゆる終結経路が持つ`execute_peace_settlement`の既存の仕様**であり、E2が新たに終結経路を1つ追加したことで初めて顕在化した。本番コード（`execute_peace_settlement`）の変更はP28-007の対象範囲を超えるため行わず、代わりにテスト側でElfinと他の全AI国家との間に長期休戦（`DiplomacyRegistry::set_truce`、Crisis宣戦前に`is_in_truce`が参照される既存の仕組み）を事前設定し、この無関係な自然発生Warがそもそも宣戦されないよう制御シナリオを隔離した。前線補給ゲートという検証対象自体・戦力比110%/90%・補給率等の本番Systemの計算は一切変更していない。

**追加で判明した事実（設計の補足修正）**: PRE-001報告書は「宣戦当日は`Frontline`が未生成のため自然にスキップされる」としていたが、System順序を再検証した結果、`WarPreparation` Set（`handle_daily_war_prep`、Diplomacyの直後）も同日内で`update_all_frontlines`を呼ぶため、**宣戦と同じ日のうちに(1)Diplomacyが宣戦 → (2)WarPreparationがFrontlineを生成 → (3)WarResolutionのステップ3.5がそのFrontlineを参照、という順で、国境が最初から存在しない宣戦はその日のうちにE2で終結し得る**ことを確認した（境界条件Bの調査中に実測）。これは危険な誤動作ではない（陸路上そもそも接触し得ない宣戦を即座に終わらせるのは契約の意図どおり）が、PRE-001の「宣戦当日は安全にスキップされる」という記述は不正確だったため、ここで訂正する。

## 4. 恒久テスト

`src/war/update.rs`の既存`mod tests`（Bevy Appを使わない、レジストリ直接操作の軽量ユニットテスト。既存の`settle_capitulation`系テスト6件と同じスタイル）に、独立した3件を追加した。

1. **`frontline_lost_without_target_state_transfer_ends_in_white_peace`（国境消失）**: state0(attacker境界州)-state1(defender境界州)-state2(defender戦争目標州、state1経由でのみ到達可能)の3州構成で開戦。第三国への割譲でstate1の所有者が変わり国境が消失する一方、戦争目標州state2はdefender所有のまま不変（＝既存E1は発火しない条件）であることを確認したうえで、ステップ3.5相当の`settle_capitulation`呼び出しがWhitePeace・理由`"Frontline Unreachable"`で終結させ、前線が削除され、戦争目標州の所有権は移転せず、攻撃側・防御側間に休戦が作成されることを確認する。
2. **`frontline_with_remaining_border_is_never_terminated_by_e2`（国境が残る停止）**: state0-state1の2州構成で開戦後、`offensive_supply_blocked=true`（war6・war9型の実症状を模す）にしても、国境（`border_region_pairs`）が消えていない限り`war_has_no_valid_frontline`は常に`false`を返し、ステップ3.5相当の抽出条件からも除外され、Warは単なる無戦闘でActiveのまま継続することを確認する。
3. **`capitulation_outcome_is_not_overwritten_by_e2_same_day`（優先順位）**: 防御側の唯一の所有州（＝戦争目標州）が既に攻撃側に占領（controller変更）されている状態を構築し、占領率100%による降伏条件とE2の国境喪失条件が同日中に同時成立し得ることを確認したうえで、ステップ3（降伏、`CedeWarGoalRegion`/`"Defender Capitulation"`）を先に実行し、その後Active War一覧の再取得でこのWarが自然に除外されること、および仮にE2を同じWarへ誤って直接適用しても`execute_peace_settlement`の二重適用防止により`status`・`end_reason`が上書きされないことを確認する。

いずれも`cargo test --lib -q war::update::`で実行、9 passed（既存6件＋新規3件）、0 failed。

既存テストへの変更（§3.2参照）: `tests/p27_005_offensive_frontline_supply_bridge_e2e_test.rs`は無変更（本番コードの`war_goals.is_empty()`ガードのみで解消）。`tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs`に`isolate_elfin_from_unrelated_wars`ヘルパーを追加し、該当テストの冒頭で1回呼ぶのみ（26行追加）。`tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`の`M120_EXPECTED`テーブルをDwarf・Oceanicの2か国分だけ更新し、理由を既存の更新履歴と同じ書式でコメントに追記した（§7で詳述）。

## 5. 動的検証

### 5.1 制御シナリオ（恒久ユニットテストで実施、§4参照）

国境消失日・E2判定日・白紙講和日は、恒久テスト内で同一ステップ（1回の`settle_capitulation`呼び出し）として検証される設計のため、日次トレースは本番一時パッチ検証（PRE-001）および下記§5.2の実App標準シナリオで取得した。

### 5.2 実App標準シナリオ（36・120・240・600か月、実装後再実測）

P28-007-PRE-001と同一の標準シナリオ・固定シードを用い、一時的な診断ハーネス（`tests/p28_007_temp_diagnostic.rs`、検証後に削除）で600か月を再実行し、恒久実装の結果がPRE-001の一時パッチ実測値と完全一致することを確認した。

- war3: 1801/03/21に国境消失、**1801/03/22**にWhitePeace・理由`"Frontline Unreachable"`で終結（PRE-001の一時パッチ実測と1バイトも違わず一致）。
- war0/war2/war4/war5/war7/war8: 勝敗の種類は全てB0（無変更）と同一。終結日・開戦日はwar3の15日早期終結に伴う決定論的だが非同一な連鎖的タイミングずれの範囲内（PRE-001報告済みの現象と同一）。
- war6・war9（war9型、国境ありの生産・補給不足停止）: `border_region_pairs`は一度も空にならず、E2は一切発火しない。600か月時点でActiveのまま継続（B0と完全一致）。
- SUMMARY（総戦争数・battles_ever・battle_days等）: PRE-001が一時パッチで実測した値と全チェックポイント（36/120/180/240/360/480/600か月）で完全一致。

### 5.3 save→load・二重ロード

war3の国境消失直前（1801/03/19、Active）でsave→load、さらに連続して2回目のloadを実施。両ロード後も`status=Active`のまま保持され、その後の進行を継続させたところ、通常実行と完全に同一の結果（1801/03/21国境消失、1801/03/22 WhitePeace・`"Frontline Unreachable"`）が得られた。新規永続フィールドを追加していないため、`border_region_pairs`はロード後も`update_all_frontlines`により毎日完全再計算され、タイマーを持たないためsave/loadの前後や二重ロードで結果が変わる余地がないことを実測で確認した。

### 5.4 独立プロセスでの決定性

同一の600か月標準シナリオを独立した2つの`cargo test`プロセスで実行し、ログを比較した。SUMMARY・WAR（終結理由・終結日・勝敗種別・border遷移・判断理由カウント）は完全に一致した。唯一の差分は、`divs=[...]`形式のDivision一覧表示順と`BLDEVT`イベントの出力順（いずれも診断ハーネス自身の表示用コードがHashMapを反復する際の順序差であり、本番の`WarResolution`・E2判定ロジックには影響しない）。実際のゲーム状態（War状態・州所有権・勝敗）に関する決定性は完全に確認された。

## 6. 保護範囲の遵守

AI募兵のB0条件、戦力評価式、補給ゲート・日次補給・戦闘計算、Railway建設AI・戦時MilitaryFactory建設AI、国境・地形データ、プレイヤーの手動講和操作、Save形式はいずれも変更していない。Save V1を維持し、新しい永続フィールド・migrationは導入していない。P28-004の平時過剰募乵とwar9型の停止は、本タスクでも一切対処しておらず、未解決のまま引き継ぐ（§5.2のとおりwar6・war9はE2の対象外として変化なし）。`execute_peace_settlement`の占領状態正常化の既存仕様（§3.2の境界条件B）は、P28-007の対象範囲を超えるため本番コードを変更していない。

## 7. 検証（個別テスト・Cargo逐次実行）

`rg --files tests`で実名確認した、War・講和・国家消滅・外交・Save・Phase 27戦闘補給・P28-002/003/005関連テストを個別実行した。

| テストファイル | 結果 |
|---|---|
| p21_019_crisis_war_peace_resolution_e2e_test | 4 passed |
| p21_021_country_elimination_e2e_test | 1 passed |
| p21_016_multilateral_war_e2e_test | 1 passed |
| p28_005_ai_railway_priority_e2e_test | 3 passed |
| p28_003_disconnected_friendly_frontline_delivery_e2e_test | 2 passed |
| p28_002_wartime_military_factory_construction_e2e_test | 3 passed |
| p27_fu_001_elimination_peace_e2e_test | 3 passed |
| p27_fu_001_ai_retreat_alternative_e2e_test | 3 passed |
| p27_007_zero_damage_battle_termination_e2e_test | 2 passed |
| p27_005_offensive_frontline_supply_bridge_e2e_test | 10 passed |
| p26_008_b_supply_aware_offensive_gate_e2e_test | 19 passed |
| p26_008_a_supply_aware_recruitment_gate_e2e_test | 30 passed |
| land_war_combat_peace_test | 4 passed |
| p21_005_army_frontline_e2e_test | 2 passed |
| p21_save_003_end_to_end_test | 4 passed |
| p26_006_rev_001_ai_save_resume_determinism_e2e_test | 9 passed |
| diplomacy_tests | 5 passed |

全て`0 failed`。これに加え、全ワークスペース実行で初めて失敗が判明した`p27_002_c_residual_fiscal_self_stabilization_e2e_test`も個別に20 passed/0 failedを確認した（§3.2・直下の差分説明参照）。

**`p27_002_c_residual_fiscal_self_stabilization_e2e_test`の正規値更新（E2による正当な世界状態の変化と判定）:**

`test_05_real_six_country_90_day_36_60_120_month_long_horizon_verification`の月120実測値テーブル（`M120_EXPECTED`）のうち、Dwarf・Oceanicの2か国分のみ以下のとおり変化した。他の4か国（Arcadia・Elfin・Passguard・Ferrowyn）は完全不変（小数点以下まで一致）。

| 国 | 旧(tax, treasury, balance) | 新(tax, treasury, balance) |
|---|---|---|
| Dwarf | (0.140, 15453.41, 206.56) | (0.140, 15156.42, 232.72) |
| Oceanic | (0.130, 4695.83, 113.36) | (0.120, 4759.42, 35.54) |

**判定根拠**: この2か国は、E2によって終結日が早まったwar3（Oceanic対Dwarf、本報告書の国名でDwarf=CountryId(2)、Oceanic=CountryId(3)、PRE-001報告書の呼称でwar3）の当事国であり、同じ地域紛争クラスタの一員である。war3の終結が1801/04/06（既存E1経由）から1801/03/22（E2経由）へ15日早まったことで、以降のAI意思決定タイミングが決定論的だが非同一に連鎖し、月120（1810年）時点の財政軌道がわずかに変化した。崩壊的な税率振動（`EXPECTED_OSCILLATIONS`、既存の`u32::MAX`=無制限許容を含む境界値テーブル）や国庫の負転は一切新たに発生しておらず（既存のオシレーション回数チェックは全てそのまま通過）、当事国でも地域紛争クラスタの一員でもない4か国は1円も変化していない。したがってこれは契約違反ではなく、E2が正しく機能したことによる、範囲の限定された正当な世界状態の変化と判定した。

**Cargo逐次実行（実測）:**
```
cargo test --all-targets -j 2
→ 2755 passed; 0 failed; 1 ignored

cargo clippy --all-targets --all-features -- -D warnings
→ warning 0件
```

非決定的PNG 9件は、両回とも個別に`git checkout --`で復元済み。`saves/savegame_v1.ron`と`tests/land_war_combat_peace_test.rs`は変更・整形していない。ワークスペース全体のformatは行っていない。

## 8. Critical / High / Medium / Low

- **Critical**: なし。
- **High**: なし。
- **Medium**: `execute_peace_settlement`の占領状態正常化ステップ（`src/war/peace.rs`、「占領状態の正常化」コメント部）が、終結Warの当事国が所有する**現在占領中の全州**を無差別に正常化する既存仕様であり、`war_id`では絞り込まれない。これはE2固有の問題ではなく、降伏・既存E1を含むあらゆる終結経路に共通する既存の挙動だが、E2が新たな終結経路を1つ追加したことで、複数Warが同一国家を介して領土を共有する場合に顕在化しやすくなった（本タスクでは`tests/p26_008_b_...`のテスト隔離で対処、本番コードは未変更）。実App上でこの衝突が起きる確率は、本タスクの600か月実測および過去のP28-00xタスク群の実測では一度も観測されていないが、理論上は否定できない。次の原子的タスク候補として記録する。
- **Low**: PRE-001報告書の「宣戦当日はFrontline未生成のため自然にスキップされる」という記述が、`WarPreparation` Set（`handle_daily_war_prep`）が同日内で先に`update_all_frontlines`を呼ぶことを見落としており不正確だった（§3.2末尾で訂正）。実害はない（国境が最初から存在しない宣戦を即日終結させるのは契約の意図どおり）が、設計ドキュメントの記録として訂正を残す。

**次の原子的タスク（1件）:** 上記Medium所見の調査・対処を検討する場合は、`execute_peace_settlement`の占領状態正常化ステップを「終結したWar自身が占領させた州」のみに限定する設計監査を、P28-007とは独立したタスクとして実施する（P28-007自体はこれを本番コードとして変更していない）。それ以外であれば、Phase 28の次の優先度はP28-004（平時過剰募兵・war9型停止、ユーザー判断待ち）に戻る。

## 9. 最終報告

1. `P28-007 COMPLETE`。E2契約を`handle_daily_war_resolution`へ恒久実装し、恒久回帰テスト3件を追加、全テストpassed・clippy 0警告を確認した。
2. 開始HEAD: `ecab2fd4ae1afc76dfbdfeab36e7b2ab83ec89f3`。コミットハッシュ・最終HEADは本報告書コミット後に別途報告する。
3. 判定式: `border_region_pairs.is_empty()`（＝既存の`NoValidFrontline`と同一条件）かつ`war_goals`が空でないこと。処理順: ステップ3（降伏・既存E1）→ステップ3.5（新設E2、Active War一覧を再取得）→ステップ4（前線再計算）。
4. 国境消失シナリオ（実App標準シナリオ、war3）: 1801/03/21国境消失→**1801/03/22**WhitePeace・`"Frontline Unreachable"`で終結（PRE-001一時パッチと完全一致）。
5. war6・war9（国境残存の生産・補給不足型）および通常勝利（war0/2/4/5/7/8）は、勝敗の種類が全てB0と同一のまま保護された（終結・開戦タイミングのみ、war3早期終結に伴う既知の連鎖的ずれ）。
6. 36/120/240/600か月のWar数・継続War数・終結理由・battles_ever/battle_daysはPRE-001の一時パッチ実測と完全一致。財政・補給面では、war3当事国クラスタ（Dwarf・Oceanic）の月120財政値が決定論的な連鎖的ずれにより微小に変化（`p27_002_c`テストの正規値を更新、§7参照）、それ以外の4か国は完全不変。
7. save→load・2回連続ロード後も通常実行と完全一致する結果を確認。Save V1を維持し、新規永続フィールド・migrationは不要と判定した。
8. 既存テストの変更: `tests/p27_005_...`は無変更（本番コードの`war_goals.is_empty()`ガードで解消）。`tests/p26_008_b_...`にWarを宣戦前から隔離するテスト専用ヘルパーを追加（本番コード無変更）。`tests/p27_002_c_...`の月120正規値テーブルをDwarf・Oceanicの2か国分のみ更新（理由をコメントに明記）。
9. Critical/High: なし。Medium 1件（`execute_peace_settlement`占領正常化の既存の無差別仕様、本タスクの対象外）、Low 1件（PRE-001報告書の宣戦当日スキップ記述の訂正）。次の原子的タスクは§8参照。
10. `cargo test --all-targets -j 2` → 2755 passed / 0 failed / 1 ignored。`cargo clippy --all-targets --all-features -- -D warnings` → 警告0件。両コマンドとも逐次実行。
11. `saves/savegame_v1.ron`は不変（本タスクでは一切参照・変更していない）。コミット対象は`strategy_game/src/war/update.rs`、`strategy_game/tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs`、`strategy_game/tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`、および本報告書の4ファイル。最終`git status --short`は本報告書末尾に別途追記する。

P27-005の手動プレイ確認は本タスクでも未実施であり、引き継ぎ事項として明記する。
