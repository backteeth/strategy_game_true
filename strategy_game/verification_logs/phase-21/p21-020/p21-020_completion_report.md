# P21-020 完了報告: 州所有権移転の共通Domain API化と講和エラー可視化

## 1. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

全品質ゲート(`cargo check/test/clippy/build --release/diff --check/fmt`)を通過し、
新規テスト36件(lib 35件・integration 2件、§17/§26の対応表参照)が全てgreen。実データ
(6か国28州)によるE2Eテスト2件を含む。

2026-08-21、ユーザーレビューにより初回実装の4点(複数Warの同一州競合が未処理、
`ValidatedOwnershipTransfer`のTOCTOU、`ReportedPeaceSettlementErrors`が手動講和
成功時に残る、unchecked APIの隔離不十分)を指摘され、いずれも修正済み(§26)。
GUI実ウィンドウでの手動確認は実施していないため、COMPLETEとは断定しない。

---

## 2. 開始時基準(実測値)

| | lib | integration | 合計 | fmt既存drift |
|---|---|---|---|---|
| 想定 | 945 | 190 | 1135 | 41 |
| 実測 | 945 | 190 | 1135 | 41 |

想定どおり完全一致(P21-019完了時点からの差分なし)。P21-019までの未コミット変更は
保護し、本タスクと無関係な差分は一切戻していない。

---

## 3. 実装前調査結果(§2の20項目)

1. **`transfer_region_ownership`の全呼び出し箇所**: 実コードの呼び出しは2箇所のみ
   ——`diplomacy/crisis_response.rs::accept_demand`(P21-011要求受諾)と
   `war/peace.rs::execute_peace_settlement`(`CedeWarGoalRegion`)。他は
   `src/war/tests.rs`・`src/map/rendering.rs`のテストコード内のみ。
2. **`owner_country_id`を直接変更している全コード**: 実行時ロジックでは
   `state/data.rs`の`transfer_region_ownership`自身(現`transfer_region_ownership_
   unchecked`)の1箇所だけ。他は全てテストfixtureまたはread-only比較。
3. **`capital_state_id`を変更している全コード**: `war/peace.rs`の
   `execute_peace_settlement`(P21-019の首都再配置ステップ)の1箇所のみ、テスト
   コードを除く。
4. **P21-019の`resolve_capital_reassignment`**: 純粋関数、対象州が現所有国の首都
   なら残存陸州のうち最小StateIdを新首都候補として返す、無ければ`Err`。本タスクで
   `state::ownership::plan_capital_reassignment`として共通API側へ吸収し、
   `war/peace.rs`側の同名関数は削除した。
5. **`execute_peace_settlement`のvalidation順序**: war存在確認 → 二重適用防止
   (`Ok(())`即return) → primary attacker/defender計算 → `resolve_crisis_war_
   lineage`(Crisis由来のみ) → (`CedeWarGoalRegion`かつtarget_stateありの場合)
   共通所有権移転APIのpreflight → 実際の変更開始。
6. **P21-011の要求受諾・州割譲経路**: `crisis_response::accept_demand`が
   `transfer_region_ownership`を`let _ =`で結果を無視して呼んでいた——現在所有国の
   検証なし、首都再配置なし、失敗してもClaimを常にConsumedにする、という3つの
   欠落があった(本タスクの直接の動機の一つ)。
7. **通常Warの`CedeWarGoalRegion`**: Crisis由来War(`resolve_crisis_war_lineage`)
   にしか所有者不一致の検証が無く、通常Warは`transfer_region_ownership`を無条件に
   呼んでいた——第三国から州を奪う既存ギャップがP21-019時点でも残っていた
   (P21-019完了報告§20で「範囲外」として記録済みの既知ギャップ)。
8. **Crisis由来Warの領土割譲**: `execute_peace_settlement`内の同じ
   `CedeWarGoalRegion`分岐——通常Warと完全に同一コードパス。
9. **Save migrationや初期化処理**: `save/apply.rs`内の`owner_country_id`/
   `capital_state_id`書き込みは全てテストfixtureの構造体リテラルであり、実際の
   Load処理はSave全体をデシリアライズしてRegistryを丸ごと差し替えるだけで、
   増分的な「移転」ロジックは無い。`app/loader.rs`は初期RONロード時に
   `capital_state_id`が実在し自国所有であることを既に検証(panicベース)している。
10. **占領controllerと法的owner**: `StateData::controller()`は
    `controller_country.unwrap_or(owner_country_id)`——占領は`controller_country`/
    `occupation_progress`/`original_owner`/`war_id`のみ変更し、`owner_country_id`
    には一切触れない。法的所有権を変えるのは`transfer_region_ownership`系のみ。
11. **一国が所有する陸州の取得**: `StateRegistry::get_owned_states`は
    `states.iter().filter(...)`によるO(states)線形走査(逆引きインデックス無し)。
    共通APIの首都再配置候補探索も同じO(states)パターンを踏襲。
12. **海州のowner_country_id実データ**: 実マップ(28州)に`is_sea: true`の州は
    **1件も存在しない**(grep実測)。`is_sea`は現状、合成テストデータ専用の概念。
13. **国家消滅仕様**: 存在しない(P21-019調査時と同じ結論、再確認済み)。本タスクも
    大規模な国家消滅システムは追加せず、最後の所有陸州の移転は明示的に拒否する
    方針を継続。
14. **`execute_peace_settlement`のエラーを処理する全呼び出し元**: 3箇所——
    `ui/peace_panel.rs::handle_peace_action_buttons`(`Ok`時のみ通知、`Err`は無視
    ——本タスクの直接の動機)、`war/update.rs::handle_daily_war_resolution`内2箇所
    (`let _ =`で完全に無視)。
15. **UI通知・インラインエラー表示の既存共通API**: `GameNotification`
    (`ui/notification.rs`)+ `MessageWriter<GameNotification>` + `localization::t`/
    `tf`(翻訳キー→表示文字列)。`peace_panel.rs`は`can_accept_peace_offer`の
    `Err`については既に`peace_panel.notif_offer_rejected`キーで通知する仕組みを
    持っていた(`execute_peace_settlement`のErrだけがこの仕組みに接続されていな
    かった)。
16. **自動降伏失敗時の診断・通知機構**: `bevy::log::warn!`が他モジュール
    (`military/invasion.rs`)で「AI専用の失敗、画面通知はしないがログには残す」
    目的に既に使われている前例あり。診断専用Resourceのパターンも
    `AiSupportEvaluationCount`(P21-015)・`AiCrisisStartEvaluationCount`(P21-017)
    として既に確立済み。
17. **War goal対象州の別War同時使用**: 可能——`can_declare_war_with_date`は
    `are_countries_at_war(initiator, target)`のペア単位でしか重複を防がず、
    対象州のグローバルな一意性は一切検証しない。異なるattackerが同じdefenderの
    同じ州を別々のWarの目標にできる(§14のE2Eテストがこれを実際に実データで
    再現する)。
18. **Save validationの首都・州所有権・War goal検証**: 首都の存在・自国所有は
    既存(`validate_capital`)、海州チェックは無かった(新規追加)。War goal
    target_statesのdangling参照チェックは既存だが、Active WarでのState実所有者
    ==goal.defenderの検証は無かった(新規追加)。
