# P21-SAVE-002F1 完了報告: 保存データと現在Worldの国・州ID集合の互換性検証

日付: 2026-08-14

## 0. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

集合不一致を適用前(Prepare段階)に拒否し、Worldが一切変更されないことを、
自動テスト18件(既存2件と合わせて計20件のシナリオ)・全検証コマンドで実証した。
実ウィンドウでの人間による目視確認は今回も未実施(監査エージェントに画面操作手段が
ないため、偽って実施済みと報告しない)。

---

## 1. 重要な前提訂正: このタスクの出発点だったP21-SAVE-002F自身の発見事項が誤りだった

本タスクはP21-SAVE-002Fの完了報告書「Finding #1」(現在Worldとの互換性検証を一切
行っていない)を根拠に発注されたが、`src/save/apply.rs`の現在のソースコードを本ラウンド
冒頭で改めて読み直した結果、**このID集合比較そのものは既にP21-SAVE-002D時点で実装
済みだった**ことが判明した。具体的には`prepare_load`関数内(旧184〜214行目付近)に、

```rust
let current_state_ids: HashSet<StateId> = ...;
let save_state_ids: HashSet<StateId> = ...;
if current_state_ids != save_state_ids { return Err(ApplyLoadError::RuntimeCompatibility { .. }); }
// Countryについても同様
```

という、`HashSet`同士の完全一致比較(単なる件数比較ではない)が既に存在し、既存の
`map_or_render_incompatibility_is_rejected_before_applying`(apply.rs)・
`apply_failure_preserves_current_state`(runtime.rs)という、この経路を実際に検証する
既存テストも既に存在していた。P21-SAVE-002Fの監査時、`check_static_master_compatibility`
(建物/技術/師団定義/世界段階という**別の**静的マスターデータを検証する関数)だけを
確認し、`prepare_load`自身が持つこの独立したCountry/State集合チェックを見落としたことが
原因であり、これは監査エージェント自身の誤りである。ユーザーへ本ラウンド冒頭で
この訂正を伝え、作業を継続した。

**このタスクの実際の内容**は、したがって「ゼロから実装する」ではなく、**既存の
比較ロジックが持っていた診断情報の不足(欠落/余剰の実際のID一覧が無く、件数だけしか
分からなかった点)を補強し、要求された20項目のテストシナリオを網羅的にカバーする**
という、当初想定より小さいが実質的な作業だった。既存の比較ロジック自体(`HashSet`の
`!=`による完全一致判定)は変更していない。

---

## 2. 原因と影響

- **原因**: `ApplyLoadError::RuntimeCompatibility`は従来`{ detail: String }`という
  人間可読な要約文字列だけを保持しており、実際にどのIDが欠落/余剰なのかを
  プログラム的に取り出す手段がなかった(ログ・通知には十分だが、テストでの精密な
  検証や将来の診断UIには不十分)。
- **影響**: 機能面(「集合が不一致なら安全に拒否する」)は既にP21-SAVE-002Dから正しく
  動作していたため、実害のあるバグではなかった。影響があったのは診断性のみ
  (「何行のStateが足りないのか」を`detail`文字列から人間が読み取る以外に確認する
  手段がなかった)。

## 3. なぜマイグレーションではなく「安全な拒否処理」なのか

このタスクは、異なるマップ構造のセーブを新しいマップ構造へ変換する機能(マイグレーション)
を一切実装しない。`commit_load`は依然として`CountryRegistry`/`StateRegistry`全体を
無条件に置き換えるだけであり、セーブ側のデータを現在Worldの構造へ合わせて調整する
処理は存在しない。本ラウンドが追加したのは、**その無条件の置き換えを実行する前に、
安全に判断できないケース(ID集合が一致しない)を確実に検出して拒否する**検証だけである。
既存描画Entity(州の色分け等)は起動時の静的データを前提に生成されるため、
ID集合が異なるセーブを黙って適用すると、描画と実データの不整合を招く恐れがある
(これが元々の`RuntimeCompatibility`検査がP21-SAVE-002Dで導入された理由そのものである)。

---

## 4. 比較する4集合

| # | 集合 | 由来 |
|---|---|---|
| 1 | 保存データ内の全CountryId | `save.countries.iter().map(\|c\| c.id)` |
| 2 | 現在のCountryRegistry内の全CountryId | `country_registry_current.countries.iter().map(\|c\| c.id)` |
| 3 | 保存データ内の全StateId | `save.states.iter().map(\|s\| s.id)` |
| 4 | 現在のStateRegistry内の全StateId | `state_registry_current.states.iter().map(\|s\| s.id)` |

