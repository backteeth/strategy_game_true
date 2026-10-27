# P32-018 新マップ経済セッションと画面・保存経路への接続設計

**設計のみ。** コード・アセット・セーブ形式は変更していない。設計の完了は画面接続の実装完了ではない。M4全体・Phase 32 は継続。
基準: HEAD `1d41018`(P32-017)、ブランチ main、origin/main と一致、未追跡は `hex_atlas/` のみ。

関連: P32-003 設計(`p32-003/p32-003_state_province_migration_design.md` §10〜12)、P32-014 保存設計、P32-015〜017 実装・報告書。
**旧V1の廃止(D11)は未承認のため、本書はV1を廃止・非対応にする決定をしない。**

---

## 0. 結論(要約)

1. **推奨起動経路**: 旧ゲーム本体(`main.rs`)には統合せず、`hex_map_preview` と同じ独立した Bevy App として、新バイナリ `map_economy_session` を作る。
   旧ゲームのプラグイン(`GameTimePlugin`/`EconomyPlugin`/`SaveGamePlugin` 等)を**1つも登録しない**ので、旧経路 System が新セッションの状態を二重更新することが構造的に起きない。
2. **正本は `MapEconomySimulation` 1つ**。画面は `Res<EconomySession>`(その薄い所有者)を読み書きし、経済値を UI 用に複製しない。
3. **操作はコマンド(Message)経由で1箇所の System が適用**する(進行・注文・取消・保存・復元)。複数日の進行も1コマンドとして同期的に完了し、注文・保存は必ず「日と日の間」に起きる。
4. **建設の受付判定は既存の `place_order`/`check_manual_build_order` をそのまま使う**。UI は判定を再実装しない。ただし「実行せずに受付可否を調べる」関数が無いため、小さな追加が必要(§5.4)。
5. **保存・復元は P32-016/017 の `save`/`load` をそのまま呼ぶ**。画面で警告を事前に見せるため、`load` の前半(読込+判定)だけを返す `inspect` を追加する(§6.2)。
6. **SaveGameV2 を先に実装する必要はない**。理由は §7。
7. 実装は3段階(S1 セッション+表示+日付進行 / S2 建設・キャンセル / S3 保存・復元)+前提整理(S0)。

---

## 1. 調査範囲

読んだ実コード: `src/main.rs`、`src/app/{mod,game_state,time,loader}.rs`、`src/bin/hex_map_preview.rs`、`src/hexmap/{view,world_view,mod}.rs`、
`src/world/overlay.rs`(`cell_status`)、`src/economy/map_state.rs`(`place_order`/`request_build`/`cancel_order`/`run_day`/`SimDate`)、
`src/economy/map_state/snapshot.rs`(`MapEconomySimulation`)、`src/economy/map_economy_file.rs`、`src/economy/map_state/static_identity.rs`、
`src/building/construction.rs`、`src/ui/{state_panel,top_bar,load_confirm}.rs`(参照)、`src/save/{mod,runtime}.rs`(参照)、`src/bin/world_economy_check.rs`、
`assets/data/world/preview/` のシナリオ群、`tests/p20_009_hardcoded_string_scan_test.rs`、ヘッドレス描画テスト3本の存在。
**調べていない**: 旧ゲームの各パネルの全 `StateId` 利用箇所(今回の接続対象外)、`app/loader.rs` の全体(建物定義読込部のみ確認)。

---

## 2. 現在の画面・実行経路の対応表

