# P21-MAP-001 事前調査レポート: プロトタイプマップの州数拡張

日付: 2026-08-11
方針: 今回はコード・RONデータ・テスト・固定証拠を一切変更していません(調査のみ)。

---

## 1. 現在のマップ構造

- 州データ: `assets/data/states.ron`(201行、10州)
- 国家データ: `assets/data/countries.ron`(44行、4か国)
- 読み込み経路: `src/app/loader.rs::load_game_data` が起動時に両ファイルを`ron::from_str`で
  `Vec<StateData>`/`Vec<CountryData>`へデシリアライズし、`StateRegistry::build(states)`で
  `Vec<StateData>` + `HashMap<usize,usize>`(StateId→Vecインデックス)のO(1)ルックアップ構造へ変換する。
- `StateData`(`src/state/data.rs:13`)は1つの型で以下を**両方**保持している(詳細は§4):
  - 戦術的な移動単位としての情報: `id`、`neighbors: Vec<StateId>`、`world_position`、`size`、
    `controller_country`(占領時の実効支配国)
  - 行政的な州としての情報: `population`、`workforce_ratio`、`education`、`living_standard`、
    `unrest`、`buildings`、`resource_deposits`、`logistics_capacity`、`integration`
- `CountryData`(`src/country/mod.rs:83`)は`capital_state_id: StateId`で1州だけを首都として参照する。
- `StateId`/`CountryId`はいずれも`#[derive(..., Serialize, Deserialize)] struct XxxId(pub usize)`の
  ニュータイプ(他の全ID型と同じ設計)。

---

## 2. 現在の国家・州一覧

| CountryId | 国名 | 首都State | 所属州 |
|---|---|---|---|
| 0 | Kingdom of Arcadia | 0 | 0(首都), 1, 2 |
| 1 | Elfin Republic | 3 | 3(首都), 4 |
| 2 | Dwarf Federation | 5 | 5(首都), 6, 7 |
| 3 | Oceanic Magic Empire | 8 | 8(首都), 9 |

| StateId | 名称 | 所属国 | 隣接州 |
|---|---|---|---|
| 0 | Arcadia Capital | 0 | 1, 2, 5 |
| 1 | Northern Frontier | 0 | 0, 3, 8 |
| 2 | Western Mage Province | 0 | 0, 5 |
| 3 | Elfin Central | 1 | 1, 4, 5, 8 |
| 4 | Forest Research Zone | 1 | 3, 7, 8 |
| 5 | Dwarf Mining Region | 2 | 0, 2, 3, 6 |
| 6 | Southern Industry | 2 | 5, 7 |
| 7 | Eastern Technology | 2 | 4, 6, 9 |
| 8 | Imperial Capital | 3 | 1, 3, 4, 9 |
| 9 | Magic Harbor | 3 | 7, 8 |

隣接関係はすべて双方向に定義済み(全10州で相互チェック済み)。**4か国すべてのペアが直接陸続き**
(0-1: 1↔3経由, 0-2: 0↔5経由, 0-3: 1↔8経由, 1-2: 3↔5経由, 1-3: 3↔8/4↔8経由, 2-3: 7↔9経由)。
これは`tests/land_war_combat_peace_test.rs`の`test_all_countries_are_directly_land_connected`が
検証している性質であり、後述§8で重要な意味を持つ。

---

## 3. ハードコードと依存テスト

### 3-1. 本番コード側のハードコード
`src/`全体を検索した限り、**州数・国数そのものをハードコードしている本番コードは存在しない**。
`StateRegistry`/`CountryRegistry`はいずれも`Vec` + `HashMap`ベースのO(1)/O(n)汎用実装で、
件数に依存する上限やバッファサイズは見当たらない。以下は関連するが「壊れる」ものではない知見:

- `src/map/rendering.rs`: 海背景`MAP_WIDTH=1800.0`/`MAP_HEIGHT=1200.0`(固定定数) — 州の配置座標が
  この範囲を超えても州スプライト自体は問題なく描画・クリック判定できるが、海の背景がそこまで
  届かず見た目上不自然になる可能性がある。既存10州のworld_positionは概ねx:-560〜600,
  y:-280〜320で、既にこの範囲の際どいところまで使っている。
