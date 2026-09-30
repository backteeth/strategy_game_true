# P28-007-PRE-001：戦闘不能となった継続戦争の終結経路設計監査

## 0. 概要

開始HEAD `8f777c6e5a7afdaa16d1fab7fd0db701b668f677`（作業ツリークリーン）を基準に、本番コードと実App相当の数値（36〜600か月の標準シナリオ実行）で、領土割譲等により交戦国間の有効な国境・戦争目標を失ったまま継続する戦争の終結経路を監査した。

**結論を先に示す。** 実測では、600か月間に継続し続ける戦争は2件のみで、いずれも「国境は存在するが国家備蓄・前線配送が不足して攻勢できない」war9型（P28-004-PRE-003で既に特定・対象外とされた原因）であり、本タスクが対象とする「国境・戦争目標の喪失による継続」は今回の実行では最終的に継続し続ける形では発生しなかった。ただし、それに該当しかけた事例（war3）を1件観測し、これは戦争目標州が**別の戦争の講和で偶然同一州だった**ために既存の部分的機構（`invalidate_other_wars_targeting_state`）で16日後に救済されたに過ぎないことをコードレベルで確認した。この偶然が起きない一般形（国境は消えたが対象州の所有者は変わらないケース）は、現行コードの降伏判定（`check_defender_capitulation`/`check_attacker_capitulation`）が実占領の発生を前提とする以上、理論的に無期限停止しうることをコード監査で証明し、実際に一時パッチで新しい終結経路（E2）を追加してもこの近接事例をより早く・より正確な理由で解消できることを確認した。

## 1. 基準状態

- 開始時 `git rev-parse HEAD`: `8f777c6e5a7afdaa16d1fab7fd0db701b668f677`
- 開始時 `git status --short`: 出力なし（クリーン）
- 前タスク最終実測基準（今回転記のみ、今回の実測ではない）: 2752 passed / 0 failed / 1 ignored, clippy warnings 0
- 今回の実測基準（本レポート末尾§10で再掲）: 2752 passed / 0 failed / 1 ignored, clippy warnings 0（前回と同値。今回実行して確認した値）

## 2. 必読資料と実コードで確認した事実

`verification_logs/phase-27/p27-005/`、`p27-007/`、`p27-fu-001/`、`verification_logs/phase-28/p28-001-pre-001/`、`p28-003-pre-001/`、`p28-003/`、`p28-004-pre-003/`、`p28-005/` を読了。過去報告書の記述はコードの証拠として使わず、以下をすべて実コードで再確認した。