| 項目 | 現状(実コード) | 新セッションでの扱い |
|---|---|---|
| ゲーム本体の起動と遷移 | `main.rs`: `DefaultPlugins`+`AppPlugin`(`init_state::<GameState>`: `MainMenu → CountrySelection → Playing`、`DataLoaderPlugin`が旧28州を読込)+旧プラグイン群+`SaveGamePlugin`/`LoadGamePlugin` | **使わない**(別App)。`GameState` は新セッションに存在しない |
| 新マップ確認画面の起動 | `bin/hex_map_preview.rs`: `HexPreviewPlugin{open_at_start, world_preview: Option<WorldPreviewPaths>}`、日本語フォントは `PreStartup` で `localization::install_japanese_capable_font` | **再利用**。新バイナリが同じ形で `HexPreviewPlugin` を使い、経済セッション用プラグインを追加 |
| 描画 | `hexmap::view`(`enter_preview`/`exit_preview`、`HexPreviewState{Closed,Open}`、`LoadedHexMap`、`HexViewSettings`、チャンク描画、河川、境界)、`hexmap::render` | **再利用・変更なし**(地形/気候/植生/ステート表示、河川、境界、パン、ズームを維持) |
| ワールド設定 | `hexmap::world_view`: `WorldPreviewPlugin`→`LoadedWorld{data: WorldPreviewData, names}`(`enter_world`)。`data.model: WorldModel` | **再利用**。セッションは `LoadedWorld.data.model` を借りる(二重に読まない) |
| セル選択→ステート | `HexSelection(Option<u32>)`(セル配列インデックス)→`map.cell_by_index`→`world::cell_status(model,map,q,r)` → `CellStatus::{Assigned{province,state},Unassigned,OutOfScope,Water,NoCell}` | **再利用**。`Assigned.state: MapStateId` をそのまま経済ステートの鍵にする(旧 `StateId` との変換なし) |
| UI の地図への透過防止 | `camera_input`: `ui_is_hovered(&Query<&Interaction>)`(`Hovered`/`Pressed` のUIノードがあれば、ホイール・ドラッグ・クリック選択を地図へ渡さない)、`DragState.started_on_ui`。パネルは `Button` を付けて `Interaction` を発行させる | **再利用**。新パネルのルートにも `Button` を付ける。キー入力は §8.4 |
| 国・ステートの既存UI | `ui::state_panel`(旧 `StateId` 用、`BuildButton`、`check_manual_build_order`)、`ui::economy_panel`、`ui::top_bar` | **使わない**(旧型)。ただし受付判定関数 `building::construction::check_manual_build_order` は新経路の `place_order` が既に共用している |
| 時間進行と停止 | `app::time`: `GameDate{..accumulator}`、`GameSpeed`、`GamePaused(true)`、`DayChangedMessage`/`MonthChangedMessage`、`DailySimulationSet`(`run_if(in_state(Playing))`) | **使わない**。新経路の日付は `SimDate`(同じ暦: `GameDate::days_in_month` を使う。うるう年なし)。実時間の累積は持たない |
| 旧日次・月次 System | `EconomyPlugin` 等が `DayChangedMessage`/`MonthChangedMessage` を読み、旧 `StateData` を更新 | **別Appなので動かない**。新経路の日次は `MapEconomySimulation::advance_day` のみ |
| 日次処理の正本 | `MapEconomySimulation::advance_day(&BuildingRegistry) -> (SimDate, DayReport)`: 日付を進めてから `registry.run_day`(建設→月次→AI評価)。`date` は「最後に処理を完了した日」 | **そのまま使う** |
| 手動建設 | `MapStateEconomyRegistry::request_build(&self, &mut [ScenarioCountry], issuer, MapStateId, BuildingType, &BuildingRegistry) -> Result<ConstructionOrderId, OrderError>`(内部 `place_order`)。費用・鉱床・上限・国家対応・重複・前提をここで判定し、支払いとキュー追加は最後に1回 | **そのまま呼ぶ**。`OrderError` の各種別を画面の文言に対応づけるだけ |
| キャンセル | `cancel_order(&self, &mut [ScenarioCountry], issuer, CancelTarget::Order(ConstructionOrderId)) -> Result<CancelReceipt, CancelError>`。他国家の注文・完成済み・取消済み・存在しない注文を拒否。返金は `building::construction::cancel_refund`(支払済み×0.5) | **そのまま呼ぶ**。画面は `ConstructionOrderId` を保持し、配列位置では指定しない |
| 保存・復元 | `economy::map_economy_file::{save, load, SaveLocation, SaveReceipt, LoadReceipt, FileError}`、`static_identity::{StaticInputs, StaticIdentity, judge, Verdict}`。保存先 `<base>/map_economy/<名前>.mapeco.ron`、原子的置換、V1スロット保護 | **そのまま呼ぶ**(CLI `world_economy_check --economy-*` と同じ関数)。`inspect`/`list_saves` を追加(§6) |
| 旧セーブUI | `ui::top_bar` のボタン→`SaveRequestMessage`/`LoadRequestMessage`→`save::runtime`(PostUpdate)、`ui::load_confirm`、`saves/savegame_v1.ron` 単一スロット、`SaveGamePlugin` は全旧プラグインの後に登録 | **使わない・変更しない**。新セッションAppには `SaveGamePlugin` を登録しない |
| 終了・再入場・破棄 | `exit_preview`: `HexPreviewEntity` を一括 despawn、`LoadedHexMap`/`HexSelection`/`HexViewport` 等を `remove_resource`。`exit_world` も同様 | **同じ規約で破棄**。セッションは `OnExit(Open)` で破棄(§4.4) |
| CLI の経済検証 | `bin/world_economy_check.rs`: `m4d_simulation`(シナリオ+資源から `MapEconomySimulation` を組み立て)は**binの中**にある | **libへ移す**(S0)。CLI と画面が同じ組み立てを使う |
| 建物定義の読込 | `app/loader.rs` が `assets/data/buildings.ron` を検証付きで読むが Bevy System 内でパニックする。bin/tests は個別に `ron::from_str` | 検証付きの純粋関数を共有化(S0。実装者が既存規約で決める) |

