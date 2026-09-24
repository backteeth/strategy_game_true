# P27-FU-003 進行中Battleを回避する代替退却先選択の恒久テスト追加 報告書

本報告書は、P27-FU-002の最終統合監査で特定された最後のLowテストカバレッジ不足(`neighboring_retreat_candidate`で「最小StateIdの候補が進行中Battleにより除外され、次善候補が選ばれる」分岐に恒久テストがない)を、P27-FU-003として恒久テストへ固定した報告書である。本番ロジックは変更していない。

## 1. 総合判定

```text
P27-FU-003 COMPLETE
RETREAT FALLBACK TEST ADDED
PRODUCTION CODE UNCHANGED
PHASE 27 BASELINE READY
NEXT PHASE READY
```

P27-FU-002のLow finding(代替候補のBattle除外分岐のテスト不足)は解消済みと判定する。

## 2. 開始HEAD

`a91ac1a7d93e2033d09e7a90cdd55fd49d541dd8`(想定と一致)。開始時`git status --short`は空。

## 3. 監査対象コードHEAD

`a91ac1a7d93e2033d09e7a90cdd55fd49d541dd8`。本番コードはこのHEADから変更していない。

## 4. P27-FU-002から引き継いだLow finding

`neighboring_retreat_candidate`の「最小StateIdの代替候補自体に進行中Battleがあり除外され、次善候補が選ばれる」分岐に恒久的な回帰テストがない。P27-FU-002では一時診断で正しく動作することを確認済みだったが、テストとして固定されていなかった。

## 5. 現行候補選択契約

`src/military/invasion.rs`の`neighboring_retreat_candidate(current_state, owner, state_registry, battle_registry)`(private)を再読して確認した。

1. `current_state`が`StateRegistry`に存在しなければ`None`。
2. `current_state`の`neighbors`だけを走査する(非隣接州は候補にならない)。
3. 隣接州が`StateRegistry`に存在しなければ除外。
4. `neighbor.controller() != owner`なら除外。
5. `battle_registry.get_ongoing_battle_in_state(neighbor_id).is_some()`なら除外。
6. 残った候補を`StateId`昇順に並べ、先頭を返す(なければ`None`)。

呼び出し元は防御側`find_retreat_destination`(`current_state`起点)と攻撃側`attacker_retreat_destination`(origin無効時、`battle_state`起点)の2つ。`controller()`は`controller_country.unwrap_or(owner_country_id)`。

## 6. 既存テストの棚卸し

`rg -n "neighboring_retreat_candidate|attacker_retreat_destination|find_retreat_destination" src tests`で確認した。候補選択に関わる既存恒久テストは以下。

- `military::invasion::tests::test_find_retreat_destination_deterministic`
- `military::invasion::tests::find_retreat_destination_result_is_unchanged_by_sharing_the_candidate_helper`
- `military::tests::test_phase15_retreat_is_deterministic`
- `military::invasion::tests::attacker_retreat_destination_*`(P27-FU-001の11件)

いずれも「候補自体に進行中Battleがあって除外される」ことを直接の対象にしていなかった。ただし一時的なミューテーション検証(§14参照)で、`attacker_retreat_destination_searches_alternatives_when_a_battle_is_ongoing_at_the_origin`がこの分岐を**間接的に**通過していることが分かった(origin州1がBattleで無効になった後、代替探索で同じ州1がBattleで除外される)。ただしこのテストは除外される候補がoriginと同一であり、除外理由が「origin無効」と「候補除外」のどちらか一意に特定できないため、専用テストとしては不十分と判断した。

「全候補がBattle中なら`None`」の恒久テストは存在しなかった(`attacker_retreat_destination_returns_none_when_no_alternative_exists`は全候補を敵controllerにして`None`を確認するもので、Battle除外による`None`ではない)。このため§6の条件に従いtest_02も追加した。

## 7. 追加したテスト名

`src/military/invasion.rs`の既存`#[cfg(test)] mod tests`内に追加した。

- `military::invasion::tests::retreat_candidate_skips_smaller_state_id_with_ongoing_battle_and_picks_next`(test_01)
- `military::invasion::tests::retreat_candidate_is_none_when_every_friendly_neighbor_has_an_ongoing_battle`(test_02)

新規E2Eファイルは作成していない(対象関数を単体テストで直接呼び出せるため)。

## 8. fixture構造

テスト用定数と2つのヘルパーを`mod tests`内に追加した。

- `FU003_OWNER = CountryId(1)`(撤退する国家)、`FU003_ENEMY = CountryId(2)`
- `fu003_states()`: 戦闘州・候補A・候補Bの3州だけを持つ`StateRegistry`。全州で`owner_country_id`と`controller_country`を明示し、既定値へ依存しない。戦闘州の`neighbors`は`[候補B, 候補A]`(StateId降順)、`StateRegistry::build`への登録順も`[候補B, 候補A, 戦闘州]`と意図的に未整列にした。
- `fu003_ongoing_battle_at(battle_registry, state_id)`: 指定州に`BattleStatus::Ongoing`のBattleを`BattleRegistry::start_battle`経由で登録する(BattleIdはレジストリが採番)。

