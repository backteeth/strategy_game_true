# P21-016 完了報告: Crisis支持コミットメントを多国間War参加へ接続

## 1. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**(二次レビュー対応後、§28)

`cargo check --all-targets` / `cargo test --lib` / `cargo test --tests` / `cargo clippy
--all-targets --all-features -- -D warnings` / `cargo build --release` / `git diff
--check` はすべて成功。実データ(6か国28州の本番マップ)を使ったE2Eテストで
Crisis拒否→支持スナップショット確定→AI宣戦布告→多国間War参加→敵味方判定共有API
→save往復までの全経路を確認済み。ただし本タスクでは対話的GUIセッションを
実行していない(§25参照)。

**二次レビューで5件の指摘を受け、すべて修正・再検証済み**(§28に詳細): (1)
justification↔Crisis↔Claimの整合性検証の欠落、(2)宣戦後にCrisis来歴が消える
問題(`War.source_crisis_id`未実装)、(3)実戦闘・占領・講和への直接接続検証の不足
(レビュー対応の中で`war/peace.rs::execute_peace_settlement`に未修正の
`.iter().next()`が残っていたことも発見・修正)、(4)代表国フォールバックが
架空の`CountryId(0)`を捏造するリスク、(5)性能測定規模が要求(最大1000 War・
陣営あたり100参加国)に未達。

---

## 2. 実装前調査結果(要約)

事前調査により、仕様書が前提としていた「Warは単一攻撃国・単一防御国のフィールドしか
持たない」という想定は誤りであることが判明した。実際には`War.attackers`/
`.defenders`はP20-16時点から既に`HashSet<CountryId>`であり、以下は当初から
複数国を正しく扱う設計だった:

- `WarRegistry::are_countries_at_war` — 陣営横断の`contains`判定で実装済み。
- `war/combat.rs`・`war/occupation.rs`・`war/peace.rs`・`war/military_ai.rs`・
  `military/invasion.rs` — いずれも
  `(attackers.contains(&a) && defenders.contains(&b)) || (逆)`という
  陣営横断パターンで実装済みで、変更不要と確認した。

一方、真に欠けていたのは以下の3点のみだった(範囲を大幅に絞り込めた):

1. **代表国(primary)の明示的フィールドが無い** — `frontline.rs`(4箇所)・
   `capitulation.rs`(2箇所)・`war_score.rs`(2箇所)・`ui/peace_panel.rs`(4箇所)が
   `attackers.iter().next()`のような`HashSet`走査順依存の「先頭要素」取得をしており、
   複数国が参加すると非決定的になるバグの温床だった。
2. **`declare_war`が常に単一国ずつしか各陣営へ追加しない**。
3. **Crisisの支持コミットメントからWar参加へ渡す経路(スナップショット)がそもそも
   存在しない**。

この結果、実装差分は仕様書の想定より大幅に小さく、「Warをゼロから多国間対応させる」
のではなく「既に多国間対応していた基盤に、代表国の明示化とCrisisスナップショットの
接続を足す」作業になった。

---

## 3. 支持コミットメント・スナップショットの設計

**捕捉タイミングと場所**: `crisis_response::apply_rejection`(プレイヤー拒否・AI拒否・
期限切れタイムアウトの全経路が通る唯一の共有関数、P21-011由来)の中で、
`WarJustification`がReadyになる直前に`build_support_snapshot(crisis)`を呼び、
`crisis.third_party_reactions`から`ThirdCountryReaction::SupportsInitiator`/
`SupportsTarget`のみを抽出する。要求国・対象国自身は防御的に除外し、
`CountryId`昇順・重複なしでソートする。

**なぜ宣戦布告時点の再照会ではなくスナップショットなのか**: 同じ国家ペアに対して
時系列で複数のCrisisが存在し得るため、宣戦布告時点で`CrisisRegistry`を
国ペアから逆引きすると「どのCrisis由来か」が曖昧になる。`WarJustification`へ
確定時点の値をコピーして持たせることで、この曖昧さを構造的に排除した。

```rust
// src/war/justification.rs — WarJustification
#[serde(default)] pub source_crisis_id: Option<DiplomaticCrisisId>,
#[serde(default)] pub committed_attackers: Vec<CountryId>,
#[serde(default)] pub committed_defenders: Vec<CountryId>,
```

`grant_completed_justification`は3引数を追加(7引数化、`#[allow(too_many_arguments)]`)。
既存正当化を「引き上げる」分岐でもこれら3フィールドを最新の呼び出し内容で上書きする
(同一国家ペアに複数Crisisが存在した場合、最後に完了したCrisisのスナップショットを
正とする — 既存の`is_ready`/`days_passed`上書きと同じ規約)。

---

## 4. War参加国モデルと代表国(primary)フィールド

`War`に`primary_attacker: Option<CountryId>` / `primary_defender:
Option<CountryId>`を追加。`#[serde(default)]`で旧Save互換(`None`)。