---

## 3. 推奨する起動経路

| 案 | 内容 | 評価 |
|---|---|---|
| **A(推奨)** | 新バイナリ `src/bin/map_economy_session.rs`。`HexPreviewPlugin{world_preview: Some(default_preview), ..}` + `EconomySessionPlugin` を追加 | 旧プラグイン0個で二重更新が構造的に無い。ウィンドウ題名・パネル見出しで「検証用」を明示できる。`hex_map_preview` の既存操作・`--shots` を変えない |
| B | `hex_map_preview --economy-session` のフラグ | バイナリは増えないが、確認画面の単機能(地理確認)と経済操作が混ざり、`--shots` 自動スクリプトと干渉しうる |
| C | 旧ゲーム本体(`GameState`)に新マップ経済を統合 | **非推奨**。`DataLoaderPlugin` は旧28州を読み、`DailySimulationSet` 全体・`SaveGamePlugin`・師団/戦争/外交が旧 `StateId`/`CountryId` に依存する。入れると新セッションを守る実行条件を旧System全部に付ける必要があり、M4〜M9(軍・所有・SaveGameV2)の未決事項が着手条件になる |

起動例: `cargo run --bin map_economy_session -- [--scenario m4e] [--operator 1] [--save-base saves]`
(`--save-base` の既定は `saves`。保存先は常に `<base>/map_economy/`。起動だけではファイルを作らない。)

---

## 4. セッション状態の正本

### 4.1 構成(Bevy非依存の中核+Bevyの薄い所有者)

```
economy::map_session            (純粋Rust・新規。CLIと画面が共有)
  SessionScenario { id, label, scenario_path, resources_path, start: SimDate }   // 既知の検証シナリオ表
  EconomySession {
      sim: MapEconomySimulation,        // ★正本。日付・国家・注文・履歴・ステート経済
      buildings: BuildingRegistry,      // 静的入力(検証済み)
      facilities: Option<LocationFacilityRegistry>,
      scenario: &'static SessionScenario,
      operator: Option<u32>,            // 操作主体の国家ラベル(未選択=None)
      dirty: bool,                      // 最後の保存/復元以降に進行・注文・取消があったか
  }
  fn build(scenario, &WorldModel, buildings) -> Result<EconomySession, _>   // m4d_simulation をlibへ移したもの
  fn apply(&mut self, cmd: SessionCommand, inputs: &StaticInputs) -> CommandOutcome

hexmap::economy_view            (Bevy・新規。src/hexmap 配下)
  Resource EconomySession をそのまま保持(複製しない)
  Resource SessionUi { selected_state: Option<MapStateId>, last_results: VecDeque<String>, save_slot, ... }  // 表示状態のみ。経済値を持たない
  Message SessionCommand
```

- 画面に表示する人口・雇用・施設・国庫・キューはすべて `session.sim` から**描画時に読む**(`MapStateEconomyRegistry::get(MapStateId)`、`sim.countries`)。
  UI 側のキャッシュ文字列は表示用で、経済の判定・更新に使わない。
- **ID**: 選択は `CellStatus::Assigned{state: MapStateId}`。旧 `StateId`/`CountryId` との数値変換は作らない。国家は検証シナリオの `label: u32`(旧 `CountryId` とは無関係)。
- **国家とステートの対応**は `ScenarioCountry.states`(経済計算上の対応関係)。法的所有・占領の規則は参照も固定もしない。
- 静的入力のうち地図・ワールドは既存リソース(`LoadedHexMap`・`LoadedWorld`)を借り、`StaticInputs{map, world, buildings, facilities}` はコマンド適用時に組み立てる(保存・復元で必要)。

### 4.2 操作主体の決め方

- シナリオの `ai_controlled: false` の国家だけが操作可能な候補(AI国家は `sim.countries[i].ai.is_some()` で判定)。
- **列挙順の最初を黙って選ばない。** `--operator <label>` で指定するか、画面の国家ボタンで明示選択する。未選択の間は建設・キャンセルを受け付けず、その理由を表示する。
- 既定シナリオは **m4e**(国家1・2が手動、国家3がAIで国庫を共有)。**m4d は全国家がAI**(`ai_controlled: true`×3)で、画面から操作できる国家がなく、本セッションの既定には使えない(CLIの保存・再開検証用)。
- `operator` は mapeco ファイルに保存されない(P32-015 の保存対象外)。復元後も現在の `operator` を維持し、存在しない/AI国家なら未選択へ戻す。

### 4.3 検証用であることの明示

ウィンドウ題名「新マップ 経済検証セッション(検証用)」、画面上部の帯「検証用の地域・設定(正式な世界設定・ゲーム開始ではありません)」、保存名の接頭の表示「新マップ経済(mapeco)」。