比較は`HashSet<T>`同士の`!=`による完全一致判定であり、単なる`.len()`比較ではない
(同数でもIDが異なれば不一致として検出される。§6のテスト5/8で実証)。

---

## 5. 実装内容

変更範囲(要求どおり最小限に限定):

- **Productionロジック**: `src/save/apply.rs`(`RuntimeCompatibilityIssue`列挙型の新設、
  `ApplyLoadError::RuntimeCompatibility`への`issue`フィールド追加、`diff_id_sets`
  ヘルパー関数の新設、`prepare_load`内の既存2箇所の更新)。
- **テスト追加**: `src/save/apply.rs`(Apply単体テスト12件)・`src/save/runtime.rs`
  (Runtime/E2Eテスト6件)。
- **re-export・doc**: `src/save/mod.rs`(`RuntimeCompatibilityIssue`の`pub use`追加、
  モジュール先頭のドキュメントコメント更新)。
- **新規報告書**: `verification_logs/phase-21/p21-save-002f1/`
  (`p21-save-002f1_completion_report.md`、本ファイル)。

具体的な変更内容:

1. **`RuntimeCompatibilityIssue`列挙型を新設**:
   ```rust
   pub enum RuntimeCompatibilityIssue {
       CountrySetMismatch { missing_from_save: Vec<CountryId>, extra_in_save: Vec<CountryId> },
       StateSetMismatch { missing_from_save: Vec<StateId>, extra_in_save: Vec<StateId> },
   }
   ```
   Country/Stateどちらの不一致かを型レベルで識別でき、それぞれについて
   「現在Worldにはあるがセーブに無いID」(`missing_from_save`)と「セーブにだけあるID」
   (`extra_in_save`)を分離して保持する。
2. **`ApplyLoadError::RuntimeCompatibility`へ`issue: RuntimeCompatibilityIssue`フィールドを追加**
   (既存の`detail: String`はそのまま維持、通知には引き続き使わない設計を維持)。
   既存の全呼び出し元は`{ .. }`パターンでマッチしていたため、フィールド追加による
   破壊的変更は発生しない(`cargo check`で確認済み)。
3. **`diff_id_sets`ヘルパー関数を新設**: 2つの`HashSet<T>`を比較し、
   `(missing_from_save, extra_in_save)`を返す。両Vecとも`sort_by_key`で明示的に
   ソート済みで返すため、`HashSet`の反復順序(プロセスごとに変わり得る)に一切
   依存しない、決定的な出力順序を保証する。
4. `prepare_load`内の既存2箇所のCountry/State集合比較を、この新しい診断情報を
   populate するよう更新(比較ロジック自体・比較タイミング・呼び出し順序は変更していない)。
5. `src/save/mod.rs`: `RuntimeCompatibilityIssue`を`pub use`へ追加、ドキュメント
   コメントを更新(スキーマ・re-export以外の変更なし)。

`export.rs`/`write.rs`/`read.rs`/`dto.rs`/`validate.rs`は一切変更していない。
`SaveGameV1`のフィールド・`version`・RON形式は不変。UI・ローカライゼーション・
カメラ・`main.rs`のPlugin登録も一切変更していない。

---

## 6. Prepare→Commit原子性が維持されること

この検査は`prepare_load`(`&World`共有参照のみを受け取る関数)の内部で行われ、
不一致が見つかった時点で即座に`Err`を返す。`commit_load`(`&mut World`排他参照)は
`prepare_load`が`Ok`を返した場合にしか呼ばれないため、この検査の追加によって
Prepare→Commitの二段階原子性(Prepare失敗時はWorld無変更)の設計は一切崩れていない。
新しいCommit側の失敗経路も追加していない(要求どおり)。§8のテスト10・11が
「不一致時に正規Resource・一時Resourceのいずれも1つも変化しない」ことを直接実証する。

---

## 7. 追加テスト一覧(18件、既存2件と合わせて計20シナリオをカバー)

### Apply単体(`src/save/apply.rs`、12件)

