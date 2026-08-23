# P22-002 完了報告: 観戦中の世界統一検知と終戦通知

## 1. 目的とスコープ

P22-001「プレイヤー世界征服検知と勝利オーバーレイ」の対称ケースとして、プレイヤー国家が
既に消滅し観戦状態になっている間に、他国同士の戦争・講和・併合によって世界の存続国家が
1か国だけになった場合を検知し、観戦者向けの終戦オーバーレイを表示する。

追加勝利条件・同盟陣営単位の勝利・複数国家残存でのゲーム終了・統一後のコマンド制限・
タイトル画面帰還・エンドクレジット・統計画面・P22-001の仕様変更・敗北システムの再設計・
AI/戦争システムの変更・既知scaffoldスタブの削除・PNG非決定性の根本修正は、いずれも
ユーザー指示により対象外とした。

## 2. 作業前の状態確認

- 作業前HEAD: `5d8e56a0815b36fecef41e37fa19c71886d9e509`(`feat: establish Phase 21 integration
  and P22 victory baseline`)
- `git status --short`: 出力なし(クリーン)
- 上記を確認した上で作業を開始した。既存の未コミット変更は存在しなかったため、保護すべき
  ユーザー変更もなかった。

## 3. 検知条件

`country::detect_world_conclusion`(`src/country/mod.rs`)が以下を`CountryRegistry`
(正規のソース)から毎フレーム再導出して判定する。HashMap走査順には依存しない
(`CountryRegistry::active_country_ids()`は`CountryId`昇順にソート済みのVecを返す、
P21-021由来の既存の唯一の列挙経路)。

1. `WorldConclusionMode`が`NotConcluded`である(既にPending/Acknowledgedなら即return、冪等)
2. `country_registry.countries.len() <= 1`なら即return(縮退データ除外)
3. `PlayerCountry`が`Some`である
4. その国家が`is_eliminated()`である(=プレイヤーが消滅済み)
5. `active_country_ids()`の長さが正確に1、かつその1件がプレイヤーIDと異なる
   (Active国家0件の場合は`len() == 1`が成立しないため自然に発火しない)

全条件を満たした瞬間にのみ`WorldConclusionMode::OverlayPending { winner_country_id,
conclusion_date }`へ遷移する。`conclusion_date`は検知した瞬間の`GameDate::display()`を
固定値として保持し(以後ゲーム内時間が進んでも上書きされない)、敗北オーバーレイを
閉じた日ではなく実際にActive国家が1か国になった日を記録する。

## 4. Resourceと状態遷移

```rust
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub enum WorldConclusionMode {
    #[default]
    NotConcluded,
    OverlayPending {
        winner_country_id: CountryId,
        conclusion_date: String,
    },
    Acknowledged,
}
```

`PlayerEliminationMode`/`PlayerVictoryMode`と同じ設計判断でSaveへは含めない
(`CountryRegistry`から毎フレーム再導出するランタイムUI状態)。`CountryPlugin`へ
`insert_resource(WorldConclusionMode::default())`として登録した。

## 5. System実行順序

```rust
detect_world_conclusion
    .in_set(DailySimulationSet::UiUpdate)
    .after(detect_player_elimination)
    .after(detect_player_victory)
    .run_if(in_state(GameState::Playing))
```

- **州所有権移転・国家消滅が反映された後に判定する**: `DailySimulationSet::UiUpdate`は
  既存コメント(P21-014)の通り最終Setであり、`WarResolution`等での州所有権移転・
  `country::elimination::apply_country_elimination`によるステータス変更は同じフレーム内で
  先行して確定している。
- **プレイヤー敗北検知との整合性**: `.after(detect_player_elimination)`を付与した。ただし
  `detect_world_conclusion`自体は`CountryRegistry`を直接読んで`is_eliminated()`判定するため、
  `PlayerEliminationMode`の値そのものには依存しない(Resource競合はない)。この`.after()`は
  「消滅判定が確定した後の状態を読む」という一貫したフレーム内順序をコードで明示するための
  ものであり、UI側の表示抑制ロジック(§6)と対になる設計判断であることをdocコメントに明記した。