### 4.4 寿命

- 生成: `OnEnter(HexPreviewState::Open)` の `enter_world` の後(`LoadedWorld` が揃ってから)。失敗(シナリオ・資源・建物の読込/検証エラー)は `SessionLoadError` を保持し、地理確認だけは続けられる(`WorldLoadError` と同じ作法)。
- 破棄: `OnExit(Open)` で `EconomySession`/`SessionUi` を `remove_resource`、UI Entity は `HexPreviewEntity` で一括削除。再入場は**新規生成**(前の状態を引き継がない=重複しない)。
- Tab(画面を閉じる)は経済セッションでは**未保存(`dirty`)なら確認を出す**か無効化する(実装者が §11 の既定に従って決める)。Esc(終了)も同様に未保存の警告を表示する。

---

## 5. 時間進行と注文の受付順

### 5.1 原則

- 時間は**停止が既定**。進行は利用者の操作(コマンド)だけ。最初の実装に実時間の自動進行(累積器)は入れない。
- 日次処理の正本は `MapEconomySimulation::advance_day` 1箇所。コマンド適用 System が `ResMut<EconomySession>` を排他的に持つため、画面フレーム内で日次処理が重複しない。
- 複数日の進行(7日/30日/…)は**1コマンド内で同期的に最後まで実行**する(上限 `MAX_DAYS_PER_COMMAND`、例 90)。処理中に別コマンドが割り込む余地がなく、日と日の間以外では保存・注文が起きない。
  - 将来の自動進行は、同じ `AdvanceDays(1)` を発行する側(累積器+1フレーム上限)として追加する。日次処理の入口は変えない。

### 5.2 System 順序(1フレーム)

```
Update (HexPreviewState::Open のみ、既存の chain の後ろ):
  [既存] handle_buttons → handle_keys → camera_input(選択更新)→ … → update_texts
  [新規] session_ui_input        // ボタン/キー → SessionCommand を発行(状態は変えない)
         apply_session_commands  // ★唯一の更新点。MessageReader で到着順に処理
         sync_session_ui         // 結果表示・パネル再構築(経済値は sim から読む)
```
既存 System が private なので、`hexmap::view` に `HexPreviewSet::{Input,Sync,Text}`(SystemSet)を足して `.after(..)` を指定する小改修が要る(S1)。

### 5.3 コマンドと受付の時点

| コマンド | 受付の時点 | 内部呼び出し | 備考 |
|---|---|---|---|
| `AdvanceDays(n)` | 常に日と日の間 | `sim.advance_day(&buildings)` を n 回 | `1 ≤ n ≤ MAX`。結果の `DayReport`(完成・保留・AI発注・月次実行)を `last_results` へ要約 |
| `Build{state, building}` | 日と日の間。**次の日の処理の前**に支払い・キュー追加 | `sim.registry.request_build(&mut sim.countries, operator, state, building, &buildings)` | operator 未選択なら受け付けずに理由表示。進捗は次に処理する日から始まる |
| `Cancel{order_id}` | 日と日の間 | `sim.registry.cancel_order(&mut sim.countries, operator, CancelTarget::Order(id))` | **識別子指定のみ**(`CancelTarget::Latest` は画面から使わない)。返金は国庫へ。進行中のキュー位置が変わっても別の注文は取り消さない |
| `Save{name}` | 日と日の間 | `map_economy_file::save(&sim, &inputs, &loc, name)` | `AdvanceDays` の途中には入らない(同一 System 内で逐次) |
| `Load{name}` | 日と日の間 | `map_economy_file::load(&mut sim, &inputs, &loc, name)` | 成功したら同フレームの**未処理コマンドを破棄**し、`last_results` に結果。時間は停止のまま |
| `SetOperator(label)` | いつでも | `operator` 更新(AI国家・存在しない国家は拒否) | 経済状態は変えない |

- 同一フレームに複数コマンドがあれば到着順に1つずつ。`Load` 成功後の残りの `AdvanceDays` は破棄(ロード直後に同じ日を再実行しない・古い画面状態の注文を新しい状態へ適用しない)。
- **ロード直後の日付**: `sim.date` は「最後に処理を完了した日」。復元直後はその日を再処理せず、次の `AdvanceDays` が翌日を処理する(`advance_day` の契約)。
- **日付表示の意味**: 「処理済み: 1800/01/10」。開始直後は「開始日 1800/01/01(まだ1日も処理していません)」と表示し、「今日」ではないことを明示する(旧ゲームの `GameDate` は別の意味なので、同じ見た目にしない)。

### 5.4 建設候補の「受付できない理由」(判定を再実装しない)