**設計判断: Option+フォールバック・アクセサ方式(センチネル値+マイグレーションパスは
不採用)**。旧SaveのWarは`attackers`/`defenders`が常に単一要素だったという不変条件を
利用し、`primary_attacker_id()`/`primary_defender_id()`は`None`の場合に
`attackers`/`defenders`集合内の最小`CountryId`へ自動的にフォールバックする:

```rust
pub fn primary_attacker_id(&self) -> CountryId {
    self.primary_attacker.unwrap_or_else(|| {
        self.attackers.iter().copied().min_by_key(|c| c.0).unwrap_or(CountryId(0))
    })
}
```

これにより、P21-014で踏んだ「`OnEnter(GameState::Playing)`は再ロード時に発火しない」
問題のような「マイグレーションパスの実行漏れ」リスクを構造的に排除した——毎回の
アクセス時点で正しい値を計算するため、ロード経路を問わず常に安全。

`frontline.rs`(4箇所)・`capitulation.rs`(2箇所)・`war_score.rs`(2箇所)・
`ui/peace_panel.rs`(4箇所)の`.iter().next()`をすべて`primary_attacker_id()`/
`primary_defender_id()`へ置き換えた。これらの既存ロジック(前線境界描画・降伏判定・
戦勝点計算・講和相手選定)は仕様の指示通り「代表国中心の二国間ロジックのまま」
据え置き、意味を変えていない。

---

## 5. `declare_war`の変更(スナップショット消費とアトミック性)

`WarRegistry::declare_war`の公開シグネチャ(2引数の国家ID + 既存引数)は変更していない
——呼び出し元(`country_ai.rs`・`diplomacy_panel.rs`)は共に、宣戦布告前に
`get_ready_justification`から`justification_id`を別途取得する既存パターンを
既に使っており、変更不要だった。

内部処理:
1. `can_declare_war_with_date`による既存検証(不変)。
2. 正当化を**消費する前**に`get_ready_justification`で`committed_attackers`/
   `committed_defenders`を読む(借用の生存中に値をコピー)。
3. **アトミック性チェック**(このいずれかに該当すれば、正当化消費・外交関係変更を
   一切行わずWar宣言全体を失敗させる):
   - `committed_attackers`と`committed_defenders`に同一国が同時に存在する。
   - `committed_attackers`が対象国(target)自身を含む、または
     `committed_defenders`が要求国(initiator)自身を含む(矛盾したデータ)。
4. 正当化消費・外交関係悪化(既存の-50.0)は不変。
5. `attackers = {initiator} ∪ committed_attackers`、`defenders = {target} ∪
   committed_defenders` — ただし**宣戦布告時点で実在しない支持国は個別に静かに
   除外する**(集合単位のアトミック拒否とは別の規則、仕様書§7の指示通り)。
6. `primary_attacker: Some(initiator)`、`primary_defender: Some(target)`を
   明示的に設定。

---

## 6. 共有敵味方判定API

```rust
// src/war/data.rs
pub enum WarSide { Attacker, Defender }
impl War {
    pub fn side_of(&self, country: CountryId) -> Option<WarSide>;
    pub fn is_participant(&self, country: CountryId) -> bool;
    pub fn are_opponents(&self, a: CountryId, b: CountryId) -> bool;
    pub fn opponents_of(&self, country: CountryId) -> Vec<CountryId>; // CountryId昇順
    pub fn sorted_attackers(&self) -> Vec<CountryId>;                // CountryId昇順
    pub fn sorted_defenders(&self) -> Vec<CountryId>;                // CountryId昇順
}
impl WarRegistry {
    pub fn is_country_at_war(&self, country: CountryId) -> bool;
    pub fn wars_for_country(&self, country: CountryId) -> Vec<WarId>; // WarId昇順
    // are_countries_at_war は既存(P20-16由来)のものを再利用、重複実装しない
}
```

既存の`WarRegistry::are_countries_at_war`は仕様が要求する意味論を既に完全に
満たしていたため、そのまま再利用し重複実装しなかった。

---

## 7. 戦闘・前線・軍事AI・占領との接続確認

事前調査(§2)の通り、`combat.rs`・`occupation.rs`・`peace.rs`・`military_ai.rs`・
`military/invasion.rs`は既に陣営横断の`contains`判定で実装されており、
**追加の接続作業は不要**と確認した(実データE2Eテストで
`are_countries_at_war(attacker_supporter, target)`等が正しく`true`を返すことを
直接検証済み、§16参照)。

`frontline.rs`の前線境界計算(`calculate_frontline_border`等)は代表国中心の
二国間概念のまま据え置いた(多国間参加者全体を結ぶ前線メッシュ描画は仕様の
対象外、既存の「二国間ロジックは維持」という指示に従う)。

---

## 8. Save/Load・バリデーション

`src/save/validate.rs`に追加した検証(`RefIndex`へ`crises: HashSet<DiplomaticCrisisId>`
を新設):

- `War.primary_attacker`/`primary_defender`(`Some`の場合)は対応する集合の要素で
  なければならない(`ParticipantMismatch`)。