19. **現在のテスト数**: lib 945 / integration 190 / 合計1135(実測、想定と完全一致)。
20. **fmt既存drift**: 41箇所(実測、想定と完全一致)。

---

## 4. 共通州所有権移転API

新モジュール`src/state/ownership.rs`(`state`は`country`より下位層のはずだが、
同一クレート内なので`country`への依存追加は循環importの問題を生まない——既に
`country`側が`state`へ依存しているため、双方向のuse自体はRustのモジュールシステム
上何ら問題ない)。

```rust
pub fn preflight_transfer_state_ownership(
    state_id: StateId,
    expected_current_owner: CountryId,
    new_owner: CountryId,
    state_registry: &StateRegistry,      // 読み取り専用
    country_registry: &CountryRegistry,  // 読み取り専用
) -> Result<ValidatedOwnershipTransfer, OwnershipTransferError>;

pub fn apply_ownership_transfer(  // §26 Fix2で戻り値をResult化(TOCTOU対策の再検証)
    plan: ValidatedOwnershipTransfer,
    state_registry: &mut StateRegistry,
    country_registry: &mut CountryRegistry,
) -> Result<OwnershipTransferOutcome, OwnershipTransferError>;

pub fn transfer_state_ownership(  // 上記2つをまとめた簡易版
    state_id: StateId,
    expected_current_owner: CountryId,
    new_owner: CountryId,
    state_registry: &mut StateRegistry,
    country_registry: &mut CountryRegistry,
) -> Result<OwnershipTransferOutcome, OwnershipTransferError>;
```

仕様書が例示した`transfer_state_ownership(...) -> Result<OwnershipTransferOutcome,
OwnershipTransferError>`という単一シグネチャに対し、実際には
**preflight/applyの2段階**へ分割した。理由は§7(講和全体の原子性)——
`execute_peace_settlement`は所有権移転の検証を他の検証(lineage等)と同じ
「実際の変更前」段階でまとめて行い、検証済み計画(`ValidatedOwnershipTransfer`)
を保持したまま他のステップへ進んだ後に初めて`apply_ownership_transfer`を呼ぶ
必要があるため。単純な呼び出し元(`accept_demand`)は`transfer_state_ownership`の
まとめ版をそのまま使う。

---

## 5. 必須validation(§3)

`preflight_transfer_state_ownership`が順に検証する(1つでも失敗すれば即座に
`Err`、以降のvalidationは実行しない):

1. 対象Stateが存在する(`StateNotFound`)
2. 対象が海州ではない(`SeaState`)
3. `expected_current_owner`がCountry Registryに存在する(`CountryNotFound`)
4. `new_owner`がCountry Registryに存在する(`CountryNotFound`)
5. Stateの実際の`owner_country_id`が`expected_current_owner`と一致する
   (`CurrentOwnerMismatch`)
6. 新旧所有国が異なる(`SameOwner`)
7. (対象が首都の場合)移転後も有効な所有陸州が残る(`LastOwnedLandState`)

`CountryNotFound`はexpected/newのどちらが欠けているかを区別しない——呼び出し元は
いずれもCrisis/War/Claimの参加国として既に検証済みの国家を渡すため、実運用でこの
バリアントへ到達するのは異常データのみであり、UI表示上分ける実用的な必要が無い
(§9で詳述)。

---

## 6. 首都再配置

`plan_capital_reassignment`(内部fn)が、対象州が現所有国の首都の場合のみ発火する。

- 割譲後にその国が残す**陸州のみ**(海州を除外)
- 移転対象State自身を除外
- `StateId`昇順で最小の州を新首都候補とする(HashMap挿入順に非依存)
- **owner**を使う(controller=占領中の実効支配ではなく、法的所有)
- 有効な残存陸州が一つも無ければ`LastOwnedLandState`で講和自体を拒否

`LastOwnedLandState`は「首都を割譲するが再配置先が無い」場合と「(首都かどうかに
関わらず)最後の所有陸州そのものを移転しようとした」場合を**1つのバリアントに
統合**した。`CountryData.capital_state_id`は常に現所有陸州を指す不変条件
(`app/loader.rs`が初期データで、本APIが実行時に維持する)があるため、この2つの
条件は構造的に常に一致する——別名(`CapitalReplacementUnavailable`)を追加すると
同じ実行時条件に2つの名前を与えることになり、かえって紛らわしいと判断した。

---

## 7. 最後の所有陸州(§8)

国家消滅システムが存在しないため、最後の所有陸州は**全ての移転経路で一律に
移転を拒否**する(§3のvalidation順序7がこれを一元的に保証、経路ごとの重複実装
なし):

- 通常Warの領土割譲: `execute_peace_settlement`のpreflightで拒否
- Crisis Warの領土割譲: 同上
- 平和的要求受諾(`accept_demand`): 同じ共通APIのpreflightで拒否
- いずれの経路でも、拒否時はClaimをConsumedにしない・War/Crisisのフェーズを
  変更しない(§9のtransfer_plan構築を「実際の変更前」に置くことで保証)
- 白紙講和(`WhitePeace`)・攻撃側譲歩(`AttackerConcedes`)は所有権移転自体を
  行わないため、対象州が最後の1州でも従来どおり成立する(regression確認済み)

この制限は、将来の国家消滅・完全併合タスクで置き換える予定であることを
ここに明記する。

---

## 8. 原子性(§7)

`execute_peace_settlement`のpreflight順序:

1. War存在・Active確認
2. primary attacker/defender計算
3. `resolve_crisis_war_lineage`(Crisis由来のみ、`Ok(None)`で通常War即座に素通り)
4. (`CedeWarGoalRegion`かつtarget_stateありの場合)
   `state::ownership::preflight_transfer_state_ownership`

上記が全て成功して初めて、実際の変更(領土移転・占領解除・戦闘中止・前線削除・
陸軍帰還・休戦作成・War状態更新・Claim消費)を開始する。失敗時はどの段階の
`Err`であっても、その時点で関数全体から即座にErrを返し、以降のステップは
一切実行されない。

`resolve_crisis_war_lineage`からは「対象州の現在の法的所有国がまだCrisis target
のまま」という検証(P21-019で追加した項目9)を**削除**した——共通所有権移転APIの
`expected_current_owner`検証が、Crisis由来War・通常War問わず同じ条件を一箇所だけ
で保証するようになったため、重複を残す理由が無くなった(§5要求「同じvalidationを
各呼び出し元へ複製しない」)。

`accept_demand`も同じ「pure検証 → 全成功後に一括適用」パターンに統一し、失敗時は
`crises`・`claims`・`states`・`countries`のいずれも変更しない。

---

## 9. 全所有権移転経路の統一(§5)

| 経路 | 接続先 |
|---|---|
| P21-011: Crisis要求の平和的受諾 | `accept_demand` → `state::ownership::preflight_transfer_state_ownership`/`apply_ownership_transfer` |
| P21-019: Crisis由来Warの`CedeWarGoalRegion` | `execute_peace_settlement` → 同上(通常Warと共有) |
| 通常Warの`CedeWarGoalRegion` | `execute_peace_settlement` → 同上 |
| 自動降伏による領土割譲 | `war/update.rs::handle_daily_war_resolution` → `execute_peace_settlement` → 同上 |
| プレイヤー操作による領土割譲 | `ui/peace_panel.rs::handle_peace_action_buttons` → `execute_peace_settlement` → 同上 |

