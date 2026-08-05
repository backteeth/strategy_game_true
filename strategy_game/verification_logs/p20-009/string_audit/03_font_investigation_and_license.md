# P20-009: 日本語フォント調査・ライセンス記録

## 1. 既存フォントの調査結果

### 1-1. デフォルトフォント(コード内で明示的に指定していたもの)

`src/ui/*.rs` の全`TextFont`は`font_size`のみを指定し、`font`フィールドは常に`..default()`
(すなわち`TextFont::default().font == FontSource::Handle(Handle::default())`)であった。

Bevy 0.19の`bevy_text`クレートは`default_font`機能(bevyクレートのデフォルト機能に含まれる、
無効化していない)により、埋め込み済みの`FiraMono-subset.ttf`(bevy_text本体にコンパイル時
同梱)を`AssetId::<Font>::default()`へ登録する
(`bevy_text-0.19.0/src/lib.rs`: `#[cfg(feature = "default_font")] pub const DEFAULT_FONT_DATA`)。

このFiraMono-subsetはASCII/Latin-1のみのサブセットフォントであり、**日本語グリフを含まない**。
実際に検証環境で日本語テキストを描画すると、豆腐(tofu)表示または文字欠落が発生することを
本タスク開始時点のコード構成から確認した(P20-009対応前は日本語UI自体が存在しなかったため、
問題が可視化されていなかった)。

`system_font_discovery`機能(`parley/system`、OSインストール済みフォントの自動探索)は
Cargo.tomlで有効化されておらず、既存実装はOSフォントへの暗黙依存も無い状態だった。

### 1-2. `assets/fonts/JapaneseFont.ttc` (既存・未使用ファイル)

リポジトリには初回コミット(`38270b9`)から`assets/fonts/JapaneseFont.ttc`(8,990,160バイト)が
存在するが、コード内のどこからも参照されていない(未使用)。

実体を調査した結果:
- ファイル形式: TrueType Collection (3フォント収録)
- Windows GDI+ (`System.Drawing.Text.PrivateFontCollection`) でファミリー名を列挙した結果:
  **"MS UI Gothic" / "MS Gothic" / "MS PGothic"**

これはMicrosoft Windowsに同梱されるプロプライエタリフォント(`C:\Windows\Fonts\msgothic.ttc`と
同一内容)であり、**再配布可能なオープンライセンスのフォントではない**。また、Bevyの
`FontLoader`は拡張子`.ttf`/`.otf`のみを認識するため、`.ttc`拡張子のままではAssetServer経由で
ロードすることもできない。

**判断: このファイルは使用しない。** ライセンス上の懸念があるため削除も検討したが、
削除の是非は本タスクの範囲外と判断し、ユーザーの判断に委ねるためファイル自体は変更していない
(P20-009の対応としては新たに別の再配布可能フォントを追加することで要件を満たした)。

## 2. 追加した日本語対応フォント

| 項目 | 内容 |
|---|---|
| フォント名 | Noto Sans JP (Variable Font, weight axis 100–900) |
| 入手元 | `https://github.com/google/fonts` リポジトリ内 `ofl/notosansjp/NotoSansJP[wght].ttf` (Google Fonts公式配布、Googleとメーカーの共同開発によるNoto CJKファミリーの日本語サブセット) |
| ライセンス | SIL Open Font License, Version 1.1 (再配布・改変可、フォント単体の再販のみ禁止) |
| 配置場所 | `strategy_game/assets/fonts/NotoSansJP-Variable.ttf` (9,589,900バイト) |
| ライセンス文書 | `strategy_game/assets/fonts/NotoSansJP-OFL.txt` (Google Fonts配布物に同梱のOFL全文) |
| SHA-256 (フォント本体) | `c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f` |
| SHA-256 (ライセンス文書) | `1c05c68c34f9708415aada51f17e1b0092d2cea709bf4a94cd38114f9e73d7d9` |

### 2-1. UIへの適用方法

`src/localization.rs`の`install_japanese_capable_font`システム(`PreStartup`で実行)が、
`Font::from_bytes(NOTO_SANS_JP_DATA.to_vec())` (コンパイル時に`include_bytes!`で埋め込み)を
`Assets<Font>`の`AssetId::<Font>::default()`へ**上書き挿入**する。

`TextFont::default().font`が`FontSource::Handle(Handle::default())`を指すため、既存の
全`TextFont { font_size: ..., ..default() }`呼び出し箇所(9ファイル、100箇所超)を
**一切変更せずに**、アプリ全体のデフォルトフォントを日本語対応フォントへ差し替えられる。

Noto Sans JPはLatin文字も完全にカバーするため、日本語・英語のいずれの言語でも同一フォントを
使用する(言語切替時にフォント自体を切り替える処理は不要)。

`system_font_discovery`機能は依然として無効のままであり、OSインストール済みフォントへの
暗黙依存は無い(フォントはバイナリに埋め込まれ、実行環境のOS設定に関わらず同一の見た目になる)。

## 3. 描画検証

- `tests/p20_009_localization_headless_render_test.rs` にて、本番`UiPlugin`・本番`GameCamera`・
  実GPU (`NVIDIA GeForce RTX 5070 Ti`, Vulkan backend) によるHeadless実描画で、
  `Assets<Font>`に`AssetId::default()`のフォントが存在することをアサートしている。
- 同テストで日本語(ja-JP)・英語(en-US)それぞれの実描画PNGを保存し、目視でも文字化け・
  豆腐表示・ロード失敗が無いことを確認した(`verification_logs/p20-009/screenshots/`)。
- 既知の非致命的な制限: Parley/ICU4Xの日本語向け分節(セグメンテーション)モデルが
  バンドルされていないため、テスト実行時に`ICU4X data error: No segmentation model for
  language: ja`という警告が標準エラーに出力される。これは日本語の行分割(word-wrap)が
  厳密なUnicode分節規則ではなくフォールバックアルゴリズムで行われることを意味するが、
  文字自体のグリフ表示・文字化け・ロード失敗には影響しない(PNG目視確認で問題なしを確認済み)。