- `WarJustification.source_crisis_id`(`Some`の場合)は実在するCrisisを参照しなければ
  ならない(`DanglingReference`)。
- `committed_attackers`/`committed_defenders`内の各国家は実在し(`DanglingReference`)、
  重複せず(`DuplicateId`)、正当化自身のinitiator/targetと一致してはならない
  (`SetOverlap`)。
- `committed_attackers`と`committed_defenders`は互いに重複してはならない
  (`SetOverlap`)。

**旧Save互換性**: `War.attackers`/`.defenders`は元々必須フィールドのため、旧Saveの
マイグレーション(「配列欠落時は`[primary]`を補う」)は実質的に不要だった——
真に新しいのは`primary_attacker`/`primary_defender`(`Option`、`None`のまま安全に
動作)と`WarJustification`の3新フィールド(`Vec`/`Option`、空/`None`が安全な
デフォルト)のみで、いずれも実RONを手書きしたデシリアライズテストで後方互換性を
直接確認した(§14参照)。仕様書が想定していた「配列欠落時のマイグレーションパス」は
実際には不要と判明した点は正直に記録する。

---

## 9. UI変更

`ui/diplomacy_panel.rs`の「進行中の戦争」一覧を、`war.attackers.iter()`(走査順が
非決定的)から`war.sorted_attackers()`/`sorted_defenders()`(`CountryId`昇順)へ
変更し、代表国(war-leader)には色ではなくテキスト接尾辞("(Leader)" /
"（主導国）"、`diplomacy_panel.war_leader_suffix`)で明示するようにした。
JA/EN両方のローカライズキーを追加。

`ui/peace_panel.rs`の講和申し入れ相手・戦争ヘッダー表示は、代表国
(`primary_attacker_id`/`primary_defender_id`)を基準とする既存の二国間ロジックの
まま据え置いた(講和は仕様の指示通り代表国中心のまま変更しない)。

---

## 10. 事前調査で発見した保護対象ファイル一覧との対応

`combat.rs`・`occupation.rs`・`peace.rs`・`military_ai.rs`・`military/invasion.rs`は
無変更(既に正しい)。変更したのは`frontline.rs`・`capitulation.rs`・`war_score.rs`・
`ui/peace_panel.rs`の「先頭要素取得」箇所のみで、いずれも**値の選び方**
(非決定的→明示的primary)を変えただけで、既存の判定ロジック自体の意味は変えていない。

---

## 11. 変更・新規ファイル一覧(P21-016で実際に touch したもののみ)

**変更**:
- `src/diplomacy/crisis_response.rs`(`build_support_snapshot`追加、
  `apply_rejection`から呼び出し、`DiplomaticCrisis`インポート追加)
- `src/war/justification.rs`(`WarJustification`3新フィールド、
  `grant_completed_justification`7引数化)
- `src/war/data.rs`(`primary_attacker`/`primary_defender`、`WarSide`、
  敵味方判定API、`declare_war`のスナップショット消費・アトミック性チェック)
- `src/war/frontline.rs`・`src/war/capitulation.rs`・`src/war/war_score.rs`
  (`.iter().next()`→`primary_attacker_id()`/`primary_defender_id()`)
- `src/ui/peace_panel.rs`・`src/ui/diplomacy_panel.rs`(代表国明示、
  ソート済み参加国一覧表示)
- `src/save/validate.rs`(新規4種の検証、`RefIndex`へ`crises`追加)
- `src/war/tests.rs`・`src/military/tests.rs`(`War`リテラルへの新フィールド追加)
- `assets/localization/{en-US,ja-JP}.ron`(`war_leader_suffix`キー追加)
- `tests/p21_013_crisis_support_e2e_test.rs`
  (`supporter_is_not_auto_added_to_the_resulting_war`をP21-016の新仕様に合わせて
  更新、§13参照)
- `tests/{p21_005_army_frontline_e2e_test,p21_008_army_offensive_e2e_test}.rs`
  (`War`リテラルへの新フィールド追加のみ、ロジック変更なし)
- `src/profiling.rs`・`src/map/{frontline_render,frontline_selection,
  offensive_line_selection}.rs`・`src/military/{invasion,update}.rs`・
  `src/save/{apply,export}.rs`・`src/ui/military_panel.rs`
  (`War`リテラルへの新フィールド追加のみ、ロジック変更なし)

**新規**:
- `tests/p21_016_multilateral_war_e2e_test.rs`

---

## 12. 新規テスト内訳

**`src/diplomacy/crisis_response.rs`(単体、+3)**:
`reject_demand_captures_support_snapshot_into_granted_justification`、
`reject_demand_snapshot_excludes_neutral_and_condemning_reactions`、
`reject_demand_snapshot_is_sorted_by_country_id_ascending`

