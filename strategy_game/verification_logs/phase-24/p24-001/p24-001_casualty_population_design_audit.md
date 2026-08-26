# P24-001 設計監査報告書: 戦死者・人口・人的資源上限・Attrition会計

## 1. 監査判定

**AUDIT COMPLETE**

## 2. 設計判定

**DESIGN DECIDED**

## 3. 開始時HEAD

`51827688a09b04f81bd9ad047e55792109ff695d`(P23-010監査報告書コミット、プロンプト指定の
基準コミットと一致)

## 4. 監査対象コードHEAD

`51827688a09b04f81bd9ad047e55792109ff695d`(本タスクは本番コード・テストコードを一切
変更していないため、開始時HEADと同一)

## 5. ユーザーセーブの保護

`saves/savegame_v1.ron`は監査を通じて一切開かず、読まず、変更していない。数値検証は
既存の単体テスト・Domain関数の仕様を手計算で追跡する形で行い、一時スクリプトや
テストコードは作成していない(§14の指示どおり)。

## 6. 現在のpopulation・workforce・manpowerの単位

* `StateData.population: u64`(単位: 人)。月次成長のみで増加し(`population::growth::
  grow_state_population`、basis point固定小数点、切り捨て)、減少させる経路は
  現行コードに一切存在しない。
* `workforce_ratio: f32`(既定0.45、`DEFAULT_WORKFORCE_RATIO`)。
  `total_workforce() = round(population * workforce_ratio)`(`StateData::civilian_
  workforce`周辺、`population::update::calculate_workforce`、**四捨五入**)。
* ただし`StateData::total_workforce()`自身(P23-004の`E/P`比率の分母として使われる、
  `economy::finance`/`economy::production`が参照)は`((population as f64) *
  (workforce_ratio as f64)) as u64`と**切り捨て**であり、`calculate_workforce`
  (四捨五入)と丸め方式が異なる。両者は「人口×workforce_ratio」という同じ概念を表す
  はずだが、実装が2箇所に分かれ丸めルールが食い違っている(既存の不整合、Phase 23
  時点でも変更されていない。P24-002が新しい丸め計算を追加する際は、このどちらに
  合わせるかを明示すべき)。
* 人的資源基準上限(manpower cap)は`population::growth::compute_manpower_cap`が
  `population * MANPOWER_CAP_RATIO_BP(=1,000, 10%) / 10,000`で算出する。
  `workforce_ratio`(45%)とは完全に独立した別の抽出比率であり、両者の合計が
  100%を超えても現行コードは何も検証しない(=同じ人口から「最大45%の労働力」と
  「最大10%の軍事動員上限」を独立に切り出せる、という設計)。

## 7. A・B・L・K・Xの現在の意味

`src/military/manpower.rs`の実装を再確認し、Phase 23監査(P23-010)の記述と完全に
一致することを確認した。変更なし。

```text
A = available_manpower
B = mobilized_manpower(累積動員負担。現在生存兵員数ではない)
L = 募兵キュー予約分 + 生存Division現在manpower合計
K = B.saturating_sub(L)
X = L.saturating_sub(B)
```

## 8. 全呼び出し経路とSystem順序

P23-010が確定した`DailySimulationSet`順序(`TimeUpdate → Economy → Research →
Diplomacy → CountryAi → WarPreparation → MilitaryAi → FrontlineOrders →
MilitaryAction → WarResolution → UiUpdate`、`app::time::configure_sets(...).chain()`
で強制)を再確認し、変更がないことを確認した。本監査で新たに着目した箇所:

* `military::update::process_daily_battles`(`MilitaryAction` Set内)で、
  `resolve_combat_day_multi`が返す`atk_results`/`def_results: Vec<(DivisionId,
  (manpower_loss, org_loss))>`を**そのDivision・その国家ごとに個別に**取得できる
  ことを確認した——`division.manpower.saturating_sub(manpower_loss)`を適用する
  直前の時点であり、将来この場所へ「損耗量を出身州のpopulationへ即座に反映する」
  フックを追加するのに最適な、既存の正規経路である(新しいSystemを追加する必要が
  ない)。
