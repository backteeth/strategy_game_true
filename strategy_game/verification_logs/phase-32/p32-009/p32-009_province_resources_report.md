# P32-009: プロヴィンス別天然資源の入力とステート経済への接続

判定(実装・検証): **COMPLETE**。M4 全体や、新マップでのゲーム進行の完成ではない。GUI の変更は無く、GUI 確認待ちは設けない。Phase 32 は継続。
開始 HEAD: `9f37180`(main。開始時に `origin/main` と一致、未追跡は `hex_atlas/` のみ)。終了 HEAD: 本コミット(`git log -1`)。

## 既存の資源値の意味(調査結果。今回この意味を変えていない)
実コード(`economy::resources::StateResourceDeposit`、`economy::production`、`app::loader`、`assets/data/resources.ron`)の調査:
| フィールド | 既存の意味 | 使われ方 |
|---|---|---|
| `base_output` | **採掘施設(Mine)1レベルあたりの月間産出能力**(稼働率100%) | 実産出 = `base_output × Mineのレベル × 稼働率`(`calculate_actual_output`)。同一ステートに同種の鉱床が複数あれば独立に加算される(線形)。**埋蔵量でも採掘済み在庫でもない** |
| `discovered` | 産出に使うか(true のみ) | 生産は`discovered`の鉱床だけを採掘。クリスタル採掘施設の建設ゲート(`has_discovered_deposit`)にも使用 |
| `development_level` | メタデータ | 保持・コピーされるだけで、現行の計算では参照されない |
| (埋蔵量・枯渇・再生) | **既存に存在しない** | 鉱床の値は時間で更新されない。今回も追加していない |
| 国家備蓄 `CountryStockpile` | 採掘済みの在庫 | 産出は生産処理が備蓄へ加える。地理上の資源値とは別物 |
- 資源の分類は既存の `ResourceType` を再利用(新しい鉱物・加工品は追加していない)。`MagicCrystal` 鉱床は汎用 Mine では採掘されない(既存仕様のまま)。精製・加工(Factory 等)は備蓄を介して接続され、鉱床には直接依存しない。

## 新しい入力形式と正本
- `economy::province_resources`(新規): `ProvinceResourcesFile`(`format_version: 1`、`scope: World|Partial`、`provinces: [(province: ProvinceId, deposits: [既存の StateResourceDeposit と同じ形])]`)。**天然資源の所在地(入力)の正本はプロヴィンス別の資源定義ファイル**(`assets/data/world/preview/province_resources.ron`)。地形 JSON・プロヴィンス ID 表・ステート所属とは別ファイルで、地理 JSON へ資源を書き足さず、ProvinceId は座標や配列位置から作り直さない。読み込みは読み取り専用で、資源を自動生成せず、入力ファイルを書き換えない(テストで確認)。海・湖はプロヴィンス(陸のみ)に無いので資源を割り当てられない。
- 経済処理が使う「ステートで利用できる鉱床」は、**導出データ**(`StateResourcesDerived`、保存・書き戻しなし)。経済側の鉱床入力の正本は、プロヴィンス別の資源定義を接続したときはその導出結果になり、**シナリオのステート単位の `resource_deposits` が空でないステートは二重入力としてエラー**(`ResourceDoubleSource`)。

## 未設定・資源なし・完全性
- **設定済みで資源あり**: `provinces` に載り `deposits` が非空。**設定済みで資源なし**: 載っていて `deposits: []`(明示)。**未設定**: ファイルに載っていない(資源ゼロにも無制限にも変換しない)。
- `scope: World` は「プロヴィンス表の全プロヴィンスを設定した」という宣言で、未設定が残れば入力検証エラー(`WorldScopeIncomplete`)。`scope: Partial`(検証地域)は全世界の資源配置の完了を意味しない。
- ステートごとの状況 `StateResourceSetup`(Complete / Incomplete / Unconfigured)と未設定プロヴィンスの一覧を結果に含める。`all_states_complete()` も全体の完了とは別(`scope` も確認する旨を明記)。
- **経済計算前の検証**: 鉱床を産出に使う施設(Mine)を持つステートの資源が Complete でなければ、`attach_province_resources` がエラー(`ResourcesNotConfigured`、未設定プロヴィンスを列挙)。エラー時はレジストリを一切変更しない。

## ステートへの集計
- 検証: 存在しない ProvinceId、同一プロヴィンスの重複定義、同一プロヴィンス内の同種資源の重複(曖昧)、`base_output` の負・NaN・無限大、非対応バージョン、不正な資源種別(パース時)をすべて拒否。部分的な集計は作らない。
- 集計: 所属関係(M0)で各プロヴィンスを**ちょうど1回**所属ステートへ。`(資源種別, discovered)` ごとに加算できる `base_output` だけを ProvinceId 昇順で加算(入力順に依存しない・浮動小数の加算順が固定)。`discovered` は加算せず別の鉱床として保持。加算できない `development_level` は経済計算で未使用の属性なので、集計後の値は「最大値」という代表値(意味を持たせない)とし、プロヴィンスごとの値は `sources` に保持する。合計が有限でなければ `NonFiniteTotal` エラー(オーバーフロー)。
- 未割当プロヴィンスの資源は、どのステートにも集計せず `unassigned_configured` に報告(架空のステートを作らない)。別ステートの資源は混ざらない。
- 追従: 導出結果は所属の署名と資源定義の内容の署名(並び順に依存しない)を持ち、`is_current_for` で古い集計を検出。所属変更後は再導出で資源が新しいステートへ移り、元のステートに残らない(データ層の再導出。占領・和平ルールは未実装)。