**`src/war/tests.rs`(単体、+9)**:
`declare_war_adds_committed_supporters_to_correct_sides`、
`declare_war_excludes_nonexistent_supporter_silently`、
`declare_war_rejects_atomically_when_supporter_appears_on_both_sides`、
`declare_war_rejects_atomically_when_supporter_equals_the_opposing_belligerent`、
`war_shared_enemy_friend_api_handles_multilateral_participants`、
`primary_id_accessors_fall_back_to_min_of_set_when_unset`、
`war_registry_is_country_at_war_and_wars_for_country_cover_supporters`、
`war_deserializes_from_pre_p21_016_ron_without_primary_fields`(実RON手書き)、
`war_justification_deserializes_from_pre_p21_016_ron_without_snapshot_fields`
(実RON手書き)

**`src/save/validate.rs`(単体、+10)**:
`primary_attacker_not_in_attackers_is_rejected`、
`primary_defender_not_in_defenders_is_rejected`、
`primary_attacker_in_attackers_is_accepted`、
`unknown_source_crisis_id_is_rejected`、`unknown_committed_attacker_is_rejected`、
`duplicate_committed_attacker_is_rejected`、
`committed_attackers_and_defenders_overlap_is_rejected`、
`committed_attacker_matching_own_initiator_or_target_is_rejected`、
`valid_committed_supporters_are_accepted`、`valid_source_crisis_id_is_accepted`

**`tests/p21_016_multilateral_war_e2e_test.rs`(実データE2E、+1)**:
`multiple_supporters_on_both_sides_become_war_participants_on_correct_sides`
——実データ(6か国28州)で、攻撃側・防御側それぞれに支持を表明した第三国
(傍観プレイヤーとは別の2か国)が、拒否→AI宣戦布告後に正しい陣営のWar参加国として
追加されること、傍観者は追加されないこと、共有API
(`are_countries_at_war`/`is_country_at_war`/`wars_for_country`)が支持国を
正しく認識すること、save往復で参加者・代表国が保持されることまでを一気通貫で検証。

**既存テストの意図的な挙動変更(+0、更新のみ)**:
`tests/p21_013_crisis_support_e2e_test.rs`の
`supporter_is_not_auto_added_to_the_resulting_war`を
`supporter_who_pledged_before_rejection_is_added_to_the_resulting_war`へ改名し、
アサーションを反転(P21-013時点は「支持国はWarへ自動追加されない」が意図的仕様
だったが、P21-016はまさにこれを反転させることが目的のため)。

---

## 13. なぜP21-013の既存テストが1件REDになったか、その対処

`cargo test --tests`を初回実行した際、`supporter_is_not_auto_added_to_the_resulting_war`
が1件REDになった。原因はP21-016の`declare_war`変更そのもの(正しい多国間接続の
結果、以前は「追加されない」ことを検証していたテストが今は「追加される」のが
正しいため)。既存メモリ上のルール(「新タスクが旧テストの意図的仕様を反転させる
場合、古いテストへ無言でダミーの協調オブジェクトを与えて通すのではなく、
[元のテストを新仕様に更新]+[必要であれば別のより狭いテストを追加]で分割する」)
に従い、元のテストの名前・アサーションを新仕様に合わせて更新した(§12参照)。
狭い方の追加テストは、`tests/p21_016_multilateral_war_e2e_test.rs`の
「傍観者は追加されない」「反対側の支持国は自分の側にしか追加されない」検証
(同一テスト内でアサーション済み)が実質的にその役割を果たしている。

---

## 14. 旧Save後方互換性の直接検証

`ron::from_str`でP21-016以前の実際のフィールド構成(`primary_attacker`/
`primary_defender`を含まない`War`、`source_crisis_id`/`committed_attackers`/
`committed_defenders`を含まない`WarJustification`)のRON文字列を手書きし、
デシリアライズが成功すること、フォールバック値が正しいことを直接検証した
(§12の2件のRON手書きテスト)。Rust構造体リテラルの`Option::None`デフォルトを
信頼するだけでなく、実際のserdeデシリアライズ経路を通したことで、
「`#[serde(default)]`の付け忘れ」のような回帰を確実に検出できる。

---

## 15. 開始時／終了時テスト数

| | lib | 統合テスト | 合計 |
|---|---|---|---|
| 仕様書記載の想定ベースライン | 760 | 173 | 933 |
| 開始時(実測、P21-015終了時点と一致) | 760 | 173 | 933 |
| 初回パス終了時(実測) | 782 (+22) | 174 (+1) | 956 (+23) |
| 二次レビュー対応後(実測、§28) | 797 (+15) | 174 (+0) | 971 (+15) |

仕様書の想定ベースラインは実測値と完全に一致した(P21-014/015のような食い違いは
発生しなかった)。二次レビュー対応で追加した15件は全てlib単体テスト(§28参照)。

---

## 16. 品質ゲート結果

- `cargo check --all-targets` — 成功(0エラー、複数回の中間チェックも含め全て成功)。
- `cargo test --lib` — 二次レビュー対応後 797 passed / 0 failed。
- `cargo test --tests`(デフォルト並列度、ヘッドレスGPUテスト含む) — 971 passed
  (lib 797 + 統合174) / 0 failed。
- `cargo clippy --all-targets --all-features -- -D warnings` — 0 warnings
  (`grant_completed_justification`の引数増加(7引数)に対し既存慣例通り
  `#[allow(clippy::too_many_arguments)]`を付与)。