* 完全消滅の追加損耗(`resolve_finished_battles`内の`resolve_participant_outcome`が
  課す`RETREAT_MANPOWER_LOSS_RATIO`による撤退時追加損失、および組織率0での包囲撃破)
  は`process_daily_battles`とは**別の関数・別のタイミング**で発生する。将来の
  casualty→population接続は、この2箇所の両方を対象にしなければ撤退時損耗を
  取りこぼす。
* `Division`構造体には出身州を保持するフィールドが**存在しない**。
  `RecruitmentQueueItem.target_state`が唯一「その募兵が完成する州」を保持するが、
  Division完成後(`recruitment::process_recruitment`)にこの情報はDivisionへ
  引き継がれず破棄される。New Game初期Division(`app::loader::spawn_debug_
  divisions`)は`country.capital_state_id`へ直接spawnされ、こちらもDivisionへ
  出身州情報を残さない。**実データに出身州追跡は一切存在しない**ことを確認した
  (§16で「出身州が存在する前提で設計しない」という指示への回答)。

## 9. 丸め逆説の実関数による再現結果

### 最小反例(単一州国家、`calculate_workforce`の四捨五入だけで再現する)

```text
前提: workforce_ratio = 0.45(既定値)、国家は州を1つだけ所有
population = 1,000,000
B = 100,000(この州の按分負担は国家の州が1つだけなので按分なしでB全額)

戦死前:
  total_wf = round(1,000,000 * 0.45) = round(450,000.0) = 450,000
  civilian_workforce = total_wf - B = 450,000 - 100,000 = 350,000

戦死1人(案B: population -= 1 と B -= 1 を同時に行う場合):
  population = 999,999
  total_wf = round(999,999 * 0.45) = round(449,999.55) = 450,000 (四捨五入で不変)
  B = 99,999
  civilian_workforce = 450,000 - 99,999 = 350,001  ← 1人死んだのに労働力が1人増える
```

この反例は最大剰余法の按分すら経由せず、`calculate_workforce`の四捨五入だけで
再現する。`0.45 * 1`は`1`未満なので、population側の変化は約55%の確率で
`total_wf`の丸め値を全く動かさない(1/0.45≈2.22人減るごとにようやく1回、
丸め境界をまたいで`total_wf`が1減る)。一方`B`は戦死者数と同じ量だけ確実に減る。
この非対称性(`total_wf`は人口の変化に対して「粗く」反応し、`B`は「正確に」反応する)
が逆説の直接の原因である。

### 一般証明(案C: population -= C、Bは不変)

`total_workforce(population)`は`population`について広義単調非減少である
(`round(k*p)`は`p`の非減少関数、`k=workforce_ratio>=0`)。よって`population`を
減らすと`total_wf`は**減るか、丸めの都合で変化しないかのどちらか**であり、
増えることは絶対にない。`B`(および国家按分後のこの州の負担)を一切変更しなければ、

```text
civilian_workforce_after = total_wf(population - C) - B
                         <= total_wf(population) - B
                         = civilian_workforce_before
```

が常に成立する(等号は丸め境界をまたがない場合)。**Bを一切動かさないことが、
逆説を封じる必要十分に近い条件である**——Phase 23が既に確立している「戦死によって
Bを減らさない」という原則(P23-006)を、population側でも一貫させるだけで、
新しい保存則やクランプを追加せずに逆説が数学的に発生しなくなる。

### 他の境界値での確認

* `workforce ratio = 0`: `total_wf`は常に0。civilian_workforceは常に
  `0.saturating_sub(B)=0`。population変化は無関係。逆説なし(自明)。
* `workforce ratio = 1.0`: `total_wf = round(population)= population`(丸めなし)。
  population -= Cで`total_wf`も正確に-Cされる。Bを動かさない案Cなら
  civilian_workforceは正確に-C(丸め境界の問題自体が発生しない)。
* `B = 0`: civilian_workforce = total_wf - 0 = total_wf。populationを減らせば
  civilian_workforceは減るか不変。逆説なし。