`place_order` は成功時に副作用(支払い・キュー追加)を持つため、判定だけを実行する公開関数が現状**無い**。選択肢:
1. **推奨**: `MapStateEconomyRegistry::check_order(&self, &ScenarioCountry, MapStateId, BuildingType, &BuildingRegistry) -> Result<(), OrderError>` を追加し、`place_order` をそれを呼ぶ形に整理する(判定コードは移すだけで複製しない)。テスト: 全(ステート×施設)で `check_order` の結果が複製した国家への `place_order` と一致。
2. 代替: 候補ごとに `ScenarioCountry` を clone して `place_order` を試す(関数追加なし。clone の費用は小さい: キュー・記録・AI履歴のみ)。選択・日付・国庫・キューが変わったときだけ再計算する。
どちらでも、UI は `OrderError` を文言に対応づけるだけで、費用・鉱床・上限・国家対応の比較は書かない。

---

## 6. 保存・復元の接続点

### 6.1 保存

- UI → `SessionCommand::Save{name}` → `map_economy_file::save`(P32-016)。保存先制限(`<base>/map_economy/`、名前規則、V1スロット拒否、シンボリックリンク/ジャンクション拒否)と原子的置換は**そのまま**継承。UIから迂回する経路を作らない。
- 表示: 保存先の相対パス、保存した「処理済み日付」、サイズ、`replaced_existing`、失敗時は `FileError` の文言(名前不正/保存先拒否/上書き拒否/I/O)。
- **既存名への上書き**は、保存前に一覧(`list_saves`)に名前があれば確認を挟む(`save` 自体は自形式の既存ファイルのみ上書き可で、V1・別kind・形式1・壊れたファイルは `OverwriteRefused`)。
- 保存名: S3 では**固定スロット名**(例 `slot-1`〜`slot-3`)+既存ファイルの一覧から選択を推奨。自由入力は、地図の操作キー(1〜4/G/R/B/F/Home/Tab/Esc)との衝突対策(入力フォーカス中はホットキーを無効化)が必要なため、別段階に分ける(§11 U5)。名前の検証自体は `check_name`(`save`/`load` 内)に任せ、UI は検証を再実装しない。

### 6.2 復元(2段階: 確認 → 実行)

追加が必要な関数(いずれも既存関数の組み合わせで、`load` の挙動は変えない):
- `map_economy_file::inspect(sim, &inputs, &loc, name) -> Result<LoadPreview{path, date, verdict}, FileError>`: 封筒検証→識別情報検証→復元先から識別情報生成→`judge` まで。**復元はしない**。`load` は `inspect` の後に `restore` する形に整理する。
- `map_economy_file::list_saves(&loc) -> Vec<SaveEntry{name, ...}>`: `<base>/map_economy/` 直下の通常ファイルのうち、名前規則に合う `*.mapeco.ron` だけ(リンク・他拡張子・V1は出さない)。

手順:
1. 利用者が一覧から選ぶ → `inspect` → 画面に「保存日付」「一致 N 項目」「警告つき許可(利用者向け文言)」「拒否(理由)」を表示。拒否・構造不正・形式1/別kind/V1 は「復元できません」と理由を出し、実行ボタンを出さない。
2. 警告のみなら「警告を確認して復元」ボタン → `Load{name}` → `map_economy_file::load`(ここで再度同じ判定が走る。`inspect` の結果は信頼しない)。
3. **失敗時に半端に切り替わらない**: `load` は `sim` を成功時の1回の代入でしか変更しない(P32-015/016)。UI はカメラ・選択・`operator`・表示状態を `load` が `Ok` を返した**後**にだけ更新する。復元で変わる経済状態はステート集合が同一(判定済み)なので、カメラ・選択は維持できる。`Err` の場合は何も触らず、結果行にだけ理由を出す。
4. 成功後: 時間は停止のまま(自動進行が無いので常に停止)。`dirty=false`。国家選択は維持(無効なら未選択)。結果行に「復元しました(処理済み 1800/04/11)」と警告があれば要約。

### 6.3 CLIとの一致

画面が呼ぶのは CLI(`--economy-save/--economy-resume`)と**同じ** `save`/`load`/`StaticInputs`。画面専用の保存実装は作らない。CLI も `economy::map_session` の組み立て(S0)を共有する。

### 6.4 保存ファイルの置き場とリポジトリ

画面で保存すると `saves/map_economy/*.mapeco.ron` がリポジトリ内にできる。**現在の `.gitignore` は `/target` のみで、このパスは無視されない**(`saves/savegame_v1.ron` だけが追跡されている)。S3 では `saves/map_economy/` を `.gitignore` に追加するか、利用者の判断で保存先を変える必要がある(§11 U6)。自動テストは常に一時ディレクトリを使う。

---

## 7. 旧V1との共存と、SaveGameV2を先に作る必要の有無

