# P27-FU-002 攻撃側撤退先検証・最終統合監査 報告書

本報告書は、P27-FU-001で実装した攻撃側撤退先の検証(有効origin優先→隣接代替探索→撤退不能時消滅)を、Phase 25〜27の戦闘・補給・装備・国家消滅・講和パイプラインへ統合した最終監査の報告書である。これは監査タスクであり、原則として本番コード・本番データ・既存テストは変更していない。

## 1. 総合判定

**P27-FU-002 AUDIT COMPLETE**。

## 2. P27-FU-001統合判定

**P27-FU-001 INTEGRATION CONFIRMED**(リグレッションなし)。

## 3. Phase 27基準状態判定

**PHASE 27 BASELINE READY**。

## 4. 次フェーズ準備判定

**NEXT PHASE READY**。Phase 27の再オープンは不要。

## 5. 開始HEAD

`ee420d97bd12ed382eb74d34a349aff3a792ca43`(想定と一致)。開始時`git status --short`は空。

## 6. 監査対象HEAD

`ee420d97bd12ed382eb74d34a349aff3a792ca43`(監査中は本番コード・データ・既存テストを一切変更していないため、開始HEADと同一)。

## 7. ユーザーSave保護

`saves/savegame_v1.ron`の読み書き・復元・ステージは一切行っていない。

## 8. 現行撤退経路(静的監査)

`rg`で全呼び出し元を確認した。

```text
resolve_finished_battles(update.rs)
├─ P27-007ゼロ損害枝: attacker_origins.get()
│    → attacker_retreat_destination(battle_state, attacker_country, origin, ...)
│         ├─ originが有効 → Some(origin)
│         └─ 無効/欠落 → neighboring_retreat_candidate(battle_state, ...)
│                          ├─ 候補あり → Some(隣接自国州の最小StateId)
│                          └─ 候補なし → None
│    → force_retreat_or_destroy(division_id, dest, ...)
│
├─ 通常組織率0枝(攻撃側): 同じattacker_retreat_destination呼び出し
│    → resolve_participant_outcome → force_retreat_or_destroy(共通)
│
└─ 通常組織率0枝(防御側): find_retreat_destination(既存、無変更)
     → neighboring_retreat_candidate(共通ヘルパー、防御側と共有)
     → resolve_participant_outcome → force_retreat_or_destroy(共通)
```

`attacker_retreat_destination`・`find_retreat_destination`・`neighboring_retreat_candidate`・`force_retreat_or_destroy`の本番呼び出し元は、`src/military/update.rs`内のこの1箇所(2枝)のみであることを`rg`で確認した(テストコード以外に他の呼び出し元は存在しない)。共通APIを迂回する攻撃側撤退経路は見つからなかった。`resolve_finished_battles`は`Battle`ごとに逐次処理され、`process_daily_battles`→`resolve_finished_battles`の順で同一関数呼び出し内(別Systemではない)に実行されるため、同一日内での二重評価・競合は構造的に発生しない。

## 9. origin有効性契約

`state_registry.get(origin)`が`Some`、かつ`controller()==owner`、かつ`battle_registry.get_ongoing_battle_in_state(origin).is_none()`の3条件全てを満たす場合のみ有効。首都連結性・補給可能性は条件に含まない(意図的な分離、P27-FU-001-PRE-001の設計どおり)。実コード(`invasion.rs:364-379`)を再読し、既存11単体テストで各条件を個別に確認済み。

## 10. origin無効条件

次を個別のテストケースで確認した。

- 戦闘開始後に敵へ奪還された: `p27_006`test_14(E2E)、`p27_fu_001_ai_retreat_alternative`test_01/03(実AI)
- 戦闘開始時点から自国支配ではなかった: 単体テスト`attacker_retreat_destination_rejects_an_origin_controlled_by_the_enemy`・`...by_a_third_country`
- 州が存在しない合成StateId: 単体テスト`attacker_retreat_destination_searches_alternatives_when_the_origin_state_does_not_exist`
- 進行中の他戦闘によって使用不能: 単体テスト`attacker_retreat_destination_searches_alternatives_when_a_battle_is_ongoing_at_the_origin`
- 国家消滅によって前提が変化: `p27_fu_001_elimination_peace`test_01/02(新規、§21参照)
- 講和またはcontroller変更によって前提が変化: `p27_fu_001_elimination_peace`test_03(新規、§22参照)