- **P22-001の勝利検知との排他性**: `.after(detect_player_victory)`を同じ理由で付与した。
  `WorldConclusionMode`と`PlayerVictoryMode`が同時に`Pending`へ遷移することは、判定条件が
  「プレイヤーが消滅済み」対「プレイヤーが唯一の生存国(=Active)」という排他的な条件である
  ため構造的にあり得ない(country側テスト`detect_world_conclusion_does_nothing_when_the_
  player_itself_is_the_sole_survivor`で確認)。
- **UI spawnより前に状態が確定する**: `setup_world_conclusion_overlay`は`OnEnter(Playing)`で
  一度だけ実行され、`sync_world_conclusion_overlay`(Update)より先に(スケジュール上必ず)
  実行される。

## 6. 敗北オーバーレイ・P22-001勝利オーバーレイとの排他制御

`ui::world_conclusion::sync_world_conclusion_overlay`は、`WorldConclusionMode`が
`OverlayPending`であっても、`PlayerEliminationMode`が`DefeatOverlayPending`(プレイヤー自身の
敗北オーバーレイをまだ閉じていない)である間は`Node.display`を`Display::None`のまま維持する。
Domain状態(`WorldConclusionMode`)は敗北オーバーレイの解除を待たずに`OverlayPending`へ
進めてよい(実際にActive国家が1か国になった日を正しく記録するため)が、**UI表示だけ**を
`PlayerEliminationMode`が`Spectating`(=敗北オーバーレイを閉じた後)になるまで抑制する設計を
選んだ。

判定は`mode.is_changed() || elimination_mode.is_changed()`のいずれかで再評価するため、
「敗北オーバーレイをPendingのまま閉じずに放置し、その後ボタンを押してSpectatingへ遷移した」
瞬間にも正しく世界統一オーバーレイが現れる(country/ui双方の統合テストで確認、§8参照)。

P22-001のプレイヤー勝利オーバーレイとの同時表示防止は、上述の通り両Modeが構造的に排他な
ため自動的に満たされる。`ui::world_conclusion::tests::player_victory_overlay_does_not_show_
alongside_the_world_conclusion_overlay`で、両UI Pluginを同一Appへ実際に登録した上で
`WorldConclusionMode`だけがPendingの状態を作り、`PlayerVictoryOverlayRoot`が非表示のままで
あることを直接確認した。

## 7. UIとlocalization

`src/ui/world_conclusion.rs`(新規)を、既存の`player_elimination.rs`/`player_victory.rs`と
同一のUI構造・スタイル・モーダル管理方式(`OnEnter(Playing)`でspawn、`OnExit(Playing)`で
despawn、Update内で`Node.display`をトグル)で追加し、`ui::mod::UiPlugin`へ
`WorldConclusionPlugin`として登録した。

localizationキー(JA/EN両方へ追加、既存の`{placeholder}`記法・`localized_text`/`translate`
経由の表示に統一、Rustコードへのハードコードなし):

| キー | EN | JA |
|---|---|---|
| `world_conclusion.title` | The World is Unified | 世界統一 |
| `world_conclusion.body` | {country} now stands as the last remaining nation, as of {date}, having unified the known world. | {country}が世界最後の存続国となり、{date}に世界を統一しました。 |
| `world_conclusion.continue_button` | Continue Spectating | 観戦を続ける |

既存のlocalization整合性テスト(`p20_009_localization_resource_test.rs`)を再実行し、
JA/ENキー集合完全一致・placeholder集合完全一致・欠落キーなしを確認した(8/8 PASS)。

`ui::peace_panel::PeaceActionButton`と同じ理由で、実ボタン押下によるE2Eテストを可能にする
ため`ui::player_elimination::ContinueSpectatingButton`と本タスクで新設した
`ui::world_conclusion::ContinueSpectatingAfterConclusionButton`を`pub`にした
(既存の公開パターンへ合わせただけで、挙動は変更していない)。

## 8. セーブ形式への影響

**変更なし。** `WorldConclusionMode`はP22-001の`PlayerVictoryMode`/`PlayerEliminationMode`と
同じ設計判断により、`CountryRegistry`(セーブ済み)から毎フレーム再導出するランタイムUI状態
として扱い、`SaveGameV1`へは一切含めていない。`SAVE_FORMAT_VERSION_V1`は据え置き。