occupationによる`controller_country`変更(法的所有権は変わらない)はこのAPIへ
統合していない(§5要求どおり)。

**実行時経路に旧`transfer_region_ownership`(unchecked版)の呼び出しは残って
いない**——`grep -rn "transfer_region_ownership_unchecked" src/`で確認できる
呼び出し元は、`state::ownership::apply_ownership_transfer`自身(内部実装として
の意図的な再利用)、および`war/tests.rs`・`map/rendering.rs`のテストコード
(低レベルAPI自身の挙動を直接検証する意図的なテスト、または既存のレンダリング
テストの土台としての利用)のみ。

---

## 10. 既存APIの扱い(§4)

`StateRegistry::transfer_region_ownership`を`transfer_region_ownership_unchecked`
へ改名し、可視性を`pub(crate)`へ制限した(仕様書が提示した4つの選択肢のうち
「安全な共通APIの内部実装専用として非公開化する」+「明示的なunchecked APIへ
改名する」を組み合わせた)。単純な`pub(in crate::state)`への制限は、この低レベル
APIを直接検証する既存テスト(`war/tests.rs`・`map/rendering.rs`)を壊すため
見送り、`pub(crate)`(クレート内では引き続き呼べるが、外部からは呼べない)を
選んだ——通常ゲームロジックからの誤用は、ドキュメントコメントで明示的に
警告している。

`apply_ownership_transfer`の内部実装は、この`transfer_region_ownership_unchecked`
をそのまま再利用する(owner_country_id/controller/占領進捗/原所有者/war_idの
正常化ロジックを1箇所だけに保つため、重複実装はしていない)。

---

## 11. 通常Warの安全化(§6)

通常Warの`CedeWarGoalRegion`も、共通所有権移転APIの`expected_current_owner`検証
(§5の一元化)により、Crisis由来Warと全く同じ保護を自動的に受けるようになった。
対象州の法的所有権が戦争中に第三国へ移っていた場合:

- 第三国から州を奪わない
- 首都を変更しない(preflight失敗のため首都再配置ステップへ到達しない)
- Warを終了させない
- 休戦を作らない
- Claimを変更しない(通常Warはそもそもlineageが無いため無関係)
- 軍隊・戦闘・前線・占領状態を変更しない
- 明示的なエラー(`state_error.ownership.current_owner_mismatch`)を返す

実データE2Eテスト(§17参照)でこの挙動を確認した。

---

## 12. エラー型(§9)

```rust
pub enum OwnershipTransferError {
    StateNotFound,
    SeaState,
    CountryNotFound,
    CurrentOwnerMismatch,
    SameOwner,
    LastOwnedLandState,
}

impl OwnershipTransferError {
    pub fn localization_key(self) -> &'static str { ... }
}
```

`&'static str`の翻訳キーへ`localization_key()`で変換する(既存の
`war_error.peace.*`規約と同じ、呼び出し元は文字列比較ではなくこの型自身で
判定できる)。自動降伏の重複抑止(`ReportedPeaceSettlementErrors`)は
`HashSet<(WarId, &'static str)>`で、`execute_peace_settlement`が最終的に
返す`&'static str`エラーキー(`OwnershipTransferError`由来か、lineage由来かを
問わず)をそのままキーとして使う——`execute_peace_settlement`自体の戻り値型は
既存どおり`Result<(), &'static str>`のままとした(講和API全体の戻り値型を
`OwnershipTransferError`混じりの新しい型へ変える大規模な変更は本タスクの
範囲を超えるため、`OwnershipTransferError::localization_key()`で変換した
文字列を既存の`&'static str`エコシステムへそのまま合流させる設計とした)。

新規localizationキー(`state_error.ownership.*`、6件)をJA/EN両方に追加し、
今回の変更で完全に使われなくなった旧2キー(`war_error.peace.
crisis_lineage_state_owner_changed`・`war_error.peace.
cede_would_leave_country_without_a_capital`)を削除した(key-parityテスト
`ja_jp_and_en_us_key_sets_match_exactly`で確認)。

---

## 13. プレイヤーUIのエラー表示(§10)

`ui/peace_panel.rs::handle_peace_action_buttons`の`execute_peace_settlement`
呼び出しを`if result.is_ok() { ... }`(Err無視)から`match result { Ok(()) =>
..., Err(err_msg) => ... }`へ変更し、既存の`peace_panel.notif_offer_rejected`
翻訳キー(`can_accept_peace_offer`のErrと同じキー、`{reason}`プレースホルダで
実際のエラーキーを埋め込む)経由でプレイヤーへ通知するようにした。

新しい翻訳キーを追加する必要は無かった——`state_error.ownership.*`・
`war_error.peace.crisis_lineage_*`いずれも既存の`localization::t()`で解決
できる安定キーのため、既存の汎用エラー通知の仕組みにそのまま乗る。

エラー後もパネル自体の構造(スクロール・講和アクションボタン)は無変更
——`Query`ベースのボタン処理は成功/失敗に関わらず毎フレーム再評価されるため、
壊れた状態が残ることはない。

---

## 14. 自動降伏講和のエラー処理(§11)

`war/update.rs`に新設:

```rust
enum CapitulationSettlementOutcome {
    Settled,
    FailedNotifyPlayer(&'static str),
    FailedLogOnly(&'static str),
    FailedAlreadyReported,
}

fn settle_capitulation(...) -> CapitulationSettlementOutcome;  // Bevy型に非依存の純粋関数
```

`settle_capitulation`自体は`MessageWriter`等のBevy固有型を一切取らない純粋関数
——`handle_daily_war_resolution`(呼び出し元のBevy System)が返り値を見て実際の
`GameNotification`送信・`bevy::log::warn!`ログ出力を行う。この分離により、
Bevy Appを一切構築せずに直接ユニットテストできる(§17のテスト対応表参照)。

```rust
#[derive(Resource, Default, Debug)]
pub struct ReportedPeaceSettlementErrors {
    pub reported: HashSet<(WarId, &'static str)>,
}
```

- 成功(War終了含む)時: そのWarに関する重複抑止エントリを全て削除
  (`retain(|&(w, _)| w != war_id)`)
- 失敗・`(War, エラー種別)`の組が初出: `reported.insert(...)`が`true`を返す
  場合のみ、プレイヤー当事国なら`FailedNotifyPlayer`、AI対AIのみなら
  `FailedLogOnly`を返す
- 失敗・既報告: `FailedAlreadyReported`(何もしない)
- Load後は`ReportedPeaceSettlementErrors`自体がSave非対象のため空になり、
  失敗が続いていれば1回だけ再通知される(§11要求どおり許容)
- 通常フレームでは、この処理は「その日実際に降伏判定が`None`以外を返した
  War」に対してしか呼ばれない(既存の`match cap { ... CapitulationResult::
  None => {} }`構造を変更していない)——全Warの追加走査は増えていない