- `src/app/settings.rs`: `CameraSettings::default()`の`map_bound_x=1200.0`/`map_bound_y=900.0`
  (カメラがパンできる範囲のクランプ)。同様にマップが広がれば調整が要る。
- どちらも1行の定数変更で済み、変更しなくても機能的には壊れない(見た目の余白/パン制限の問題)。

### 3-2. `assets/data/states.ron`/`countries.ron`を直接読むテスト
**`tests/land_war_combat_peace_test.rs`のみ**(全文検索で確認)。この1ファイルが本番データを
`std::fs::read_to_string`で直接読む唯一のテストであり、かつこのファイル自体が保護対象。
2つのテスト関数がある:

1. `test_all_countries_are_directly_land_connected`
   - `assert_eq!(country_ids.len(), 4, ...)` — 国数が正確に4であることに依存
   - 全国家ペアが直接隣接することを検証(§2の性質に依存)
2. `test_land_war_declaration_combat_and_peace_flow`
   - `CountryId(0)`=Arcadia、`CountryId(3)`=Oceanic を決め打ち
   - `StateId(8)`=Oceanic首都、`StateId(1)`=Arcadia側の隣接州 を決め打ち
   - `StateId(1)`が`StateId(8)`と隣接することをアサート
   - `occupy_state(StateId(9), ...)`で講和条件計算用にもう1州(9)を直接指定

他のテスト(`src/military/tests.rs`、`src/war/tests.rs`、`src/country/country_ai.rs`のテスト等)は
すべて**自前で合成した`StateData`/`CountryData`**を使っており、本番RONファイルを一切読まない。
したがって**本番マップを拡張してもこれらは無影響**。リスクは`land_war_combat_peace_test.rs`
1ファイルに完全に閉じている。

### 3-3. 既存の大規模ワールド生成基盤(想定外の朗報)
`src/profiling.rs`(P20-008由来)に、**任意の州数(実績: 137州・200州・1000州超)を決定論的に
合成し、本番の全プラグイン構成・`DailySimulationSet`実行順序へ直接投入して動作させる仕組みが
既に存在する**(グリッド状の4方向隣接で州を自動生成)。`tests/profile_workload_correctness_test.rs`
がこの仕組みで137州・200州ワールドの正しさを既に回帰テストしており、`src/bin/profile_1000_states.rs`
で1000州規模の性能計測実績もある。これは「本ゲームのシミュレーションエンジンが
数十州規模に耐えられるか」という懸念に対する強力な傍証になる — 24〜30州は既に実証済みの
規模よりずっと小さい。ただしこの生成器は均一グリッドで、要衝・突出部・袋小路のような
意図的な地形デザインは行っていないため、今回の目的(手作りマップの拡張)には直接使えない。

---

## 4. StateとProvinceの役割分析

`StateData`は現在、**戦術的な移動単位**と**行政的な州**の両方を1つの型で兼ねている(§1参照)。
これは陸軍の移動(`military/movement.rs`)・経路探索(`military/pathfinding.rs`)・
戦闘開始判定(`military/invasion.rs`)・前線国境計算(`war/frontline.rs`)が「1師団=1State単位で
移動・停在する」ことを前提にしている一方、人口・雇用・建物・資源鉱床・不満度・統合度といった
経済シミュレーション(`economy`/`population`/`building`モジュール)も同じStateを単位に計算している
ためである。

**推奨: 今回はProvince/State分離を行わず、現在のStateを移動単位のまま24〜30州へ拡張する**

理由:
1. 今回明記された目的(複数進軍経路・広狭の国境・迂回/包囲・複数師団配置・要衝/袋小路/突出部・
   複数地点同時戦闘)は、いずれも「州同士のグラフ構造」の問題であり、州の内部をさらに
   細分化する動機にはならない。24〜30個の移動ノードがあれば、上記すべてを表現するのに
   十分な自由度がある(§6の設計案で全項目を満たせることを具体的に示す)。