Load直後の再検知について実コードを確認し、以下がいずれも起きないことを確認した。

- **毎フレームの多重spawn**: `setup_world_conclusion_overlay`は`OnEnter(Playing)`で1回のみ
  実行される(既存2オーバーレイと同一パターン)。
- **勝者国家IDの参照切れ**: `winner_country_id`は`active_country_ids()`が返す実在するActive
  国家のIDのみを格納する。表示側(`sync_world_conclusion_overlay`)は
  `country_registry.get(*winner_country_id)`が`None`を返しても`unwrap_or_default()`で
  空文字列にフォールバックし、panicしない。
- **Active国家0件でのpanic**: `active_ids.len() == 1`の条件チェックにより、0件では
  そもそも`OverlayPending`へ遷移しない(`detect_world_conclusion_does_nothing_when_no_
  country_is_active`で確認)。
- **プレイヤー勝利と観戦者通知の同時発火**: §6・country側テストで確認済み。
- **敗北通知より先に世界統一通知が表示される**: §6の表示抑制ロジックと、UI側テスト
  `overlay_stays_hidden_while_the_defeat_overlay_has_not_been_dismissed`・統合テストで
  確認済み。

Load直後は`PlayerEliminationMode`と同様`WorldConclusionMode`も常に`NotConcluded`から始まり、
`detect_world_conclusion`が同じフレームでロード後の`CountryRegistry`を見て正しい状態へ
再遷移する(既に世界統一済みのセーブをロードした場合、`PlayerEliminationMode`が
`Spectating`へ即座に再遷移した後――両者は同じUiUpdate Set内で処理されるため実質同フレームで
――世界統一オーバーレイも正しく再表示される)。

## 9. テスト

### country側(9件、`src/country/mod.rs`)

1. `detect_world_conclusion_transitions_when_player_is_eliminated_and_one_rival_remains_active`
   — 正常系: 遷移すること
2. `detect_world_conclusion_records_the_winner_id_and_conclusion_date` — 勝者ID・成立日の
   正確な記録
3. `detect_world_conclusion_does_nothing_while_the_player_is_still_active` — プレイヤーが
   Activeなら発火しない
4. `detect_world_conclusion_does_nothing_when_the_player_itself_is_the_sole_survivor` —
   プレイヤー自身が唯一の生存国ならP22-002は発火しない(P22-001との境界)
5. `detect_world_conclusion_does_nothing_when_two_or_more_rivals_remain_active` — Active
   2か国以上では発火しない
6. `detect_world_conclusion_does_nothing_when_no_country_is_active` — Active 0か国では
   発火しない
7. `detect_world_conclusion_does_nothing_for_a_degenerate_single_country_world` — 縮退世界
   では発火しない
8. `detect_world_conclusion_does_not_revert_acknowledged_back_to_pending` — 冪等性
9. `detect_world_conclusion_does_not_update_again_after_multiple_days_once_pending` —
   Pending後の複数日進行で多重更新されない

### UI側(7件、`src/ui/world_conclusion.rs`)

1. `overlay_is_spawned_hidden_by_default`
2. `overlay_stays_hidden_while_the_defeat_overlay_has_not_been_dismissed` — Pendingでも
   敗北通知未処理なら表示しない
3. `overlay_shows_once_the_defeat_overlay_has_been_dismissed` — 敗北通知を閉じた後に表示
4. `overlay_transitions_from_hidden_to_shown_when_the_defeat_overlay_is_dismissed_afterward`
   — 非表示→表示への実際の遷移
5. `overlay_hides_again_once_acknowledged` — Acknowledged後は再表示しない
6. `continue_button_transitions_mode_and_closes_overlay` — ボタン押下でAcknowledgedへ
7. `player_victory_overlay_does_not_show_alongside_the_world_conclusion_overlay` —
   P22-001プレイヤー勝利オーバーレイとの非同時表示(両UI Pluginを実際に同一Appへ登録)

### 統合テスト(1件、`tests/p22_002_world_conclusion_e2e_test.rs`)