## 経済への接続
`MapStateEconomyRegistry::attach_province_resources(derived, world, file)` が、集計結果を各 `MapStateEconomy.resource_deposits`(生産処理の入力)へ設定する。**生産の式・処理は P32-007/008 の共通処理(`process_country_production`)のまま**で、旧経路用・新経路用の別式は作っていない。鉱床を国家備蓄へ直接加算することはなく(テストで確認)、人口・施設・備蓄をプロヴィンスへ複製していない。旧ゲームの通常経路・公開 API は無変更。

## 検証用データと集計結果(`province_resources.ron`。正式な分布・バランスではない)
| ステート(所属数) | 設定 | 集計(経済への入力) |
|---|---|---|
| MapState#20(7) | 全7設定(資源なし明示4) | Iron 25+15=**40**(Province 18309/18310)、Coal **30**(18311)。Complete |
| MapState#21(13) | 3設定(資源なし明示1)、**未設定10** | Iron 20(発見済み)、Iron 12(**未発見**・別の鉱床として保持)。Incomplete |
| MapState#22(6) | 設定なし | 空。Unconfigured |
| (未割当 17903) | 設定済み | Iron 99 は**どのステートにも集計しない** |
同一ステート内の複数プロヴィンスに分かれた資源(20)、異なるステートの同種資源 Iron(20 と 21)、資源なしの明示、未割当プロヴィンスの資源、未設定を検出するケース(21・22)を含む。`scope: Partial`。

## 旧経路との比較
- 旧入力(シナリオにステート単位で Iron 40・Coal 30)と、新入力(ステート単位を空にしてプロヴィンス別 25+15・30 を接続)で、**1か月の生産結果(全備蓄・研究力・Iron 産出・雇用・失業・物流・稼働率)が完全一致**。M4-B の6か月・複数国家シナリオも、国家の全状態(国庫・収支・備蓄・人的資源)と各ステートの人口・雇用・生活水準・不満度が**毎月完全一致**。
- 総量が同じなら分割に依存しない: Iron 40 を 1 プロヴィンスに置いても 4 プロヴィンスに分けても結果が一致。同じ総量を 7 プロヴィンスのステート(20)と 6 プロヴィンスのステート(22)に置いても結果が一致(プロヴィンス数による増幅なし)。
- 独立した手計算: 人口 200000・比率 0.5・Mine 1 → 稼働率 0.1(雇用 10000 / 潜在労働力 100000)、産出 Iron = 40×1×0.1 = 4.0、Coal = 30×0.1 = 3.0。他のステート・未割当の資源は混ざらない。資源不足時の既存の制限(入力検証で Mine 施設の稼働は労働力・物流・入力資源で制約)は共通の生産処理のまま。

## 必須チェックの結果
- `cargo test -j 1 -- --test-threads=1`(標準出力・標準エラーを `full_test_j1_threads1.log` に保存。他のビルド・テストは並走させていない): **終了コード 0 / 3140 passed / 0 failed / 6 ignored(133 スイート)**。前回の 3124 から +16(新規 `p32_009_province_resources_test`)。メモリ不足による中断なし。既存の人口・動員・雇用・生産・財政・新旧比較(`p32_007`/`p32_008`、`economy_tests`、`p26_det_001_rev_001`、`p21_009_magic_crystal` ほか)は全体実行に含まれ成功。
- `cargo clippy -j 1 --all-targets --all-features -- -D warnings`: 警告 0。
- `world_economy_check`: 実行済み(終了コード 0)。出力 `world_economy_check_output.txt` に、資源の出典・ステート集計・生産計算への入力と結果、旧入力との一致(true)を表示。
- 全体テストが既存の検証スクリーンショット(p20-007 / p20-009 / p21-save-002e の計9枚)を再生成したため、事前に変更が無かったことを確認した上で `git checkout` で復元し、コミットに含めていない。

## 変更ファイル
`src/economy/province_resources.rs`(新規: 入力形式・検証・集計・署名)、`src/economy/map_state.rs`(`attach_province_resources`、新しいエラー種別)、`src/economy/mod.rs`(モジュール公開)、`src/world/aggregate.rs`(所属署名を crate 内で再利用できるように `pub(crate)` 化)、`src/bin/world_economy_check.rs`(資源の出力)、`assets/data/world/preview/province_resources.ron`(検証用データ)、`tests/p32_009_province_resources_test.rs`(16件)。

## 既存ゲーム・アセット・セーブへの影響
なし。地理 JSON・既存マップアセット・既存ゲームデータ・`saves/`・セーブ形式・旧 State 系・`hex_atlas/` は未変更。追加したデータファイルは検証用の `province_resources.ron` のみで、通常起動・地理プレビューには注入されない。

## 未接続部分と次の実装単位
未接続(対象外): 正式な全世界の資源配置、資源の自動生成、枯渇・再生・採掘速度の新ルール、位置施設(港・要塞・鉄道駅)、AI・建設キュー(鉱床ゲート判定を含む)、占領による資源利用の変更、法的所有・部分割譲、セーブ。
次の候補: 位置施設のプロヴィンス入力層(同じ未設定/設定済みの区別を使う)、または M4-C(経済状態の分類・建設キュー・AI の税率調整/建設の新ステートへの接続。AI の建設判断は鉱床の有無を参照するため、今回の導出結果を入力にする)。着手前にユーザーへ確認する。