* `Bが潜在労働力(total_wf)を上回る`: `civilian_workforce = total_wf.saturating_
  sub(B) = 0`のまま。populationがわずかに減っても`total_wf`が0近辺なら
  引き続き0(`saturating_sub`が下限0でクランプするため、逆説どころか変化自体が
  観測できない領域)。
* `population 0`: `total_wf=0`、`civilian_workforce=0`。以後population -= Cは
  `saturating_sub`でunderflowせず0のまま(u64のため、既存の`calculate_workforce`
  自体は負値を扱わない設計)。
* `population丸め境界の直前/直後`(例: `population * 0.45`が`X.4999...`と
  `X.5000...`をまたぐ1人差): 上記最小反例と同型。案Cなら`total_wf`が最大1減る
  だけで、civilian_workforceが増えることはない。
* `複数州への按分`(国家按分の最大剰余法、`apportion_mobilized_manpower`):
  Bを一切動かさない案Cでは、そもそも「Bの按分結果が変わる」入力自体が発生しない
  ため、按分アルゴリズムのタイブレークや剰余処理を通じた逆説の混入経路も存在
  しない。

**結論**: 案A(現状維持)・案C(populationのみ減算、Bは不変)は逆説を起こさない。
案B(populationとBを同時に減算)・按分後の値を再利用するあらゆる案は、按分の
タイミングと丸め方式が完全に一致しない限り逆説の危険を常に抱える。`.min()`や
`saturating_sub()`で症状を隠す代わりに、「populationとBを同じイベントで
同時に減らさない」という設計上の制約そのもので逆説を排除する。

## 10. 候補案ごとの数値例

| 案 | 戦死後のpopulation | 戦死後のB | 戦死後のL | 戦死後のK | 逆説 | Phase23互換 |
|---|---|---|---|---|---|---|
| A(現状維持) | 不変 | 不変 | -C | +C | 発生しない(populationが動かないため) | 完全互換(無変更) |
| B(population・B同時減算) | -C | -C | -C | 不変 | **発生する**(§9で実証) | K=B-Lの意味を破壊(K不変のまま損失を隠す) |
| C(populationのみ減算) | -C | 不変 | -C | +C | 発生しない(§9で証明) | 完全互換(既存API無変更、populationにだけ新しい書き込み経路を追加) |
| D(population減算+別会計ストック) | -C | 不変 | -C | +C | 発生しない(Cが不変なら§9の証明と同型) | 互換だが新規フィールド(累積戦死者数等)が要る分、複雑 |
| E(人口をコホート分離) | 設計次第 | - | - | - | 理論上は最も厳密だが未評価 | Phase24の最小実装としては過剰 |

案D・Eは案Cと比べて「populationを減らす」という核の操作は同じであり、追加する
のは表示・統計目的の付随情報(例: 国家別・州別の累積戦死者数)である。この付随
情報を永続化するかどうかは§20で判断する。

## 11. 候補案比較表

| 評価軸 | A(現状維持) | B(population+B同時減算) | C(populationのみ) | D(C+別ストック) |
|---|---|---|---|---|
| 数学的一貫性 | 保存則の対象がそもそも無い(population不変) | **破綻**(§9で逆説を実証) | 保存則が成立(§9で証明) | 成立(Cと同型) |
| ゲーム上の自然さ | 戦死しても人口が減らないのは不自然 | 自然(だが逆説で自壊) | 自然 | 自然+統計表示が可能 |
| Phase23互換性 | 完全 | Kの意味(B-Lの差分)が実質壊れる | 完全(既存API無変更) | 完全 |
| 経済連動 | 二重控除なし(そもそも未接続) | 二重控除の危険(§9) | 二重控除なし(§9で証明) | 二重控除なし |
| 州人口連動 | 該当なし | 出身州追跡が必要(§7で未実装と確認) | 同左 | 同左 |
| 長期回復 | 該当なし(population自体不変) | population増加でBがどう回復するか未定義で矛盾 | population増加は既存の月次成長がそのまま処理、Bには触れないため矛盾なし | 同左 |
| セーブ互換性 | 変更なし | 新規フィールド次第でV1/V2どちらもあり得る | **V1維持可能**(§7・§20) | Dの追加ストック次第でV2寄り |
| 決定性 | 該当なし | 按分依存で決定性リスクあり | 直接減算のみで按分を経由しないため決定的 | 同左 |
| 実装規模 | 0(変更なし) | 中〜大(Bの意味変更を伴う) | **小**(population直接減算+出身州フィールド追加) | 中(Cに統計フィールド追加) |
| 将来拡張 | 拡張の土台がない | Attritionを足すとさらに逆説が悪化 | Attritionも同じ経路で自然に拡張可能 | 同左+統計UIへ拡張しやすい |

