# P32-017 規則改訂(`ECONOMY_RULES_REVISION`)と判定表の運用メモ(開発用)

## 1. 現在の管理方法

- `ECONOMY_RULES_REVISION`(`src/economy/map_state/snapshot.rs`)は **手で上げる整数**。現在値は 1。
- スナップショットは `rules_revision` として保存する。さらに P32-017 から、同じ値を識別情報の項目
  `economy_rules` にも入れる(項目別の拒否理由として表示するため)。
- **コード上の式・定数の変更は自動検出できない。** ソース全体やバイナリのハッシュは使わない
  (無関係な変更まで互換性違反にしないため)。したがって、下の「改訂番号を上げる対象」を変えた人が
  同じ変更の中で番号を上げる運用に依存する。上げ忘れは検出されない(保証しない範囲)。

## 2. 改訂番号を上げる対象(再開した結果が、同じ保存状態・同じ静的入力から変わりうる変更)

| 対象 | 場所 |
|---|---|
| 日次処理の順序・内容(建設→月次経済→履歴記録→AI評価) | `economy/map_state.rs` の `run_day` 周辺 |
| 建設の注文受付・支払い・進捗・完成・取消し・返金(返金率 0.5 を含む) | `economy/map_state.rs` |
| 月次経済の式(雇用・物流・稼働率・生産・財政・人口増加・経済状態の判定) | `economy/state_access.rs`・`production.rs`・`finance.rs`・`economic_state.rs`・`population::growth` |
| 履歴の窓幅(供給4・鉄鉱不足4・工場飢餓4)と履歴の記録規則 | `military/supply_production_history.rs`・`economy/iron_shortage.rs`・`economy/factory_starvation.rs` |
| AIの評価周期・優先度・判断規則 | `country/country_ai.rs`・`country/ai_military.rs` と、`economy/map_state.rs` のAI評価 |
| 入力の許容範囲を変えて、既存の保存状態の意味が変わる検証(`snapshot.rs` の復元時検証) | `economy/map_state/snapshot.rs` |
| 保存対象の項目の追加・削除・意味の変更 | `snapshot.rs`(あわせて `SNAPSHOT_SECTION_VERSION` も上げる) |

建物定義(`assets/data/buildings.ron`)の値の変更は規則の改訂ではなく静的入力の変更で、`building_computation`
で検出して拒否する。

## 3. 識別情報の方式の版(`IDENTITY_SCHEME_VERSION`)を上げる対象

- 項目の追加・削除・名前の変更、または正規化のバイト列の変更(`static_identity.rs` の `compute_item`)。
- ファイルの `format_version` も同時に見直す(項目の追加は「欠落した項目」として旧ファイルを拒否する)。

## 4. 「表示用」だった値を計算に使うようになったとき

判定表(`static_identity::RULES`)の `Warn` 項目は、**現在の保存対象の計算が読まない**ことを根拠にしている。
次の変更を入れるときは、同じ変更で必ず:

1. その項目の `class` を `Reject` へ変える(または、計算に使う部分だけを別の項目へ分けて `Reject` にする)。
2. 内部コンテキスト(`snapshot.rs` の `context_id`)にその入力を加える。
3. `ECONOMY_RULES_REVISION` を上げる(新しい式で結果が変わるため)。
4. テスト `the_judgement_table_covers_every_item_once_and_the_warn_set_is_the_enumerated_list` の期待する警告項目の
   一覧を更新し、`warn_class_changes_restore_with_warnings_...` の対象から外す。

対象になりうる例: 地形・気候・植生(補給・移動・戦闘・人口増加に使うとき)、河川(補給・移動に使うとき)、
位置施設(港・要塞・鉄道駅が補給・貿易・防御・費用に影響するとき)、ステートの代表プロヴィンス(経済上の拠点に使うとき)、
建物の表示名(識別子として使うとき)。

## 5. 残る制約・責務の変更(今回は実装しない)

- 鉱床の発見・開発が実行時に変わるようになる場合、`discovered`・`development_level` は静的定義ではなく
  動的状態になる。そのとき、保存対象(動的状態)と静的入力(`resource_deposits`)の責務を分け直す
  必要がある(今回は新しい発見システムを実装しない)。
- 識別情報は内容識別であり、電子署名ではない。ファイルを書き換えられる者は、自分で計算した値を書ける。
  改ざん検知や作成者の認証が必要になった場合は別の仕組みが必要。
- 位置施設の入力は、呼び出し側が渡した場合だけ比べる(渡さない保存・渡さない復元は「提供の有無」の差として
  警告になる)。