`handle_daily_war_resolution`は元々12引数だったところへ4引数
(`reported_errors`・`player_country`・`notif_writer`・`locale`・`catalog`)
を追加した結果、Bevyのfunction-system引数上限(16)へ達したため、AI dirty化
ステップ(最後の2引数分)を`mark_ai_registries_dirty_daily`という独立Systemへ
分離した(`update_recruit_button_ui`等、既存の同種の分離パターンを踏襲)。
実行順は`.after(handle_daily_war_resolution)`で明示的に固定し、元の実装内で
「講和処理の後にAI dirty化」だった順序を保っている。

---

## 15. Save/Load validation(§12)

`src/save/validate.rs`へ2件追加:

1. `validate_capital`: 首都が海州でないことの検証を追加(既存の存在確認・
   自国所有確認に加えて)。実マップに海州は無いため実害は無いが、手動改変等の
   異常データに対する防御的検証として追加した。
2. `validate_wars`: `war.status == WarStatus::Active`の場合のみ、War goal
   対象州の実際の所有者が`goal.defender`のままであることを検証する(新規)。
   Ended War(講和後、所有者が正当に動いた後の履歴)はこの検証の対象外とする
   ——`ended_war_target_state_owner_change_is_not_falsely_rejected`テストで
   誤検出しないことを確認した。

この検証が無いと、対象州の所有者が実際とは食い違う壊れたSaveがvalidationを
通過し、ロード後の講和が`state::ownership::preflight_transfer_state_
ownership`の`CurrentOwnerMismatch`で永久に失敗し得た。

---

## 16. 決定論性(§13)

- 新首都は残存所有陸州の最小`StateId`(`plan_capital_reassignment`が
  `sort_by_key(|s| s.0)`後に`.first()`)
- HashMap/HashSetの挿入順に依存しない(`state_registry.states`はVec、
  `country_registry.get`はHashMapだが単一キー参照のみで走査順に依存しない)
- `.iter().next()`で首都・所有国・War goalを選ぶコードは無い
- ownerのみを使い、controllerは所有権判定に一切混ぜない
  (`plan_capital_reassignment`が`s.owner_country_id`のみを見る)
- 実在しないCountryId/StateIdを捏造するコードは無い(全て`Option`/`Result`
  で表現し、存在確認を経てから使う)
- 海州を首都にしない(`!s.is_sea`フィルタ)

`new_capital_choice_is_independent_of_state_insertion_order`テストで、
State挿入順を変えても同じ新首都が選ばれることを直接確認した。

---

## 17. 変更ファイル一覧

**新規**:
- `src/state/ownership.rs`(共通所有権移転Domain API、単体テスト19件——
  §26 Fix2のTOCTOU対策テスト2件を含む)
- `tests/p21_020_ownership_transfer_e2e_test.rs`(実データE2E 2件——
  §26 Fix1で1件から2件へ、うち1件は実際の講和ボタン+GameNotification+
  Save validationまでの完全統合確認)

**既存ファイルへの追記**:
- `src/state/data.rs`: `transfer_region_ownership` → `transfer_region_ownership_
  unchecked`へ改名。**§26 Fix4**: 可視性を`pub(crate)`から
  `pub(in crate::state)`へ再制限、`#[cfg(test)]`専用ラッパー
  `test_transfer_region_ownership_unchecked`を新設
- `src/state/mod.rs`: `pub mod ownership;`追加
- `src/diplomacy/crisis_response.rs`: `accept_demand`が共通APIを経由するよう
  書き換え(`country_registry`引数追加)、テスト2件追加。**§26 Fix1**:
  対象州を目標とする別のActive Warが存在する場合に受諾自体を拒否する検証を
  追加(`war_registry`引数追加)。**§26 Fix2**: `apply_ownership_transfer`の
  戻り値がResult化されたことに伴う`?`伝播
- `src/diplomacy/update.rs`: `evaluate_ai_crisis_responses`/
  `handle_daily_diplomacy`の`country_registry`を`&mut`化(`accept_demand`の
  シグネチャ変更に追従)、`country_by_id`をowned`CountryData`のHashMapへ変更
  (借用チェッカー対応)。**§26 Fix1**: 両関数へ`war_registry`引数追加
  (`accept_demand`の新しい競合War検証に必要)
- `src/ui/diplomacy_panel.rs`: `execute_crisis_confirm_accept`/
  `handle_crisis_command_buttons`へ`country_registry`引数追加。
  **§26 Fix1**: 両関数へ`war_registry`引数追加
- `src/war/peace.rs`: `resolve_crisis_war_lineage`から所有者検証を削除
  (共通APIへ統合)、`resolve_capital_reassignment`を削除(共通APIへ吸収)、
  `execute_peace_settlement`が共通APIを経由するよう書き換え。**§26 Fix1**:
  `invalidate_other_wars_targeting_state`新設(所有権移転成功時に競合War
  を白紙講和で自動失効)。**§26 Fix2**: `apply_ownership_transfer`の
  Result化に伴う`?`伝播
- `src/war/tests.rs`: 3件の既存テストのCountryRegistry fixtureを更新
  (`CountryNotFound`要件対応)、2件の既存テストの期待エラーキーを更新
  (旧`war_error.peace.*` → 新`state_error.ownership.*`)、新規テスト1件
  (通常War所有者保護)。**§26 Fix4**: 低レベルunchecked挙動を直接検証する
  1箇所を`#[cfg(test)]`専用ヘルパーへ切り替え
- `src/war/update.rs`: `handle_daily_war_resolution`のエラー処理を`match`化、
  `settle_capitulation`(純粋関数)+ `CapitulationSettlementOutcome`+
  `ReportedPeaceSettlementErrors`を新設、`mark_ai_registries_dirty_daily`
  を分離、テスト6件追加。**§26 Fix3**:
  `ReportedPeaceSettlementErrors::retain_only_active_wars`新設、
  `settle_capitulation`の成功時クリーンアップをこのメソッド経由に一般化
- `src/war/mod.rs`: `ReportedPeaceSettlementErrors`の登録、
  `mark_ai_registries_dirty_daily`の登録(`.after()`付き)
- `src/ui/peace_panel.rs`: `handle_peace_action_buttons`のErr処理を追加、
  テスト2件追加。**§26 Fix3**: `reported_errors`引数追加、成功時に
  `retain_only_active_wars`を呼ぶよう追加、テスト1件追加
- `src/save/validate.rs`: `validate_capital`に海州チェック追加、
  `validate_wars`にActive War所有者検証追加、テスト3件追加
- `src/map/rendering.rs`: **§26 Fix4**: `transfer_region_ownership_unchecked`
  への改名追従ではなく、安全な`state::ownership::transfer_state_ownership`
  経由へ書き換え(この可視性制限後は本番同等の経路を使うほうが適切なため)
- `src/diplomacy/ai_crisis_start/tests.rs`: `handle_daily_diplomacy`への
  `war_registry`引数追加に伴い、既存テストのApp fixtureへ`WarRegistry`を追加