**採用: 案C。次点: 案D(P24-002完了後、UI要件が固まれば案Cの上に統計ストックを
追加する形で自然に拡張可能なため、却下ではなく将来候補として保留)。却下: 案B
(§9で数学的に逆説を実証したため)。案A(現状維持)は「Kをpopulationへ接続しない」
という選択肢であり、これも有効な最終結論だが、ユーザーが今回接続を志向している
ため次点にも置かない。案Eは時期尚早として却下(過剰実装、Phase24の最小範囲を
超える)。**

## 12. 採用する推奨設計

**populationは軍人を含む国内総人口の抽象的な近似値であり続ける。戦闘・撤退時の
実損耗Cが確定した瞬間に、その損耗を負ったDivisionの出身州(新設フィールドで追跡)
のpopulationから直接Cを減算する。国家のB(mobilized_manpower)は一切変更しない
(Phase23の原則を継続)。populationの減算は、月次の按分・丸め計算を一切経由しない
直接操作であり、按分由来の丸め逆説を構造的に回避する。**

具体的には:

1. `Division`へ`origin_state_id: Option<StateId>`を追加する(`#[serde(default)]`、
   Save V1のまま移行可能。既存Save由来の`None`は「出身州不明、population減算を
   スキップ」として安全に扱う)。
2. `RecruitmentQueueItem`完成時(`recruitment::process_recruitment`)に、
   `target_state`を新Divisionの`origin_state_id`へコピーする。
3. `app::loader::spawn_debug_divisions`(New Game初期配置)で、spawn時点の
   `country.capital_state_id`を`origin_state_id`へ設定する。
4. `military::update::process_daily_battles`で`manpower_loss`を適用する直前に、
   `division.origin_state_id`が現在も**その師団のowner国家が実効支配する州**
   であれば、その州の`population`から同量を`saturating_sub`する(出身州が既に
   他国領・支配権喪失なら**減算しない**、§7参照)。
5. `resolve_finished_battles`の撤退時追加損耗・包囲撃破時の残存manpowerについても
   同じ関数を再利用して同量を減算する。
6. `military::manpower::compute_manpower_ledger`・`B`・`K`の定義・算出式は
   一切変更しない。

## 13. 却下案と理由

* **案B(populationとBを同時に減算)**: §9で丸め逆説を実際の関数から再現した。
  按分タイミングとBの丸め方式を完全に一致させない限り再発する構造的欠陥であり、
  場当たり的なクランプでの延命は明示的に禁止されている(プロンプト§5末尾)。
* **案E(人口のコホート分離)**: 民間・軍人・非労働人口をストックとして分離する
  設計は理論上最も厳密だが、既存の`workforce_ratio`(集計比率としての設計)全体を
  再設計する規模になり、P24-002の最小実装としては過剰。将来Phase24以降の本格
  拡張候補として保留する。
* **出身州割当の代替案(§7候補2〜5)**: 「現在所有する州へ人口比按分」「募兵時点の
  州別動員負担へ比例配分」は、いずれもpopulation減算を按分計算(largest remainder
  等)経由にしてしまい、§9で実証した逆説の再発経路そのものになるため却下。
  「出身州を持たず国家人口だけ別Resourceで持つ」案は、既存の州単位UI
  (州パネルの人口表示)と矛盾する二重の真実源を生むため却下。「新規に出身州・
  人口コホートを永続化する」案は、結果として採用案(出身州フィールド追加)と
  ほぼ同じだが「コホート」という追加の粒度は過剰と判断し、単純な
  `Option<StateId>`のみを採用した。

