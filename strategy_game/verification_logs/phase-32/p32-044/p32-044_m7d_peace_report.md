# P32-044 (M7-d) 講和の提案・受諾と、降伏後の自動講和
基準コミット 9b984cf。実装は `src/world/peace.rs`(新規)と、`war_record.rs`・`territory.rs` の拡張。純粋な Rust・Bevy 非依存。**検証用シナリオの所有・戦争・支配は検証入力で、正式な世界データではない。**

## 0. 状態
- 実装済み・自動検証済み(`tests/p32_044_peace_test.rs` 17 件。全体の結果は `checks.md`)。CLI で再現(§3)。GUI の実操作確認は今回は不要(`hex_map_preview` は未変更)。**ユーザーの実操作確認はなし。**
- 範囲外(後続): 前線消失・長期停滞の白紙講和(M7-d 内の後続項目、§6 に調査)、勝敗表示と GUI の E2E(M7-e)、保存(M8)、本体への統合(M9)。
- 着手時の確認: HEAD = origin/main = 9b984cf、作業差分は `hex_atlas/`(未追跡)のみ。

## 1. 提案と受諾(`world::peace`)
- `PeaceProposal` = 戦争 ID・提案国・受領国・条件(`WhitePeace` / `CedeTarget` / `AcceptDefeat`)・提案日。提案(`propose_peace`)は読み取り専用で、戦争が稼働中・日が開戦以後・提案国と受領国が存続する対立する代表国の組・条件に対する権限があることを確かめる。
- 照会 `check_acceptance`(読み取り専用)は、**受諾の時点**の日付・存続・法的所有・戦争点数・降伏条件で全条件を検証し、通れば `plan_end_war` の計画まで返す。受諾 `accept_peace` はその計画を `apply_end_war` で適用する(適用の直前に再検証される)。
- 条件(定数: `WHITE_PEACE_MIN_WAR_DAYS = 30`、`WHITE_PEACE_MIN_SCORE = -10`、`CESSION_MIN_SCORE = 60`):
  - 白紙講和: どちらの代表国が提案してもよい。`今日 − 開始日 ≥ 30` かつ攻撃側視点の総点 ≥ −10(視点は提案国によらず共通)。領土移転なし。
  - 割譲: 攻撃側代表国 → 防御側代表国。防御側代表国が目標の法的所有者で、総点 ≥ 60 **または防御側単独の降伏条件**(M7-c の `defender_reasons` が空でない)。双方成立時の**最終裁定(`verdict`)とは区別**する(テストで、最終裁定が攻撃側降伏でも防御側単独の条件で割譲を受諾できることを確認)。
  - 敗北受諾: 攻撃側代表国 → 防御側代表国。領土移転なしの防御側勝利(`WarResult::DefenderVictory`、理由 `DefeatAccepted`)。点数・日数の条件はない。
- 拒否(状態は変わらない。拒否の前後で Territory・Conflicts・MapMilitary の一致をテスト): 未知の戦争、終結済み(二重受諾・目標の所有国が変わって失効した戦争を含む)、開戦前、権限のない国(防御側からの割譲・敗北、第三国)、受領国でない国の受諾、受諾日が提案日より前、前提の失効(受諾時の点数・日数)、計画の失敗(首都候補なし・戦闘の中止指定が必要)。
- 記録での判別: 提案による割譲 = `EndReason::Cession`、敗北受諾 = `DefeatAccepted`、白紙講和 = `WhitePeace`。自動の降伏 = `DefenderSurrender`(割譲・攻撃側の勝利)/ `AttackerSurrender`(領土移転なしの防御側勝利)。`EndWarRequest.basis`(`EndBasis::Agreement` / `Surrender`)で渡す。