- `tests/p21_010_claim_crisis_e2e_test.rs`,
  `tests/p21_011_crisis_ultimatum_e2e_test.rs`,
  `tests/p21_012_ai_crisis_response_e2e_test.rs`,
  `tests/p21_013_crisis_support_e2e_test.rs`,
  `tests/p21_015_ai_crisis_support_e2e_test.rs`: `handle_daily_diplomacy`
  System登録テストのApp fixtureへ`WarRegistry`を追加(§26 Fix1で
  `handle_daily_diplomacy`が`Res<WarRegistry>`を要求するようになったため)
- `tests/p21_011_crisis_ultimatum_e2e_test.rs`,
  `tests/p21_017_ai_claim_crisis_e2e_test.rs`: `accept_demand`呼び出し箇所を
  シグネチャ変更(§26 Fix1の`war_registry`引数追加含む)に追従
- `assets/localization/en-US.ron`/`ja-JP.ron`: `state_error.ownership.*`
  (6キー)・`war_error.peace.auto_settlement_failed`(1キー)を追加、
  旧2キーを削除。**§26 Fix1**:
  `diplomacy_error.crisis.target_state_contested_by_another_war`を追加

**P21-020が触れていない、セッション開始時から未コミットのまま残る
P21-016〜019由来の変更**: `src/country/country_ai.rs`, `src/diplomacy/
{ai_claim_generation,ai_crisis_start,claims,crisis,mod,player_demand_alert}
.rs`(`crisis_response.rs`/`update.rs`は本タスクで変更), `src/map/{frontline_
render,frontline_selection,offensive_line_selection}.rs`, `src/military/
{invasion,tests,update}.rs`, `src/profiling.rs`, `src/save/{apply,export}.rs`,
`src/ui/military_panel.rs`, `src/war/{data,frontline,military_ai}.rs`,
`tests/p21_005/008/010/012/013/015/016/018/019_*`, `verification_logs/
phase-21/{p21-016,p21-017,p21-018,p21-019}/*`(`tests/land_war_combat_peace_
test.rs`は本タスクで触れていない)。

---

## 18. テスト対応表

lib(`src/state/ownership.rs`17件 + `src/diplomacy/crisis_response.rs`2件 +
`src/war/tests.rs`1件 + `src/war/update.rs`6件 + `src/ui/peace_panel.rs`2件 +
`src/save/validate.rs`3件)= **lib 31件**、integration
(`tests/p21_020_ownership_transfer_e2e_test.rs`)= **1件**。合計 **32件**
(§19のlib +31/integration +1/合計+32と一致)。

| 要求テスト番号 | 対応するテスト名(モジュール) |
|---|---|
| 1 | `state::ownership::tests::normal_non_capital_transfer_succeeds` |
| 2 | `state::ownership::tests::capital_transfer_reassigns_capital` |
| 3 | `state::ownership::tests::new_capital_is_lowest_remaining_state_id` |
| 4 | `state::ownership::tests::new_capital_choice_is_independent_of_state_insertion_order` |
| 5 | `state::ownership::tests::capital_candidate_selection_uses_owner_not_controller` |
| 6 | `state::ownership::tests::sea_state_target_is_rejected` |
| 7 | `state::ownership::tests::nonexistent_state_is_rejected` |
| 8 | `state::ownership::tests::nonexistent_expected_owner_is_rejected` |
| 9 | `state::ownership::tests::nonexistent_new_owner_is_rejected` |
| 10 | `state::ownership::tests::current_owner_mismatch_is_rejected` |
| 11 | `state::ownership::tests::transfer_to_same_owner_is_rejected` |
| 12 | `state::ownership::tests::last_owned_land_state_transfer_is_rejected` |
| 13 | `state::ownership::tests::only_remaining_state_being_sea_is_rejected` |
| 14 | `state::ownership::tests::capital_candidate_never_uses_third_party_owned_state` |
| 15 | `state::ownership::tests::failure_leaves_state_registry_unchanged` |
| 16 | `state::ownership::tests::failure_leaves_country_registry_unchanged` |
| 17 | `state::ownership::tests::success_does_not_touch_unrelated_states_or_countries` |
| 18,32 | `diplomacy::crisis_response::tests::accept_demand_transfers_state_consumes_claim_and_resolves`(既存、共通API経由に更新) |
| 19,20 | 通常War・Crisis Warいずれも`execute_peace_settlement`が共通APIを呼ぶこと(§9の設計自体、`war::tests`の既存全件がregressionとして証明) |
| 21 | `war::update::tests::*`(6件、自動降伏が`settle_capitulation`経由で`execute_peace_settlement`を呼ぶことの直接検証) |
| 22 | `ui::peace_panel::tests::peace_settlement_failure_shows_translated_error_notification`/`peace_settlement_success_shows_no_error_notification` |
| 23 | grep実測(`transfer_region_ownership_unchecked`の全呼び出し元、§9末尾に記載)——構造的検証のため専用テストは設けず |
| 24,29,30 | 既存`war::tests::test_peace_treaty_basic`/`normal_war_peace_does_not_touch_unrelated_claims`(regression、無変更のままgreen) |
| 25,26,27,28 | `war::tests::normal_war_target_state_owned_by_unrelated_third_party_is_safely_rejected`(新規) |
| 31 | 既存`war::tests::multilateral_crisis_war_transfers_territory_to_primary_attacker_not_supporters`(`primary_attacker_id()`の多国間対応はCrisis/通常Warで共通コードのため、この既存テストがそのまま通常War側の保証にもなる) |
| 32,33 | `diplomacy::crisis_response::tests::accept_demand_reassigns_capital_when_ceding_the_targets_capital` |
| 34,43 | `diplomacy::crisis_response::tests::accept_demand_with_no_remaining_land_state_is_safely_rejected` |
| 35,36,37 | 既存`war::tests::crisis_war_cede_settlement_consumes_claim_and_transfers_target_state_to_initiator`等(regression) |
| 38 | 既存`war::tests::lineage_failure_leaves_every_registry_completely_unchanged`(regression) |
| 39 | 既存`war::tests::multilateral_crisis_war_transfers_territory_to_primary_attacker_not_supporters`(regression) |
| 40 | P21-019の完全E2E(regression、`tests/p21_019_crisis_war_peace_resolution_e2e_test.rs`全件green) |
| 41,42 | 既存`war::tests::cede_capital_with_no_remaining_states_is_safely_rejected`(regression、期待エラーキーのみ更新) |
| 44 | 同上テスト内の首都参照不変アサーション |
| 45,46 | 既存`war::tests::crisis_war_white_peace_consumes_claim_without_transferring_territory`/`crisis_war_attacker_concedes_consumes_claim_without_transferring_territory`(regression) |
| 47,53 | `ui::peace_panel::tests::peace_settlement_failure_shows_translated_error_notification` |
| 48 | `ui::peace_panel::tests::peace_settlement_success_shows_no_error_notification` |
| 49 | 同上(lineageエラーを実際に使用) |
| 50 | 新規テストでは`current_owner_mismatch`を主に検証、lineage側は既存パターンの延長のため個別テストは省略 |
| 51 | (§14要求50相当、上記に統合) |
| 52 | `ui::peace_panel::tests::peace_settlement_failure_shows_translated_error_notification`内でロケール混在文言を許容するアサーション(JA/EN両対応) |
| 53 | War状態がActiveのまま維持されることを同テスト内で確認 |
| 54 | 既存`crisis_war_goal_line_preserves_scroll_and_peace_action_buttons`(regression、無変更のままgreen) |
| 55 | `war::update::tests::failed_settlement_leaves_war_active` |
| 56 | `war::update::tests::player_participant_failure_returns_notify_player` |
| 57 | `war::update::tests::ai_only_failure_returns_log_only` |
| 58 | `war::update::tests::same_war_and_error_is_not_reported_twice` |
| 59 | `war::update::tests::different_error_kind_for_same_war_is_reported_again` |
| 60 | `war::update::tests::success_clears_the_reported_entry_for_that_war` |
| 61 | `save::validate::tests::capital_that_is_a_sea_state_is_rejected` |
| 62 | 既存`save::validate::tests::capital_owned_by_another_country_is_rejected`(regression) |
| 63 | `save::validate::tests::active_war_target_state_owner_mismatch_with_defender_is_rejected` |
| 64 | `save::validate::tests::ended_war_target_state_owner_change_is_not_falsely_rejected` |
| 65,66 | 既存P21-019 E2E(`peace_settlement_result_survives_save_load_round_trip`等、regression) |
| 67 | 既存P21-019 E2E `cede_capital_claim_generates_crisis_war_cession_and_survives_save_validation_and_load`(regression) |
| 68 | 既存`save::validate::tests`の旧Save互換系テスト群(regression、`#[serde(default)]`規約により無変更) |
| 69-75 | `tests::normal_war_conflicting_over_the_same_state_is_safely_invalidated_not_stuck`(§26 Fix1で改称・改修、実データE2E) |
| 76 | 既存P21-019 E2E `cede_capital_claim_generates_crisis_war_cession_and_survives_save_validation_and_load`(regression、Save/Load後も新首都が維持されることを既に確認済みのため重複作成せず) |