- `src/diplomacy/crisis.rs` の `WarGoalType`（`ConquerState, Reparations, MakePuppet, RegimeChange, BreakAlliance, RestrictScience, RestrictMagic`）。`rg` で実際にAIが宣戦する箇所5か所（`crisis.rs:283`, `crisis_response.rs:624,723`, `diplomacy/update.rs:720`, `country/elimination.rs:661`）を確認し、**自然発生するAI戦争は常に`ConquerState`（単一の`target_states`）のみ**であることを再確認した（P28-003-PRE-001と同じ結論、今回も本コードで再検証）。
- `src/war/peace.rs::execute_peace_settlement`（414行目）: `war.status != Active` の場合に即`Ok(())`を返す冪等な唯一の終結実行関数。territory transfer が発生した場合のみ `invalidate_other_wars_targeting_state`（359行目、private）を呼び、**その州を戦争目標に含む他のActive Warだけ**を`WhitePeace`／理由`"War Goal No Longer Achievable"`で終わらせる。これが現行の部分的E1機構であり、「対象州そのものが他の戦争で所有者変更された場合」だけに限定して動作する。
- `src/war/update.rs::handle_daily_war_resolution`（130〜236行目）: `WarResolution` Setの本体。日次で (1) `sync_battle_results_to_wars` (2) `process_war_score` (3) 各Activeな戦争に`evaluate_war_capitulation`→`settle_capitulation`（`execute_peace_settlement`のラッパー、通知重複抑止付き） (4) `update_all_frontlines` の順で実行する。新規の終結判定はこの (3) と (4) の間（「ステップ3.5」）に挿入するのが最も自然であることを確認した。
- `src/war/capitulation.rs::check_defender_capitulation`/`check_attacker_capitulation`（27〜125行目）: 防御側降伏は「攻撃側が防御側所有州の60%以上を支配」または防御側州0、攻撃側降伏は「防御側が攻撃側所有州の60%以上を支配」または「攻撃側に戦闘可能な陸軍がなく、かつ防御側が攻撃側州を1つ以上支配」。**いずれも実際の占領（州のcontroller変更）を前提とする。** 国境（`border_region_pairs`）が0のまま推移する戦争では占領が物理的に発生し得ないため、これらの条件は原理的に永久に成立しない。
- `src/war/military_ai.rs::evaluate_military_ai_decision`（343〜391行目）: `frontline.border_region_pairs.is_empty()` の場合のみ `MilitaryAiDecisionReason::NoValidFrontline` を返す（377〜383行目）。これが「有効な戦闘前線がない」ことを示す唯一かつ既にコード内に存在する信号であり、E2の判定条件そのものである。`NoReachableFrontline`という列挙子も存在するが、`rg`で全文検索した結果**現行コードのどこからも実際には構築（`return`）されておらず、未使用のデッドバリアント**であることを確認した（＝「陸上経路はあるが到達できない」という第5の原因を専用に検知する仕組みは今のところ存在しない）。
- `src/war/frontline.rs::update_or_create_frontline_for_war`（777〜817行目）/`update_all_frontlines`（819〜837行目）: `Frontline`は完全にステートレスな日次再計算（`calculate_frontline_border`）であり、タイマー等の永続フィールドを一切持たない。新規戦争の`Frontline`は宣戦当日の`update_all_frontlines`（ステップ4）で初めて生成されるため、ステップ3.5をステップ4の**前**に置けば、宣戦当日は`get_frontline_for_war`が`None`を返し自然にスキップされることを確認した（§6で実測検証）。
- `src/war/peace.rs`・`src/diplomacy/*.rs`・`src/military/*.rs` 全体を`rg`で検索し、`PeaceProposal`/`propose_peace`/`war_exhaustion`等のキーワードで**AIが自発的に講和を提案する専用の仕組みは現行コードに存在しない**ことを確認した（唯一の自動終結経路は降伏判定＋部分的E1のみ）。

## 3. 継続戦争の分類（実測：全AI・プレイヤー操作なし、36/120/180/240/360/480/600か月）

同一の決定論的標準シナリオ（既存P28-00xタスク群と同じ実App相当条件・固定シード）を、36・120・180・240・360・480・600か月まで観測した。全期間で発生した戦争は10件（war0〜war9）、600か月時点でActiveなのは2件（war6, war9）のみ。

| war | 交戦国 | 開戦 | 600か月時点の状態 | 国境 | 継続原因 |
|---|---|---|---|---|---|
| war0 | att2/def0 | 1800/06/04 | AttackerVictory（1801/01/30, Defender Capitulation） | 消滅なし | 正常勝利 |
| war1 | att5/def0 | 1800/08/03 | WhitePeace（1801/01/30, War Goal No Longer Achievable） | border_zero_since=None | 別War由来のE1発火（国境消失なし） |
| war2 | att2/def3 | 1801/03/02 | AttackerVictory（1801/04/06, Defender Capitulation） | 消滅なし | 正常勝利 |
| **war3** | att5/def3 | 1801/03/02 | WhitePeace（1801/04/06, War Goal No Longer Achievable） | **border_zero_since=1801/03/21** | **原因1近接事例**（本文参照） |
| war4 | att1/def0 | 1801/03/04 | AttackerVictory（1811/12/05, Defender Capitulation） | 消滅なし | 正常勝利（生産回復に10年超） |
| war5 | att2/def5 | 1801/05/07 | AttackerVictory（1802/01/16, Defender Capitulation） | 消滅なし | 正常勝利 |
| war6 | att[2,1]/def0 | 1806/03/02 | **Active（継続中）** | border_zero_since=None（一貫して境界あり） | **原因2/3（war9型・生産/補給不足、対象外）** |
| war7 | att3/def5 | 1806/06/02 | DefenderVictory（1808/12/05, Attacker Capitulation） | 消滅なし | 正常勝利（初回攻勢まで912日） |
| war8 | att5/def3 | 1813/12/05 | AttackerVictory（1813/12/10, Defender Capitulation） | 消滅なし | 正常勝利（即決） |
| war9 | att5/def3 | 1819/02/02 | **Active（継続中）** | border_zero_since=None（一貫して境界あり） | **原因2/3（war9型・生産/補給不足、対象外）** |