## 14. 民間労働力の正規式

**変更しない。** `civilian_workforce() = employed_workforce + unemployed_
workforce`のまま、`employed_workforce`/`unemployed_workforce`は引き続き
`population::update::update_state_population_and_employment`が
`calculate_workforce(state.population, state.workforce_ratio).saturating_sub(
mobilized_burden)`から毎月再導出する。採用案(populationのみ直接減算)は
この既存の式・経路を一切変更しない——population自体が下がった状態で次の月次
更新が走れば、`calculate_workforce`が自動的に小さくなった`total_wf`を返す
だけであり、新しい民間労働力の式は不要である。UI・雇用・生産・生活水準・不満度は
引き続き同じ値(`civilian_workforce()`または`total_workforce()`)を参照すればよく、
参照先の変更は発生しない。

## 15. 戦死時の帳簿遷移

```text
戦闘損耗C(実際に測定された値、出身州が現在も自国支配下の場合):
  A: 不変
  B: 不変
  L: -= C(既存どおり)
  K: += C(既存どおり)
  X: 不変(0のまま、既存どおり)
  origin_state.population: -= C  ← 新規
  他の全ての州: 不変
  civilian_workforce(origin_state): 次の正規月次更新で自動的に反映(即時再計算
    しない、P23-008/P23-006-FIX-001と同じ「雇用計算を先行実行しない」原則を踏襲)

戦闘損耗C(出身州が既に他国支配下、または出身州情報がない場合):
  上記のうちorigin_state.populationの行だけ実行しない(スキップ)。
  それ以外は同じ。
```

## 16. 州人口への損失割当方式

**採用: Divisionの募集元州(出身州)へ全損失を割り当てる(§7候補1)。**
`origin_state_id: Option<StateId>`を`Division`へ追加し、完成時に
`RecruitmentQueueItem.target_state`から、New Game初期配置時に
`capital_state_id`から設定する(§12参照)。移動・占領・州所有権変更後も
`origin_state_id`は不変(「どこで生まれた部隊か」という後から変わらない属性として
扱う、`current_state`とは独立)。出身州が他国領になった場合は、その時点以降の
戦死をその州のpopulationへ反映しない(他国の人口を誤って減らさないため、
プロンプト§4の「他国の人的資源を誤って移転しない」不変条件を満たす最も単純な
選択)。出身州のpopulationが損失量より少ない場合は`saturating_sub`で0止まりに
する(populationのunderflowを起こさない)。複数州への端数配分は発生しない
(1つのDivisionの出身州は1つに定まるため、最大剰余法のような端数処理自体が
不要)。既存Save V1から`origin_state_id`は復元できない(旧フィールドが存在しない
ため`None`になる)ため、旧Saveをロードした場合はロード時点で保有していた
Divisionについてのみ「出身州不明」としてpopulation減算をスキップし続ける
(migrationは行わない、§20参照)。

## 17. 人的資源上限の定義

**変更しない。** `A + B <= cap`・`A + L <= cap`のいずれも、新しい必須不変条件
としては導入しない(プロンプト§14が明示的に対象外としている)。`compute_
manpower_cap(population)`は今回の設計でも「現在の所有population(=人口が減れば
自動的に下がる)から算出される、平時の動員余地の目安」という既存の位置づけの
ままとする。populationが戦死で減れば、その州が寄与する国家全体のcapもわずかに
下がるが、これは既存の`recover_manpower`が既に扱っている「capが現在値を下回る
状況」(P23-001でテスト済みの意図的挙動: 既存の余剰を没収しない)と同じ経路で
自然に吸収される。Kをcap消費に含めるかどうかという問い自体が、`A+B<=cap`を
強制しない前提では意味を持たない(強制しないため無関係)。

## 18. 月次回復規則