2. Province/State分離は、新しいID型・新しいRegistry・経路探索/移動/戦闘/描画/クリック判定/
   前線計算のほぼ全てをProvince粒度に書き換える規模の変更であり、今回のデータ拡張
   (RONファイルの追記のみ)とは1桁以上コスト・リスクが異なる。
3. HoI4がProvince/State分離を必要としたのは、State数が100を超え、かつ1つのStateが
   地理的に離れた・あるいは内部に複数の戦術的な要衝を含む広大な地域を表す必要が
   あったため。今回の目標規模(24〜30)ではその必要性がまだ生じていない
   (「数百州への拡張は今回の対象外」という指示とも整合する)。
4. 前線システムを実装する前の段階でProvince/Stateを分離すると、前線側の実際の要求
   (州内で前線がどう振る舞ってほしいか)が固まっていないまま設計することになり、
   後で前線システムの実装時に手戻りが生じるリスクが高い。

**再検討すべきタイミング**: (a) 将来的に州数が「数百」の領域へ本当に拡張される場合、
(b) 前線システムの実装を通じて「1つのStateの中で前線が湾曲してほしい」「1つの広いStateに
複数師団が別々の方向を向いて駐留する必要がある」等、State粒度では表現できない具体的な
要求が判明した場合。

---

## 5. 推奨する州数と国家別配分

- 合計州数: **28州**(目標レンジ24〜30の中央よりやや上、既存10州+新規18州)
- 国家数: **4か国のまま**(既存の4か国を維持。5〜6か国への拡張は物語設定の追加発明が
  必要になり、今回のスコープ外と判断。追加したい場合は§11で選択可能)
- 国別配分: Arcadia 7州、Elfin Republic 7州、Dwarf Federation 7州、Oceanic Magic Empire 7州
  (指定レンジ「各国5〜8州程度」に収まり、かつ4か国均等)

---

## 6. 推奨する隣接構成

### 6-1. 基本方針: 「純増拡張」(既存10州は一切変更しない)

既存10州(ID 0〜9)の`id`・`name`・`neighbors`・`owner_country_id`等を**一切変更せず**、
新規18州(ID 10〜27)を追加し、必要な箇所で既存州の`neighbors`リストへ**追記のみ**行う
(既存の隣接関係は削除しない)、という設計にする。理由:

- §3-2の`land_war_combat_peace_test.rs`が依存する具体的なStateId/CountryIdの参照
  (`StateId(1)`が`StateId(8)`と隣接、`CountryId(0)`/`CountryId(3)`が存在し直接隣接、等)は
  **既存の隣接関係を保つ限りすべて成立し続ける**ため、このテストファイル(保護対象)を
  **一切変更せずに済む可能性が高い**(実装後に実際に`cargo test`で確認は必要)。
- 国数を4のまま維持するため、`assert_eq!(country_ids.len(), 4, ...)`も無変更で成立する。
- 全国家ペア直接隣接という性質も、既存の隣接ペアを消さない限り自動的に維持される。

これは§8で述べる「既存マップをテスト用fixtureとして分離する」よりコストが低い代替案であり、
今回は**こちらを第一候補として推奨**する(§11で両案を選べるようにしてある)。

### 6-2. 新規18州の構成案

各国の既存領域から外側へ拡張する形で配置する(座標は方向性の目安。厳密な数値は
実装時に決定する)。

**Arcadia (+4州、計7州)**
| ID(仮) | 名称 | 役割 | 接続先 |
|---|---|---|---|
| 10 | Eastern March | 前線(Elfin方面への新ルート) | 既存0、既存3(Elfin Central) |
| 11 | Southern Hold | 前線(Dwarf方面への新ルート) | 10、既存5(Dwarf Mining) |
| 12 | Frontier Fort | **突出部(salient)** — Dwarf領内へ突き出し、友軍隣接は11のみ、他は既存5・既存6(Dwarf側3州)に囲まれる | 11、既存5、既存6 |
| 13 | Mountain Watch | **要衝(chokepoint)** — Arcadia⇔Oceanic間の唯一の接続路 | 既存1(Arcadia)、既存8(Oceanic首都) |