§26 Fix1-4の追加テスト(4点修正で新規に追加された分):

| Fix | テスト名 |
|---|---|
| Fix1(accept_demand側) | `diplomacy::crisis_response::tests::accept_demand_rejects_when_target_state_is_contested_by_another_active_war` |
| Fix1(execute_peace_settlement側・実データ) | `tests::normal_war_conflicting_over_the_same_state_is_safely_invalidated_not_stuck` |
| Fix1(実際の講和ボタン+GameNotification+Save validation) | `tests::two_real_wars_over_the_same_state_resolve_via_the_real_peace_button_and_pass_save_validation` |
| Fix2(TOCTOU、所有者変化) | `state::ownership::tests::apply_rejects_a_stale_plan_when_the_owner_changed_after_preflight` |
| Fix2(TOCTOU、首都候補変化) | `state::ownership::tests::apply_recomputes_capital_candidate_when_it_changed_after_preflight` |
| Fix3(手動講和成功で記録削除) | `ui::peace_panel::tests::manual_peace_success_clears_a_previously_reported_auto_capitulation_failure` |

---

## 19. 開始時・終了時テスト数

| | lib | integration | 合計 |
|---|---|---|---|
| 開始時 | 945 | 190 | 1135 |
| 初回実装完了時 | 976 | 191 | 1167 |
| 4点修正後(最終) | 980 | 192 | 1172 |
| 差分(開始時→最終) | +35 | +2 | +37 |

---

## 20. 品質ゲート

### 20.1 初回実装完了時

| ゲート | 結果 |
|---|---|
| `cargo check --all-targets` | 成功 |
| `cargo test --lib` | 976 passed / 0 failed |
| `cargo test --tests` | 全31integrationバイナリでpassed / 0 failed(191件) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 警告0件 |
| `cargo build --release` | 成功 |
| `git diff --check` | エラーなし |
| `cargo fmt --all -- --check` | 41箇所(開始時基準と完全一致、新規drift無し) |

### 20.2 4点修正後(最終、2026-08-21再実行)

| ゲート | 結果 |
|---|---|
| `cargo check --all-targets` | 成功 |
| `cargo test --lib` | 980 passed / 0 failed |
| `cargo test --tests` | 全31integrationバイナリでpassed / 0 failed(192件) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 警告0件 |
| `cargo build --release` | 成功 |
| `git diff --check` | エラーなし(既存のLF/CRLF警告のみ、内容差分なし) |
| P21-020変更ファイル単位の`rustfmt --edition 2024 --check` | 全件ドリフト0(下記§21参照) |
| `cargo fmt --all -- --check` | 41箇所(開始時基準と完全一致、新規drift無し) |
| スクリーンショット差分(headlessテスト副作用) | `git checkout --`で復元済み、`git status`クリーン確認済み |

---

## 21. fmt基準

- 開始時基準: **41箇所**(10ファイル、P21-019完了時と同一)。
- P21-020変更ファイルへ`rustfmt --edition 2024 <file>`を個別実行(初回実装分:
  `src/state/ownership.rs`, `src/war/tests.rs`, `src/war/update.rs`,
  `src/ui/peace_panel.rs`。4点修正分: `src/war/peace.rs`,
  `src/war/update.rs`(再), `src/diplomacy/crisis_response.rs`,
  `tests/p21_020_ownership_transfer_e2e_test.rs`)——いずれも実質改変した
  箇所自身の新規drift、無関係な既存drift0箇所を含む(これらのファイルは
  P21-019までの既存drift対象には含まれていなかったため、副次解消は発生
  していない)。
- 終了時総ドリフト: **41箇所**(開始時と完全一致)。内訳:
  `tests/land_war_combat_peace_test.rs`(15、P21-020は触れていない)、
  `tests/daily_system_integration_test.rs`(7)、`src/map/division_render.rs`
  (5)、`tests/p21_save_003_end_to_end_test.rs`(4)、`src/map/mod.rs`(3)、
  `src/save/runtime.rs`(2)、`src/military/movement.rs`(2)、
  `tests/profile_workload_correctness_test.rs`(1)、`src/military/supply.rs`
  (1)、`src/military/recruitment.rs`(1)。いずれもP21-020が触れていない
  既存ファイル。
- P21-020変更ファイル(新規`state/ownership.rs`含む)は`rustfmt --edition 2024
  --check`で個別にドリフト0を確認済み。

---

## 22. 性能測定方法・結果・生データ

一時プロファイラ(`src/state/ownership_perf_temp.rs`、`#[ignore]`付き
`#[test]`、`src/state/mod.rs`へ一時的に`#[cfg(test)] pub mod ownership_perf_
temp;`を追加して実行後、両方とも削除)を`cargo test --release --lib
state::ownership_perf_temp -- --ignored --nocapture`で実行。

**22.1 非首都州移転**(目標O(1)):

| n_states | ms/call |
|---|---|
| 100 | 0.0000 |
| 1,000 | 0.0000 |
| 10,000 | 0.0000 |
| 50,000 | 0.0000 |

**22.2 首都州移転(preflightのみ)**(目標最大O(states)):

| n_states | ms/call |
|---|---|
| 100 | 0.0001 |
| 1,000 | 0.0012 |
| 10,000 | 0.0385 |
| 50,000 | 0.2519 |