- `cargo build --release` — 成功。
- `git diff --check` — エラーなし(LF→CRLF警告のみ、既存の全ファイルで共通の
  無害な警告)。

上記は二次レビュー対応(§28)を含めた最終状態での結果。初回パス終了時点では
782 lib / 956 合計だった。

各`cargo test --tests`実行後、ヘッドレスレンダーテストが汚したチェックイン済み
スクリーンショット(P20-007/P20-009/P21-SAVE-002E)を`git checkout --`で復元した
(既知のリスク、[[project_headless_test_output_risk]]参照、本タスクとは無関係)。

---

## 17. fmt基準との差分(スコープ事故と修正)

このタスク中、`cargo fmt -- <P21-016で編集した25ファイル>`を実行したところ、
**明示的にファイル引数を渡したにもかかわらずワークスペース全体が再フォーマット
された**(既知の落とし穴、[[feedback_cargo_fmt_scope]]参照——今回はさらに
「複数ファイル引数」の形でも同じ挙動になることを新たに確認した)。これにより
P21-016と無関係な11ファイル(`map/division_render.rs`・`map/mod.rs`・
`military/{movement,recruitment,supply}.rs`・`save/runtime.rs`・`war/peace.rs`・
統合テスト4本)に、コミット済みHEADの時点から存在していた純粋な空白/改行位置の
既存driftが意図せず修正された。

`git diff HEAD`で内容を確認したところ全て空白のみの変更(ロジック変更なし)と
確認できたが、本タスクのスコープ外である以上、無言で自己修正せず一旦ユーザーへ
確認を取った([[feedback_git_self_correction]]の方針通り)。ユーザーの指示で
`git checkout --`によりこの11ファイルを元に戻し、P21-016のdiffをスコープ内に
限定した。

最終的にP21-016が実際に編集した全ファイルは`cargo fmt --all -- --check`で
0 diffを確認済み。上記11ファイル(44 hunks、HEAD時点から存在する既存drift、
本タスクと無関係)は今回意図的に触れずに残した。

---

## 18. 性能測定方法と結果(二次レビュー対応で軸2を要求規模まで拡張、§28参照)

一時プロファイラ`src/bin/profile_p21_016.rs`(測定後削除済み、Cargo.tomlの
`[[bin]]`エントリも削除済み)で2軸を測定、結果は
`verification_logs/phase-21/p21-016/perf/{results.csv,summary.txt}`に保存
(下表は二次レビュー対応後の最終測定値)。

**軸1: 支持スナップショット生成(`crisis_response::reject_demand`経由、
third_party_reactions件数でスケーリング、200回試行)**

| 件数 | mean(ms) | p50(ms) | p95(ms) |
|---|---|---|---|
| 0 | 0.0001 | 0.0001 | 0.0002 |
| 8 | 0.0003 | 0.0003 | 0.0004 |
| 100 | 0.0025 | 0.0024 | 0.0027 |
| 1000 | 0.0176 | 0.0173 | 0.0197 |

`third_party_reactions`件数に対しほぼ線形にスケールしており(0→1000件で
約110倍の増加に対し処理時間も約110倍)、`build_support_snapshot`の
`O(reactions)`実装通りの挙動を確認した。1000件という非現実的な規模でも
0.02ms未満であり、日次シミュレーションへの影響は無視できる。

**軸2: 敵味方判定共有API(War数×陣営あたり参加国数、最大1000 War×陣営あたり
100参加国、要求規模まで拡張測定。2000回試行、War数×参加国数≥50,000の
組み合わせのみプロファイラ自体の実行時間を抑えるため200回試行)**

| war_count | participants/side | is_country_at_war mean(ms) | are_countries_at_war mean(ms) |
|---|---|---|---|
| 1 | 1 | 0.00004 | 0.00004 |
| 1 | 8 | 0.00004 | 0.00004 |
| 1 | 50 | 0.00004 | 0.00004 |
| 1 | 100 | 0.00004 | 0.00004 |
| 10 | 1 | 0.00018 | 0.00021 |
| 10 | 8 | 0.00017 | 0.00019 |
| 10 | 50 | 0.00004 | 0.00020 |
| 10 | 100 | 0.00011 | 0.00019 |
| 100 | 1 | 0.00062 | 0.00172 |
| 100 | 8 | 0.00075 | 0.00172 |
| 100 | 50 | 0.00040 | 0.00171 |
| 100 | 100 | 0.00121 | 0.00173 |
| **1000** | **1** | 0.01627 | 0.01991 |
| **1000** | **8** | 0.01762 | 0.02081 |
| **1000** | **50** | 0.01148 | 0.02089 |
| **1000** | **100** | 0.00639 | **0.02327** |

両APIとも`war_count`に対してほぼ線形(`self.wars.values()`を走査する実装通り)、
`participants_per_side`には(誤差範囲内で)ほぼ無依存——`HashSet::contains`が
O(1)であるため、1陣営あたりの参加国数がいくら増えても単一クエリのコストは
増えない。要求された最大規模(1000 War×陣営あたり100参加国)でも1クエリ
0.024ms未満であり、実際のゲーム規模(6か国、同時進行War数はごく少数)を
大幅に上回る余裕がある。

