# P22-003 完了報告: 終局状態のロード再導出とオーバーレイ順序E2E

## 1. 目的とスコープ

P22-001(プレイヤー勝利)・P22-002(観戦者向け世界統一)の終局UI状態
(`PlayerEliminationMode`/`PlayerVictoryMode`/`WorldConclusionMode`)が、セーブファイルを
実際のread→validate→apply経路でロードした直後にも正しく再導出されることを自動テストで
固定する。機能追加ではなく、既存3タスクの設計(`CountryRegistry`から毎フレーム再導出する
ランタイムUI状態、セーブ非対象)を維持したまま、ロード直後の残留バグの有無を調査し、
見つかった場合は最小差分で修正する。

## 2. 作業前の状態確認

- 作業前HEAD: `a23f42dbe7ce4aa1b82304296e0c367f0739e951`(`feat: notify observers when the
  world is unified`)
- `git status --short`: 出力なし(クリーン)
- 保護すべき未コミット変更は存在しなかった。

## 3. 調査結果

### 3.1 ロード時の既存挙動(修正前)

`src/save/runtime.rs::handle_load_requests`(`PostUpdate`)を実ソースで確認した結果、
`apply_validated_save`が`ApplyLoadOutcome::Success`を返した後の処理は、
`PlayingEntryMode`/`NextState<GameState>`の制御、または`CameraResetRequestMessage`の
送信のみであり、`PlayerEliminationMode`/`PlayerVictoryMode`/`WorldConclusionMode`への
言及は`src/save/*.rs`のどこにも存在しなかった(`grep`調査で0件)。

### 3.2 状態残留バグの有無

**あり(推測ではなくテストで確認)。** 新規追加したP22-003 E2Eテスト(§7)で、修正前の
コードに対して以下を実際に再現させた:

- 前セッションで`PlayerVictoryMode::Acknowledged`のまま、敗北＋世界統一済みセーブを
  ロードすると、`Acknowledged`が残り続け、ロードした終局セーブなのに`WorldConclusionMode`
  への遷移判定自体は行われる(`WorldConclusionMode`は別Resourceなので無関係)が、
  もし同じ理屈で`WorldConclusionMode`自体が`Acknowledged`のまま残っていれば、世界統一が
  成立しているセーブをロードしても通知が出ない。
- 逆に、Pending状態(古い勝者国家ID・古い日付を保持)のまま通常のセーブをロードすると、
  そのPendingは`NotConcluded`へ戻らず古い値を保持し続け、実際には世界が統一されていない
  のに古い情報を表示し続ける経路になり得る。

### 3.3 原因

`PlayerEliminationMode`/`PlayerVictoryMode`/`WorldConclusionMode`はいずれも
`#[derive(Resource, Default, ...)]`で、対応するPluginの`build()`時に`insert_resource`
されるだけであり、実行中に明示的にリセットする経路がロード成功時に存在しなかったため。
これらのResourceが「`CountryRegistry`から再導出される」設計自体は正しいが、その
再導出は`detect_*`System側の「既に`NotConcluded`/`NotEliminated`でなければ何もしない」
という冪等ガードにより、**一度Pending/Acknowledgedになった値は上書きされない**設計に
なっている(P21-021/P22-001/P22-002がそれぞれ意図した「一度閉じたら再表示しない」という
仕様そのもの)。ロードは`CountryRegistry`を丸ごと置き換えるが、この3つのResourceには
一切触れないため、冪等ガードが「新しいセーブの状況」ではなく「古いセッションの状況」を
見たまま固まってしまう。

### 3.4 採用したリセット責務

`src/save/runtime.rs`に新規関数`reset_terminal_ui_state(world: &mut World)`(非`pub`、
このファイル内だけで完結)を追加し、`handle_load_requests`の
`ApplyLoadOutcome::Success`分岐の先頭(既存の`requested_from_country_selection`分岐より前)
で1回だけ呼ぶ。責務は「ロード成功が確定した直後に、終局UI用の3Resourceを初期状態へ戻す」
ことだけであり、複製せず1箇所に閉じ込めた。呼び出しは`Success`分岐内のみで、
`Failure`側では一切呼ばない(ロード失敗時は現在の3Resourceを変更しないという契約を
そのままコードで表現)。

### 3.5 副次的に発見した同一フレーム順序の抜け(修正)

