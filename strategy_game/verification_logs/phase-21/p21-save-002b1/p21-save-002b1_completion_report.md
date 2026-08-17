# P21-SAVE-002B1: 置換失敗時の旧セーブ保護テスト 完了報告

**実施日**: 2026-08-13
**性質**: `src/save/write.rs`の置換(rename)処理をテスト可能な最小構造へ整理し、rename失敗を
決定的に注入するテストを追加した。P21-SAVE-002Bで既に実装済みの本番8ステップ保存手順・
Resource→DTO変換・PostUpdate接続・DTO定義・UI・ロード・`main.rs`・GameState・P21-005は
一切変更していない。**既存のP21-SAVE-002B報告書(`verification_logs/phase-21/p21-save-002b/`)
は上書きせず、本ファイルを新規作成した。**

---

## 1. 最終判定

**COMPLETE**

これをもってP21-SAVE-002Bを正式に`COMPLETE`とする(旧セーブ保護の必須条件が、
既に実装済みのCreateTempFile失敗ケースに加え、置換(rename)失敗ケースでも直接実証された)。

---

## 2. 置換処理のテスト可能化方法

`src/save/write.rs`に、置換処理(手順7)だけを外部から差し替え可能にした非公開関数
`write_save_file_with_replace`を追加した:

```rust
fn write_save_file_with_replace<F>(
    save: &SaveGameV1,
    config: &SavePathConfig,
    replace: F,
) -> SaveOutcome
where
    F: FnOnce(&Path, &Path) -> std::io::Result<()>,
{ /* 本番と全く同じ8ステップ。手順7だけがreplace(&temp_path, &final_path)を呼ぶ */ }
```

公開関数`write_save_file`は、これへ`std::fs::rename`をラップした薄いクロージャを渡すだけの
ラッパーになった:

```rust
pub fn write_save_file(save: &SaveGameV1, config: &SavePathConfig) -> SaveOutcome {
    write_save_file_with_replace(save, config, |from, to| fs::rename(from, to))
}
```

(`fs::rename`を関数アイテムとして直接渡すと、`P: AsRef<Path>`の総称性に起因する
高階トレイト境界[HRTB]エラー`implementation of FnOnce is not general enough`が発生したため、
具体的な`&Path`シグネチャを持つクロージャで明示的にラップする必要があった。これは型システム上の
制約であり、動作の変更点ではない。)

**要件充足の確認**:
- 公開APIを増やしていない: `write_save_file_with_replace`は`pub`でも`pub(crate)`でもない
  プレーンな`fn`であり、モジュール内(テストの`mod tests`は`use super::*`で子スコープとして
  アクセスできる)からしか呼べない。`write_save_file`の公開シグネチャは一切変更していない。
- 新規依存を追加していない(`std::path::Path`は既存の`std`のみ)。
- `unsafe`は使っていない。
- 本番の8ステップ保存手順(親ディレクトリ作成→シリアライズ→一時ファイルオープン→
  write_all→flush→sync_all→置換→完了)は変更していない。手順7の実装が`fs::rename`の
  直接呼び出しから`replace(...)`呼び出しへ変わっただけで、本番経路(`write_save_file`)が
  実行する処理内容は完全に同一(`replace`に`fs::rename`そのものを渡しているため)。
- 最終ファイルを先に削除する経路(`remove_file(final) → rename`)は導入していない。
- テスト都合の分岐を`SaveGameV1`へ持ち込んでいない(`SaveGameV1`自体は一切変更していない。
  分岐は`write.rs`内の関数境界だけで完結している)。

---

## 3. Rename失敗の注入方法

`write::tests::rename_failure_preserves_existing_final_file_and_does_not_panic`が、
`write_save_file_with_replace`を直接呼び出し、`replace`引数へ次のクロージャを渡す:

```rust
|_, _| Err(std::io::Error::other("injected rename failure for testing"))
```

このクロージャは引数を無視して常に`Err`を返すため、決定的(実行環境やタイミングに依存しない)
に手順7だけを失敗させる。手順1〜6(親ディレクトリ作成・シリアライズ・一時ファイルオープン・
write_all・flush・sync_all)は本番と全く同じコードパスを通って**実際に成功する**ため、
「一時ファイルの書き込み・flush・syncまでは成功させ、置換関数だけが意図的に失敗する」という
要求を正確に満たす(既存の`write_failure_preserves_existing_final_file_and_does_not_panic`が
使うCreateTempFile失敗注入とは異なる失敗地点)。