## 11. 代替探索契約

戦闘州(`battle_state`)を起点に、隣接州のうち`controller()==owner`かつ進行中の別Battleがない州をStateId昇順で1件選ぶ。防御側`find_retreat_destination`と完全に同じ`neighboring_retreat_candidate`を共有しており、コードの複製はない。

**本監査で新たに直接確認した項目**(一時診断テストで検証、コミットには含めない): 代替候補の中で最小StateIdの州に進行中の別Battleがある場合、その州を正しくスキップして次に小さいStateIdの州を選ぶことを、攻撃側`attacker_retreat_destination`と防御側`find_retreat_destination`の両方で確認した(結果はいずれもState2、State1はスキップ)。この分岐(「originではなく代替候補自体が進行中の別Battleで除外される」)は、Phase 15由来の`find_retreat_destination`にも、P27-FU-001の新規11単体テストにも、恒久的な回帰テストが存在しないことが判明した(§38のLow findingとして記録)。

## 12. 退路なし契約

`attacker_retreat_destination`が`None`を返した場合、`force_retreat_or_destroy`の既存消滅処理(装備全損→`remove_division`)がそのまま適用される。この関数自体はP27-FU-001で一切変更されておらず、組織率0による脱落・両陣営ゼロ損害による強制撤退の両方から共有される既存契約のままである。`p27_006`test_12・`p25_007`各テスト等で無変更の通過を確認した。

## 13. 防御側契約の保護

`find_retreat_destination`は`neighboring_retreat_candidate`へ委譲するようリファクタされたのみで、判定条件・優先順位は無変更。単体テスト`find_retreat_destination_result_is_unchanged_by_sharing_the_candidate_helper`で直接確認済み。`p26_005`・`p26_006`等、防御側撤退に関わる既存テストは全件無変更で通過(§18参照)。

## 14. 既存13テストの個別分類(再監査)

P27-FU-001報告書の分類を実コード・実テストと突き合わせて再確認した。

| ファイル | 分類 | 再確認結果 |
|---|---|---|
| `p25_004`test_y | A | `create_battle`が`attacker_origin`引数を受け取り`player_home`を使用。fixtureのみの変更、契約の弱体化なし |
| `p25_006_rev_001`(5件) | A | `create_battle`が`attacker_origin`引数を追加。装備会計の検証目的は無変更 |
| `p25_007`(3件) | A | `create_battle`内部で`state_owned_by(app, attacker_country)`を使用。呼び出し元は無変更 |
| `p26_005`test_d5 | A | 同上パターン |
| `p26_006`test_fully_disconnected | A | `ARCADIA_CAPITAL`をoriginに使用(戦闘州自身の流用を修正) |
| `p27_006`test_12 | B | `pre_battle_controller_overrides`で唯一の既定代替も無効化、真の「代替なし」を構築 |
| `p27_006`test_11 | D | `pre_battle_controller_overrides`でStateId(4)を戦闘開始前にArcadia支配へ変更(実データの既定controllerを正しく反映) |

分類Cは0件(既存テストで無効origin挙動自体を専用に検証していたものはなかった)。単純な期待値反転は1件もなく、いずれも「fixtureが実際にどの州をorigin/代替として構築していたか」を実データ突き合わせで個別に修正したものであることを確認した。合成州と実マップ州の混同は残っていない。`land_war_combat_peace_test.rs`は無変更(§18で確認)。

## 15. 制御シナリオ結果(§7の20項目)

既存・新規テストで以下のとおり網羅を確認した(単体=`invasion.rs`、E2E=各テストファイル)。