**変更しない。** `population::growth::recover_manpower`はそのまま。population
減少後にcapが縮んでも、`recover_manpower`は`current >= cap`なら何もしない
という既存分岐が自然に対応する。population増加(既存の月次成長)後にcapが
再び伸びれば、`recover_manpower`が既存のとおり毎月capへ向けてavailableを
回復させる——これは「population増加によって戦争損失から再び動員可能になる」
(プロンプト§8末尾の問い)への回答でもあり、Aの回復を通じて自然に実現される
(Kそのものは減らないが、Aが回復すれば新たな募兵・補充は可能になる、という
Phase23までの意味と矛盾しない)。

## 19. Attritionの会計分類と推奨System順序

Attritionの実装自体は今回対象外(§14)だが、将来接続する際の会計分類を決定する。

* **恒久損失(死亡・事故死)**: 戦闘損耗と同じ`K`へ計上し、同じ`origin_state_id`
  経由でpopulationへも反映する(戦闘損耗と全く同じ会計経路を再利用する。
  Attrition専用の新しい状態や損失区分は作らない)。
* **一時的戦線離脱(負傷・疾病)**: 本設計では対象外のまま(`organization`の
  低下として既存の仕組みで表現するか、Phase24以降の別タスクで検討)。manpower
  そのものは減らさない。
* **脱走・行方不明**: 本設計では対象外。将来実装する場合も「恒久損失」の一種として
  Kへ計上するのが自然(生存兵員としては二度とカウントされないため)。
* **装備損失**: manpower会計と無関係(対象外のまま)。

**推奨System順序**: `募兵進行 → 補充 → Attrition → 移動 → 戦闘`
(プロンプト§9の第一候補)。理由: 補充がその日最初に人員を埋めた後でAttritionが
適用されるため、「その日発生したAttrition損失は同日中に補充されない」という
P23-008の既存原則(「その日の戦闘損失を同日中に補充しない」)と対称になり、
実装・説明が一貫する。Attritionを補充より前に置くと、同日中にAttrition→補充の
順で相殺され「実質Attritionが起きなかったように見える」フレームが生じ、
テストでも観測しづらくなる。同日のAttrition損失とその日の戦闘損失は同じ`K`へ
積み上がるため、区別なく合算されて構わない(Kは「原因を問わない累積回復不能
損失」という位置づけのまま)。対象は非戦闘中の全Divisionとし(戦闘中Divisionは
戦闘自体の損耗で扱う)、自国領・同盟国領・海上ではAttritionを適用しない、敵地
(占領中含む)でのみ適用する、という区別が最も一般的なゲームデザインだが、
具体的な発生条件・料率は根拠がないため確定しない(プロンプト§9末尾の指示どおり)。
補給システム未実装の現段階では、`logistics_ratio`(既存フィールド)を入力条件の
候補にできる。国家単位の予算按分は不要(Attritionは支出を伴わないため、
補充のような予算制約の概念が存在しない)。日次蓄積値をsaveへ持つ必要はない
(その日その場で計算し即座に適用する設計であれば、補充と同じく永続化不要)。

## 20. Save V1維持またはV2移行の判断

**Save V1を維持する。** 新設する`Division.origin_state_id: Option<StateId>`は
`#[serde(default)]`で追加する(既存の`Division.combat_id: Option<BattleId>`・
`destination: Option<StateId>`と全く同じ確立済みパターン、`military/data.rs`で
複数の前例を確認済み)。旧Save由来のDivisionは`None`(出身州不明)として扱われ、
それらのDivisionが以後戦死してもpopulation減算はスキップされるだけで、
ロード自体は失敗しない・破損しない。`K`は引き続き`B-L`から導出可能。州別
population損失は「その時点以降に発生した戦死の分だけ」再構築される
(旧Saveロード前に発生していた戦死はpopulationへ反映されないままになるが、
これは新フィールド導入前のデータに対する不可逆な制約であり、明示的な
migrationは行わない。Loaded Gameで初回だけ値が変わることはない——ロード時に
何も計算し直さない、既存の値をそのまま復元するだけ)。不正な旧Saveの検出は
既存の`save::validate`の枠組みをそのまま使う(`origin_state_id`はOptionなので
新たな検証ルールを追加する必要が薄い)。