## 2. 日次の自動講和(`run_auto_peace`。`advance_day` の終了後に呼ぶ)
1. その日の確定した戦闘(`BattleFinished`)を `BattleLedger` に集計(**講和より先**。同じ戦闘は二度数えない)。
2. 開始時に稼働中の戦争を `MapWarId` 昇順に、**各適用の直前に現在の状態を読み直して** M7-c の降伏を判定。先行する割譲・国家消滅で連鎖終結した戦争は `SkippedEnded`(処理済み)として飛ばし、継続する戦争は最新の状態(所有・支配・師団・台帳)で判定する。
3. 防御側降伏 = 目標の割譲(攻撃側勝利)、攻撃側降伏 = 領土移転なしの防御側勝利。**提案の受諾条件(30 日・点数)は介さず**、共通の終結 API(`end_war`)を通す。
4. 適用に失敗した講和は戦争を稼働中のまま残し(状態は変わらない)、理由を `Failed` に記録して次回に再判定する。
- 割譲で首都を失う国の候補: 現行の候補(自国が支配するプロヴィンス)に加え、**同じ講和で敵対が解けて法的所有国へ返還されるプロヴィンス**(返還後の支配)を含める。選定順は現行どおり(`MapStateId` 昇順の最初の候補があるステート → 代表地点 → 自国が支配する最小 `ProvinceId`)。割譲・占領正常化・首都確定を 1 回の終結で一括適用する。別戦争で敵対が続いて返還されず候補がない場合は、現行どおり `NoCapitalCandidate` で拒否する。**単独の領土移転(`plan_transfer` / `apply_transfer`)には現行の規則のまま**(テストで確認)。
- 休戦(M7-b の終結 API が付ける): 直接成立した講和は、降伏を含め、存続する代表国間に終結日から 1825 日(終了日の前日まで有効)。目標失効・国家消滅による連鎖終結には新設しない。消滅国との休戦は除去(その講和で一度設定した消滅国との休戦も除く)。別戦争で続く敵対は維持。

## 3. 実行したコマンドと結果(`strategy_game/` で。出力全文は各ファイル。いずれも終了コード 0)
共通: `cargo run -q -j 1 --bin world_move_check -- <引数>`。A = `--scenario assets/data/world/preview/division_war_scenario.ron`、B = `.../division_war_elimination_scenario.ron`。`--brief` は日ごとの師団表示を省く。`--propose DAY:WAR:提案国ラベル:white|cede|defeat`(受領国は相手の代表国)、`--accept DAY:WAR:受諾国ラベル`(その戦争の直近の提案を受諾)、`--auto-peace`(各日の `advance_day` の終了後に自動講和。**明示したときだけ有効**)、`--war-score`(単独では判定の表示だけで何も適用しない)。各ファイルの 1 行目に実際のコマンドを記録している。
| ファイル | 引数(A を省略) | 期待 | 実出力(抜粋) |
|---|---|---|---|
| cli_01_white_peace_29_30 | `--brief --propose 1:1:1:white --accept 29:1:2 --accept 30:1:2 --days 30` | 29 日は拒否、30 日で成立。休戦 1825 日 | `accept REJECTED: 白紙講和には開戦から 30 日以上が必要です(現在 29 日) (nothing changed)` → 30 日: `MapWar#1 ended: white peace (explicit white peace)`、`truce set: country 1-2 until day 1855` |
| cli_02_cession_total_60 | `--brief --war-score --days 5 --control 1:<22 の 6 つ>:2 --control 1:17696:2 --control 1:17905:2 --control 1:18112:2 --propose 5:1:2:cede --accept 5:1:1` | 総点 60 で受諾。22 が国家2 へ、首都は 18109 | `total: 60`、`query ... acceptable; total score 60 (defender-alone surrender condition: not met)`、`MapWar#1 ended: attacker victory (explicit cession)`、`MapWar#2 ended ... target's owner changed`、`capital moved: country 1 Province#17266 -> Province#18109`、`truce set: country 1-2 until day 1830` |
| cli_03_cession_total_59_rejected | 上と同じで国家2 の支配を 22 + 18109 + 17905 + 18112 に | 総点 59 で拒否 | `total: 59`、`accept REJECTED: 割譲には総点 60 以上、または防御側単独の降伏条件が必要です(総点 59、降伏条件は不成立) (nothing changed)` |
| cli_04_defeat_authority_double_accept | `--brief --days 4 --propose 2:1:1:cede --propose 2:1:3:white --propose 2:1:2:defeat --accept 3:1:1 --accept 4:1:1` | 防御側の割譲提案・第三国の提案は拒否。敗北受諾は成立し領土は動かない。再受諾は拒否 | `proposal REJECTED: CountryId 1 は CedeTarget を提案する権限がありません`、`CountryId 3 と 2 は MapWar#1 の対立する代表国の組ではありません`、`MapWar#1 ended: defender victory (attacker's defeat accepted ...)`、再受諾 `MapWar#1 は 3 日に終結済みです (nothing changed)` |
| cli_05a_war_score_only_no_application | `--brief --war-score --days 1 <国家2 が 22・20 を全域>` | 判定は表示するが何も適用しない | 戦争1 `verdict: DEFENDER SURRENDERS`、戦争記録は ACTIVE のまま |
| cli_05b_auto_target22_only_continues | `--brief --auto-peace --days 1 <国家2 が 22 だけ全域>` | 占領率 50% で戦争1 は継続 | `AUTO-PEACE day 1: no surrender condition is met in any active war` |
| cli_05c_auto_defender_surrender_22_and_20 | `--brief --auto-peace --days 1 <国家2 が 22・20 を全域>` | 戦争1 が防御側降伏。22 が国家2 へ、20 は返還、首都 18109。戦争2 は目標失効、戦争3 は継続、1-3 の敵対は維持、休戦は 1-2 だけ | `MapWar#1 ended: attacker victory (AUTOMATIC: defender surrender ...)`、`MapWar#2 ended: white peace (target's owner changed ...)`、`hostility ended: [1-2]; kept by other wars: [1-3]`、`capital moved: ... 17266 -> 18109`、`occupation returned ... Province#18109: country 2 -> country 1`、`truce set: country 1-2 until day 1826`、`hostile pairs: 1-3 3-4`、`MapWar#2 was already ended by an earlier peace of the day (chain); skipped` |
| cli_06_auto_attacker_surrender | `--brief --auto-peace --days 1 <国家1 が 21 を全域>` | 攻撃側降伏 = 領土移転なしの防御側勝利 | `MapWar#1 ended: defender victory (AUTOMATIC: attacker surrender (no territory moves))` |
| cli_07_auto_last_state_elimination | B で `--brief --auto-peace --days 2 --end-war 1:5:white <2 日目に国家1 が 21 を全域>` | 最後のステートの割譲で国家2 が消滅。戦争7 は消滅で決着、戦争6 は支持国を除いて継続、休戦 2-4・1-2 を除去 | `MapWar#8 ended: attacker victory (AUTOMATIC: defender surrender ...)`、`MapWar#7 ended: attacker victory (defender primary eliminated)`、`MapWar#6: eliminated supporter country 2 removed from its side`、`truce: none kept (a party was eliminated)`、`truce cleared: 1-2`・`2-4` |
| cli_08_auto_failed_capital_then_next_day | `--brief --auto-peace --days 2 <国家3 が 22・20 を全域>` | 1 日目: 戦争2 は首都候補なしで失敗(稼働中のまま)、戦争3 は成立。2 日目: 戦争2 を再判定して成立し国家1 が消滅 | 1 日目 `MapWar#2: defender surrenders but the peace could NOT be applied: ... 首都にできる(自国が支配する)プロヴィンスがありません (nothing changed; the war stays active ...)`、`MapWar#3 ended ...`。2 日目 `MapWar#2 ended ...`、`MapWar#1 ended: attacker victory (defender primary eliminated)` |