| # | テスト名 | 内容 |
|---|---|---|
| 1 | `matching_country_and_state_id_sets_succeed` | 集合が完全一致すれば成功 |
| 2 | `vector_order_alone_does_not_affect_compatibility` | Vec順序だけが違っても成功 |
| 3 | `country_missing_from_save_is_rejected` | Countryがセーブ側で1件不足 |
| 4 | `country_extra_in_save_is_rejected` | Countryがセーブ側に1件余分 |
| 5 | `country_same_count_different_ids_is_rejected` | Country同数・別ID |
| 6 | `state_missing_from_save_is_rejected` | Stateがセーブ側で1件不足 |
| 7 | `state_extra_in_save_is_rejected` | Stateがセーブ側に1件余分 |
| 8 | `state_same_count_different_ids_is_rejected` | State同数・別ID |
| 9 | `country_and_state_mismatches_are_each_independently_diagnosable` | 両方不一致時はState側が先に報告され、Stateだけ一致させるとCountry側が正しく報告される(両経路とも到達可能) |
| 10 | `runtime_compatibility_failure_leaves_all_normative_resources_unchanged` | 全17正規Resourceが不変 |
| 11 | `runtime_compatibility_failure_leaves_all_transient_resources_unchanged` | 全13一時/UI Resourceが不変 |
| 12 | `runtime_compatibility_error_ids_are_deterministically_sorted` | 複数ID(5,2,9の順で挿入)が常に昇順[2,5,9]で返る |

### Runtime/E2E(`src/save/runtime.rs`、6件)

| # | テスト名 | 内容 |
|---|---|---|
| 13 | `different_country_id_set_causes_apply_failure` | 異なるCountry集合のRONロード→Apply失敗 |
| 14 | `different_state_id_set_causes_apply_failure` | 異なるState集合のRONロード→Apply失敗(`issue`の中身まで確認) |
| 15 | `runtime_compatibility_failure_emits_exactly_one_notification` | 失敗通知は1件だけ |
| 16 | `runtime_compatibility_failure_does_not_request_camera_reset` | `CameraResetRequestMessage`が発行されない |
| 17 | `runtime_compatibility_failure_does_not_change_game_paused` | `GamePaused`が変化しない |
| 18 | `runtime_compatibility_failure_does_not_reexecute_on_next_frame` | 次フレームで`LoadExecutionCount`が増えない |

### 既存カバレッジで代替(新規テスト不要と判断、19/20)

- **19(実7か国28州データでの通常Save→Load成功)**・**20(ロード後の再セーブ成功)**は、
  `tests/p21_save_002e_end_to_end_test.rs::save_change_load_restores_state_a_new_ids_do_not_collide_and_resave_works`
  が既に実データ(`DataLoaderPlugin`経由の本物の7か国28州マップ)でこの両方を検証
  している。本ラウンドの変更後にこのテストを再実行し、回帰がないことを確認した
  (§8参照)。「変更は必要なテストに限定する」との指示に沿い、既にカバーされている
  シナリオを重複して新規実装することはしなかった。

**新規テスト合計: 18件**。既存の`map_or_render_incompatibility_is_rejected_before_applying`
(apply.rs)・`apply_failure_preserves_current_state`(runtime.rs)は変更していない
(両方とも現在も pass)。

---

## 8. 全検証結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | **PASS** |
| `cargo test --lib` | **407 passed / 0 failed**(002F終了時点389 + 新規18) |
| `save::`モジュールテスト(rustfmt後の再確認) | **214 passed / 0 failed**(`cargo test --lib`407件の内数) |
| 既存安全統合テスト(8バイナリ、headless-render PNG2本を除く) | **59 passed / 0 failed**(6+5+14+4+4+8+9+9) |
| `cargo test --test p21_save_002e_end_to_end_test` | **3 passed / 0 failed**(§7の19/20を含む回帰確認、1バイナリ) |
| `p21_save_002e_headless_render_test` | `--no-run`でコンパイルのみ確認(§9参照、本ラウンドでは未実行) |
| `cargo clippy --all-targets --all-features -- -D warnings` | **PASS**(warning 0) |
| `cargo build --release --all-targets` | **PASS**(6分41秒) |
| `cargo fmt --check`(読み取りのみ) | 開始時点82 diff hunks(既知の74 + 本ラウンドの未整形8)。
`rustfmt --edition 2024 src/save/apply.rs src/save/runtime.rs`のみを個別に実行
(`main.rs`/`lib.rs`/`mod.rs`は一度も対象にしていない)、事後**74 diff hunks**
(既知ベースラインへ復帰、`src/save/`配下は0)。`git status`で他ファイルが
一切変化していないことを確認 |
| `git diff --check` | **PASS**(LF/CRLF警告のみ) |
| 一時ファイル/プロセス残留確認 | `saves/`ディレクトリなし、一時テストディレクトリなし |

**テスト集計**(002F終了時点452件から本ラウンドで+18):