## 21. 必須不変条件とテスト計画

| # | 不変条件 | 検証方法(P24-002で追加予定) |
|---|---|---|
| 1 | 戦死後に民間労働力が増えない | Domain単体テスト: `civilian_workforce`が実測casualty後の月次更新で減少または不変であることを、§9の反例と同じ数値(population=1,000,000、B=100,000、C=1)で固定 |
| 2 | 戦死後にavailableが増えない | Domain単体テスト: `A`はcasualty処理関数の入出力対象外であることをアサート(触れていないことの確認) |
| 3 | 戦死者を復員できない | 既存P23-007の`demobilize_division`は生存Divisionのみ対象(既存不変条件、変更なし)。回帰テストとして既存`p23_007_division_demobilization_e2e_test`の継続PASSで確認 |
| 4 | 補充によってKが減らない | 既存P23-008の回帰テスト(`reinforcement_never_touches_...`系)がそのまま該当、変更なし |
| 5 | 生存者復員でKが変わらない | 既存P23-007の`successful_recruitment_leaves_permanent_losses_unchanged`型のテストを継続 |
| 6 | populationがunderflowしない | Domain単体テスト: `casualty > origin_state.population`のケースで`saturating_sub`により0止まりになることを固定 |
| 7 | 通常状態で`X==0` | 既存P23-006〜010の全テストが継続してこれを固定(変更なし) |
| 8 | 他国の人口・人的資源へ波及しない | Domain単体テスト: 出身州が他国支配下のケースでpopulationが変化しないことを固定(§16のスキップ経路) |
| 9 | 州別損失合計と国家損失合計が一致する | 統合テスト: 複数Divisionが異なる出身州を持つケースで、Σ(州population減少量)== Σ(実測casualty)を固定 |
| 10 | 端数配分が決定的 | 該当なし(§16のとおり端数配分自体が発生しない設計、按分を経由しないことをコメントで明記) |
| 11 | save/load前後で結果が一致する | 統合テスト: casualty発生後にsave→load、`origin_state_id`・population・B/L/K/Xが一致することを固定(P23-010の`a_save_and_reload_partway_through_matches_an_uninterrupted_run`と同じ手法を再利用) |
| 12 | 同じ初期状態と入力で結果が一致する | P23-010の決定性再実行テストと同じ手法(正規化スナップショット比較)を再利用 |
| 13 | population増加後の回復規則が明確 | §18のとおり既存`recover_manpower`をそのまま使うため、既存P23-001テストの継続PASSで十分(新規回復式を導入しないため新規テスト不要) |
| 14 | 国家消滅・領有変更でも損失が増殖・消失しない | 統合テスト: 出身州の所有権が戦闘後に変わるケースで、それ以降のpopulation減算がスキップされ続けることを固定 |
| 15 | 動員0ではPhase23の人口・雇用・経済挙動を維持する | 既存P23-001〜004の全回帰テストの継続PASSで確認(casualty処理自体が発火しないため、動員0のケースはそもそも新コードパスに触れない) |

## 22. P24-002の最小実装範囲

**含める**:

* `Division.origin_state_id: Option<StateId>`の追加(`#[serde(default)]`)。
* `RecruitmentQueueItem`完成時・New Game初期配置時に`origin_state_id`を設定する
  最小限のコード変更。
* `process_daily_battles`(通常の日次戦闘損耗)・`resolve_participant_outcome`
  (撤退時追加損耗・包囲撃破)の両方から呼び出す、新しい純粋関数
  `apply_casualty_to_origin_state(state_registry, origin_state_id, owner,
  casualty)`(出身州の現在の実効支配国が`owner`と一致する場合のみpopulationを
  減算、それ以外はスキップ)。
* §21の不変条件1・2・6・8・9・11・12・14を固定する新規テスト。
* 既存P23-001〜010の全回帰テストが継続してPASSすることの確認。
* UI(州パネルの人口表示)は既存の`state.population`をそのまま表示している
  ため、コード変更なしで新しい値へ自動追従することの確認(表示先の変更は不要)。