Divisionは、防御側`find_retreat_destination`がDivisionの所有国を引くために必要な1件だけを作成した。戦争状態は構築していない。

## 9. 戦闘州

`StateId(10)`(owner・controllerとも`FU003_ENEMY`)。

## 10. 最小StateId候補

候補A = `StateId(11)`。owner・controllerとも`FU003_OWNER`、戦闘州の隣接州。通常なら有効な撤退先。

## 11. 次善候補

候補B = `StateId(12)`。owner・controllerとも`FU003_OWNER`、戦闘州の隣接州、Battleなし。

## 12. 進行中Battleの配置

- test_01: 候補A(`StateId(11)`)のみにOngoingのBattle。
- test_02: 候補A(`StateId(11)`)と候補B(`StateId(12)`)の両方にOngoingのBattle。

候補Aの除外理由はBattleだけで、敵controller・州不存在・非隣接・国家不一致などは混ぜていない。test_01の冒頭で「Battleがなければ候補Aが選ばれる」ことを前提条件として同じfixtureで確認しており、候補Aが除外される原因がBattle以外にないことをテスト自身が保証している。

## 13. 実際の戻り値

test_01:

| 呼び出し | Battle | 結果 |
|---|---|---|
| `neighboring_retreat_candidate`(前提確認) | なし | `Some(StateId(11))` |
| `neighboring_retreat_candidate` | 候補Aのみ | `Some(StateId(12))` |
| `attacker_retreat_destination`(origin=`None`) | 候補Aのみ | `Some(StateId(12))` |
| `find_retreat_destination` | 候補Aのみ | `Some(StateId(12))` |

test_02: `neighboring_retreat_candidate` → `None`。

失敗時メッセージには期待候補と除外理由(例: "expected candidate B(12) because candidate A(11) has an ongoing battle")を含め、`assert_eq!`の左右値と合わせて実際の候補が判別できるようにした。

## 14. 入力順非依存

fixtureの隣接リスト・レジストリ登録順をStateId降順にしたうえで、test_01が候補Bを正しく選ぶことを確認した(内部のStateId昇順ソートによる決定的選択が入力順に依存しないこと)。

**ミューテーション検証**(一時的、既に元へ戻した): 本番の`neighboring_retreat_candidate`のBattle除外条件を一時的に無効化(`if false && ...`)して`cargo test --lib military::invasion`を実行したところ、追加した2件がいずれも失敗し(他に上記の既存1件も失敗、合計17 passed / 3 failed)、追加テストがこの分岐を実際に守っていることを確認した。変更はEditで元に戻し、`git diff -U0`でハンクが`mod tests`内の追加1件のみであることを確認した。

## 15. 全候補Battle中契約の既存カバー有無

既存の恒久テストには存在しなかった(§6参照)。test_02として追加した。

## 16. 防御側非回帰

`find_retreat_destination`は無変更。既存の`test_find_retreat_destination_deterministic`・`find_retreat_destination_result_is_unchanged_by_sharing_the_candidate_helper`・`military::tests::test_phase15_retreat_is_deterministic`はいずれも無変更で通過。test_01では防御側経路でも候補Aを除外し候補Bを選ぶことを直接確認した。

## 17. origin優先契約の保護

`attacker_retreat_destination`は無変更。既存の`attacker_retreat_destination_uses_the_valid_origin_and_does_not_search_alternatives`(有効originが代替候補より優先される)は無変更で通過。

## 18. 退路なし契約の保護

`force_retreat_or_destroy`は無変更。既存の`attacker_retreat_destination_returns_none_when_no_alternative_exists`と`tests/p27_006_low_effective_power_damage_e2e_test.rs`のtest_12(代替なし→消滅)は無変更で通過。test_02でBattle除外により候補が尽きた場合も`None`が返り、呼び出し側の既存撤退不能時処理へ委ねられることを確認した。

### §8で求められた既存契約の対応表

| 契約 | 固定しているテスト |
|---|---|
| Battleのない有効な最小StateId候補が選ばれる | `attacker_retreat_destination_picks_the_smallest_state_id_among_multiple_alternatives`、`attacker_retreat_destination_searches_alternatives_when_origin_is_missing`、test_01の前提確認 |
| 敵controller州は候補にならない | `attacker_retreat_destination_rejects_an_origin_controlled_by_the_enemy`(敵controllerの州3は常に選ばれない)、`attacker_retreat_destination_returns_none_when_no_alternative_exists` |
| 非隣接州は候補にならない | 専用の恒久テストはない。実装が`current_state_data.neighbors`だけを走査するため構造的に保証されている(本タスクの対象外として範囲を広げなかった、§22参照) |
| 全候補無効なら`None` | `attacker_retreat_destination_returns_none_when_no_alternative_exists`(敵controller)、test_02(Battle) |
| 有効originが代替候補より優先 | `attacker_retreat_destination_uses_the_valid_origin_and_does_not_search_alternatives` |
| 防御側契約無変更 | §16の3件 |
| 退路なし時の消滅処理無変更 | `p27_006`test_12 |
| StateId基準の決定性 | `attacker_retreat_destination_is_independent_of_hashmap_or_vec_iteration_order`、`test_find_retreat_destination_deterministic`、test_01(降順fixture) |