---

## 19. 一時ファイルの削除確認

`src/bin/profile_p21_016.rs`を削除し、`Cargo.toml`の対応する`[[bin]]`エントリも
削除した後、`cargo check --all-targets`で再確認し、削除漏れがないことを確認済み。

---

## 20. 発見した既存不具合・仕様差

- 仕様書は「Warは単一攻撃国・単一防御国のフィールドしか持たない」という前提で
  書かれていたが、実際には`HashSet<CountryId>`により最初から多国間対応の土台が
  あった(§2参照)。
- 仕様書が想定していた「旧Save配列欠落時のマイグレーションパス」は、
  `attackers`/`defenders`自体が元々必須フィールドだったため実際には不要だった
  (§8参照)。
- P21-013の既存テスト1件が、P21-016による意図的な仕様反転の結果としてREDになった
  (バグではなく想定内の反転、§13参照)。
- **二次レビュー対応中に発見**: `war/peace.rs::execute_peace_settlement`に、
  §2/§4/§10で「事前調査済み・無変更で正しい」と報告していたにもかかわらず、
  実際には未修正の`war.attackers.iter().next()`/`war.defenders.iter().next()`
  (領土移転先・winner・休戦相手の選定に使われる)が残っていた。初回パスの
  事前調査は`peace.rs`内の別関数(`relocate_hostile_territory_divisions`、
  多国間対応済み)を見て「ファイル全体が正しい」と誤って一般化していたのが原因。
  `primary_attacker_id()`/`primary_defender_id()`へ修正済み(§28参照)。

---

## 21. Save DTOへの影響

`War`・`WarJustification`はどちらもSave DTOへ直接シリアライズされる型であり
(`SavedWarRegistry`が`War`をそのまま保持)、別のDTO変換層は存在しない。新フィールドは
全て`#[serde(default)]`付きの`Option`/`Vec`のため、DTOスキーマの構造自体に変更は
不要だった。

---

## 22. 決定論性の保証状況

- `build_support_snapshot`の出力は`CountryId`昇順ソート済み(`HashMap`走査順に
  非依存)。
- `War::sorted_attackers`/`sorted_defenders`・`opponents_of`は全て`CountryId`昇順。
- `WarRegistry::wars_for_country`は`WarId`昇順。
- `ui/diplomacy_panel.rs`の戦争一覧表示は`sorted_attackers`/`sorted_defenders`
  経由に変更し、`HashSet`走査順への依存を排除した。
- `declare_war`のアトミック性チェック・支持国除外はいずれも入力(`committed_attackers`/
  `committed_defenders`、それ自体が既にソート済み)への線形走査のみで、
  非決定的な要素は含まない。

---

## 23. 保護対象ファイル・既存回帰チェック

`combat.rs`・`occupation.rs`・`military_ai.rs`・`military/invasion.rs`は無変更。
`peace.rs`は二次レビュー対応で1関数のみ修正(§20・§28参照)。既存の全971テスト
(旧933+新38)がP21-016適用後も成功しており、P21-011〜015の既存機能(Crisis状態
機械・支持表明・AI応答・国力ランク・AI支持判断)への回帰は確認されていない。
さらに二次レビュー対応で、`combat.rs`/`occupation.rs`/`peace.rs`が実際に多国間
参加者を正しく扱うことを直接exerciseするテストを追加した(§28参照、初回パスでは
コード読解による確認のみだった)。

---

## 24. 既知の未対応事項

初回パス時点で「justification-pair-matches-crisis-parties」検証が未実装だった件は
二次レビュー対応(§28)で実装済み。二次レビュー後の時点で残っている既知の
未対応事項はない(§28の5項目はすべて対応済み)。

---

## 25. GUI手動確認の実施有無

**未実施**。本タスクは実データE2Eテスト(自動化された`App`経由)による検証のみで、
対話的なGUIセッション(実際にウィンドウを起動してのプレイ確認)は行っていない。
以下は次回GUI確認時のチェックリスト:

1. Crisisで複数国が支持を表明した状態を作り、拒否させてWar開始まで進める。
2. 外交パネルの「進行中の戦争」一覧で、両陣営の全参加国名が表示され、代表国に
   "(Leader)"/"（主導国）"の接尾辞が付いていることを目視確認。
3. 支持国の師団が実際に前線へ配置・戦闘に参加できることを確認。
4. 支持国が占領地域の帰属判定で正しく扱われることを確認。
5. 講和交渉パネルが代表国同士の講和として問題なく機能することを確認
   (支持国宛の講和は仕様上未実装)。
6. Save→Load後も上記表示・参加国が保持されることを確認。
7. JA/EN両ロケールで表示崩れがないことを確認。

---

## 26. まとめ