- 旧ゲームは従来どおり V1(`saves/savegame_v1.ron`、`SaveGamePlugin`/`LoadGamePlugin`、トップバーのボタン)。新セッションは mapeco(`saves/map_economy/*.mapeco.ron`)。画面上は別のApp・別の見出しで区別し、新セッションに「旧セーブ」ボタンは出さない。
- mapeco の読込側は、`kind` が無く `version` を持つファイルを「旧ゲームのV1セーブ」と識別して拒否する(P32-016)。V1の読込・書込・検証コードは変更しない。**V1を廃止・非対応にする決定はしない(D11は未承認)。**
- **SaveGameV2 の全面実装が先に必要になるのは、新セッションを旧ゲーム本体に統合する場合だけ**。その場合の依存: `GameState::Playing` の全System、`save::dto/validate/apply` が旧 `StateId`/`CountryId`(`validate.rs` は5800行規模)、師団の `current_state`、戦争の `occupied_states`、前線、`capital_state_id` が新マップの `ProvinceId`/`MapStateId` に載らないこと(P32-003 §10)。
  経済セッション単体の動作・保存・復元にはこれらは不要で、mapeco が独立した封筒として成立している(P32-014 の「Aを、Bが取り込める節として設計」)。
- **軍・戦争・所有の保存節をダミーで埋めない。** 新経路にその実装が無い間は、mapeco に含めず、画面でも「保存されるのは経済のみ」と表示する。

---

## 8. 画面構成と操作

### 8.1 レイアウト(1600×900 を基準。左右パネル幅は既存定数 `LEFT_PANEL_W=250`/`RIGHT_PANEL_W=280`)

```
┌ 上部帯 ────────────────────────────────────────────────────────────────────┐
│ 【検証用】新マップ経済セッション  処理済み 1800/04/11  国家: [1][2](選択)  │
│ [+1日][+7日][+30日]              ※正式な世界設定ではありません              │
├ 左パネル(既存)──┬ 地図(既存の描画・パン・ズーム)─┬ 右パネル ──────────────┤
│ 表示モード/河川/ │                               │ 選択セル(既存の説明)    │
│ 境界/全体表示    │                               │ ステート経済(新)        │
│ 保存・復元(新)   │                               │ 建設候補(新)            │
│                  │                               │ 建設キュー(新)          │
├ 下部ログ(新)────────────────────────────────────────────────────────────────┤
```
新パネルのルートはすべて `Button`(`Interaction` を発行)で、地図へ透過させない。`fit_camera` は左右パネル幅のみを考慮するため、上部帯・下部ログは地図に重ねる細いオーバーレイにして `LEFT/RIGHT_PANEL_W` を変えない(全体表示の計算を変えない)。

### 8.2 表示項目(最低限)

| 領域 | 内容(データ源) |
|---|---|
| 日付 | `sim.date`(処理済み日付。開始直後は「未処理」) |
| 操作主体 | `operator`(未選択なら「国家を選択してください」)。AI国家は選択肢に出さない(表示は「AI」) |
| 国家 | 国庫 `countries[..].treasury`、税率、月次収支、(必要なら)備蓄 |
| 選択ステート | 人口、雇用/失業、物流、施設レベル(`buildings`)、稼働率 |
| 建設候補 | `BuildingType::ALL` のうち `is_state_level_economic_facility`:費用(`BuildingDefinition.construction_cost`)、受付可なら「発注」、不可なら `OrderError` に対応する理由 |
| 建設キュー | operator の `construction_queue`:施設・対象ステート・`progress/required_progress`・支払済み・状態・[キャンセル](行に `ConstructionOrderId` を保持)、キャンセル時の返金額(`cancel_refund`) |
| 結果 | 発注/拒否、完成・保留・AI発注(`DayReport`)、キャンセルと返金、保存・復元の結果 |

### 8.3 選択の種類ごとの表示(前の情報を残さない)

選択が変わるたびにステート欄・建設候補を**全再構築**し、`selected_state` を新しい値で置き換える(古い対象へ発注ボタンを残さない)。

| `CellStatus` | 表示 | 建設 |
|---|---|---|
| `Assigned{state}` かつシナリオの経済ステート | ステート経済・候補・キュー | 可(operator が対応国家のとき) |
| `Assigned{state}` だがシナリオの経済ステートに無い | 「このステートは検証シナリオの経済に含まれていません」 | 不可 |
| 経済ステートだが operator の国家に対応しない | 経済は表示。「他国家のステート(操作できません)」 | 不可(`NotMappedToCountry`) |
| `Unassigned` | 「ステート未割当」 | 不可 |
| `OutOfScope` | 「検証範囲外」 | 不可 |
| `Water` | 「水域(経済の対象外)」 | 不可 |
| 選択なし/`NoCell` | 「地図のセルを選択してください」 | 不可 |

### 8.4 入力とキー