## 19. 本番コード無変更

`git diff -U0 -- src/military/invasion.rs`のハンクは`@@ -713,0 +714,106 @@ mod tests {`の1件のみ(純粋な追加106行、削除0行)。`#[cfg(test)] mod tests`は381行目から始まり、追加位置はその内部である。本番関数本体・public API・State/Battle/Division構造体・Save構造体・本番RONデータはいずれも変更していない。テスト用importの追加もない(既存の`use super::*`と既存importで足りた)。

## 20. Save V1維持

Save V1を維持。新規永続フィールドなし、migration不要、旧Save挙動変更なし。Save関連ファイルは変更していない。

## 21. 決定性

独立プロセスで`cargo test --lib military::invasion`を5回実行し、5回とも20 passed / 0 failed / 0 ignored、追加2件はいずれも毎回ok。テストは乱数・時間経過・浮動小数点比較に依存しない。

## 22. Critical／High／Medium／Low

```text
Critical: 0
High: 0
Medium: 0
Low: 1
```

- Low(P27-FU-001から継続、P27-FU-002で「現行マップ設計上の制約」と判定済み): 実マップ前線(StateId(3))が単一回廊構造で、代替退却先が自然に存在する状況が限定的。実装不具合ではなく、本タスクの範囲外。

P27-FU-002で特定したテストカバレッジ不足のLowは本タスクで解消した。

参考(findingとしては数えない): 「非隣接の自国州が候補にならない」ことを直接固定する専用テストはない。実装が隣接リストだけを走査するため構造的に保証されており、本タスクの中心契約から外れるためテストを追加しなかった。必要であれば同様のテスト追加タスクとして扱える。

## 23. Phase 27基準状態判定

**PHASE 27 BASELINE READY**。本タスクのコミットをPhase 27の最終基準HEADとする。

## 24. 次フェーズ準備判定

**NEXT PHASE READY**。Phase 27の追加監査・再オープンは不要。

## 25. 実行コマンドと正確な結果

Cargoは全て逐次実行した(並行実行なし)。全体テストは開始時基準と比較条件を揃えるため`-j 2`で実行した。

- `cargo test --lib military::invasion`: 20 passed / 0 failed / 0 ignored
- `cargo test --lib military::invasion::tests::retreat_candidate_skips_smaller_state_id_with_ongoing_battle_and_picks_next -- --exact`: 1 passed
- `cargo test --lib military::invasion::tests::retreat_candidate_is_none_when_every_friendly_neighbor_has_an_ongoing_battle -- --exact`: 1 passed
- `cargo test --test p27_fu_001_ai_retreat_alternative_e2e_test`: 3 passed
- `cargo test --test p27_fu_001_elimination_peace_e2e_test`: 3 passed
- `cargo test --test p27_007_zero_damage_battle_termination_e2e_test`: 2 passed
- `cargo test --test p27_006_low_effective_power_damage_e2e_test`: 17 passed
- `cargo test --test land_war_combat_peace_test`: 4 passed(無変更)
- `cargo test --all-targets --no-fail-fast -j 2`: 2708 passed / 0 failed / 1 ignored(82バイナリ)
- `cargo clippy --all-targets --all-features -- -D warnings`: 警告0
- `cargo test --lib military::invasion`を独立プロセスで5回: 全回20 passed / 0 failed / 0 ignored
- ミューテーション検証(一時、元へ戻し済み): 17 passed / 3 failed(追加2件を含む)

## 26. 全テストpassed／failed／ignored

**2708 passed / 0 failed / 1 ignored**(開始時2706 passed + 追加2件)。1 ignoredは既存の`profile_p21_021_elimination_scaling`(本タスクと無関係)。

## 27. clippy警告数

0。

## 28. ユーザーSave保護

`saves/savegame_v1.ron`の読み書き・復元・ステージは一切行っていない。`tests/land_war_combat_peace_test.rs`は無変更・未整形。

## 29. コミット対象

- `src/military/invasion.rs`(`#[cfg(test)] mod tests`内へのテスト追加のみ)
- `verification_logs/phase-27/p27-fu-003/p27-fu-003_retreat_candidate_fallback_test.md`(本報告書)

テスト実行で再生成された既知の非決定的PNG9件は、`git checkout -- <path>`で個別に復元し、コミットに含めていない。

## 30. 最終`git status --short`

コミット後は空。