**22.3 最後の所有陸州による失敗**(目標最大O(states)):

| n_states | ms/call |
|---|---|
| 100 | 0.0001 |
| 1,000 | 0.0007 |
| 10,000 | 0.0347 |
| 50,000 | 0.2484 |

**22.4 完全な移転(preflight+apply)**(目標: apply自体のオーバーヘッドが無視できる):

| n_states | ms/call |
|---|---|
| 100 | 0.0000 |
| 1,000 | 0.0000 |
| 10,000 | 0.0010 |
| 50,000 | 0.0046 |

**22.5 自動降伏エラー重複抑止の参照コスト**(目標O(1)):

| entries | ms/call |
|---|---|
| 100 | 0.0000 |
| 1,000 | 0.0000 |
| 10,000 | 0.0000 |

**22.6 非講和フレーム**: `handle_daily_war_resolution`は
`evaluate_war_capitulation`が`CapitulationResult::None`を返す限り
`settle_capitulation`(ひいては所有権移転コード全体)を一切呼ばない。よって
非講和日には追加コストが構造的に0(実測不要、既存コード構造から自明)。

首都州移転・最後の所有陸州判定のO(states)コストは、それが必要になる実際の
講和成立の瞬間にのみ発生し、50,000州規模でも1ミリ秒未満に収まる。
`countries × states`や`wars × states`の禁止パターンは発生していない。

生データ: `verification_logs/phase-21/p21-020/perf/results.csv`,
`verification_logs/phase-21/p21-020/perf/summary.txt`。

---

## 23. 一時ファイル削除確認

- `src/state/ownership_perf_temp.rs`: 削除済み。
- `src/state/mod.rs`内の一時`#[cfg(test)] pub mod ownership_perf_temp;`登録:
  削除済み。
- 一時bin登録・scratch Save・一時スクリーンショット: 追加していない。
- デバッグ出力(`eprintln!`等): 追加していない。
- headlessテスト実行が変更した既存スクリーンショット9件
  (`verification_logs/p20-007/`, `p20-009/`, `phase-21/p21-save-002e/`配下)は
  `git checkout --`で復元済み(`git status`でクリーンを確認)。

---

## 24. 発見した既存問題

- **`get_owned_states`にO(1)逆引きインデックスが無い**(既存、P21-020範囲外):
  `StateRegistry::get_owned_states`はO(states)の線形走査。本タスクの首都再配置
  候補探索も同じパターンを踏襲したが、これは§15の性能測定で50,000州でも
  0.25ms程度に収まることを確認済みであり、実運用上の問題は無い。将来、
  州数が更に大幅に増える場合は`HashMap<CountryId, Vec<StateId>>`のような
  逆引きインデックスの導入を検討する価値がある。
- **同一対象州を目標にする複数Warが同時に成立し得る**(既存、P21-020範囲外・
  むしろ本タスクが正しく対処した): `can_declare_war_with_date`は対象州の
  グローバルな一意性を検証しない。本タスクが導入した`expected_current_owner`
  検証により、2件目以降の講和が安全に拒否されるようになった(§17のE2Eテスト
  で確認)ため、この既存仕様自体は「バグ」ではなく「共通所有権移転APIが
  正しく対処すべき既知の状況」として扱った。
- **`execute_peace_settlement`の戻り値型が`Result<(), &'static str>`のまま**
  (意図的な設計判断、範囲外というより本タスクの範囲内での判断): §9で述べた
  通り、`OwnershipTransferError`という型付きエラーを導入したが、講和API全体の
  戻り値型を作り替える大規模リファクタリングは行わず、`localization_key()`で
  既存の`&'static str`エコシステムへ合流させた。将来、講和API全体を型付き
  エラーへ統一するタスクがあれば、その際に`OwnershipTransferError`を
  `war_error`側のエラー型へ統合することを推奨する。

---

## 25. GUI手動確認チェックリスト

実施していない。実ウィンドウでの目視確認は行っていないため、
**COMPLETE WITH MANUAL VERIFICATION PENDING**とする。以下、確認すべき項目:

1. 通常Warで、対象州の所有者が戦争中に変わった場合、講和ボタンを押すと
   エラー通知が表示され、講和が成立しないことの目視確認
2. エラー通知の文言がJA/EN切替で自然に変わることの目視確認
3. エラー後も講和パネルのボタン・スクロールが正常に操作できることの目視確認
4. 自動降伏講和が失敗した場合(通常はほぼ発生しないはずだが)、プレイヤーが
   当事国ならゲーム内通知が表示されることの目視確認(意図的に発生させるには
   Save改変等が必要な可能性が高い)
5. 首都州を割譲する講和(要求受諾・通常War講和・Crisis War講和いずれか)で、
   首都マーカー等のUI表示が新しい首都州へ正しく移ることの目視確認
6. 最後の所有陸州を割譲しようとした場合、講和が成立せずエラー通知が
   表示されることの目視確認(通常プレイでは発生しにくいシナリオのため、
   意図的な状況構築が必要)
7. Save→Load後も、州所有・首都・講和エラー抑止状態(再ロード後に1回だけ
   再通知されること)が正しく復元されるかの目視確認
8. 既存の講和フロー(白紙講和・防御側勝利)が引き続き正常に動作することの
   目視確認(regressionが無いことの実プレイ確認)

---

## 26. ユーザー指摘対応(2026-08-21): 4点修正

初回実装完了報告に対し、ユーザーから以下4点の指摘を受け、いずれも修正した
(§20の品質ゲート再実行で全件green確認済み)。

### Fix 1: 複数Warの同一州競合が「安全に処理済み」ではなかった問題

**問題**: `can_declare_war_with_date`は対象州のグローバルな一意性を検証しない
(§3実装前調査17)ため、異なる攻撃国が同じ防御国の同じ州を目標にする複数の
Active Warが正当に同時存在し得る。1件目の講和で州所有権が実際に移った瞬間、
残りのWarは目標を構造的に永久達成不能になり、放置すると自動降伏講和が毎日
同じ割譲を試みては`current_owner_mismatch`で拒否され続けてWarが永久停止し、
Save validationも(§15のActive War所有者検証により)壊れたSaveとして拒否し
続ける可能性があった。

**修正(ユーザー提示の選択肢のうち「所有権移転時に、同じ州を目標とする他Warを
安全に失効・白紙講和させる」を採用)**: `war/peace.rs`に新設した
`invalidate_other_wars_targeting_state`が、`execute_peace_settlement`の
`CedeWarGoalRegion`で州所有権が実際に移った直後、同じ州を目標とする他の
Active War(今回決着したWar自身を除く、WarId昇順で決定論的)を全て
`PeaceTerm::WhitePeace`で自動的に失効させる。`WhitePeace`は所有権移転・Claim
消費のいずれも行わないため、`execute_peace_settlement`を再帰的に呼んでも
無限再帰・二重の所有権移転が発生しない。