E2Eテスト作成中に、ロード成功→リセット→検知Systemの再判定という流れ自体は正しく動作する
一方、その直後のオーバーレイ表示(`sync_*_overlay`)が期待通りその場で反映されないケースを
実際に検出した。原因は、`ui::player_elimination::sync_player_elimination_overlay`・
`ui::player_victory::sync_player_victory_overlay`・
`ui::world_conclusion::sync_world_conclusion_overlay`のいずれも、対応する
`country::detect_*`System(同じ`Update`スケジュール、`DailySimulationSet::UiUpdate`)との
明示的な実行順序を持たず、同一フレーム内でどちらが先に実行されるかがBevyのスケジューラ
任せ(未規定)だったこと。detect側が確定させた状態をsync側がまだ読む前に同じフレーム内で
走ってしまうと、オーバーレイの表示切替が実質1フレーム遅延する。

これはP22-003の主題(終局Resourceのロード残留)そのものではないが、「ロード直後に敗北通知と
世界統一通知の表示順序が崩れる」というユーザー指定の懸念事項に直結する、同種の同一フレーム
順序問題だったため、最小差分で`.after()`を3箇所追加して固定した(§5参照)。UI全面再設計・
新規Resource追加は行っていない。

## 4. New Game経路の調査

`grep -rn "NextState<GameState>" src/`で、`GameState`遷移を要求する箇所を全て洗い出した
結果、以下の3箇所のみだった。

1. `src/app/loader.rs::transition_to_country_selection` — 起動直後Startupの一度きりの
   `CountrySelection`遷移。
2. `src/save/runtime.rs::handle_load_requests` — ロード成功時の`CountrySelection`→
   `Playing`遷移(今回の修正対象)。
3. `src/ui/country_selection.rs` — 「Start Game」ボタンによる`CountrySelection`→
   `Playing`遷移(New Game)。

`Playing`から`CountrySelection`へ戻る遷移はコードベース中に一切存在しない(該当する
`NextState<GameState>::set(GameState::CountrySelection)`呼び出しは`transition_to_country_
selection`自身の1箇所のみで、これはStartupでしか呼ばれない)。つまり「New Game」は
プロセス生涯で最初の1回しか起こり得ず、その瞬間は`CountryPlugin`等の`Plugin::build()`が
`insert_resource(default)`する初期化タイミングと必ず一致する。したがって、New Game経路に
今回同種の状態残留リスクは存在しないと判断し、追加のリセット処理は実装していない。

## 5. 実装内容

### 変更ファイル

- `src/save/runtime.rs`: `reset_terminal_ui_state`関数の追加、`handle_load_requests`の
  Success分岐からの呼び出し、`PlayerEliminationMode`/`PlayerVictoryMode`/
  `WorldConclusionMode`のimport追加、既存テストfixture(`setup_minimal_save_app`)への
  この3 Resourceの`insert_resource`追加(この3つが無いと`handle_load_requests`が
  `world.resource_mut`でpanicするため、既存テスト11件が新たに必要とした)。
- `src/ui/player_elimination.rs`: `sync_player_elimination_overlay`に
  `.after(crate::country::detect_player_elimination)`を追加。ボタンマーカー
  `ContinueSpectatingButton`は前回セッション(P22-002)で既に`pub`化済み(変更なし)。
- `src/ui/player_victory.rs`: `sync_player_victory_overlay`に
  `.after(crate::country::detect_player_victory)`を追加。ボタンマーカー
  `ContinuePlayingButton`を新たに`pub`化(既存の`ContinueSpectatingButton`/
  `ContinueSpectatingAfterConclusionButton`と同じ理由、外部crateのE2Eテストから実ボタン
  押下を駆動するため)。
- `src/ui/world_conclusion.rs`: `sync_world_conclusion_overlay`に
  `.after(crate::country::detect_world_conclusion).after(crate::country::detect_player_
  elimination)`を追加(このSystemは`WorldConclusionMode`と`PlayerEliminationMode`の
  両方を読むため両方の後に固定)。
- `tests/p22_003_terminal_state_load_e2e_test.rs`(新規): §7参照。

### リセットされるResource

`PlayerEliminationMode`・`PlayerVictoryMode`・`WorldConclusionMode`の3つ。いずれも
`Default`実装の初期状態(`NotEliminated`/`NotConcluded`/`NotConcluded`)へ戻す。

### 成功ロード／失敗ロードの違い

