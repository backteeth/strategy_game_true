# P32-004 M0: ステート・プロヴィンスのデータ層

判定: **COMPLETE(M0 のデータ層のみ)**。ステート・プロヴィンス制全体の実装完了ではない。既存ゲームのシミュレーション・セーブ・マップ描画・旧 State 系・アセットには接続していない。Phase 32 は継続。push していない。

- 開始 HEAD: `decb39f`(main)。終了 HEAD: 本コミット(`git log -1`)。
- 開始時 `git status --short`: `?? hex_atlas/` のみ。最終: `?? ../hex_atlas/` のみ(コミット後)。AGENTS.md 等の追加指示は無し。
- 設計書: `verification_logs/phase-32/p32-003/p32-003_state_province_migration_design.md`(M0 と §11 の判断 D1〜D4・D12 を採用方針として実装)。

## 変更ファイル
- `src/world/`(新規。Bevy 非依存): `ids.rs`(型付き ID)、`issue.rs`(診断)、`fingerprint.rs`(地理識別情報)、`files.rs`(ファイル形式・パーサー・初回表の生成ヘルパー)、`model.rs`(検証済み `WorldModel` と検索)、`validate.rs`(地理データとの照合)、`mod.rs`。
- `src/lib.rs`: `pub mod world;`。`src/hexmap/data.rs`: 6 近傍を返す `hex_neighbors` を追加(既存の `river_neighbor` を再利用、加算のみ)。
- `tests/p32_004_world_model_test.rs`(31 件)、`tests/fixtures/p32_004/`(小地域のテスト用データ。**正式な世界設定ではない**)。

## 採用した型(D1)
`ProvinceId(u32)` / `MapStateId(u32)` / `HexCoord{q,r}`。旧 `common::StateId(usize)` とは別型で、相互変換を提供しない(型 ID が別であることをテスト)。旧 State 系は一切改名・変更していない。

## ファイル形式(RON、`format_version: 1`)
- **プロヴィンス表**: `coverage`(`World`=全陸セルを網羅 / `Partial`=検証用の小地域)、`geography`(同期時点の地理識別情報)、`next_province_id`(高水位標)、`retired`(廃止 ID)、`provinces: [{id,q,r}]`。地形・気候・植生は**保存しない**(地理 JSON が唯一の正)。
- **ステート配置**: `states: [{id, name_key, provinces:[…], representative?, allow_disconnected?}]`。
- 非対応の版は、他フィールドの形に関わらず `UnsupportedVersion` で失敗(版を先に読む)。壊れた RON は `Parse`、読めないファイルは `Io`(パス付き)。読み込みは読み取り専用で、ID・所属の自動生成や上書きはしない。
- `generate_initial_province_table` は**明示的に呼ぶ初回生成ヘルパー**(陸セルを `(r,q)` 昇順に 0 から採番)。通常の読み込みからは呼ばれない。本格的な同期ツールは後続。

## 所属関係の正本
**ステート配置の `provinces`(ステート→プロヴィンス)のみ**。プロヴィンス→ステートの逆引きは `WorldModel` 構築時に導出する索引で、保存せず、正本と矛盾し得ない。内部は ID 昇順に正規化され、入力配列の順序が違っても同じ内容なら `WorldModel` が `==` になる(テスト済み)。未割当は `state_of == None`(ダミーステートや ID=0 へ置き換えない)。

## ID の安定性(D2)
ID は表に保存された値で、JSON の配列位置・列挙順・座標から決まらない。属性変更や配列の並べ替えでは既存 ID は変わらない。`next_province_id` が全発行済み ID(使用中+廃止)より大きいことを検証し、廃止 ID の再利用・使用中 ID の廃止扱いをエラーにする。廃止 ID を所属に使うとエラー(「retired」と明示)。

## 地理データとの照合(D12)
`compute_fingerprint(&HexMap)` が 5 種を別々に算出: `grid`(orientation/size/origin/画像寸法/extent)、`coordinates`(全セル座標)、`land_set`(陸セルの座標集合。**座標集合が同じでも陸↔水で変わる**)、`attributes`(地形・気候・植生)、`rivers`(正規化済み河川の辺)。
正規化: セルは `(r,q)` 昇順に整列、固定長リトルエンディアン、文字列は長さ前置、種別ごとのタグ、自前の FNV-1a 64bit(標準の `DefaultHasher` は版で変わるため不使用)。`HashMap` の列挙順に依存しない。`algorithm` 版も保存し、不一致は比較不能として失敗。JSON の整形・セル/河川の並べ替えで指紋が変わらないことをテスト(小地域と実地図)。

変更の扱い:
| 変更 | 検出 | 重大度 |
|---|---|---|
| グリッド定義 | `GridChanged` | Error |
| 陸セルの追加/削除、陸↔水(座標集合が同じでも) | `LandSetChanged` + 個別(`ProvinceCellMissing` / `ProvinceCellIsWater` / `LandCellWithoutProvince`。対象 ID・座標を列挙) | Error |
| 水域セル座標の追加/削除(陸は不変) | `CoordinatesChanged` | Warning |
| 気候・植生・陸種別の属性変更 | `AttributesChanged` | Info(ID・所属は維持) |
| 河川の変更 | `RiversChanged` | Info |