---

## 4. 失敗後も維持された旧セーブ

テストの手順(指示された順序を厳守):
1. 内容A(`game_speed: 11`)を通常の`write_save_file`で保存 → `SaveOutcome::Success`
2. 最終ファイルを読み戻し、`ron::from_str::<SaveGameV1>`でDeserializeした`game_speed`が
   `11`であることを確認(RONのバイト列比較ではなく、Deserialize後の代表フィールドによる
   意味的な比較)
3〜5. 内容B(`game_speed: 22`)の保存を`write_save_file_with_replace`+上記の失敗注入
   クロージャで開始。手順1〜6は成功し、手順7(置換)だけが`Err`を返す
6〜7. 戻り値が`SaveOutcome::Failure { error: SaveError::Rename(_), .. }`であることを確認
8. 最終ファイルを再度読み戻し、Deserialize後の`game_speed`が依然として`11`(内容Aのまま)
   であり、`22`(内容B)へ変化していないことを確認
9. `config.temp_path().exists()`が`false`であることを確認(一時ファイルの後始末)
10. テスト自体がここまで到達して完了していること自体が、`panic!`しなかったことの証拠
11. 続けて通常の`write_save_file`で内容C(`game_speed: 33`)を保存し、`SaveOutcome::Success`
    が返ること、最終ファイルの内容が`33`へ正しく更新されること、`.tmp`が残らないことを確認
    (rename失敗後も通常の保存経路が壊れていないことの確認、回帰なし)

全項目パス。既存の`second_save_safely_updates_the_same_slot_on_windows`
(Windows上でのrename成功による正常上書き)テストは変更せずそのまま維持した。

---

## 5. 一時ファイルの後始末

`write_save_file_with_replace`内、手順7の失敗分岐は既存のCreateTempFile/Write/FlushOrSync
失敗時と全く同じパターン(`let _ = fs::remove_file(&temp_path);`、結果は無視し
`panic!`しない)を使っている。今回追加したテストが、rename失敗後に実際に`.tmp`が
存在しないことを直接確認した(§4の手順9)。

---

## 6. 通常保存への回帰がないこと

- 既存8件の`write.rs`テスト(`creates_missing_temp_directory_automatically`、
  `saved_file_round_trips_through_ron_from_str`、`saved_ron_contains_version_field`、
  `no_tmp_file_remains_after_successful_save`、
  `second_save_safely_updates_the_same_slot_on_windows`、
  `write_failure_preserves_existing_final_file_and_does_not_panic`、
  `save_outcome_records_success_and_failure_structurally`、
  `tests_use_only_os_temp_dir_never_the_repository_saves_directory`)は一切変更せず、
  全て引き続きパスすることを確認した。
- 今回追加したテスト自体の§4手順11(rename失敗の直後に通常保存を行い、内容Cへ正しく
  更新できること)が、rename失敗からの回復が正常に機能することを直接実証している。
- `save::export::tests`(20件)・`save::runtime::tests`(6件)・`save::dto::tests`(16件)は
  一切変更しておらず、全て引き続きパスする。

---

## 7. テスト数の訂正と最新総数

P21-SAVE-002B報告書の記載を、本追補で以下の通り訂正する(既存報告書本文は上書きしない):

- **P21-SAVE-002Bで新規追加されたテストは34件**(export.rs 20件、write.rs 8件、
  runtime.rs 6件。P21-SAVE-002B報告書§16の内訳表記に軽微な数え違いがあったが、
  §17の合計値[195→229、+34]自体は当初から正しかった)
- **既存の旧セーブ保護を直接確認した失敗はCreateTempFile失敗**
  (`write_failure_preserves_existing_final_file_and_does_not_panic`、P21-SAVE-002Bで実装済み)
- **今回新たにRename失敗での保護を直接確認した**
  (`rename_failure_preserves_existing_final_file_and_does_not_panic`、本ラウンドで追加)