**Elfin Republic (+5州、計7州)**
| ID(仮) | 名称 | 役割 | 接続先 |
|---|---|---|---|
| 14 | River Delta | 前線(Arcadia方面、Arcadia-Elfin国境を広げる) | 既存3、既存1(Arcadia) |
| 15 | Eastern Woods | 前線(Oceanic方面への新ルート) | 既存4、既存9(Magic Harbor) |
| 16 | Highland | 前線(Dwarf方面への新ルート) | 既存4、既存6(Southern Industry) |
| 17 | Outpost | **袋小路(dead-end)** — 隣接州は15のみ | 15 |
| 18 | Garrison | 内陸支援州(14・15を接続し奥行きを持たせる) | 14、15 |

**Dwarf Federation (+4州、計7州)**
| ID(仮) | 名称 | 役割 | 接続先 |
|---|---|---|---|
| 19 | Western Reach | 前線(Arcadia方面への新ルート、既存国境を広げる) | 既存5、既存2(Western Mage Province) |
| 20 | Foothills | 内陸支援州 | 19、21 |
| 21 | Deep Mine | 内陸支援州 | 20、既存6 |
| 22 | Southern Watch | 前線(Oceanic方面、既存6-7間の奥行き強化) | 既存6、既存7 |

**Oceanic Magic Empire (+5州、計7州)**
| ID(仮) | 名称 | 役割 | 接続先 |
|---|---|---|---|
| 23 | Northern Shore | 内陸支援州(首都北側の奥行き) | 既存8、24 |
| 24 | Coastal Road | 内陸支援州 | 23、既存9 |
| 25 | Eastern Cape | **袋小路(dead-end)** — 隣接州は既存9のみ | 既存9 |
| 26 | Southern Bastion | 前線(Dwarf方面への新ルート) | 22(Dwarf)、既存9 |
| 27 | Inner Sanctum | 内陸支援州(首都周辺の奥行き) | 既存8 |

### 6-3. 要求項目との対応表

| 要求 | 満たす箇所 |
|---|---|
| 複数の進軍経路 | Arcadia⇔Dwarf間だけで「0/2⇔5」「11⇔5」「12⇔5,6」の複数ルートが存在 |
| 3州以上が接する広い国境 | Arcadia-Dwarf国境: Arcadia側{0,2,11,12} vs Dwarf側{5,19} 計6州が関与する広い接壌帯 |
| 1州だけでつながる要衝 | State 13 "Mountain Watch"(Arcadia⇔Oceanicの唯一の接続) |
| 迂回可能な複数経路 | 上記進軍経路の複数性がそのまま迂回路になる |
| 包囲可能な突出部 | State 12 "Frontier Fort"(友軍接続は1本のみ、他3方をDwarfに囲まれる) |
| 到達不能な飛び地を作らない | 袋小路(17, 25)は隣接州1つを持ち、孤立(隣接0)ではない。全州が既存州経由で本国と連結 |
| 隣接関係を必ず双方向で定義 | 実装時に両側へ追記する方針を明記(§9の自動テストで機械検証) |
| 海上移動が未実装、島を必須経路にしない | `is_sea`州・海洋依存経路を新設していない |

### 6-4. 副産物として得られる互換性

この設計は意図して「既存の全国家ペア直接隣接」という性質も壊さない(Arcadia-Oceanicは
既存の1-8に加え新設13経由でも繋がり、他のペアも既存リンクを保持したまま)。そのため
§3-2で挙げた`test_all_countries_are_directly_land_connected`は**恐らく無変更で成立し続ける**
(要実機検証)。

---

## 7. 変更候補ファイル