Crisis支持コミットメントを、拒否/期限切れ時点のスナップショットとして
`WarJustification`へ確定させ、宣戦布告時にそのスナップショットを消費して
多国間War参加国を構成する接続を実装した。既存の`HashSet`ベースの多国間対応基盤
(combat/occupation/peace/military_ai)は無変更のまま活用でき、真に必要だった変更は
「代表国の明示化」「スナップショットの捕捉と消費」「共有敵味方判定API」
「該当UIのソート済み表示」の4点に絞り込めた。全956テストが成功し、
実データE2Eで全体の接続を確認済み。

---

## 27. メモリ更新内容

`project_p21_016_multilateral_war.md`を新規作成し、`MEMORY.md`のインデックスへ
追記、`project_phase21_status.md`の末尾サマリーを更新した(二次レビュー対応後、
§28の内容も反映して再更新)。

---

## 28. 二次レビュー対応(2026-08-20)

初回パスの完了報告に対し、5件の指摘を受けた。すべて再検証・修正済み。

### 28.1 指摘1: Save整合性検証(justification↔Crisis↔Claim)の欠落

`validate_war_justifications`に、`WarJustification.source_crisis_id`が
`Some`の場合の追加検証を実装:
- **initiator不一致を拒否**: `justification.initiator != crisis.initiator`なら
  `ParticipantMismatch`。
- **target不一致を拒否**: `justification.target != crisis.target`なら
  `ParticipantMismatch`。
- **正常な組み合わせを受理**: 両者が一致する場合は問題なし。
- **Claim ID整合性(可能な範囲で)**: 由来Crisisが`related_claim_id`を持つ場合、
  そのClaimの`claimant_country`がCrisisの`initiator`と、`target_state`が
  Justificationの`target_state`と一致するかを検証。Claim自体の参照切れは
  既存の`unknown_related_claim_reference_is_rejected`(P21-011由来)が別途検証
  済みのため、ここでは実在するClaimの中身の整合性のみを見る。

新規テスト(+8、`src/save/validate.rs`):
`justification_initiator_mismatch_with_source_crisis_is_rejected`、
`justification_target_mismatch_with_source_crisis_is_rejected`、
`justification_pair_matching_source_crisis_is_accepted`、
`source_crisis_claim_claimant_mismatch_is_rejected`、
`source_crisis_claim_target_state_mismatch_is_rejected`、
`source_crisis_with_consistent_claim_is_accepted`、
`unknown_war_source_crisis_id_is_rejected`、`valid_war_source_crisis_id_is_accepted`
(最後の2件は28.2の`War.source_crisis_id`自体の参照整合性検証に対応)。

### 28.2 指摘2: War開始後にCrisis来歴が消える

`War`に`source_crisis_id: Option<DiplomaticCrisisId>`を追加(`#[serde(default)]`、
旧Save/通常の二国間Warでは`None`)。`declare_war`が正当化を消費する直前に
`justification.source_crisis_id`を読み取り、そのままWarへ引き継ぐ
(`committed_attackers`/`committed_defenders`と同じタイミング・同じ借用スコープで
読む)。これにより、宣戦後もWarから元Crisisを`war.source_crisis_id`経由で
逆引きできる。Save検証にも`War.source_crisis_id`の参照整合性チェックを追加
(§28.1参照)。全てのWarリテラル構築箇所(既存24+箇所)へ`source_crisis_id: None`
を追加(通常の二国間Warは`None`のまま)。

### 28.3 指摘3: 実際の戦闘参加の直接検証が不足

`process_combat`(`war/combat.rs`)・`process_occupation`(`war/occupation.rs`)は
初回パス時点でテストが一切存在しなかった(コード読解のみで「正しい」と
判断していた)。二次レビュー対応で5件の直接テストを追加(`src/war/tests.rs`):

- `attacker_side_supporter_division_can_attack_defender_via_process_combat` —
  攻撃側支持国の師団が防御側代表国の師団と同じ州にいる場合、`process_combat`
  経由で実際に被害が生じることを確認。
- `same_side_divisions_never_engage_in_combat` — 攻撃側代表国と攻撃側支持国、
  および防御側代表国と防御側支持国(いずれも同陣営ペア)は`process_combat`で
  交戦しないことを確認。
- `supporter_occupation_succeeds_via_process_occupation` — 攻撃側支持国の師団が
  防御側代表国所有の州を`process_occupation`経由で実際に占領完了
  (`controller_country`が支持国自身になる)まで進むことを確認。
- `peace_settlement_clears_hostility_for_all_participants_including_supporters` —
  講和成立後、代表国・支持国を問わず全参加国間の`are_countries_at_war`/
  `is_country_at_war`が`false`になることを確認。
- `hostility_persists_only_for_a_still_active_second_war` — 支持国が別のWar
  (War B)にも参加している場合、War Aだけが講和で終結しても、War Bが
  Activeである限り`is_country_at_war`/`are_countries_at_war`が正しく維持
  されることを確認。