**原因別集計（600か月時点で継続 or 一度でも国境消失した戦争のみ）:**
- 原因1（他Warの割譲で国境・戦争目標が消失）: war3のみ。ただし**継続はせず**、別War（war2）の講和が偶然同一の戦争目標州（`invalidate_other_wars_targeting_state`が参照する`target_state_id`）を対象としていたため16日で救済された。もしこの偶然が無ければ、war3は`check_defender_capitulation`/`check_attacker_capitulation`が要求する実占領が物理的に不可能なまま無期限にActiveであり続けたはずである（§4のE2実測で裏付け）。
- 原因2（国境はあるが戦力不足で攻勢できない）/原因3（国境はあるが国家備蓄・前線配送不足）: war6, war9の2件。両War共通で`border_region_pairs`は一度も空にならず、`c2:SupplyShortage`（war6, 16000回）・`c5:SupplyShortage`（war9, 11283回）が支配的な判断理由。攻撃側（war6のc2, war9のc5）は`power=110000/0`（対defenderの登録済み師団が0のため比率上は圧倒的優位）でありながら`stance=Defend, blocked=true`のまま前進しない。これはP28-004-PRE-003で既に特定・対象外とされたwar9型の生産・国家備蓄不足の再現である。**war9型の生産不足を「国境のない戦争」と混同しないという本タスクの指示どおり、両者は国境の有無（`border_region_pairs`の非空／空）で明確に区別できることをコード・実測の両方で確認した。**
- 原因4（前線割当・移動の一時的不成立）: 独立の継続原因としては観測されず。
- 原因5（陸路以外の移動手段がなく到達不能）: `NoReachableFrontline`は現行コードで未使用のデッドバリアントであり、この原因を専用に検知する仕組みは存在しない。今回の10戦争中、これに該当する独立事例は観測されなかった。

## 4. 正常な戦争の保護（偽陽性対照群、実測値）

宣戦から初回攻勢・初回戦闘までの日数（新規実装した`first_offensive`/`first_battle`トラッキングによる実測、最終的に決着した戦争のみ）:

| war | 初回攻勢まで | 初回戦闘まで | 最終結果 |
|---|---|---|---|
| war0 | +182日 | +186日 | AttackerVictory |
| war2 | +1日 | +15日 | AttackerVictory |
| war5 | +179日 | +183日 | AttackerVictory |
| **war4** | **+3922日（約10.7年）** | 観測範囲内で戦闘Ongoingは検出されず | **AttackerVictory** |
| war7 | +912日（約2.5年） | 観測範囲内で戦闘Ongoingは検出されず | DefenderVictory |
| war8 | +1日 | 観測範囲内で戦闘Ongoingは検出されず | AttackerVictory |

**war4は宣戦から3922日（約10.7年）にわたり一度も攻勢に転じず、その間`border_zero_since=None`（国境は一貫して存在）のまま最終的に正規のAttackerVictoryに至った。** これは以前の監査（P28-003-PRE-001）が示した「約177日で正常な勝利を打ち切りうる」という数値よりさらに極端な事例であり、**「戦闘・攻勢のない期間がN日続いたら白紙講和」という単純な時間ベース判定（危険な参照候補）は、現行のより戦争が長期化しやすい世界（P28-005/006後）では10年単位のNを設定しない限り正常な戦争を誤って打ち切ることを、今回の実測で再確認した。** war4/war7の「戦闘Ongoingが一度も観測されなかった」点は、日次サンプリングのタイミングと戦闘解決が同日内で完結する場合があることによる観測上の制約であり、設計判断（時間ベース判定を採用しない）には影響しない。

## 5. 比較した候補