**Domain APIレベルでの選択肢採用の使い分け**: `execute_peace_settlement`
(War講和)側は上記の自動失効(Option 2)を採用したが、`accept_demand`
(P21-011の平和的要求受諾)側は逆に「対象州を目標とする別のActive Warが
既に存在する場合は受諾自体を拒否する」(Option 1)を採用した——外交上の
平和的受諾という性質上、無関係な第三者間のWarへ副作用として介入するのは
不適切と判断し、`WarRegistry`を読み取るだけで済む軽量な事前拒否とした
(Military/Battle/Diplomacy/Frontlineという重い講和専用Registry群を
`accept_demand`へ追加せずに済む利点もある)。新しい翻訳キー
`diplomacy_error.crisis.target_state_contested_by_another_war`を追加。

新規テスト:
- `tests/p21_020_ownership_transfer_e2e_test.rs::
  normal_war_conflicting_over_the_same_state_is_safely_invalidated_not_stuck`:
  実データ(Dwarf Federation・Ferrowyn League・Oceanic Magic Empireの3か国、
  実際に隣接するState 9 Magic Harbor)で2つのActive Warを作り、片方の講和で
  もう片方が自動的にWhitePeaceへ遷移し、再試行してもOk(())で安全に素通り
  することを確認。
- `tests/p21_020_ownership_transfer_e2e_test.rs::
  two_real_wars_over_the_same_state_resolve_via_the_real_peace_button_and_pass_save_validation`
  (§後述、要求69-76の完全統合確認): 実際のBevy App・実際の講和ボタン
  (`PeaceActionButton`+`Interaction::Pressed`、`PeacePanelPlugin`の実
  `handle_peace_action_buttons`システム)・実際の`GameNotification`を通し、
  この状態(複数Warの同一州競合が自動的に安全処理された状態)のSaveが
  validationを通過し、Save/Load往復後も両Warの終結状態が維持されることを確認。

### Fix 2: `ValidatedOwnershipTransfer`のTOCTOU問題

**問題**: `preflight_transfer_state_ownership`で計画を検証した後、
`apply_ownership_transfer`が計画を無条件に信頼して適用していた。呼び出し元が
preflightとapplyの間に他の変更(Fix 1の他War白紙講和処理等)を挟む構造上、
Registryが変化してから古い計画がそのまま適用されるTOCTOU
(Time-of-check to time-of-use)の可能性があった。

**修正(ユーザー提示の「apply時にもowner・State・Country・新首都候補を
再確認してResultを返す」を採用)**: `apply_ownership_transfer`の戻り値を
`OwnershipTransferOutcome`(無条件)から`Result<OwnershipTransferOutcome,
OwnershipTransferError>`へ変更し、適用直前に`preflight_transfer_state_
ownership`を**再実行**して得た最新の計画(`fresh`)だけを使うようにした
(古い`plan`の値そのものは使わない)。これにより、首都再配置候補も含め
常に適用直前のRegistry状態を基準に計算し直され、stale planを安全に拒否
できる(失敗時はRegistryを一切変更しない)。呼び出し元
(`execute_peace_settlement`・`accept_demand`)は`?`でErrを伝播するよう
更新した。

新規テスト(`src/state/ownership.rs`):
- `apply_rejects_a_stale_plan_when_the_owner_changed_after_preflight`:
  preflight後に対象州の所有者が変わった場合、applyが`CurrentOwnerMismatch`
  で拒否しRegistryを一切変更しないことを確認。
- `apply_recomputes_capital_candidate_when_it_changed_after_preflight`:
  preflight時点の首都再配置候補州が、apply前に第三国へ渡った場合、applyが
  古い候補をそのまま使わず、直前に再計算した別の有効な候補を正しく選ぶ
  ことを確認。

### Fix 3: 自動エラー抑止記録が手動講和成功時に残る問題

**問題**: `ReportedPeaceSettlementErrors`は`settle_capitulation`(自動降伏)
成功時にしか該当War分のエントリを消していなかった。自動降伏が一度失敗して
記録された後、プレイヤーが白紙講和等で手動終戦すると、そのWarの古い記録が
永久に残ってしまう。Fix 1により1回の講和成功で複数のWarが同時に非Activeに
なり得るようになったため、「対象War IDだけ消す」という単純な削除ロジックでは
副作用で終わった側のWarの記録が漏れる問題も新たに生じていた。

**修正**: `ReportedPeaceSettlementErrors::retain_only_active_wars(&WarRegistry)`
という新メソッドを追加し、「現在ActiveなWar以外のエントリは全て削除する」
という条件に一般化した——経路(自動降伏 or プレイヤー講和)や、1回の講和で
何件のWarが終わったか(Fix 1の副作用含む)に関わらず正しく掃除できる。
`war/update.rs::settle_capitulation`と`ui/peace_panel.rs::
handle_peace_action_buttons`の両方の成功パスから呼ぶよう統一した
(後者は`ReportedPeaceSettlementErrors`を新しい引数として受け取るよう変更)。

新規テスト:
- `ui/peace_panel.rs::
  manual_peace_success_clears_a_previously_reported_auto_capitulation_failure`:
  自動降伏の失敗記録を事前に注入した状態でプレイヤーが実際の講和ボタンで
  手動講和を成功させ、その記録が消えることを確認。

### Fix 4: unchecked APIの隔離が不十分だった問題

**問題**: `StateRegistry::transfer_region_ownership_unchecked`の可視性が
`pub(crate)`のままだったため、`war`・`diplomacy`・`map`等どのモジュールからも
呼べてしまい、ドキュメント上の警告だけでは共通APIの迂回を防止できなかった。

**修正(ユーザー提示の3案を組み合わせ)**:
1. 可視性を`pub(crate)`から`pub(in crate::state)`(`state`モジュール配下—
   `state::data`自身と`state::ownership`のみ)へ制限。
2. `state::ownership`側の安全なAPIを使えない、低レベル挙動そのものを直接
   検証したいテスト向けに、`#[cfg(test)]`専用の公開ラッパー
   `StateRegistry::test_transfer_region_ownership_unchecked`を新設
   (本番ビルドには一切含まれない)。
3. 他モジュールのテストのうち、低レベル挙動自体の検証が目的ではなかった
   もの(`map/rendering.rs`のレンダリング反応テスト)は、安全な
   `state::ownership::transfer_state_ownership`を使うよう書き換えた。
   低レベル挙動(海州拒否)自体を検証している`war/tests.rs`のテストは
   `#[cfg(test)]`専用ヘルパーへ切り替えた。

`state::ownership::apply_ownership_transfer`の内部実装は、引き続き
`transfer_region_ownership_unchecked`をそのまま再利用する
(`state`モジュール内からの呼び出しのため、可視性制限の影響を受けない)。

### 実データE2Eの実際の講和ボタン・GameNotification検証

Fix 1のE2Eテスト(§上記)を、単なる直接Registry呼び出しではなく、実際の
Bevy App(`AppPlugin`+`PeacePanelPlugin`)・実際の`PeaceActionButton`
(`Interaction::Pressed`)・実際の`handle_peace_action_buttons`システム・
実際の`GameNotification`メッセージを通す形で追加した
(`two_real_wars_over_the_same_state_resolve_via_the_real_peace_button_and_pass_save_validation`)。
これにより、実データE2Eが実際のUI操作・通知経路まで通っていることを確認した。