| ファイル | 変更内容 | 保護対象か |
|---|---|---|
| `assets/data/states.ron` | 既存10州は無変更、新規18州(ID 10〜27)を追記 | **保護対象**(既存内容の変更は不可、追記のみなら許容されるか要確認) |
| `assets/data/countries.ron` | 変更なし(国数据え置きのため) | 明示的な保護リストには無いが`land_war_combat_peace_test.rs`が依存 |
| `assets/data/resources.ron` | 任意: 新規州に資源鉱床を追加する場合のみ | 非保護 |
| `src/map/rendering.rs` | `MAP_WIDTH`/`MAP_HEIGHT`定数の見直し(推奨、必須ではない) | 非保護 |
| `src/app/settings.rs` | `CameraSettings`の`map_bound_x`/`map_bound_y`見直し(推奨) | 非保護 |
| `src/app/loader.rs` | 変更不要(汎用実装のまま動作する見込み)。`capital_state_id`の存在・所有権検証を`validate_data`へ追加することを推奨(§9参照、任意) | 非保護 |
| `tests/land_war_combat_peace_test.rs` | **§6の純増拡張案が成立すれば変更不要の見込み**(要実機検証) | **保護対象** |

---

## 8. 既存小規模マップを残す方法

2つの選択肢がある(§11のNEEDS USER DECISIONで選択可能にしてある):

### 案A(今回推奨): 純増拡張、fixtureの分離は行わない
§6-1の方針どおり、既存10州をそのまま残しつつ本番`states.ron`へ追記する。
`land_war_combat_peace_test.rs`は本番データを直接読み続けるが、既存の隣接関係を
壊していないため、実装後に`cargo test --test land_war_combat_peace_test`を実行して
実際に無変更のまま通ることを確認するだけでよい(通れば本セクションの目的は達成済み ——
「既存の10州相当のシナリオ」は拡大後のマップの中に部分集合として現に残り続ける)。

### 案B: 完全な地形再設計 + fixtureをテスト専用ファイルへ分離
より自由な地形デザイン(既存10州の座標・隣接関係に縛られない再設計)をしたい場合、
`land_war_combat_peace_test.rs`が読むファイルを`assets/data/states.ron`から専用の
`tests/fixtures/small_map_states.ron`(仮)のようなテスト専用コピーへ切り替える必要がある。
これは**保護対象ファイルの変更(読み込みパスの書き換え)を伴う**ため、事前の明示的な
承認が必須。案Aよりコストが高いが、地形デザインの自由度は最大になる。

いずれの案でも、州数が今後さらに増える(数百州規模)場合には、大きな本番RONを
「最小限の決定論的regressionフィクスチャ」として使い続けるのは可読性・保守性の面で
無理が出てくるため、**案Bへの移行はいずれ避けられない**と見ている。今回の24〜30州
規模では案Aで十分と判断する。

---

## 9. 自動テスト計画

ユーザー指定の最低限の項目に対応させる形で整理する。

| # | テスト内容 | 実装場所の候補 |
|---|---|---|
| 1 | 全州IDが一意 | `src/app/loader.rs::validate_data`に既存(`Duplicate StateId`パニック) — 追加テスト不要、既存機構がそのまま機能する |
| 2 | 所有国IDが有効 | 同上、既存(`references unknown CountryId`パニック) |
| 3 | 隣接関係が双方向 | **新規**: `tests/`配下に`assets/data/states.ron`を読み、`∀s, ∀n∈s.neighbors: s.id ∈ get(n).neighbors`を検証するテスト(現状これを直接検証するテストは見当たらなかった — 拡張前の10州でも実は明示テストされていない) |
| 4 | 不正な自己隣接がない | **新規**: `∀s: s.id ∉ s.neighbors`を検証(同上、既存にはない) |
| 5 | 全本土州が接続されている | **新規**: `is_sea==false`の全州についてBFS/DFSで単一連結成分になることを検証 |
| 6 | 経路探索が複数経路から正しく経路を選ぶ | 既存`military::pathfinding::find_path`の単体テストを拡張マップ規模のデータで追加(決定的な最短路選択の確認) |
| 7 | 全州を描画・クリックできる | 既存`ui_headless_render_test.rs`(P20-007)パターンを流用した見た目確認、または`map::selection::handle_state_click`の単体テストを拡張州数で追加 |
| 8 | 師団が新しい州を移動できる | 新規州IDを終点とする`try_issue_move_order`/`process_movement`の単体テスト追加 |
| 9 | 敵軍との接敵戦闘が発生する | 新規州(特にState 12のような突出部)での`process_army_arrival`戦闘開始テスト追加 |
| 10 | 占領と所有権変更が機能する | 既存`occupy_state`/`transfer_region_ownership`のテストパターンを新規州IDで追加(ロジック自体は既存のまま再利用) |
| 11 | ドラッグ選択と一括移動に回帰がない | 既存`src/map/army_selection.rs`のテスト群(`drag_select_*`等)をそのまま再実行して回帰がないことを確認(データ非依存のため理論上無影響、実行して確認) |
| 12 | 既存テストと固定証拠が失われていない | `cargo test`全件・`cargo fmt --check`(既知ベースライン比較)・保護ファイルSHA256比較・`verification_logs/p20-007`等の証拠画像が意図せず変更されていないことを確認(このセッションの既存ワークフローをそのまま踏襲) |
| 13(追加) | 拡張前後の処理時間比較 | `src/profiling.rs`の枠組みは合成データ専用で本番RONを読まないため直接は使えない。代わりに、拡張前後それぞれで`cargo test --test daily_system_integration_test`や`DailySimulationSet`を手動で複数日ぶん回す簡易ベンチマーク(`Instant`計測)を一時的に書いて比較するか、`profile_workload_correctness_test.rs`と同じ手法で`state_count=10`と`state_count=28`相当の合成ワールドを生成し比較する(後者の方が既存基盤を再利用でき低コスト) |
| 14(追加) | `capital_state_id`の存在・所有権検証 | 現状`validate_data`は`capital_state_id`が実在する州か、その州を実際に所有しているかを検証していない(§3-1で発見)。手作業で18州を追記する際にtypoで気づかれないリスクがあるため、この検証を`validate_data`へ追加することを推奨(小さな追加、保護対象外) |