1. 有効originへ正常撤退 — 単体`...uses_the_valid_origin...`、E2E`p27_006`各所
2. originが戦闘中に敵へ奪還→代替撤退 — `p27_006`test_14、実AI`p27_fu_001_ai_retreat_alternative`test_01/03
3. 開始時から無効なorigin→代替撤退 — 単体`...rejects_an_origin_controlled_by_the_enemy/a_third_country`(代替へフォールバックする分岐を同時に確認)
4. origin無効・代替なしで消滅 — `p27_006`test_12
5. 複数攻撃Divisionが個別originへ撤退 — `p27_006`test_11
6. 複数Divisionの一部だけorigin無効 — `p27_006`test_15
7. origin・代替候補複数時の決定性 — 単体`...picks_the_smallest_state_id_among_multiple_alternatives`
8. ゼロ損害終了による強制撤退 — `p27_007`test_01、`p27_006`test_08
9. 組織率0による既存撤退 — `p25_004`test_y等多数
10. 通常の戦闘敗北による撤退 — 上記と同一経路(`resolve_participant_outcome`)
11. 攻撃側国家の消滅 — `p27_fu_001_elimination_peace`test_01
12. 戦争終結・講和との同日競合 — `p27_fu_001_elimination_peace`test_02/03。実際の`DailySimulationSet`順序(`MilitaryAction`→`WarResolution`、§16で確認)により、講和・国家消滅の自動処理は常に同日の戦闘解決より後に実行されるため、両テストは実際の同日実行順を正しく再現している
13. 無関係な第三国への非影響 — `p27_fu_001_ai_retreat_alternative`test_02
14. 防御側撤退結果の非回帰 — 単体`find_retreat_destination_result_is_unchanged_by_sharing_the_candidate_helper`
15. save→load(撤退前・当日・後) — `p27_006`test_17(recapture=当日、no_retreat=day1/3)、`p25`/`p26`系各ファイルのsave/loadテスト
16. 2回連続save→load — `assert_save_load_matches`の`save_twice`分岐で全対象dayに対して実施済み(`p27_006`test_10・test_17)
17. 代替候補が敵支配州しか存在しない場合 — test_12と等価(候補0件)
18. 代替候補が進行中の別戦闘州しか存在しない場合 — 本監査の一時診断で直接確認(§11参照、恒久テストなしがLow finding)
19. originは有効だが代替候補も存在する場合 — 単体`...uses_the_valid_origin_and_does_not_search_alternatives`(originが優先されることを確認)
20. 合成StateIdをoriginへ渡した場合 — 単体`...searches_alternatives_when_the_origin_state_does_not_exist`

## 16. Division状態整合性

`resolve_finished_battles`は`battle_ids`をスナップショットしてから逐次処理し、各Battleの`attacker_division_ids`/`defender_division_ids`は`remaining_attackers`/`remaining_defenders`で明示的に再構築される。`force_retreat_or_destroy`は撤退時に`combat_id=None`を設定し、消滅時は`remove_division`で`MilitaryRegistry`から即座に除去するため、Division複製・同一Divisionの複数州存在・Battle参加者への消滅済みDivision残留は構造的に発生しない。`p27_006`test_16(共有代替先での消失・重複なし)で直接確認済み。DailySimulationSetの`MilitaryAction`は`WarResolution`より先に実行されるため、`resolve_finished_battles`完了後にBattleが空でなければ次のSystem(`update_war_occupied_states`等)へ正しい状態で引き継がれる。

## 17. manpower保存則

`force_retreat_or_destroy`はP27-FU-001で変更されておらず、`RETREAT_MANPOWER_LOSS_RATIO`(10%)による追加損失と`saturating_sub`によるclamp後の`actual_casualty`を装備損失計算へ使う既存式のまま。`p25_006_rev_001`の装備会計テスト群で無変更の通過を確認。

## 18. Equipment保存則

`calculate_proportional_equipment_loss`・`try_destroy_equipment`はいずれも無変更。消滅時は必ず`try_destroy_equipment(remaining_equipment)`で全損させてから`remove_division`する既存の共通不変条件(P25-006-REV-001由来)を、代替探索経由の撤退・消滅の両方が引き続き満たすことを`p27_fu_001_elimination_peace`test_01(消滅時解散件数の一致)で確認した。国家備蓄には一切触れない(既存契約どおり)。

## 19. controller整合性

`attacker_retreat_destination`・`neighboring_retreat_candidate`はいずれも状態を変更しない純粋関数であり、controller変更は行わない。攻撃側勝利時の占領(`occupy_state`)は`resolve_finished_battles`内の別分岐(§8の呼び出し経路参照)であり、撤退処理とは独立している。防御側勝利時のcontroller契約(変更なし)も無変更で維持されている。

## 20. Battle削除