日数の境界(29/30、59/60、−11/−10)・提案後の前提失効・権限・二重受諾・拒否時の状態の一致は、CLI でなく自動テストで確認した(下記)。

## 4. 自動検証(`tests/p32_044_peace_test.rs`、17 件)
- 白紙講和: 29 日拒否・30 日受諾(休戦の境界を含む)/ 総点 −11 拒否・−10 受諾(どちらの代表国が提案しても同じ)。
- 割譲: 総点 59 拒否・60 受諾(首都移動・休戦)/ 防御側単独の降伏条件での受諾(総点 51)と、双方成立・総点 0(最終裁定は攻撃側降伏)でも受諾できること / 目標の所有者が防御側でない場合の拒否。
- 敗北受諾(領土移転なし・占領は法的所有国へ返還)/ 権限違反・未知の戦争・受領国でない国・受諾日が提案日より前・開戦前の拒否(状態の一致)/ 提案後の前提失効(割譲 57、白紙講和 −15)・二重受諾・目標失効した戦争の受諾の拒否。
- 首都候補: 返還されるプロヴィンスを含めると 18109 に決まる / 単独の移転は現行どおり `NoCapitalCandidate` / 別戦争で敵対が続くと候補がなく、講和は原子的に拒否される。
- 自動講和: 22 だけで継続・20 も全域で降伏(理由 `DefenderSurrender`、戦争2 は連鎖で飛ばす、戦争3 の継続、休戦は 1-2 のみ)/ 攻撃側降伏(`AttackerSurrender`、領土移転なし)/ 双方成立・総点 0 は攻撃側降伏、その日の戦闘(防御側の勝利)が講和より先に計上されて +5 → 0 となり裁定が変わる、再投入は二重計上しない / 首都候補なしの失敗は戦争を稼働中のまま残して状態を変えず、翌日に再判定されて成立(国家1 消滅)/ 同日の後続の戦争は先行する講和後の所有で判定される(国家2 の所有が 21 だけになって初めて攻撃側降伏)/ 最後のステートの割譲(国家消滅・関連戦争の終結・支持国の除去・消滅国との休戦の除去)/ `assess_active_wars` は何も適用しない。