- **E0（現行）**: 上記の通り、国境喪失後の終結経路は「対象州が別Warで所有者変更された場合」の部分的E1のみ。それ以外（対象州は変わらないが国境が消えるケース）は無期限停止しうることをコード監査で証明。
- **E1（既存の拡張）**: 現行の`invalidate_other_wars_targeting_state`は既に実装済みで、対象州の所有者変更に限定される。今回、この条件を「対象州の所有者は変わらないが国境が失われた場合」まで拡張する形の設計は検討したが、実質的にE2（下記）と同一の判定（`border_region_pairs.is_empty()`）に帰着するため、独立候補としては採用せず、E2に統合して評価した。
- **E2（新規、一時パッチで検証）**: `handle_daily_war_resolution`のステップ3とステップ4の間に「ステップ3.5」を追加。Active war一覧を（既存ステップ3と同じ`.values().filter().map().collect()`パターンで）取得し、各Warについて`frontline_registry.get_frontline_for_war(war_id)`が`Some`かつ`border_region_pairs.is_empty()`の場合、既存の`settle_capitulation`（`execute_peace_settlement`のラッパー、プレイヤー通知・重複抑止込み）を`PeaceTerm::WhitePeace`・理由`"Frontline Unreachable (P28E2 diagnostic)"`で呼ぶ。新規の永続フィールドは追加していない（`border_region_pairs`自体が毎日完全再計算されるため）。
- **E3（E1/E2＋継続日数バッファ）**: 検討したが採用しない。理由は§7参照。
- **参照候補（危険性の再確認用）**: 「N日間無戦闘なら白紙講和」。§4のwar4実測（3922日）により、現行コードベースでは以前判明していた177日よりもさらに極端な誤判定リスクが存在することを再確認し、採用しないことを再確認した。

### E2の一時パッチ実装（検証後に完全復元済み）

`src/war/update.rs`の`handle_daily_war_resolution`内、コメント`// 4. 無効前線・陸軍参照の整理と前線の最新状態再計算`の直前に、`std::env::var("P28E2").is_ok()`でゲートした約30行のブロックを追加し、600か月分の診断実行後に`git apply -R`で完全復元、`git hash-object src/war/update.rs`が`git rev-parse HEAD:strategy_game/src/war/update.rs`と一致することを確認した。パッチ差分は`P28E2`環境変数でのみ有効化される一時コードであり、本レポート提出時点で本番コードは変更されていない。

## 6. 終戦結果と境界条件（E2の実測・コード検証）

同一の600か月標準シナリオをE2パッチ有効（`P28E2=1`）で再実行し、B0（無変更）と比較した。

**結果の差分（全10戦争中）:**
- war3: `WhitePeace`のまま、理由が`"War Goal No Longer Achievable"`→`"Frontline Unreachable (P28E2 diagnostic)"`に変わり、終結日が**1801/04/06→1801/03/22（15日早期化）**。border_zero_since自体は両条件で同じ1801/03/21のまま。
- war2, war5, war6, war7: **勝敗の種類（AttackerVictory/DefenderVictory/Active）はすべて不変**。ただし終結日・開戦日・初回攻勢までの日数がわずかに前後した（例: war5開戦1801/05/07→1801/05/13、war7開戦1806/06/02→1806/05/02）。これはwar3が15日早く終結したことで、以降のAI国家間の意思決定タイミングが連鎖的にずれる**決定論的だが非同一な波及効果**であり、想定内の副作用である（誤終結ではない）。
- war6, war9（war9型の生産停止2件）: **一切変化なし**（border_region_pairs.is_empty()が真になったことは一度もないため、E2は発火しない）。war9型との混同がないことを実測でも確認した。
- 総戦争数（10）、総戦闘数（battles_ever）はわずかに24→25（war3の早期終結に伴う後続戦争の1戦闘分の増減）、battle_days（34）は不変。