- ボタン主体。既存ホットキー(1〜4/G/R/B/F/Home/Tab/Esc)は変更しない。新規ホットキーは最小限(例 `.`=+1日)。テキスト入力を入れる段階では、入力フォーカス中は既存ホットキーを無効化する仕組み(`TextInputFocus` 相当)を同時に用意する。
- 地図への透過防止は既存の `ui_is_hovered` と `DragState.started_on_ui` に任せる。

### 8.5 露出しない情報

識別用ダイジェスト、内部型名、`ItemKey` 名(`land_terrain` 等)、`ConstructionOrderId` の生の `Order#c-s` は通常画面に出さない(注文は「Farm(MapState#20)」のような施設+ステートで表示、IDは行に内部保持)。詳細(判定項目名・理由・ダイジェスト)は「詳細」展開またはログ(`info!`)にまとめる。
文言は `hexmap` 配下の既存作法(日本語リテラル。`p20_009_hardcoded_string_scan_test` の走査対象は `src/ui/*.rs` と通知を作る一部のモジュールで、`src/hexmap/` は対象外)に合わせる。新パネルを `src/ui/` に置くと翻訳キー化が必要になる。

---

## 9. 段階的な実装計画

| 段階 | 内容 | 利用者が実際に確認できること |
|---|---|---|
| **S0 前提整理(コード、画面なし)** | ① `m4d_simulation` 相当を `economy::map_session`(lib)へ移し、CLI をそれ経由に。② `SessionScenario` の既知表(m4d / m4e)。③ 建物定義の検証付き読込の共有化。④ `check_order`(§5.4)。⑤ `inspect`/`list_saves`(§6.2)。 | CLI が従来と同じ出力(P32-016/017 のテスト・別プロセス再開が緑のまま) |
| **S1 セッション生成・経済表示・日付進行** | 新バイナリ `map_economy_session`、`EconomySessionPlugin`(生成/破棄、`SessionCommand`、`apply_session_commands`、上部帯、ステート経済、`+1/+7/+30日`、操作主体の選択、`HexPreviewSet`) | 起動すると新マップが表示され「検証用」と分かる。ステートを選ぶと人口・雇用・施設・国庫が出る。日付を進めると値が月次で変わる。再入場で重複しない |
| **S2 手動建設とキャンセル** | 建設候補(受付可否と理由)、発注、建設キューと進捗、キャンセル(識別子指定)、返金表示、AI国家の発注ログ | 国家1で Farm 等を発注→進捗が日数で進む→完成で施設が増える。予算不足・他国家・最大レベル等が理由つきで拒否される。キャンセルで返金される |
| **S3 保存・復元** | 保存スロット/一覧、`inspect` による確認、警告/拒否の表示、`load`、`.gitignore`(U6)、未保存の確認 | 保存→アプリ再起動→復元で同じ状態。静的入力を変えた場合は警告つき/拒否が理由つきで出る。拒否・失敗時は現在状態が変わらない |

各段階で旧ゲーム(`cargo run`)の起動・操作・V1保存が変わらないことを確認する。GUI の手動確認項目は S1〜S3 の実装タスクで定義する(今回は設計のみで、確認待ちは作らない)。

---

## 10. 検証計画(実装タスクで自動化するもの)

| 観点 | 方法 |
|---|---|
| CLIと画面が同じ遷移 | 同じコマンド列(進行・発注・取消・保存・復元)を、(a) `EconomySession::apply` 直接、(b) ヘッドレス Bevy App(`MinimalPlugins`+セッション System、既存の `tests/*headless*` の作法)で流し、`snapshot` が一致。CLI の `--economy-*` 出力とも一致 |
| 日次・月次の二重実行なし | 1コマンド=N回の `advance_day`。フレーム数を増やしても日付が増えない(コマンドが無ければ進まない)。月次は `DayReport.monthly_ran` の回数が暦どおり |
| 注文・支払い・完成・返金の重複なし | 復元・再入場・同フレーム複数コマンドで、国庫・施設レベル・`next_order_seq`・`order_log` が一意に進む(P32-016 のテストを GUI 経路でも再実行) |
| 他国家への誤操作拒否 | operator=1 でステート21(国家2)へ発注 → `NotMappedToCountry`。国家2の注文IDで取消 → `NotIssuersOrder`。AI国家を operator にできない。未選択では全拒否 |
| UI入力が地図へ透過しない | ヘッドレスで UI ノードにホバー/押下を与え、`camera_input` が選択・パン・ズームを変えない(既存の `ui_is_hovered` 経路) |
| 表示切替・再入場で重複しない | `Tab` 往復を複数回:`EconomySession`/UI Entity が常に1つ、破棄後に残骸なし |
| 保存・再開と連続実行の一致 | 画面経路の保存→別プロセス(CLI `--economy-resume`)再開が連続実行と一致(逆も) |
| ロード失敗で現在状態不変 | 拒否・壊れたファイル・形式1・V1コピー・別シナリオで、`sim`・選択・カメラ・operator が不変(`assert_eq!` で全体比較) |
| 旧ゲームの維持 | 既存の全体テスト(旧V1テスト・ヘッドレス描画)が緑。`src/save/`・`saves/savegame_v1.ron` に差分なし |
| 保存先制限の維持 | UI から渡した名前でも `check_name`/保存先検証が効くこと(パス区切り・予約名・V1スロット名で拒否) |

---

## 11. 判断事項

### 11.1 実装前にユーザー判断が必要なもの

| # | 事項 | 推奨 | 代替 |
|---|---|---|---|
| **U1** | 起動経路 | A: 新バイナリ `map_economy_session`(§3) | B: `hex_map_preview` のフラグ。C(本体統合)は非推奨 |
| **U2** | 操作主体の決め方 | 画面で明示選択+`--operator` で事前指定(未選択は操作不可) | シナリオ形式へ「操作国家」項目を追加(シナリオ形式の変更が必要) |
| **U3** | 既定シナリオ | m4e(手動国家1・2+AI国家3)+資源 m4d | 新しい画面専用シナリオを作る(アセット追加) |
| **U6** | 画面から保存したファイルの置き場 | `saves/map_economy/` を `.gitignore` に追加(S3) | 保存先を別ディレクトリにする/追跡対象にする |

### 11.2 実装者が既存仕様に従って決めてよいもの

- U4 未保存のままの Tab/Esc:`dirty` の警告表示(既定は確認表示。無効化でも可)。
- U5 保存名:S3 は固定スロット+一覧(推奨)。自由入力は入力フォーカス対策と同時の後続段階。
- `MAX_DAYS_PER_COMMAND`、+7/+30 ボタンの刻み、画面文言、パネルの細かな配置、ログの保持件数。
- `check_order` を関数追加で実現するか clone 試行にするか(§5.4)。
- 建物定義読込の共有化の形(`app/loader.rs` の検証部を純粋関数へ切り出して再利用するか、新しい純粋関数を追加するか)。
- `HexPreviewSet` の具体的な切り方。

### 11.3 着手条件にしないもの

正式な世界設定、新マップ上の戦争・占領・軍移動・位置施設の効果、法的所有・部分割譲、補給の計算単位、旧セーブ/V2/D11 は、この経済セッションの着手条件ではない(いずれも未決のまま維持)。

---

## 12. 次の最小実装単位

**P32-019(S0): 経済セッションの中核の lib 化と CLI の共有化(画面なし)**
- `economy::map_session`(`SessionScenario` 表、`EconomySession`、`SessionCommand`/`apply`)、`m4d_simulation` のlib移動、CLI をその上へ。
- `check_order`、`inspect`、`list_saves` の追加(各々の等価性テスト付き)。
- 完了条件: P32-016/017 のテストと別プロセス再開が緑、新規テスト(コマンド列の決定性・`check_order`≡`place_order`・`inspect`≡`load` の判定)が通る。コード変更だが画面・アセット・セーブ形式は変えない。
- その後 S1(画面表示・日付進行)→ S2 → S3。

---

## 13. 実コードとの対応確認・差分

- 確認した事実: 旧ゲームと `hex_map_preview` は別 `App`(旧プラグインは新プレビューに登録されていない)/ `place_order`・`cancel_order` は識別子指定・他国家拒否・返金を持つ / `advance_day` は `date` を「最後に処理を完了した日」として進める / `SimDate` は `GameDate` と同じ暦 / `save`/`load` は `StaticInputs` を要求し、`load` は成功時のみ `sim` を置換する / m4d は全国家AI、m4e は手動国家1・2 / `.gitignore` は `/target` のみ。
- 設計時点で**無い**もの(実装で追加): 受付可否だけの判定関数、`inspect`、`list_saves`、`economy::map_session`(lib側の組み立て)、検証付きの建物定義読込関数、`HexPreviewSet`。
- 本書で変更したもの: この設計書のみ。コード・アセット・セーブ形式・`saves/`・`hex_atlas/` は変更していない。

## 14. リスクと未確認

- 新規 Bevy System の実機での見え方(日本語フォント、パネルのレイアウト、`Interaction` の透過防止)は実装段階の手動確認が必要(今回は未確認)。
- `hex_map_preview` の `--shots` 自動スクリプトは新バイナリでは使わない。スクリーンショット検証が必要なら S1 で別途定義する。
- Tab 往復で `LoadedHexMap` を読み直すコスト(約8MBのJSON)は既存の挙動。
- 自動進行(実時間)は含めていない。追加するときは累積器と1フレーム上限を設け、日次の入口を変えない。
