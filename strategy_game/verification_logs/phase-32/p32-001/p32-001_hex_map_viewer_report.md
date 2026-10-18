# P32-001 ヘックスマップ読み込み・描画・操作

判定: **PARTIAL**(河川以外は実装・検証済み。河川描画は direction の定義資料が無いため未実装)。
実マウス/キーボード操作による GUI 確認は未実施のため、その項目は手動確認待ち。Phase 32 は閉鎖しない。push していない。

- 開始 HEAD: `084b6941244dac2406bd396ceea549cddee927dc`(ブランチ main)。終了 HEAD: このコミット(`git log -1`)。
- 開始時 `git status --short`: `?? hex_atlas/`(ユーザーの未追跡ディレクトリ。コミットに含めない)。
- 最終 `git status --short`(コミット後): `?? ../hex_atlas/` のみ。

## 入力
- 実パス: `C:\Users\hirom\Desktop\strategy_game\hex_atlas\hex-atlas-map.json`(`hex-atlas-map(1).json` は存在せず、同ディレクトリにはこの1ファイルのみ。旧版との混在なし)。
- サイズ 8,038,888 bytes、SHA-256 `dc6dc7342688c969795556f6fc6679bcc2fa669325e652656ca24c4ec571faa5` = 提示値と一致。
- 取り込み先: `strategy_game/assets/data/hex_atlas_map.json`(既存 `assets/data/*.ron` と同じ cwd 相対読み込み規約)。原本と sha256 一致を確認、テストでも原本があれば内容一致を検査。原本は未変更。
- hex_atlas 内に制作ツールのソース・仕様書は無い(JSON のみ)。

## 変更ファイル
- `Cargo.toml`/`Cargo.lock`: `serde_json = "1"` 追加(lock に既存の版。Bevy/Rust は更新なし)。
- `src/lib.rs`: `pub mod hexmap;`。`src/localization.rs`: `install_japanese_capable_font` を `pub` 化(日本語フォントを確認画面で再利用)。
- `src/hexmap/data.rs`: DTO → 検証 → 不変 `HexMap`。 `geometry.rs`: 座標変換・逆変換・外接矩形・選択。 `render.rs`: 色・凡例・チャンクメッシュ。 `view.rs`: 専用カメラ/UI/選択(`HexPreviewState` Closed/Open)。
- `src/bin/hex_map_preview.rs`: 起動用バイナリ(`--shots <dir>` で自動スクリーンショット)。
- `tests/p32_001_hex_map_test.rs`、`assets/data/hex_atlas_map.json`、本報告書とスクリーンショット。

既存ゲームへの影響: 既存の州・国家・セーブ・カメラ・`GameState` には接続していない。メインのバイナリは無変更。旧マップのゲーム進行は影響なし(全体テスト 3030 passed)。

## データ(今回の入力)
schemaVersion 9 / kind `hex-atlas-game-map` / pointy / size 2.5 / 画像 1774×887 / extent q -118..409, r 0..235。セル 96,760、河川 3,297、重複なし、河川参照元欠落なし(テストで確認)。
地形: sea 62,055 / lake 194 / plains 17,276 / hills 7,553 / plateau 4,793 / highland 2,997 / mountain 1,892 / unclassified 0。
気候: A 4,996 / B 7,523 / C 11,686 / D 8,337 / E 1,945 / unclassified 62,273。植生: rainforest 4,355 / forest 5,886 / grassland 17,487 / none 5,529 / unclassified 63,503。
陸上 34,511 のうち気候未分類 24、植生未分類 1,254(診断のみ、補完しない)。件数はローダーに固定していない(テスト側のみ)。
格子は疎(96,760 < 124,608)で、負の q を含む。

検証仕様: kind・schemaVersion(9のみ)・orientation(pointyのみ)・size 正有限・origin 有限・画像寸法>0・extent 順序・整数座標・extent 外/重複セル・属性値(未知文字列は `UnknownValue` エラー。terrain/climate/vegetation の `unclassified` は有効値)・河川の参照元セル/重複/direction 範囲 0..=5。
制作ツールの追加識別子(疎林・サバナ等)は識別子不明のため未対応(読み込み時に明確なエラー)。

## 座標系
x = origin.x + √3·size·(q + r/2)、y = origin.y + 1.5·size·r(画像座標、Y下向き)。画像→ワールドの Y 反転は `geometry::image_to_world/world_to_image` の 1 組のみ(拡大率 1:1、原点そのまま)。逆変換はキューブ丸め。辺・頂点ちょうどの点はこの丸め規則で決定的に 1 セルへ割り当てる。
整合確認: 全セル中心が画像矩形内(制作ツールは中心基準で出力している)。境界セルの頂点は最大で半セル分(約2.2)画像外へ出る(236 セル)。セルは削らず、外接矩形は頂点込みで算出し、初回表示・全体表示に使用。