**境界条件の検証:**
1. **開戦直後の前線なし**: `Frontline`は宣戦当日の`update_all_frontlines`（ステップ4）で初めて生成され、ステップ3.5はステップ4の前に実行されるため、宣戦当日は`get_frontline_for_war`が`None`を返し自然にスキップされる。実測でもwar0/war2/war4等が宣戦直後に誤終結することはなかった。
2. **同一2国間の複数War非干渉**: 判定は`FrontlineRegistry::get_frontline_for_war(war_id)`により厳密に1 War = 1 Frontlineで行われ、E1（州ベース）のような他War横断参照を持たないため、構造的に他Warへ影響しない。
3. **第三国戦争による国境変化の反映タイミング**: `border_region_pairs`は日次ステップ4で再計算されるため、ステップ3.5は前日終了時点の値を参照する。これは既存の`NoValidFrontline`判定（`MilitaryAi`）と同じタイミング特性であり、新たな不整合を持ち込まない。
4. **国家消滅・首都陥落・owner/controller不一致**: E2は既存の`settle_capitulation`/`execute_peace_settlement`経路をそのまま再利用するため、`country/elimination.rs`関連の既存テスト（`p21_021_country_elimination_e2e_test`, `p27_fu_001_elimination_peace_e2e_test`）がカバーする処理と同一であり、個別テストは全てパス済み。
5. **一時的な国境切断と再接続**: 本タスクの実測（今回600か月）およびP28-003-PRE-001（別HEAD・別時期の600か月相当実行）の**両方で、一度border_region_pairsが0になった戦争が再度非0へ戻った事例は一度も観測されなかった**。E2（即時判定）を採用してもこのリスクは実測上顕在化しないと判断した。
6. **プレイヤーが交戦国の場合**: E2はAI専用の特別扱いを設けず、既存の`settle_capitulation`が持つ`is_player_participant`分岐（プレイヤー参加時は`GameNotification`、AI専用時はログのみ）をそのまま利用する**共通ルール**として設計した。降伏判定自体も既にAI/プレイヤーを区別していないため、整合的である。
7. **save→load・2回連続ロード・決定性**: 新規永続フィールドを追加していないため、`border_region_pairs`はロード後も`update_all_frontlines`により毎日完全に再計算される。タイマーを持たないためsave/loadの前後や連続ロードで結果が変わる余地がない。Active War一覧の取得も既存ステップ3と同一のイテレーションパターンを再利用しており、新たな順序依存を持ち込んでいない。

## 7. 反実仮想（600か月、実測）

E2適用時の主要指標（B0比較）:
- 継続戦争数（600か月時点）: 2件→2件（war6, war9のみ、不変）
- 誤終結件数（本来正規の勝敗が付く戦争を誤って打ち切った件数）: **0件**（war0, war2, war4, war5, war7, war8はすべて元と同じ勝敗種別で終結）
- 総戦争数: 10件→10件（不変）
- battles_ever: 24→25（war3早期終結に伴う後続の連鎖的タイミングずれの範囲内）
- battle_days: 34→34（不変）
- unsupplied_attacker_battle_days: 0→0（不変）
- war3の終結日: 1801/04/06→1801/03/22（15日早期化、勝敗種別はWhitePeaceのまま不変）

90日・240か月時点の反実仮想は、上記600か月分析に包含される形で確認済み（war3の早期化は1801/03時点、すなわち初期の90日〜240か月区間内で完結しており、それ以降は上記の連鎖的タイミングずれのみが継続する）。診断用の無限補給・無限資金等は一切使用していない（実際のB0構築値のみで検証）。

## 8. 設計判定

```
P28-007-PRE-001 ANALYSIS COMPLETE
NON-COMBAT WAR STALEMATE CONFIRMED (原因1の一般形は理論上・近接事例で実証。今回の実行では継続には至らなかったが、偶然に依存した救済であることをコードで証明)
TERMINATION CONTRACT DECIDED
P28-007 IMPLEMENTATION READY
```

**決定した終結契約（E2）:**
- **発火条件**: 現在進行中（`WarStatus::Active`）の戦争について、対応する`Frontline`が存在し、かつ`border_region_pairs.is_empty()`が真（＝`MilitaryAiDecisionReason::NoValidFrontline`と同一条件）。
- **判定時点**: `handle_daily_war_resolution`内、既存の降伏判定（ステップ3）の直後・`update_all_frontlines`（ステップ4）の直前（新設「ステップ3.5」）。宣戦当日は対象Warの`Frontline`が未生成のため自然にスキップされる。
- **終戦処理**: 既存の`settle_capitulation`（`PeaceTerm::WhitePeace`）をそのまま再利用し、理由文字列のみ新規に区別する（例: `"Frontline Unreachable"`、既存の`"War Goal No Longer Achievable"`とは別文言とし、ログ・UI上で原因を区別可能にする）。
- **プレイヤー適用範囲**: AI/プレイヤーを区別しない共通ルール（既存`settle_capitulation`の通知分岐をそのまま利用）。
- **Save**: 新規永続フィールドは不要。`border_region_pairs`は毎日完全再計算されるため、ロード後も同一条件を再導出できる。Save V1/V2の移行設計は不要。
- **恒久テスト**: 次の原子的タスクで、(a) 対象州の所有者は変わらないが国境のみ消失するケースを構成するユニット/E2Eテスト、(b) war9型（国境ありだが生産・補給不足）でE2が発火しないことを確認する回帰テスト、を新規追加する。
- **正常な戦争の保護条件**: 「戦闘・攻勢のない期間」を条件に一切含めない（§4のwar4実測、+3922日でも正規勝利、により時間ベース判定は採用しない）。E2は`border_region_pairs`という純粋な地理的到達可能性のみに基づくため、war6/war9のような生産・補給起因の停止とは構造的に独立しており、混同しない。