| 区分 | 内訳 | 件数 |
|---|---|---|
| `cargo test --lib` | — | **407件PASS** |
| 既存安全統合テスト | 8バイナリ | **59件PASS** |
| `p21_save_002e_end_to_end_test` | 1バイナリ | **3件PASS** |
| **本ラウンドで実行した合計** | — | **469件PASS** |
| `p21_save_002e_headless_render_test` | 1件、コンパイル確認のみ(§9参照、本ラウンドでは未実行) | (実行対象外) |
| **現行の管理上のテスト総数** | 469件(本ラウンド実行分)+1件(未実行だが証跡は002Eのまま有効) | **470件** |

---

## 9. Headless証拠保護

`tests/p21_save_002e_headless_render_test.rs`は`verification_logs/phase-21/p21-save-002e/screenshots/`
への固定パス書込を行うため、指示どおり本ラウンドでは`cargo test --no-run`による
コンパイル確認のみを行い、実行(証跡再生成)は行わなかった。既存の5枚のPNGは
一切変更していない。この証跡出力設計自体の改修は本タスクのスコープ外である。

---

## 10. 人間によるGUI確認が引き続き未実施であること

本ラウンドもコード変更のみであり、実ウィンドウでの`cargo run`操作による確認は
実施していない(監査エージェントに画面操作手段がないため)。P21-SAVE-002Fの報告書に
記載された10項目の人間確認チェックリストは、今回のID集合検証に関する追加確認事項
(下記)と合わせて、引き続きユーザー自身による実施が必要である。

**追加確認事項(推奨、必須ではない)**: 公開関数シグネチャとDTO/RONスキーマは不変。
ただし公開エラー型`ApplyLoadError::RuntimeCompatibility`の構造を拡張し、
`RuntimeCompatibilityIssue`をre-exportした(§5参照、既存の`{ .. }`パターンマッチとは
互換)。UI導線・通知経路は一切変更していない。以上より、P21-SAVE-002Fの既存10項目
チェックリストを再実施すれば十分であり、追加の目視確認項目はない。

---

## 11. git status(本ラウンド開始時点との比較)

開始時点と終了時点で、`src/save/`配下の3ファイル(§5と対応: `apply.rs`=
Productionロジック+テスト追加、`runtime.rs`=テスト追加のみ、`mod.rs`=re-export・doc)と
`verification_logs/phase-21/p21-save-002f1/`(本報告書、新規)以外、一切のファイルが
変化していない。`src/save/`配下の`dto.rs`/`export.rs`/`read.rs`/`validate.rs`/
`write.rs`は変更していない。P21-SAVE-002A〜002Fの既存報告書は一切上書きしていない。
`main.rs`/`lib.rs`/`mod.rs`をrustfmtへ直接渡す操作は一度も行っていない
(`src/save/mod.rs`は編集したが、フォーマット済みだったため`rustfmt`実行対象には
含めなかった。§8参照)。

```
 M strategy_game/src/save/apply.rs (差分は git status には出ない: src/save/ は未追跡)
 M strategy_game/src/save/runtime.rs (同上)
 M strategy_game/src/save/mod.rs (同上)
?? strategy_game/verification_logs/phase-21/p21-save-002f1/ (新規)
```
(`src/save/`ディレクトリ全体がリポジトリ未追跡のため、`git status`上は個々のファイル
差分ではなく`?? strategy_game/src/save/`という1行のディレクトリ表示のまま。本ラウンドの
開始時点・終了時点いずれでもこの表示に変化はない。)

---

## 12. 「起動直後ロード」への移行可否

**READY**。本ラウンドで発見・訂正したP21-SAVE-002Fの誤り(§1)を除き、コード欠陥は
発見されなかった。ID集合の互換性検証は既に堅牢に機能しており(002Dから継続、本ラウンドで
診断性のみ強化)、20項目のテストシナリオ全てで適用前の安全な拒否とWorld無変更が
実証されている。「起動直後ロード」タスクを開始する際は、以下を設計時に考慮することを
推奨する:

- 本ラウンドで強化した`RuntimeCompatibilityIssue`の`missing_from_save`/`extra_in_save`を、
  起動直後ロードの失敗時ユーザー向けメッセージ(「このセーブは現在のマップと互換性が
  ありません」等)の材料として再利用できる。
- 本ラウンドは意図的にマイグレーション機能を実装していない。異なるマップ構造間の
  変換が必要になった場合は、別途独立したタスクとして扱うこと。

002F1受入後の順序:
1. 人間によるP21-SAVE-002Fの既存10項目`cargo run`確認(未実施のまま持ち越し)。
2. 別タスクとして「起動直後ロード」。
3. その受入後にP21-005へ戻る。

複数スロット・オートセーブ・クイックセーブ・マイグレーションは、本ラウンド・
「起動直後ロード」タスクいずれにも含まれない。