## 描画方式
約128ワールド単位のチャンク(実データ 98)ごとに 1 メッシュ(1セル=頂点6・三角形4、頂点色)。モード切替は頂点色のみ書換え。ヘックス境界線はチャンクごとの LineList(共有辺は二重に引かない)で、ズームアウト時(scale≥0.7)と G キーで非表示。選択枠は黒縁+黄色の輪(約3px幅をズームに追従、小メッシュのみ更新)。全セル独立の Mesh/Material/Entity は無し。
水域は気候・植生モードでも青系、水域の未分類(青)と陸上の未分類(マゼンタ)を区別。`none`(砂色)と `unclassified` も別色。凡例に件数付き。

## 河川(未実装)
JSON の河川 (q,r,direction) は保持・検証済み(範囲 0..=5 は六角形の辺数に基づく仮定)。direction が指す辺・流下方向・重複辺正規化の有無を裏付ける資料(制作ツールのソース/エクスポート仕様)が hex_atlas 内にも他にも見つからなかったため、推測では描画していない。画面にも「河川描画は未実装」と表示。
必要な資料: 「制作ツールの direction を定義する箇所」または「方向番号と六辺の対応表」(Y軸の向きを含む)。

## 起動・操作
`strategy_game/` で `cargo run --release --bin hex_map_preview`。ホイール=カーソル中心ズーム、左/右/中ドラッグ=パン(左は4px以内の離しでクリック=選択)、1/2/3=地形/気候/植生、Home または「全体表示」ボタン、G=境界線、Tab=画面を閉じる/開く(開き直しで再読込、Entity・リソースを全破棄)、Esc=終了。UI 上の操作は地図へ渡さない。読み込み失敗時はマップ Entity を作らずエラーを右パネルに表示し、Tab/Esc で復帰できる。

## 検証結果
- `cargo test -j 2`(全体): **3030 passed / 0 failed / 6 ignored**(124 スイート、約12分)。※並列ビルドで OS エラー 1455(ページファイル不足)が出たため `-j 2` で実行。
- `cargo clippy --all-targets --all-features -- -D warnings`: 警告 0。
- 新規: `tests/p32_001_hex_map_test.rs` 19 件(実データ件数・集計・負q・疎格子・河川参照・全セル中心往復と選択・内部点選択・辺中点の決定性・欠損/地図外/非有限の非選択・チャンク網羅・壊れたJSON/非対応スキーマ/orientation/kind・未知属性・重複/extent外・不正size/画像/extent/非整数・不正河川・欠損ファイル)、`view.rs` 単体 5 件(カーソル→ワールド、カーソル固定ズーム、全体表示、極端値/NaN の clamp)。
- 全体テストが既存の検証スクリーンショット(p20-007/p20-009/p21-save-002e)を上書きしたため、`git checkout` で復元しコミットに含めていない。

## GUI 確認(`--shots` による自動実行。実入力操作ではない)
Windows / NVIDIA GeForce RTX 5070 Ti / release / 1600×900 で実地図を表示し、`screenshots/` に保存:
`01_overview_terrain`(全体の形)、`02_overview_climate`、`03_overview_vegetation`(none と未分類の区別、水域の識別)、`04_selected_zoomed`(山岳セル q=236,r=6 を選択、属性表示)、`05_after_fit_selected`(全体表示へ復帰)。→ **全体表示・3モード・属性表示・ズーム表示・全体復帰は目視確認済み**。
**手動確認待ち**: 実ホイール/ドラッグ/クリックによるズーム・パン・選択、ズーム/パン後の選択位置、UI 上でのクリック・ホイールの非透過、ウィンドウ縮小・縦横比変更、Tab による開閉の重複なし、河川位置。

## 性能(release、上記環境、実データ)
読込(read+parse+検証) 約48 ms / メッシュ生成 約29 ms / 98 チャンク・メッシュ 198(塗り98+線98+選択2)・マップ Entity 198。全体表示のアイドル 40 フレーム: 平均 約7.6〜8.1 ms、最大 約21〜23 ms(スクリプト実行・スクリーンショット前の測定。パン/ズーム中のフレーム時間は未測定)。debug 版は未測定。過去の「2000州」とは条件が異なり比較しない。FPS は保証しない。

## 既知の制限・未解決
河川未実装(上記)。パン/ズーム中の性能と実操作の GUI 確認が未了。モード切替は全チャンクの頂点色(約58万頂点)を書き換える(切替時のみ)。今回未使用の植生等は識別子が不明で未対応。ヘックス隣接・方向の番号付けはグリッド線の重複除去専用で、河川 direction とは無関係。

## 次への接続点
`HexMap`(座標→セル検索 `cell(q,r)`、セル属性、河川レコード)を州/国家の割り当て層から参照する。州=複数ヘックスの集合とする案は未確定で、今回は既存 `StateRegistry` に触れていない。