`resolve_finished_battles`自体はBattleを削除しない(`BattleStatus`を更新するのみ)。既存の`cleanup_finished_battles`(無変更)が別途removeする既存契約のまま。`p27_fu_001_elimination_peace`test_02で、防御側国家消滅後に当該国が関与する進行中Battleが残らないことを確認した。

## 21. 国家消滅

新規`p27_fu_001_elimination_peace_e2e_test.rs`で動的に確認した。

- **攻撃側消滅**(test_01): 代替探索経由で撤退し生存していたDivisionが、消滅処理により他のDivisionと同様に一度だけ解散される(`divisions_disbanded`件数が消滅直前の所有Division総数と一致)。復活なし。消滅済み国家への再適用はエラーとなり、二重処理は起きない。
- **防御側消滅**(test_02、戦闘継続中): `country::elimination::apply_country_elimination`をそのまま呼び出すだけで、当該国が防御側だったWarが消滅処理の一部として終結/除去され(`other_wars_ended`/`wars_supporter_removed`)、以後の日次処理でパニックしない。

既存の`country::elimination`公開APIは一切変更していない。

## 22. 講和

新規test_03で、代替探索経由で撤退し生存していたDivisionが存在する状態で白紙講和(`PeaceTerm::WhitePeace`)を成立させ、次を確認した。撤退時に適用済みのmanpower/equipment損失が再適用されない(二重適用なし)。既存の講和処理§2「占領状態の正常化」(占領のみだった代替撤退先の支配が法的所有国=Elfinへ戻る、無変更の既存ロジック)と§5「敵国領残存陸軍の帰還」(`relocate_hostile_territory_divisions`、無変更)により、Divisionは敵領へIdleのまま取り残されず、Arcadia自身が支配/所有する陸州(第三国領ではない)へ正しく帰還する。既存の`war::peace`公開APIは一切変更していない。

## 23. P27-004回帰

`tests/p27_004_idempotent_movement_orders_e2e_test.rs`: 6 passed、無変更。`rg`で`start_or_maintain_movement`等の移動APIへの参照が本タスクの変更ファイル(`invasion.rs`・`update.rs`)に存在しないことを確認し、攻撃側撤退修正が移動APIを迂回・破壊していないことを構造的に確認した。

## 24. P27-005回帰

`tests/p27_005_offensive_frontline_supply_bridge_e2e_test.rs`: 10 passed、無変更。`daily_supply.rs`に撤退関連関数への参照は存在せず(`rg`で確認)、攻撃側前線補給資格判定は無変更。

## 25. P27-006回帰

`tests/p27_006_low_effective_power_damage_e2e_test.rs`: 17 passed(test_11・test_12はP27-FU-001でfixture修正済み、それ以外は無変更)。`combat_calc.rs`に撤退関連関数への参照は存在せず(`rg`で確認)、M・T・Cの損害計算式は無変更。

## 26. P27-007回帰

`tests/p27_007_zero_damage_battle_termination_e2e_test.rs`: 2 passed、無変更。両陣営ゼロ損害時の即日防御側勝利・攻撃側撤退契約はそのまま維持され、撤退先の決定だけがP27-FU-001の新関数経由になっている(§8参照)。再攻撃ループの非形成は`p27_fu_001_ai_retreat_alternative`test_01/03で代替探索を伴うシナリオでも再確認済み。

## 27. P27-008統合結果

P27-008が確認した根本原因(戦闘継続中に出撃元が敵支配へ変わってもorigin検証なしで撤退していた)は、`p27_006`test_14で直接再現・解決を確認済み(P27-FU-001時点)。本監査では、移動→到着→前線補給→戦闘開始→実損害→組織率撤退→回復→再攻撃→占領→追撃という統合経路のうち、撤退判定部分がP27-FU-001の新関数に置き換わったこと以外に経路構造の変更がないことを、§8の呼び出しグラフと全File横断`rg`検索で確認した。P27-008自身の150日診断相当の再実行は本監査では行っていない(全体テストスイート・個別ファイルの無変更通過で経路の健全性を代替確認)。

## 28. Save V1／V2判断