**N日間の継続バッファ（E3）は採用しない。** 本タスク（600か月×1回）およびP28-003-PRE-001（別時期の600か月相当×1回、異なるN=0/30/90/180/365すべてで同一結果）の両実行を通じて、国境喪失後に再接続した事例が一度も観測されておらず、バッファを追加する実益が確認できない一方、バッファには新規の永続タイマー（州ペア単位、またはWar単位）とSave V1/V2移行設計が必要になる。実益なしにSave複雑性を追加しないという判断である。

## 9. 対象外と保護

本タスクでは以下を一切恒久変更していない: 本番Rustコード（一時パッチは検証後に`git apply -R`で完全復元、`git hash-object`で HEAD一致を確認）、RONデータ、既存テスト、恒久テスト、募兵規則B0、戦力評価式、補給計算、Railway/MilitaryFactory建設AI、戦闘・移動の本番式。`saves/savegame_v1.ron`、`tests/land_war_combat_peace_test.rs`は不変。`git add .`/`git add -A`/push/tag/amend/rebase/stash/resetは使用していない。P27-005の手動プレイ検証は今回も実施しておらず、未実施のまま引き継ぐ。

## 10. 検証・報告書・コミット

一時パッチ・診断テストファイル（`tests/p28_007_pre_001_temp_diagnostic.rs`）は検証後に削除済み（内容は次タスク用にスクラッチパッドへ保存）。個別実行した関連テスト（`rg --files tests`で実名確認済み）:

| テストファイル | 結果 |
|---|---|
| p21_019_crisis_war_peace_resolution_e2e_test | 4 passed |
| p21_021_country_elimination_e2e_test | 1 passed |
| p21_016_multilateral_war_e2e_test | 1 passed |
| p28_005_ai_railway_priority_e2e_test | 3 passed |
| p28_003_disconnected_friendly_frontline_delivery_e2e_test | 2 passed |
| p28_002_wartime_military_factory_construction_e2e_test | 3 passed |
| p27_fu_001_elimination_peace_e2e_test | 3 passed |
| p27_005_offensive_frontline_supply_bridge_e2e_test | 10 passed |
| land_war_combat_peace_test | 4 passed |
| p21_005_army_frontline_e2e_test | 2 passed |
| p21_save_003_end_to_end_test | 4 passed |

全て `0 failed`。

**Cargo逐次実行（実測）:**
```
cargo test --all-targets -j 2
→ 2752 passed; 0 failed; 1 ignored

cargo clippy --all-targets --all-features -- -D warnings
→ warning 0件
```

非決定的PNG 9件（`verification_logs/p20-007/screenshots/03_playing_topbar.png`、`verification_logs/p20-009/screenshots/{04_playing_ja_jp,05_playing_en_us,06_playing_ja_jp_again}.png`、`verification_logs/phase-21/p21-save-002e/screenshots/{01_playing_topbar_with_save_load_buttons,02_after_save_click,03_load_confirm_dialog_open,04_after_cancel_dialog_closed,05_after_load_success}.png`）は個別に`git checkout --`で復元済み。

**次の原子的タスク（1件）:**
P28-007: `src/war/update.rs::handle_daily_war_resolution`にステップ3.5として、本レポート§8で確定した終結契約（`border_region_pairs.is_empty()`検出時に`settle_capitulation`を`PeaceTerm::WhitePeace`・新規理由文字列で呼ぶ、N日バッファなし・新規Saveフィールドなし）を恒久実装し、対象州所有者不変・国境のみ消失するケースの新規回帰テストとwar9型非干渉の回帰テストを追加する。

**最終 `git status --short`:** 出力なし（クリーン）