---

## 10. 手動確認項目

- 実機`cargo run`で拡張後のマップ全体をカメラパン/ズームで一周し、全州が描画され
  クリックできることを目視確認
- 新設した要衝(State 13)・突出部(State 12)・袋小路(17, 25)を実際に師団で移動して
  意図通りのボトルネック/孤立しやすさになっているか確認
- 各国の初期陸軍(首都に1個ずつ、`spawn_debug_armies`)が正しい位置に表示されることを確認
- 軍事パネル・複数選択・ドラッグ選択が28州・(初期は4個程度の)陸軍規模で違和感なく動作するか確認
- 前線国境計算(`calculate_frontline_border`)が新しい広い国境・要衝を含めて妥当な前線を
  生成するか(前線システム自体は別タスクだが、既存のborder計算ロジックへの影響として)
- マップの見た目の余白(§3-1のMAP_WIDTH/HEIGHT・カメラ境界)が不自然でないか

---

## 11. NEEDS USER DECISION

1. **§8の案A(純増拡張・fixture分離なし)と案B(完全再設計・fixture分離)のどちらを取るか**
   (本レポートは案Aを推奨しているが、地形の自由度を優先するなら案Bも選べる)
2. 国家数を4のまま維持するか、5〜6か国へ増やすか(増やす場合、新規国家の名称・設定・
   国旗色等の物語要素を追加発明する必要がある — 今回のレポートでは4か国維持を前提に
   §5〜6を設計した)
3. §6-2の新規18州の名称・役割配分案をそのまま採用するか、方向性(どの国がどの方角へ
   拡張するか)を変更するか
4. 州数を28ではなく24または30に寄せたいか(4か国均等でなくてよければ調整余地あり)
5. §9-14で見つけた`capital_state_id`検証の抜けを、このタスクの一部として先に直すか、
   別タスクとして切り出すか

---

## 12. 実装可否

**READY WITH DECISIONS**

技術的な障壁は見当たらない(データローダー・経路探索・戦闘・占領・前線計算・描画・
選択・ドラッグ選択のいずれも州数に依存しない汎用実装であることを確認済み。さらに
既存の`profiling.rs`基盤が137〜1000州規模での正しい動作実績を持つため、28州規模での
性能面のリスクは極めて低いと判断する)。唯一の設計判断が必要な論点は§8・§11の
fixture分離方針であり、これさえ決まれば§6の具体的な州構成案に沿って
`assets/data/states.ron`への追記から着手できる。