## 制作途中とゲーム開始用の検証の違い
| 項目 | Authoring(制作途中) | GameStart |
|---|---|---|
| 未割当の陸プロヴィンス | Warning(件数・ID 一覧) | **Error** |
| `Coverage::Partial` の表 | Info(「全域の検証は未完了」) | **Error** |
| 陸セルにプロヴィンスが無い | World 表なら Error / Partial 表なら Warning | 同左(Partial は上記で失敗) |
| 構造・地理の不整合 | Error | Error |
`ValidationReport.world_coverage_verified` は、`World` の表で地理との不整合が無い場合のみ true。小地域の表が読めても true にならない(テスト済み)。今回は検証 API までで、ゲーム開始処理には接続していない。

## 検出する問題(構造は構築時にまとめて報告、地理は照合時)
プロヴィンス ID 重複、同一セルへの重複、`next_province_id` 違反、廃止 ID 違反、ステート ID 重複、空ステート、存在しないプロヴィンスへの所属、二重所属(両ステート ID を報告)、代表プロヴィンスの非所属、存在しない/水域のセルを指すプロヴィンス、陸セルのプロヴィンス欠落、未割当、存在しないステート参照(`check_state_ref`)、地理との不整合(上表)。
連結性: 飛び地・島は**エラーにしない**。複数部分に分かれたステートは Warning、`allow_disconnected` で抑止。連結性の規則自体は未決のまま(設計書 §9)。

## テスト結果
- `cargo test -j 2`(全体): **3073 passed / 0 failed / 6 ignored**(126 スイート。P32-002 の 3042 から +31=新規テスト)。`cargo clippy -j 2 --all-targets --all-features -- -D warnings`: 警告 0。既存テストは削除・変更していない。
- `p32_004_world_model_test` 31 件: ID 型の区別、ID の読み込みと検索、配列並べ替えで ID・所属不変、属性変更で ID 維持、座標追加/削除/陸水変更の検出(座標集合が同じ場合を含む)、水域座標のみの変更、グリッド変更、重複 ID/座標・不正参照・二重所属・空ステート・廃止 ID の拒否と複数問題の同時報告、制作途中の未割当許容とゲーム開始用の拒否、Partial 表、非連結の警告、RON 往復、非対応版/壊れたファイル/欠落ファイル、**失敗しても入力ファイルと実行時地図アセットのバイト列が変わらない**こと。
- 全体テストが既存の検証スクリーンショット(p20-007 / p20-009 / p21-save-002e の計 9 枚)を再生成したため、事前に変更が無かったことを確認した上で `git checkout` で復元し、コミットに含めていない。

## 実地図で確認できた範囲(`assets/data/hex_atlas_map.json`。メモリ上の検証で、正式な世界設定ではない)
全陸セルから初回表を生成し(陸プロヴィンス数=陸セル数、1 陸ヘックス=1 プロヴィンス)、(1) ステート無しで Authoring は成功し全未割当を診断、`world_coverage_verified=true`、GameStart は未割当で失敗、(2) 全プロヴィンスを検証用の 1 ステートに入れると GameStart が成功(国境確定ではない)、(3) 実地図の島は連結性 Warning でありエラーにならない、(4) 実 JSON のセル/河川を逆順にしても指紋・ID 表が同一、(5) 陸セル 1 つの気候変更は属性変更のみ、(6) 平原→湖の変更は座標集合不変のまま陸集合変更として検出。
**未確認(後続)**: 実地図の正式なステート配置(未作成。今回の対象外)、全世界の本番データでの GameStart 通過。
件数・SHA-256 は固定条件にしていない(実地図の期待値は入力から動的に導出)。

## 既存ゲーム・アセット・セーブへの影響
なし。`world` は他モジュールから参照されず、旧 State 系・セーブ形式・`saves/`・実行時マップアセット・`hex_atlas/` は変更していない。`hexmap::data` には関数 1 つを追加しただけ。

## 未決事項(設計書 §11 のうち今回確定していないもの)
部分割譲の可否、法的所有の管理単位、占領・支配国の導出、前線計画の単位、移動・占領速度、施設・資源の配置、補給計算の単位、旧セーブ/V2 方針、海域・湖のゲーム上の扱い(湖・海は地理セルとして保持するのみ)、ステートの連結性ルール(Warning 止まり)、D13(湖の所属)。

## 発見事項と後続タスク
- 本格的な同期ツール(差分表示・ID 追記・廃止登録)と編集 UI は未実装(指示どおり後続)。現状は指紋と検証で「再同期が必要」を検出するところまで。
- 小さな既知の挙動: 陸↔水の変更のみが `land_set` に反映されるため、`Terrain::Unclassified` は陸として扱う(`is_water` が偽)。制作ツールの地形未分類の扱いが変わる場合は再確認が必要。
- 次の実装単位(推奨): **M1** — 実地図の小地域(例: 島 1 つ)にテスト用ではない検証用ステート配置を置き、プレビューで境界・所属を表示する前に、まず配置ファイルのローダーを `assets` 側の規約(`assets/data/world/`)へ置く判断(配置場所・ファイル名)をユーザーに確認する。