- 成功(`ApplyLoadOutcome::Success`): `apply_validated_save`が`World`へ新しい
  `CountryRegistry`等を適用した直後に、この3 Resourceをリセットする。以後、同じ
  `Update`スケジュール内(通常は次フレーム)の`detect_player_elimination`/
  `detect_player_victory`/`detect_world_conclusion`が、ロード後の`CountryRegistry`
  だけを見て正しい状態へ再導出する。
- 失敗(`ApplyLoadOutcome::Failure`またはread/validateの`Err`): `reset_terminal_ui_state`
  は一切呼ばれない。現在のゲーム状態(`CountryRegistry`含む)・3つの終局Resourceは
  いずれも変更されない(既存の`apply_validated_save`のPrepare/Commit二段階設計により、
  失敗時はWorldへの書き込み自体が最初から発生しない)。

### New Gameへの影響

§4の調査により影響なし(追加コードなし)。

### セーブ形式への影響

**なし。** `SAVE_FORMAT_VERSION_V1`は据え置き。`SaveGameV1`へ新規フィールドを追加して
いない。3つの終局Resourceは引き続きセーブ非対象のランタイムUI状態のままである
(ユーザー指示「終局Resourceを安易にセーブ対象へ追加しない」を遵守)。

## 6. 実クライアント確認

実ウィンドウを起動・操作できる環境がないため、GUI目視確認は未実施。代わりに、下記§7の
自動E2Eで、実際のSave/Load Message経路・実際のボタンEntity(`Interaction::Pressed`)・
実際の3つのUI Overlay Root(`Node.display`)を用いて、要求された全シナリオを検証した。

## 7. E2Eテスト(`tests/p22_003_terminal_state_load_e2e_test.rs`、7件)

いずれも実7ゲームプラグイン(`AppPlugin`/`CountryPlugin`/`StatePlugin`/`BuildingPlugin`/
`EconomyPlugin`/`ResearchPlugin`/`PoliticsPlugin`/`DiplomacyPlugin`/`WarPlugin`/
`MilitaryPlugin`、実データ`assets/data/countries.ron`6か国)+ 実`SaveGamePlugin`/
`LoadGamePlugin`(`SaveRequestMessage`→`handle_save_requests`、`LoadRequestMessage`→
`handle_load_requests`という正式なメッセージ駆動経路)+ 実3終局UI Plugin
(`PlayerEliminationPlugin`/`PlayerVictoryPlugin`/`WorldConclusionPlugin`)を組み合わせた
統合テスト用Appを使用した。

**設計上の簡略化(P22-002と同じ判断)**: 消滅そのものを引き起こす講和・州移転の正しさは
既存E2E(P21-019/020/021)で検証済みのため、`CountryStatus::Eliminated`への遷移を
`CountryRegistry`へ直接反映してから正式なセーブ経路で保存する。ただし今回、この直接反映が
`save::validate`の不変条件(「Eliminated国家は州もDivisionも所有してはならない」)に
違反して最初から`InvalidRange`/`ParticipantMismatch`でロード失敗することを実際に発見した
ため、消滅国家の所有州を併合国へ再割当し、残存Divisionを解散する後処理を追加して、
検証を実際に通過する現実的な消滅済みセーブを作った。

| シナリオ | テスト名 | 結果 |
|---|---|---|
| A. プレイヤー勝利済みセーブ | `scenario_a_player_victory_save_loads_into_a_fresh_app_and_shows_only_the_victory_overlay` | PASS |
| B. 敗北＋他国世界統一済みセーブ | `scenario_b_defeat_and_world_conclusion_save_shows_defeat_before_world_conclusion` | PASS |
| C. 通常プレイ中セーブ(残留状態の初期化) | `scenario_c_normal_save_resets_stale_terminal_state_from_a_previous_session` | PASS |
| D-1. 勝利Acknowledged→敗北＋世界統一済みセーブ | `scenario_d_same_app_state_does_not_leak_across_loads::victory_acknowledged_then_loading_a_defeat_and_conclusion_save` | PASS |
| D-2. 世界統一Acknowledged→プレイヤー勝利済みセーブ | `scenario_d_same_app_state_does_not_leak_across_loads::conclusion_acknowledged_then_loading_a_player_victory_save` | PASS |
| D-3. Pending→通常セーブ | `scenario_d_same_app_state_does_not_leak_across_loads::pending_state_then_loading_a_normal_save` | PASS |
| E. ロード失敗 | `scenario_e_a_failed_load_leaves_the_current_game_and_terminal_state_untouched` | PASS |