- DTO既存16件を含む`save`関連テスト総数:
  - P21-SAVE-002B完了時点: dto 16 + export 20 + write 8 + runtime 6 = **50件**
  - 本ラウンド完了後: dto 16 + export 20 + write **9**(+1) + runtime 6 = **51件**

---

## 8. main.rsへの登録(今回も未登録)

指示通り、`SaveGamePlugin`を`main.rs`へは登録していない(`git diff`で`main.rs`は
無変更のまま)。判断の記録:

- `SaveGamePlugin`内部の`PostUpdate`登録(`handle_save_requests.run_if(in_state(GameState::Playing))`)
  はP21-SAVE-002Bで実装完了済みであり、本ラウンドでも変更していない。
- 本番`App`(`main.rs`)へのプラグイン接続は、UI・`SaveRequestMessage`の実際の発行元を
  追加するタスク(P21-SAVE-002Eを想定)で行う。
- 現時点の`cargo run`からは、セーブ要求を発行する手段(ボタン等)が一切存在しないため、
  セーブ機能は実質的に未接続のままである。
- タスクC(ロード前のバージョン・参照整合性検証)・タスクD(DTOからResourceへの適用)は、
  いずれも「既に書き出されたセーブファイルを読む」ことから始まる設計であり、
  `SaveGamePlugin`が`main.rs`へ登録されているかどうかに依存しない
  (テストは`SaveGamePlugin`を明示的に追加した独立App、またはファイルI/O層単体で完結する)。

---

## 9. 変更ファイル一覧

正直に、変更した全ファイルを列挙する:

| ファイル | 種別 | 内容 |
|---|---|---|
| `src/save/write.rs` | 変更 | `write_save_file_with_replace`(非公開、置換処理を注入可能にする関数)を抽出、`write_save_file`をその薄いラッパーへ変更、rename失敗を実証する新規テスト1件を追加 |
| `verification_logs/phase-21/p21-save-002b1/p21-save-002b1_completion_report.md` | 新規 | 本報告書 |

上記以外のファイル(`export.rs`/`runtime.rs`/`mod.rs`/`dto.rs`を含む`src/save/`の他ファイル、
ゲームコード・アセット・既存テスト・`main.rs`)は一切変更していない。既存の未コミット差分
(P21-004/P21-004A/P21-SAVE-001/002A/002A1/002B由来)はそのまま、本タスクでは一切手を
加えていない(`git status`で確認済み)。

---

## 10. 全検証結果

| コマンド | 結果 |
|---|---|
| 新規置換失敗テスト単体 | 成功、1 passed; 0 failed |
| 全save関連テスト(`cargo test --lib save::`) | 成功、51 passed; 0 failed(50→51、+1) |
| `cargo test --lib -- --list` | 成功、230件検出(229→230、+1) |
| 既存288件を含む安全なテスト全体(lib 230 + 統合8バイナリ59) | 成功、289 passed; 0 failed(headless描画2バイナリは既存の運用慣習通り今回も未実行) |
| `cargo check --all-targets` | 成功(warning 0件) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 成功、warning 0件 |
| `cargo build --release --all-targets` | 成功 |
| `cargo fmt --check` | `write.rs`の関数シグネチャ変更に伴う新規diff1箇所が発生 → 手動整形で解消 → 既存ベースライン81件へ復帰(`cargo fmt`コマンド自体は実行せず、既存81件を一括修正していない) |
| `git diff --check` | 終了コード0、空白関連エラーなし(LF/CRLF警告のみ、既存dirtyファイル由来) |
| テスト後の一時ファイル残留確認 | リポジトリ相対`saves/`ディレクトリなし(`find`で該当なし)、OS一時ディレクトリの残骸もなし(`TempTestDir`のDropガードで全て削除済み) |

---

## 11. P21-SAVE-002Cへの移行可否

**READY**

これをもってP21-SAVE-002Bを正式に`COMPLETE`とする。「書き込みまたは置換失敗時に既存
ファイルを維持する」という必須条件は、CreateTempFile失敗(既存)とRename失敗(本ラウンド新規)
の両方で直接実証され、いずれのケースでもパニックせず、後始末も正しく行われることを確認した。
次のP21-SAVE-002C「ロード前のバージョン・参照整合性検証」(現在のゲーム状態には一切
触れない読み取り専用フェーズ)へ進める状態にある。