## 5. 旧実装・M7-b との差(記録)
- 休戦: 直接成立した講和(降伏を含む)だけに付き、連鎖終結には付けない。M7-b と同じ規則で、旧(連鎖終結にも付く)との差は P32-042 報告書 §3・§8 のとおり。今回の指示でこの規則に確定した(P32-042 報告書 §3 の「未決」は解消)。
- 旧の自動降伏は提案条件(30 日・点数)を介さない。新も同じ。
- 首都: M7-a の規則に「同じ講和で返還されるプロヴィンスの返還後の支配」を加えた(単独の移転は従来のまま)。

## 6. 前線消失・長期停滞による白紙講和(M7-d 内の後続項目。実装していない。調査のみ)
旧の日次(`war/update.rs` の `handle_daily_war_resolution`)の順序: ①戦闘結果の集計(`sync_battle_results_to_wars`)→ ②戦勝点(`process_war_score`)→ ③降伏と自動講和(`evaluate_war_capitulation` → `settle_capitulation`)→ ③.5 有効な前線を失った戦争の終結(P28-007)→ ③.6 長期停滞の白紙講和(`war/stalemate.rs`: E1 = 一方に師団がない 365 日、M1 = 双方に師団があり攻勢の意思がない 730 日)→ 停滞記録の整理 → 前線の更新。各段は直前の段で終結した戦争を取り直して除く。
- **③.5 前線消失** の入力: `FrontlineRegistry` の戦争ごとの `Frontline.border_region_pairs` が空(宣戦当日など前線が未生成なら対象外)。`war_goals` が空の戦争は対象外(占領による実質的勝利を白紙講和にしない)。新マップに**ない**もの: 前線(`Frontline`)・国境の接触の概念(D7)。必要: ステートの隣接(`StateAdjacency` は M6 で導出済み)から「目標ステートへ攻撃側の到達可能な国境がない」を定義し直すか、D7 の設計に合わせる。
- **③.6 停滞** の入力: 進展の署名 `progress_signature`(戦闘の集計値と、所有者と支配者が異なる州の集合)が 365/730 日変化せず進行中の戦闘もない、生存師団の有無(陣営全体)、攻勢の意思(前線計画が Offensive・軍事 AI の直近判断が `SufficientAdvantage`・移動中/戦闘中/占領中の師団)、AI 同士(プレイヤーが主要国・支持国にいない)、`can_accept_peace_offer`(白紙講和の受諾判定)、停滞の起点記録 `WarStalemateRecord`(保存される)。新マップに**ない**もの: 前線計画・軍事 AI の判断記録・プレイヤー国の区別・停滞記録(`WarRecord` にも `BattleLedger` にもない)。今回ある: 戦闘の確定数(`BattleLedger`)、実効支配の変化(`MoveEvent::ControlChanged`)、移動中・戦闘中の師団(`MapMilitary`)、白紙講和の受諾判定(`check_acceptance`)。
- 接続する順序案: `run_auto_peace` の降伏の処理(旧 ③)の**後**に、③.5 → ③.6 の順で追加する。各段は降伏と同じく ID 昇順に、各適用の直前に最新状態を読み直し、終結済み(連鎖を含む)を飛ばす。停滞の起点記録は戦争記録とは別の実行時状態(保存は M8)で、日次の更新は `advance_day` のイベント(`BattleFinished`・`ControlChanged`・移動)から行う。AI の意思の入力は M9(AI 統合)までないので、M1 は最初は対象外とし、E1 相当(一方に戦闘可能な師団がない)から始めるのが現実的。終結は白紙講和(`EndWarRequest::white_peace`)で、休戦は M7-b の規則(直接の講和には付く)をそのまま使う。