**Save V1を維持している。新規永続フィールドは追加されていない。** `git show ee420d9`の構造体差分を確認し、`struct`/新規`pub`フィールド追加が0件であること、`src/save/`配下のファイルが変更対象に含まれていないことを直接確認した。構文互換性(旧Saveがパースできる)と意味的互換性(パース後の値が正しく解釈される)はいずれも影響を受けていない——`Battle.attacker_origins`自体はPhase 21から存在する既存フィールドであり、P27-FU-001はその「使い方」を変えただけで、フィールド自体もSave DTOも変更していない。

## 29. save/load分岐

`p27_006`test_17で、代替探索(recapture)シナリオの当日(day1)と、退路なし消滅シナリオの前後(day1・day3)の両方について、単発save/load・2回連続save/loadの双方が連続実行と一致することを確認した(§15項目15・16参照)。

## 30. 決定性

- StateId昇順タイブレーク: 単体テスト7番目
- HashMap/Vecイテレーション順序非依存: 単体テスト10番目
- 登録順反転: `p27_006`test_13(無変更)
- 独立プロセス5回: 本監査で`p27_004`〜`p27_fu_001`系8ファイルを5回連続実行し、全回で件数一致を確認(§36)

## 31. 国家間分離

`p27_fu_001_ai_retreat_alternative`test_02で、無関係な第三国(Passguard)の備蓄・Division状態(位置・manpower・equipment・status)がいずれも影響を受けないことを確認した。

## 32. 90日結果

`p27_fu_001_ai_retreat_alternative`test_01/03で、実App・実AI経由で90日間観測。代替探索を伴う撤退後も再攻撃ループは形成されず、終了は1回だけだった(P27-007由来のゲート挙動が新しいcooldownなしで維持されることを確認)。

## 33. 36か月結果

`P27-002-C`の36か月チェックポイントを含む六か国長期テスト(16件)は本監査でも無変更で全通過。

## 34. 120か月結果

`P27-002-C`の120か月六か国テストは無変更で全通過し、正規値と一致した。同テストは`Battle`型を一切参照しておらず(`rg`で確認)、既存Phase 27各タスクで独立に再確認されてきた「自然な通常AI運用では戦闘自体がほぼ発生しない」という知見と構造的に整合する(撤退関連関数への参照が`daily_supply.rs`/`combat_calc.rs`/`frontline.rs`等のいずれにも存在しないことは§8で確認済みであり、これらのシステムがBattleを生成しない限り新しい撤退経路も評価されない)。

## 35. 自然発火と制御シナリオの区別

自然な120か月運用での新しい代替探索経路の発火回数は、構造的に0回である(Battleが自然発生しないため)。この「0件」自体を修正成立の証拠とはせず、§10・§15・§21・§22の制御下シナリオ(必要に応じて人為的な第二橋頭堡等を用いて構築)を主証拠として扱った。

## 36. 単一回廊Low finding

P27-FU-001で確認された「実マップ前線(StateId(3))はArcadia側隣接州が1つ(StateId(1))だけの単一回廊構造」について、次のとおり判定する。

**判定: 現行マップ設計上の制約**(実装不具合ではない)。理由: `attacker_retreat_destination`・`neighboring_retreat_candidate`のロジック自体は任意個数の隣接自国州を正しく扱える(単体テストで複数候補時の決定性を確認済み)。制約は純粋に現在の州データ(`neighbors`・`owner_country_id`)の静的な構成に由来する。この前線が実プレイで拡張される可能性(隣接州の追加占領等)は既存の占領システムで自然に起こり得るため「将来のマップ拡張で自然に解消し得る事項」の側面もあるが、現時点でこれを「独立したマップ設計タスクが必要」と断定するか「意図的に維持すべき事項」とするかはゲームデザイン上の判断であり、本監査の範囲外である。州接続・controller・地形データは変更していない。

## 37. P27-005手動確認未実施

**P27-005の手動プレイ確認は本監査でも実施していない。** 本監査で実施したのは、自動テスト検証(cargo test)・実App制御シナリオ検証(SystemState経由の直接呼び出し)・実App自然AI運用検証(90日/36/120か月)のみであり、人間による手動プレイでの確認は一切行っていない。この事実は、本監査の自動検証完了判定(P27-FU-002 AUDIT COMPLETE)とは独立した、別個の未完了事項として明記する。

## 38. Critical／High／Medium／Low