`defeat_overlay_shows_before_world_conclusion_overlay_and_neither_reappears_after_dismissal`
— 実際の`detect_player_elimination`/`detect_world_conclusion`System・実際の
`PlayerEliminationPlugin`/`WorldConclusionPlugin`・実際のボタン押下
(`Interaction::Pressed`)を使い、以下を1本で固定した。

1. プレイヤー国家と対戦国がActive(両オーバーレイとも非表示)
2. `CountryRegistry`へプレイヤー国家の`CountryStatus::Eliminated`を反映(講和・州移転の
   出力そのもの)→対戦国だけがActiveになる
3. 敗北オーバーレイが先に表示される(世界統一のDomain状態は既にPendingだが、UIはまだ
   出ない)
4. 実ボタン押下で敗北通知を閉じる
5. 世界統一オーバーレイが表示される(勝者国名"Passguard"・成立日"1850/06/15"を含む)
6. 実ボタン押下で世界統一通知を閉じる
7. 5フレーム経過後も両オーバーレイとも再表示されない

**設計上の簡略化(意図的)**: 消滅そのものを引き起こす講和・州移転の正しさ自体は
`tests/p21_019_crisis_war_peace_resolution_e2e_test.rs`・
`tests/p21_020_ownership_transfer_e2e_test.rs`・
`tests/p21_021_country_elimination_e2e_test.rs`が既に実データ(6か国28州の本番マップ、
実際の正当化→宣戦→講和ボタン押下)でE2E検証済みのため、本テストは合成2か国世界の
`CountryRegistry`へ結果(`CountryStatus::Eliminated`)を直接反映する形を取り、
`country::CountryPlugin`全体(AI・国力再評価等の無関係な重量級System群)ではなく
`detect_player_elimination`/`detect_world_conclusion`の2 Systemだけを実際の`.after()`
順序付きで直接登録した。本テストの主眼(消滅後のオーバーレイ表示順序)には十分な忠実度
であり、「既存のUIテスト基盤で困難な場合はDomain状態遷移とUI表示条件を分離して直接
テストする」という許容範囲内の設計判断である。

### 実行コマンドと結果

| コマンド | 結果 |
|---|---|
| `git diff --check` | PASS、exit 0(autocrlf警告のみ) |
| `rustfmt --edition 2024 --check`(本タスクで変更した5ファイル個別指定、保護対象は含めず) | PASS、drift 0(適用後) |
| `cargo check --all-targets` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、警告0 |
| `cargo test --lib country::tests::detect_world_conclusion` | PASS、9 passed |
| `cargo test --lib ui::world_conclusion` | PASS、7 passed |
| `cargo test --lib country::tests::` | PASS、24 passed(P22-001・敗北検知回帰込み) |
| `cargo test --lib ui::player_victory` / `ui::player_elimination` | PASS、4+4 passed(回帰) |
| `cargo test --test p20_009_localization_resource_test` | PASS、8 passed |
| `cargo test --lib save::` | PASS、300 passed |
| `cargo test --test p21_save_002e_end_to_end_test --test p21_save_003_end_to_end_test` | PASS、7 passed |
| `cargo test --test p22_002_world_conclusion_e2e_test` | PASS、1 passed |
| `cargo test --all-targets` | PASS、**1267 passed; 0 failed**(lib 1073 + integration 194) |

`tests/land_war_combat_peace_test.rs`は今回のrustfmt対象から明示的に除外し、整形・変更して
いない(既知の保護対象)。

### 未実施の手動確認

- 実クライアントGUI上でのオーバーレイの見た目・レイアウト・ボタン操作感
- 6か国マップでの実プレイによる「プレイヤー消滅後に他国が世界統一する」到達自体

## 10. 発見した問題(今回修正しなかったものを含む)

- 前回セッション(P22-MAINT-001)で報告済みの、`cargo test --all-targets`実行による
  Playing画面PNG9枚の非決定的再生成は今回も再現した。個別`git checkout --`で復元済み
  (根本修正は対象外)。
- 前回報告済みの`tests/land_war_combat_peace_test.rs`の既知rustfmt drift、および
  `military::ai::process_military_ai`/`military::combat::process_combat`(引数なし版)の
  scaffoldスタブは、今回のコミット差分に含まれていない(いずれも触れていない)。

## 11. 次の推奨タスク

会話内の最終報告(§次の推奨タスク)を参照。