**通したsave経路**: `SaveRequestMessage`書込→`handle_save_requests`(実RON書き込み)→
別Appまたは同一Appで`LoadRequestMessage`→`handle_load_requests`(`read_and_validate_save_
file`→`apply_validated_save`)の完全な正式経路。手書きRON文字列の直接構築は行っていない。

**オーバーレイ表示順序**: シナリオBで、ロード直後は`WorldConclusionMode`が既に
`OverlayPending`へ進んでいても`WorldConclusionOverlayRoot`の`Node.display`は`None`のまま
であり、`PlayerEliminationOverlayRoot`だけが`Flex`であることを確認。敗北オーバーレイの
実ボタン(`ContinueSpectatingButton`)を押した後にだけ世界統一オーバーレイが`Flex`へ
切り替わることを確認した。

**同一App内ロード結果**: シナリオD-1〜D-3で、意図的に汚した3Resourceの値(古い
`Acknowledged`・古い勝者国家ID・古い日付)が、ロード成功後は一切残らないことを確認した。

## 8. 検証結果

| コマンド | 結果 |
|---|---|
| `git diff --check` | PASS、exit 0(autocrlf警告のみ) |
| `rustfmt --edition 2024 --check`(本タスクで変更した5ファイル個別指定、保護対象は含めず) | PASS、drift 0(適用後) |
| `cargo check --all-targets` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、警告0 |
| `cargo test --test p22_003_terminal_state_load_e2e_test` | PASS、7 passed |
| `cargo test --lib country::tests::` | PASS、24 passed(P22-001・P22-002・敗北検知回帰込み) |
| `cargo test --lib ui::player_victory` / `ui::player_elimination` / `ui::world_conclusion` | PASS、4+4+7 passed |
| `cargo test --test p20_009_localization_resource_test` | PASS、8 passed |
| `cargo test --lib save::` | PASS、300 passed(fixture更新後) |
| `cargo test --test p21_save_002e_end_to_end_test --test p21_save_003_end_to_end_test` | PASS、7 passed |
| `cargo test --test p22_002_world_conclusion_e2e_test` | PASS、1 passed(P22-002回帰) |
| `cargo test --all-targets` | PASS、**全件PASS**(lib 1073 + integration、新規+8[P22-003が7件、既存fixtureの11件は既存テストとしてカウント済み]) |

`tests/land_war_combat_peace_test.rs`は変更・整形していない(既知の保護対象)。

### 未実施の目視確認

実クライアントGUI上での、以下の見た目・操作感。

- プレイヤー勝利済みセーブのロード直後に勝利オーバーレイが表示されること
- 敗北＋世界統一済みセーブで、敗北→世界統一の順に表示されること
- ボタンが押せること、オーバーレイを閉じた後に観戦・プレイを続行できること
- 国家名、日付、改行、ボタン位置の崩れがないこと
- JA/EN両方でplaceholderが正しく展開されること(自動テストではlocalization整合性
  ―キー集合・placeholder集合の一致―のみ確認、実際のレンダリング結果は未確認)

## 9. 手動確認手順(ユーザー向け)

1. ゲームを起動し、いずれかの国でプレイを開始する。
2. デバッグ等の手段でプレイヤー国家以外の全国家を消滅させ(または通常プレイで世界征服を
   達成し)、その状態でセーブする。
3. ゲームを再起動し(または一旦タイトルへ戻れないため、プロセスを再起動し)、「続きから」
   で同じセーブをロードする。
4. ロード直後に「世界統一」(勝利)オーバーレイが表示され、国家名・日付が読めることを
   確認する。「プレイを続ける」ボタンで閉じ、通常操作に戻れることを確認する。
5. 別途、プレイヤー国家が消滅し、かつ残り1か国だけが生き残っている状態のセーブを作り、
   同様にロードする。まず敗北オーバーレイが表示され、「観戦を続ける」を押した後にだけ
   世界統一オーバーレイが表示されることを確認する。
6. 日本語・英語の両方の言語設定で2〜5を再確認し、文言・placeholder展開・レイアウトに
   崩れがないことを目視する。

## 10. 次の推奨タスク

会話内の最終報告(§次の推奨タスク)を参照。