**含めない**(プロンプト§14と同一、再確認):

* Attritionの実処理。
* 装備・物資・負傷者Resource。
* AIの募兵・補充・復員判断変更。
* 徴兵法、`war_exhaustion`。
* 戦闘式変更(`combat_calc`の式自体には触れない、その出力値を消費するだけ)。
* UI再設計・新規ローカライズキー(既存の人口表示がそのまま新しい値を映すため
  不要)。
* `A+B<=cap`等の新しい不変条件の強制。
* Save V2(§20のとおりV1のまま実装可能)。

## 23. 対象外を先行実装していないこと

本タスクでは`src/`配下のRustコードを一切変更していない
(`git status --short`は`saves/savegame_v1.ron`のみ、§26参照)。populationの
減算処理、B・L・Kの意味変更、人的資源上限の強制、Attrition System、負傷者等の
新規Resource、装備・補給消費、AI判断変更、UI変更、ローカライズ変更、セーブV2、
新規セーブフィールドのいずれも実装していない。数値検証に用いた計算はすべて
既存関数の仕様を手計算で追跡したものであり、一時スクリプトやテストコードは
作成・コミットしていない。

## 24. 発見事項と重要度

**Critical: 0件、High: 0件、Medium: 1件、Low: 1件**

### Medium: `StateData::total_workforce()`と`population::update::calculate_
workforce()`の丸め方式の不一致

両者は概念的に同じ「population × workforce_ratio」を計算するが、前者は切り捨て
(`as u64`)、後者は四捨五入(`.round()`)。現状は前者がP23-004の`E/P`比率
(`economic_workforce_ratio`)の分母として、後者が民間労働力(`civilian_
workforce`)の算出元として、それぞれ独立に使われているため実害は今のところ
顕在化していないが、P24-002でpopulation側に新しい書き込み経路(casualty減算)が
入ることで、この2つの式が異なる場面で異なる丸め結果を返す既存の不整合が、
将来より目につきやすくなる可能性がある。P24-002のスコープでは修正しない
(既存の2つの経路のどちらも変更しないため)が、Phase24以降でどちらかへ統一する
かどうかの検討候補として記録する。

### Low: `Division`に出身州フィールドが存在しない(既存の欠落、本監査で確認)

§8・§16のとおり。P24-002で`origin_state_id`を新設することで解消される
(対応する実装候補は§22に記載済み)。

## 25. 実行したコマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `cargo test --lib population::` | PASS、46 passed |
| `cargo test --lib military::` | PASS、177 passed |
| `cargo test --lib economy::` | PASS、6 passed |
| `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test` | PASS、8 passed |
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test` | PASS、7 passed |
| `cargo test --test p23_006_manpower_ledger_e2e_test` | PASS、22 passed |
| `cargo test --test p23_007_division_demobilization_e2e_test` | PASS、24 passed |
| `cargo test --test p23_008_daily_reinforcement_e2e_test` | PASS、21 passed |
| `cargo test --test p23_009_ai_recruitment_e2e_test` | PASS、22 passed |
| `cargo test --test p23_010_phase23_manpower_lifecycle_audit_test` | PASS、9 passed |
| `cargo test --all-targets` | PASS、**1585 passed**、**0 failed**、1 ignored(基準と完全一致) |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、**警告0** |

基準状態(1585 passed / 0 failed / 1 ignored、clippy警告0)から一切変化していない
ことを確認した。既知の非決定的PNG9枚は今回のテスト実行でも再生成されたため、
個別ファイル単位で`git checkout --`により復元した(ディレクトリ単位の復元は
行っていない)。`tests/land_war_combat_peace_test.rs`は変更・整形していない。

## 26. 監査対象コードの`git status --short`

```
 M saves/savegame_v1.ron
```

`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、本タスクでは一切
参照・変更・ステージ・コミットしていない。本番コード(`src/`配下)・既存テスト
コードへの変更は一切なく、本報告書のみが新規ファイルである。