**このテスト追加の過程で、`war/peace.rs::execute_peace_settlement`に未修正の
`war.attackers.iter().next()`/`war.defenders.iter().next()`が残っていることを
発見した**(初回パスの事前調査が同ファイル内の別関数を見て「ファイル全体が
正しい」と誤って一般化していたのが原因、§20参照)。領土移転先・winner・休戦相手
(`diplomacy_registry.set_truce`)の選定に使われていたため、`primary_attacker_id()`/
`primary_defender_id()`へ修正した。この修正により、`war_error.peace.no_attacker`/
`no_defender`という2つのエラーキーが到達不能になった(空集合は`declare_war`が
構造的に作れないため元々`Err`経路は事実上デッドコードだったが、アクセサが
`panic`する設計(§28.4)に変わったことで、`Result`の`?`によるエラー伝播自体が
不要になった)。コード・テストでの参照は元々存在しなかったため、この変更に
伴う破壊的変更はない(未使用となったローカライズキー2件は害がないため残置)。

### 28.4 指摘4: 主導国フォールバックの安全性

**既存Save validationの空陣営拒否(そのまま維持、新規追加なし)**:
`src/save/validate.rs`の`validate_wars`関数、`empty_attackers_is_rejected`/
`empty_defenders_is_rejected`テスト(いずれもP21-016以前、P20-16時代から存在)。
`war.attackers.is_empty()`/`war.defenders.is_empty()`なら`EmptyCollection`で
拒否する既存チェックがそのまま該当要件を満たしていたため、変更不要と確認した。

**`declare_war`が空陣営を作れないことの直接テスト(新規追加)**:
`declare_war_never_produces_empty_attacker_or_defender_sets` — 支持国が全員
(存在しない国家IDのみで)個別除外される最悪ケースでも、`attackers`/`defenders`
は要求国/対象国自身を常に無条件で含むため空にならないことを確認。

**アクセサの再設計(架空のCountryId(0)を返さない)**:
`primary_attacker_id()`/`primary_defender_id()`の`.unwrap_or(CountryId(0))`
フォールバックを`.expect("...")`(panic)へ変更。上記2つの安全策
(Save validationの空集合拒否 + `declare_war`の構造的な空集合不能性)により、
この関数が呼ばれる時点で`attackers`/`defenders`が空であることはあってはならない
不変条件違反であり、実在しない国を実在するかのように装う値を返すより、
ここで明示的にpanicする方が安全と判断した。新規`#[should_panic]`テスト
`primary_attacker_id_panics_instead_of_fabricating_a_country_on_a_truly_empty_set`
で、`declare_war`を経由しない直接構築(唯一の再現経路)によるpanicを確認。

### 28.5 指摘5: 性能測定条件が未達

軸2(敵味方判定共有API)を要求規模(最大1000 War・陣営あたり最大100参加国)まで
拡張して再測定した。結果は§18に統合済み。最大規模(1000 War×100参加国)でも
1クエリあたり0.024ms未満で、両APIとも`war_count`に線形・`participants_per_side`に
ほぼ無依存という初回パスの結論が、より大きなスケールでも変わらず成立することを
確認した。

### 28.6 二次レビュー対応の最終品質ゲート

`cargo check --all-targets` / `cargo test --lib`(797 passed) / `cargo test --tests`
(971 passed) / `cargo clippy --all-targets --all-features -- -D warnings`(0
warnings) / `cargo build --release` / `git diff --check` すべて成功。
`cargo fmt --all -- --check`は、P21-016が触れた全ファイルで0 diffを再確認
(このタスク以前から存在する無関係な44 hunks/11ファイルの既存driftのみ残存、
§17と同一、意図的に未着手)。ヘッドレステストが汚したスクリーンショットは
毎回`git checkout --`で復元済み。

### 28.7 二次レビュー対応の変更・新規ファイル一覧

**追加変更**:
- `src/war/data.rs` — `source_crisis_id`フィールド追加、`declare_war`が
  引き継ぐよう変更、`primary_attacker_id`/`primary_defender_id`をpanic方式へ
  再設計。
- `src/save/validate.rs` — justification↔crisis↔claim整合性検証、
  `War.source_crisis_id`参照整合性検証を追加。
- `src/war/peace.rs` — `execute_peace_settlement`の`.iter().next()`を
  `primary_attacker_id()`/`primary_defender_id()`へ修正(バグ修正)。
- `src/war/tests.rs` — 多国間戦闘/占領/講和の直接テスト5件、空集合系テスト2件を
  追加。
- 既存24+箇所の`War`リテラル構築(`src/war/{tests,military_ai,frontline}.rs`・
  `src/military/{tests,update,invasion}.rs`・`src/save/{validate,apply,export}.rs`・
  `src/ui/military_panel.rs`・`src/map/{frontline_render,frontline_selection,
  offensive_line_selection}.rs`・`src/profiling.rs`・
  `tests/{p21_005_army_frontline_e2e_test,p21_008_army_offensive_e2e_test}.rs`)
  へ`source_crisis_id: None`を追加(ロジック変更なし)。

**新規テスト内訳(+15、全てlib単体テスト)**: §28.1(+8)、§28.3(+5)、§28.4(+2)。