- Critical: 0
- High: 0
- Medium: 0
- Low: 2
  1. (P27-FU-001から継続、再評価済み)実マップ前線が単一回廊構造のため、実AI統合だけでは「代替が見つかって生存する」経路の自然な到達が非常に限定的。ゲームデザイン上の制約であり実装不備ではない(§36)。
  2. (本監査で新規発見)`neighboring_retreat_candidate`の「最小StateIdの代替候補自体が進行中の別Battleで除外され、次善候補が選ばれる」分岐(§7項目18)に、恒久的な回帰テストが存在しない(Phase 15由来の`find_retreat_destination`にも、P27-FU-001の新規単体テストにも無し)。本監査の一時診断で実際の挙動(正しくスキップされる)は直接確認済みであり、機能上の不具合ではないが、テストカバレッジの穴として次の原子的タスク候補に記録する。

## 39. Phase 27再オープン要否

**不要。** 見つかった2件のLow findingはいずれも「機能上の不具合」ではなく、「マップ設計上の制約」と「テストカバレッジの穴」であり、Phase 27の契約・完了判定を無効化しない。

## 40. 次の原子的タスク

**P27-FU-003(候補、未着手): `neighboring_retreat_candidate`の「最小候補が進行中Battleで除外される」分岐に対する恒久的単体テストの追加。** 攻撃側(`attacker_retreat_destination`経由)・防御側(`find_retreat_destination`経由)の両方について、最小StateIdの候補に進行中Battleがある場合に次善候補が選ばれることを検証する単体テストを`src/military/invasion.rs`の`mod tests`へ追加する。実装コードの変更は不要(既に正しく動作していることを本監査で確認済み)。優先度Low。

## 41. 実行コマンドと正確な結果

Cargoは全て逐次実行した(並行実行なし)。Windowsのページファイル不足を避けるため、全体実行のみ`-j 2`を使用した。

- `cargo test --lib military::invasion`: 18 passed
- `cargo test --lib military::update`: 35 passed
- `cargo test --lib military::movement`: 12 passed
- `cargo test --lib military::daily_supply`: 44 passed
- `cargo test --lib military::combat_calc`: 75 passed
- `cargo test --test p27_fu_001_ai_retreat_alternative_e2e_test`: 3 passed
- `cargo test --test p27_fu_001_elimination_peace_e2e_test`: 3 passed
- `cargo test --test p27_007_zero_damage_battle_termination_e2e_test`: 2 passed
- `cargo test --test p27_006_low_effective_power_damage_e2e_test`: 17 passed
- `cargo test --test p27_005_offensive_frontline_supply_bridge_e2e_test`: 10 passed
- `cargo test --test p27_004_idempotent_movement_orders_e2e_test`: 6 passed
- `cargo test --test p26_008_b_supply_aware_offensive_gate_e2e_test`: 19 passed
- `cargo test --test land_war_combat_peace_test`: 4 passed(無変更)
- `cargo test --test p25_004_combat_equipment_loss_e2e_test`: 5 passed
- `cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test`: 8 passed
- `cargo test --test p25_007_equipment_combat_effectiveness_e2e_test`: 21 passed
- `cargo test --test p26_005_supply_combat_strength_e2e_test`: 10 passed
- `cargo test --test p26_006_capital_connectivity_supply_e2e_test`: 18 passed
- `cargo test --all-targets --no-fail-fast -j 2`: **2706 passed / 0 failed / 1 ignored**(82バイナリ)
- `cargo clippy --all-targets --all-features -- -D warnings`: 警告0

上記のうち`p27_004`〜`p27_fu_001`系8ファイルを独立プロセスで5回連続実行し、全回で件数一致・0 failedを確認した(§30参照)。

**注記(§38とは別の、事務的な訂正)**: P27-FU-001報告書は最終テスト件数を「2706 passed / 0 failed / 0 ignored」と記載していたが、本監査で実測したところ実際は**1 ignored**(`profile_p21_021_elimination_scaling`、Phase 21由来の既存プロファイリング専用テスト、`#[ignore]`属性付きで長らく存在、P27-FU-001の変更とは無関係)であることが判明した。開始時ベースライン(§1参照)・本監査中の全実行のいずれでも一貫して1 ignoredであり、リグレッションではなく、P27-FU-001報告書側の転記ミスである。

## 42. 最終`git status --short`

コミット後は空(本報告書のみをステージ・コミットしたため)。
