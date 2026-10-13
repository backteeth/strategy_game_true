use crate::app::time::DayChangedMessage;
use crate::common::WarId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::localization::Loc;
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::frontline::FrontlineRegistry;
use bevy::prelude::*;
use std::collections::HashSet;

/// P21-020: 自動降伏講和(`execute_peace_settlement`)が失敗した場合の通知・警告を、
/// 同じ`(War, エラー種別)`の組について1度だけに抑えるための診断専用Resource。
/// Saveへは保存しない――Load直後は空になり、失敗が続いていれば1回だけ再通知される
/// (§11要求どおり許容される挙動)。
#[derive(Resource, Default, Debug)]
pub struct ReportedPeaceSettlementErrors {
    pub reported: HashSet<(WarId, &'static str)>,
}

/// P21-021: `settle_capitulation`(自動降伏講和)が国家消滅発生時に内部で
/// `country::elimination::apply_country_elimination`を呼ぶために必要な
/// 追加Registry群を1つのSystemParamへまとめる。`handle_daily_war_resolution`は
/// 既にBevyのSystemParamタプル引数数上限(16)に達しているため、
/// `localization::Loc`/`diplomacy::update::EliminationSideEffectRegistries`と
/// 同じ「複数Resourceを1つのSystemParamへ束ねる」パターンを踏襲する。
#[derive(bevy::ecs::system::SystemParam)]
pub struct WarResolutionEliminationExtras<'w> {
    pub army_registry: ResMut<'w, crate::military::army::ArmyRegistry>,
    pub justification_registry: ResMut<'w, crate::war::justification::WarJustificationRegistry>,
    pub power_registry: ResMut<'w, crate::country::power::CountryPowerRegistry>,
    /// P29-003: 停滞戦争の白紙講和で、軍事AIの直近判断(攻勢可能か)を参照する。
    /// 資源を持たない最小Appでも動くよう`Option`。
    pub military_ai_registry: Option<Res<'w, crate::war::military_ai::MilitaryAiRegistry>>,
}

impl ReportedPeaceSettlementErrors {
    /// P21-020(ユーザー指摘対応): このWarがActiveでなくなった(存在しない場合も
    /// 含む)エントリを全て削除する。
    ///
    /// **単に`war_id != 対象`だけで削除すると不十分な理由**: `execute_peace_
    /// settlement`のCedeWarGoalRegionは、州所有権が実際に移った場合、同じ州を
    /// 目標とする*他の*Warも副作用として白紙講和で自動的に終結させる
    /// (`invalidate_other_wars_targeting_state`参照)。そのため、1回の講和成功
    /// (自動降伏経由・プレイヤー講和経由問わず)で複数のWarが同時にActiveでは
    /// なくなり得る——「今回終わらせた1件のWar IDだけ」を消すのでは、副作用で
    /// 終わった側のWarの古い記録が永久に残ってしまう。「現在ActiveなWar以外は
    /// 全て消す」という条件にすることで、経路や副作用の数に関わらず正しく
    /// 掃除できる。
    ///
    /// 全ての講和成功経路(自動降伏の`settle_capitulation`、プレイヤー講和の
    /// `ui::peace_panel::handle_peace_action_buttons`)から呼ばれることを想定する。
    pub fn retain_only_active_wars(&mut self, war_registry: &WarRegistry) {
        self.reported.retain(|&(w, _)| {
            war_registry
                .wars
                .get(&w)
                .is_some_and(|war| war.status == WarStatus::Active)
        });
    }
}

/// 戦争前処理システム (WarPreparation Set)
/// 宣戦布告等で変更された全前線の初期化および最新状態同期を行う
pub fn handle_daily_war_prep(
    mut day_events: MessageReader<DayChangedMessage>,
    state_registry: Res<StateRegistry>,
    war_registry: Res<WarRegistry>,
    military_registry: Res<MilitaryRegistry>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
) {
    for _event in day_events.read() {
        // 全前線の最新状態再計算・初期化同期
        crate::war::frontline::update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
    }
}

/// P21-020: `handle_daily_war_resolution`から分離した、軍事AI・国家AI作戦状態の
/// dirty化のみを行う独立System。P21-020でエラー通知系の引数(4件)が
/// `handle_daily_war_resolution`へ追加され、Bevyのfunction-system引数上限
/// (`update_recruit_button_ui`等が既に同じ理由で分離済み)へ達したため分離した。
/// `MessageReader<DayChangedMessage>`は各Systemが自分専用の読み取りカーソルを
/// 持つため、同じMessageを複数Systemで独立に読んでも問題ない(既存の
/// `handle_daily_war_prep`/`handle_daily_war_resolution`の分割と同じ前提)。
///
/// P26-008-PRE-002: 旧実装(`mark_ai_registries_dirty_daily`)は`DayChangedMessage`の
/// たびに全国家を無条件で`mark_all_dirty()`しており、週次(研究AI・戦争準備AI)・
/// 月次(建設AI)・3日おき(戦線計画AI)というAI評価cadenceの設計意図を実質的に
/// 無効化していた(`verification_logs/phase-26/p26-008-pre-001/`で実測確認)。
/// 本Systemは`war::ai_change_tracking::AiChangeSnapshot`を使い、州支配
/// (controller/owner)または戦争(status/参加国)が**実際に変化した**国家だけを
/// `mark_country_dirty`する――このSystemが`handle_daily_war_resolution`の後に
/// 置かれるのは、その日の講和・国家消滅・州所有権移転(`MilitaryAction` Setでの
/// 占領も含め、その日確定した最終状態)が全て反映された後の状態を、前日終値の
/// スナップショットと比較するため(スナップショット自体はSaveしない、P26-006の
/// 首都連結性と同じ「再計算専用」方針――理由は`ai_change_tracking`モジュールdoc参照)。
pub fn mark_ai_registries_dirty_on_change(
    mut day_events: MessageReader<DayChangedMessage>,
    state_registry: Res<StateRegistry>,
    war_registry: Res<WarRegistry>,
    mut snapshot: ResMut<crate::war::ai_change_tracking::AiChangeSnapshot>,
    mut ai_registry: ResMut<crate::war::military_ai::MilitaryAiRegistry>,
    mut country_ai_registry: ResMut<crate::country::country_ai::CountryAiRegistry>,
) {
    for _event in day_events.read() {
        let current = crate::war::ai_change_tracking::AiChangeSnapshot::capture(
            &state_registry,
            &war_registry,
        );
        let affected = snapshot.diff_affected_countries(&current);
        for country_id in affected {
            ai_registry.mark_country_dirty(country_id);
            country_ai_registry.mark_country_dirty(country_id);
        }
        *snapshot = current;
    }
}

/// P28-007: 国境(有効な前線)を失ったまま継続するWarを白紙講和させる際の`end_reason`。
/// 既存の州所有権変更に起因する`"War Goal No Longer Achievable"`(E1,
/// `peace::invalidate_other_wars_targeting_state`)とは別の文言にして、ログ・UI上で
/// 原因を区別できるようにする。
const FRONTLINE_UNREACHABLE_REASON: &str = "Frontline Unreachable";

/// 戦争解決・後処理システム (WarResolution Set)
/// 戦闘結果の集計、戦勝点更新、降伏判定・自動講和、戦後整理を行う
/// (AI dirty化は`mark_ai_registries_dirty_on_change`へ分離済み、上記コメント参照)。
#[allow(clippy::too_many_arguments)]
pub fn handle_daily_war_resolution(
    mut day_events: MessageReader<DayChangedMessage>,
    mut state_registry: ResMut<StateRegistry>,
    mut war_registry: ResMut<WarRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
    mut battle_registry: ResMut<BattleRegistry>,
    mut diplomacy_registry: ResMut<DiplomacyRegistry>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
    mut crisis_registry: ResMut<CrisisRegistry>,
    mut claim_registry: ResMut<ClaimRegistry>,
    mut country_registry: ResMut<CountryRegistry>,
    mut reported_errors: ResMut<ReportedPeaceSettlementErrors>,
    player_country: Res<PlayerCountry>,
    mut notif_writer: MessageWriter<GameNotification>,
    loc: Loc,
    mut elimination_extras: WarResolutionEliminationExtras,
) {
    for event in day_events.read() {
        let current_date_str = format!("{:04}/{:02}/{:02}", event.year, event.month, event.day);

        // 1. 終了した戦闘勝敗の戦争データへの集計
        crate::war::combat::sync_battle_results_to_wars(&battle_registry, &mut war_registry);

        // 2. 戦勝点更新
        crate::war::war_score::process_war_score(&state_registry, &mut war_registry);

        // 3. 降伏判定と自動講和終結
        let active_war_ids: Vec<WarId> = war_registry
            .wars
            .values()
            .filter(|w| w.status == WarStatus::Active)
            .map(|w| w.id)
            .collect();

        for war_id in active_war_ids {
            if let Some(war) = war_registry.wars.get(&war_id) {
                let cap = crate::war::capitulation::evaluate_war_capitulation(
                    war,
                    &state_registry,
                    &military_registry,
                );

                let settlement = match cap {
                    crate::war::capitulation::CapitulationResult::DefenderCapitulated => Some((
                        crate::war::peace::PeaceTerm::CedeWarGoalRegion,
                        "Defender Capitulation",
                    )),
                    crate::war::capitulation::CapitulationResult::AttackerCapitulated => Some((
                        crate::war::peace::PeaceTerm::AttackerConcedes,
                        "Attacker Capitulation",
                    )),
                    crate::war::capitulation::CapitulationResult::None => None,
                };

                if let Some((term, reason)) = settlement {
                    let outcome = settle_capitulation(
                        war_id,
                        term,
                        reason,
                        &current_date_str,
                        &mut state_registry,
                        &mut war_registry,
                        &mut military_registry,
                        &mut battle_registry,
                        &mut diplomacy_registry,
                        &mut frontline_registry,
                        &mut crisis_registry,
                        &mut claim_registry,
                        &mut country_registry,
                        &mut elimination_extras.army_registry,
                        &mut elimination_extras.justification_registry,
                        &mut elimination_extras.power_registry,
                        &mut reported_errors,
                        &player_country,
                    );
                    report_settlement_outcome(
                        outcome,
                        war_id,
                        "AutoCapitulation",
                        &mut notif_writer,
                        &loc,
                    );
                }
            }
        }

        // 3.5. 有効な前線(border_region_pairs)を失ったまま継続するWarの終結(P28-007)。
        // 対象州の所有者自体は変わらないが、国境・戦争目標が構造的に成立不能になった
        // 戦争を、既存の白紙講和経路(`settle_capitulation`)で終結させる。ステップ3
        // (降伏判定、およびその内部で`execute_peace_settlement`が副作用として呼ぶ
        // `invalidate_other_wars_targeting_state`=既存のE1)が同日中に既にこのWarを
        // 終結させている場合があるため、Active War一覧をここで改めて取得し直し、
        // 降伏・既存E1による終結を誤って上書きしない。
        //
        // `war_goals`が空のWarは対象外とする(P28-007回帰検証で発見した境界条件、
        // P28-007-PRE-001の一時パッチでは未検証): 自然発生するAI戦争は必ず単一の
        // `WarGoalType::ConquerState`を持つ(`src/diplomacy/crisis.rs`等、宣戦5箇所で
        // 確認済み)。`war_goals`が空なのは、戦争目標を伴わない純粋な戦闘・補給機構
        // テスト用fixtureだけに現れる人為的な状態であり、このようなWarは
        // `check_defender_capitulation`/`check_attacker_capitulation`の前提
        // (`war.war_goals.first()`)自体が成立しないため降伏判定で終わる経路が
        // 最初から存在しない。攻撃側が防御側の到達可能な全領域を占領し尽くす
        // ことでもborder_region_pairsは空になり得るが、これは「構造的停止」では
        // なく「占領による実質的勝利」であり、E2が白紙講和で誤って占領状態を
        // 正常化(無効化)してはならない。実際のAI戦争はConquerStateを持つため、
        // この除外は現行の対象原因(国境・戦争目標喪失)の挙動を一切変えない。
        let frontline_stalled_war_ids: Vec<WarId> = war_registry
            .wars
            .values()
            .filter(|w| w.status == WarStatus::Active && !w.war_goals.is_empty())
            .map(|w| w.id)
            .collect();

        for war_id in frontline_stalled_war_ids {
            if !war_has_no_valid_frontline(war_id, &frontline_registry) {
                continue;
            }

            let outcome = settle_capitulation(
                war_id,
                crate::war::peace::PeaceTerm::WhitePeace,
                FRONTLINE_UNREACHABLE_REASON,
                &current_date_str,
                &mut state_registry,
                &mut war_registry,
                &mut military_registry,
                &mut battle_registry,
                &mut diplomacy_registry,
                &mut frontline_registry,
                &mut crisis_registry,
                &mut claim_registry,
                &mut country_registry,
                &mut elimination_extras.army_registry,
                &mut elimination_extras.justification_registry,
                &mut elimination_extras.power_registry,
                &mut reported_errors,
                &player_country,
            );
            report_settlement_outcome(
                outcome,
                war_id,
                "FrontlineUnreachable",
                &mut notif_writer,
                &loc,
            );
        }

        // 3.6. 長期停滞したAI戦争の白紙講和(P29-003、診断 P29-003-DIAG-001 の E1)。
        // ステップ3(降伏・E1)・3.5(国境消失)が同日に既に終結させた戦争は対象外
        // (停滞期間の更新は Active War のみを対象にするため、ここで改めて取得し直される)。
        // 条件の詳細は`crate::war::stalemate`。講和は既存の`settle_capitulation`
        // (`execute_peace_settlement`、領土移転なしの白紙講和)で行う。
        let stalemate_war_ids = crate::war::stalemate::update_stalemate_records(
            &mut war_registry,
            &state_registry,
            &military_registry,
            &battle_registry,
            &frontline_registry,
            &country_registry,
            elimination_extras.military_ai_registry.as_deref(),
            player_country.0,
            &current_date_str,
        );
        for war_id in stalemate_war_ids {
            let outcome = settle_capitulation(
                war_id,
                crate::war::peace::PeaceTerm::WhitePeace,
                crate::war::stalemate::STALEMATE_END_REASON,
                &current_date_str,
                &mut state_registry,
                &mut war_registry,
                &mut military_registry,
                &mut battle_registry,
                &mut diplomacy_registry,
                &mut frontline_registry,
                &mut crisis_registry,
                &mut claim_registry,
                &mut country_registry,
                &mut elimination_extras.army_registry,
                &mut elimination_extras.justification_registry,
                &mut elimination_extras.power_registry,
                &mut reported_errors,
                &player_country,
            );
            report_settlement_outcome(
                outcome,
                war_id,
                "StalemateWhitePeace",
                &mut notif_writer,
                &loc,
            );
        }
        // 終結した戦争(連鎖終結を含む)の停滞記録を、Save へ残さないよう直ちに削除する。
        crate::war::stalemate::prune_inactive_records(&mut war_registry);

        // 4. 無効前線・陸軍参照の整理と前線の最新状態再計算
        crate::war::frontline::update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
    }
}

/// P28-007: このWarに対応する`Frontline`が存在し、かつ`border_region_pairs`が
/// 空(＝`MilitaryAiDecisionReason::NoValidFrontline`と同一条件、
/// `src/war/military_ai.rs`参照)かどうかを判定する。純粋関数としてユニットテスト
/// 可能にしておく(`settle_capitulation`と同じ設計方針)。宣戦当日などまだ
/// `Frontline`が生成されていない(`None`)場合は、構造的な停止ではないため`false`。
fn war_has_no_valid_frontline(war_id: WarId, frontline_registry: &FrontlineRegistry) -> bool {
    frontline_registry
        .get_frontline_for_war(war_id)
        .is_some_and(|fl| fl.border_region_pairs.is_empty())
}

/// P21-020/P28-007: `settle_capitulation`の結果に応じて、プレイヤー通知またはログ
/// 出力を行う。降伏判定(ステップ3)と前線喪失判定(ステップ3.5)の両方から共通で使う。
fn report_settlement_outcome(
    outcome: CapitulationSettlementOutcome,
    war_id: WarId,
    tag: &str,
    notif_writer: &mut MessageWriter<GameNotification>,
    loc: &Loc<'_>,
) {
    match outcome {
        CapitulationSettlementOutcome::Settled | CapitulationSettlementOutcome::FailedAlreadyReported => {}
        CapitulationSettlementOutcome::FailedNotifyPlayer(err_key) => {
            notif_writer.write(GameNotification {
                message: loc.tf(
                    "war_error.peace.auto_settlement_failed",
                    vec![("reason", loc.t(err_key))],
                ),
            });
        }
        CapitulationSettlementOutcome::FailedLogOnly(err_key) => {
            bevy::log::warn!(
                "[{tag}] {war_id:?} settlement failed (AI-only, no player notification, will \
                 not repeat until the error or war changes): {err_key}"
            );
        }
    }
}

/// P21-020: `settle_capitulation`の結果。呼び出し元(Bevy System)がこれを見て、
/// 実際の通知(`GameNotification`)・ログ(`bevy::log::warn!`)を行う――
/// `settle_capitulation`自体はBevy固有の型(`MessageWriter`等)に依存しない
/// 純粋関数のため、Bevy Appなしで直接ユニットテストできる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapitulationSettlementOutcome {
    /// 成功(War終了含む)。
    Settled,
    /// 失敗・このWarとエラー種別の組み合わせは初出・プレイヤーが当事国
    /// → 翻訳キーを添えて`GameNotification`を送るべき。
    FailedNotifyPlayer(&'static str),
    /// 失敗・初出・AI対AIのみ → 翻訳キーを添えてログにだけ残すべき。
    FailedLogOnly(&'static str),
    /// 失敗だが同じ(War, エラー種別)を既に報告済み → 何もしない。
    FailedAlreadyReported,
}

/// P21-020: 自動降伏講和を実行し、その`Result`を無視しない。
///
/// 成功(War終了含む——`execute_peace_settlement`は既存War終了済みでも`Ok(())`を返す
/// 二重適用防止分岐を持つ)なら、このWarについて過去に記録した重複抑止エントリを
/// 全て削除する。失敗時はWarをActiveのまま維持し(`execute_peace_settlement`自身が
/// 全か無かの原子性を保証するため、ここでは何もしなくて良い)、`(War, エラー種別)`の
/// 組が初出の場合にのみ`FailedNotifyPlayer`/`FailedLogOnly`を返す
/// (プレイヤーが参加国かどうかで分岐)。同じ組が既に報告済みなら
/// `FailedAlreadyReported`を返し、毎日無制限に再通知・再ログしない。
#[allow(clippy::too_many_arguments)]
fn settle_capitulation(
    war_id: WarId,
    term: crate::war::peace::PeaceTerm,
    reason: &str,
    current_date_str: &str,
    state_registry: &mut StateRegistry,
    war_registry: &mut WarRegistry,
    military_registry: &mut MilitaryRegistry,
    battle_registry: &mut BattleRegistry,
    diplomacy_registry: &mut DiplomacyRegistry,
    frontline_registry: &mut FrontlineRegistry,
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &mut ClaimRegistry,
    country_registry: &mut CountryRegistry,
    army_registry: &mut crate::military::army::ArmyRegistry,
    justification_registry: &mut crate::war::justification::WarJustificationRegistry,
    power_registry: &mut crate::country::power::CountryPowerRegistry,
    reported_errors: &mut ReportedPeaceSettlementErrors,
    player_country: &PlayerCountry,
) -> CapitulationSettlementOutcome {
    // 通知要否の判定(プレイヤーが当事国か)は、講和実行前の参加国集合を見て決める
    // (講和成功時はWarのstatusが変わるだけで参加国集合自体は変わらないため、
    // 実行前後どちらで見ても同じ判定になるが、実行前に確定させておく方が明快)。
    let is_player_participant = war_registry.wars.get(&war_id).is_some_and(|w| {
        player_country
            .0
            .is_some_and(|p| w.attackers.contains(&p) || w.defenders.contains(&p))
    });

    let result = crate::war::peace::execute_peace_settlement(
        war_id,
        term,
        reason,
        current_date_str,
        state_registry,
        war_registry,
        military_registry,
        battle_registry,
        diplomacy_registry,
        frontline_registry,
        crisis_registry,
        claim_registry,
        country_registry,
        army_registry,
        justification_registry,
        power_registry,
    );

    match result {
        Ok(()) => {
            reported_errors.retain_only_active_wars(war_registry);
            CapitulationSettlementOutcome::Settled
        }
        Err(err_key) => {
            if reported_errors.reported.insert((war_id, err_key)) {
                if is_player_participant {
                    CapitulationSettlementOutcome::FailedNotifyPlayer(err_key)
                } else {
                    CapitulationSettlementOutcome::FailedLogOnly(err_key)
                }
            } else {
                CapitulationSettlementOutcome::FailedAlreadyReported
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DiplomaticCrisisId, StateId};
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::state::data::StateData;
    use crate::war::data::War;
    use crate::war::peace::PeaceTerm;

    #[allow(clippy::type_complexity)]
    fn env() -> (
        StateRegistry,
        WarRegistry,
        MilitaryRegistry,
        BattleRegistry,
        DiplomacyRegistry,
        FrontlineRegistry,
        CrisisRegistry,
        ClaimRegistry,
        CountryRegistry,
        ReportedPeaceSettlementErrors,
        crate::military::army::ArmyRegistry,
        crate::war::justification::WarJustificationRegistry,
        crate::country::power::CountryPowerRegistry,
    ) {
        let states = StateRegistry::build(vec![
            StateData {
                id: StateId(0),
                owner_country_id: CountryId(0),
                ..Default::default()
            },
            StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                ..Default::default()
            },
        ]);
        let mut war_reg = WarRegistry::default();
        war_reg.wars.insert(
            WarId(1),
            War {
                id: WarId(1),
                name: "Test War".to_string(),
                attackers: [CountryId(0)].into_iter().collect(),
                defenders: [CountryId(1)].into_iter().collect(),
                primary_attacker: None,
                primary_defender: None,
                source_crisis_id: None,
                war_goals: vec![WarGoal {
                    attacker: CountryId(0),
                    defender: CountryId(1),
                    goal_type: WarGoalType::ConquerState,
                    target_states: vec![StateId(1)],
                    base_peace_cost: 20.0,
                    international_concern: 10.0,
                    completion: 0.0,
                    is_primary: true,
                }],
                start_date: "1936/01/01".to_string(),
                end_date: None,
                duration_days: 0,
                war_score: 0.0,
                attacker_war_exhaustion: 0.0,
                defender_war_exhaustion: 0.0,
                occupied_states: std::collections::HashSet::new(),
                status: WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: std::collections::HashSet::new(),
            },
        );
        let country_reg = CountryRegistry {
            countries: vec![0, 1]
                .into_iter()
                .map(|id| crate::country::CountryData {
                    id: CountryId(id),
                    capital_state_id: StateId(999), // 首都再配置ロジックを誘発しない
                    ..Default::default()
                })
                .collect(),
        };
        (
            states,
            war_reg,
            MilitaryRegistry::default(),
            BattleRegistry::default(),
            DiplomacyRegistry::default(),
            FrontlineRegistry::default(),
            CrisisRegistry::default(),
            ClaimRegistry::default(),
            country_reg,
            ReportedPeaceSettlementErrors::default(),
            crate::military::army::ArmyRegistry::default(),
            crate::war::justification::WarJustificationRegistry::default(),
            crate::country::power::CountryPowerRegistry::default(),
        )
    }

    /// Crisis由来War(source_crisis_id=Some)にして、lineage検証が必ず失敗する
    /// (Crisis自体を登録しない、dangling)ようにした環境を返す――
    /// `execute_peace_settlement`のErrを安定して発生させるための最小構成。
    #[allow(clippy::type_complexity)]
    fn env_with_guaranteed_settlement_failure() -> (
        StateRegistry,
        WarRegistry,
        MilitaryRegistry,
        BattleRegistry,
        DiplomacyRegistry,
        FrontlineRegistry,
        CrisisRegistry,
        ClaimRegistry,
        CountryRegistry,
        ReportedPeaceSettlementErrors,
        crate::military::army::ArmyRegistry,
        crate::war::justification::WarJustificationRegistry,
        crate::country::power::CountryPowerRegistry,
    ) {
        let mut e = env();
        e.1.wars.get_mut(&WarId(1)).unwrap().source_crisis_id = Some(DiplomaticCrisisId(99));
        e
    }

    /// 要求テスト55: 自動講和失敗でWarがActiveのまま。
    #[test]
    fn failed_settlement_leaves_war_active() {
        let (
            mut state_reg,
            mut war_reg,
            mut mil_reg,
            mut battle_reg,
            mut dip_reg,
            mut fl_reg,
            mut crisis_reg,
            mut claim_reg,
            mut country_reg,
            mut reported,
            mut army_reg,
            mut justification_reg,
            mut power_reg,
        ) = env_with_guaranteed_settlement_failure();

        let outcome = settle_capitulation(
            WarId(1),
            PeaceTerm::CedeWarGoalRegion,
            "Defender Capitulation",
            "1936/06/01",
            &mut state_reg,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(None),
        );

        assert!(matches!(
            outcome,
            CapitulationSettlementOutcome::FailedLogOnly(_)
        ));
        assert_eq!(
            war_reg.wars.get(&WarId(1)).unwrap().status,
            WarStatus::Active
        );
    }

    /// 要求テスト56: プレイヤー当事国への通知(FailedNotifyPlayer)を返す。
    #[test]
    fn player_participant_failure_returns_notify_player() {
        let (
            mut state_reg,
            mut war_reg,
            mut mil_reg,
            mut battle_reg,
            mut dip_reg,
            mut fl_reg,
            mut crisis_reg,
            mut claim_reg,
            mut country_reg,
            mut reported,
            mut army_reg,
            mut justification_reg,
            mut power_reg,
        ) = env_with_guaranteed_settlement_failure();

        let outcome = settle_capitulation(
            WarId(1),
            PeaceTerm::CedeWarGoalRegion,
            "Defender Capitulation",
            "1936/06/01",
            &mut state_reg,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(Some(CountryId(0))), // attacker=player
        );

        assert!(matches!(
            outcome,
            CapitulationSettlementOutcome::FailedNotifyPlayer(_)
        ));
    }

    /// 要求テスト57: AI対AIのみの失敗はFailedLogOnlyを返す(プレイヤー非当事国)。
    #[test]
    fn ai_only_failure_returns_log_only() {
        let (
            mut state_reg,
            mut war_reg,
            mut mil_reg,
            mut battle_reg,
            mut dip_reg,
            mut fl_reg,
            mut crisis_reg,
            mut claim_reg,
            mut country_reg,
            mut reported,
            mut army_reg,
            mut justification_reg,
            mut power_reg,
        ) = env_with_guaranteed_settlement_failure();

        let outcome = settle_capitulation(
            WarId(1),
            PeaceTerm::CedeWarGoalRegion,
            "Defender Capitulation",
            "1936/06/01",
            &mut state_reg,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(Some(CountryId(2))), // 第三国、この戦争の当事国ではない
        );

        assert!(matches!(
            outcome,
            CapitulationSettlementOutcome::FailedLogOnly(_)
        ));
    }

    /// 要求テスト58: 同一(War, エラー種別)の組を2回目以降はFailedAlreadyReported。
    #[test]
    fn same_war_and_error_is_not_reported_twice() {
        let (
            mut state_reg,
            mut war_reg,
            mut mil_reg,
            mut battle_reg,
            mut dip_reg,
            mut fl_reg,
            mut crisis_reg,
            mut claim_reg,
            mut country_reg,
            mut reported,
            mut army_reg,
            mut justification_reg,
            mut power_reg,
        ) = env_with_guaranteed_settlement_failure();

        let first = settle_capitulation(
            WarId(1),
            PeaceTerm::CedeWarGoalRegion,
            "Defender Capitulation",
            "1936/06/01",
            &mut state_reg,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(None),
        );
        let second = settle_capitulation(
            WarId(1),
            PeaceTerm::CedeWarGoalRegion,
            "Defender Capitulation",
            "1936/06/02",
            &mut state_reg,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(None),
        );

        assert!(matches!(
            first,
            CapitulationSettlementOutcome::FailedLogOnly(_)
        ));
        assert_eq!(second, CapitulationSettlementOutcome::FailedAlreadyReported);
    }

    /// 要求テスト59: 原因(エラー種別)が変われば、同じWarでも新しく報告される
    /// (既に記録済みなのが*別の*エラーキーなら抑止対象にならない)。
    #[test]
    fn different_error_kind_for_same_war_is_reported_again() {
        let (
            mut state_reg,
            mut war_reg,
            mut mil_reg,
            mut battle_reg,
            mut dip_reg,
            mut fl_reg,
            mut crisis_reg,
            mut claim_reg,
            mut country_reg,
            mut reported,
            mut army_reg,
            mut justification_reg,
            mut power_reg,
        ) = env_with_guaranteed_settlement_failure();

        // このWarについて、実際に今回発生するのとは異なるエラーキーを
        // 既報告として記録しておく(dangling source_crisis_idの実際の失敗理由は
        // "war_error.peace.crisis_lineage_missing_crisis"だが、ここでは
        // 全く別のキーを事前記録する)。
        reported
            .reported
            .insert((WarId(1), "war_error.peace.some_unrelated_previous_error"));

        let outcome = settle_capitulation(
            WarId(1),
            PeaceTerm::CedeWarGoalRegion,
            "Defender Capitulation",
            "1936/06/01",
            &mut state_reg,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(None),
        );

        // (War, 異なるエラーキー)の組は初出のため、既報告扱いにならない。
        assert!(matches!(
            outcome,
            CapitulationSettlementOutcome::FailedLogOnly(_)
        ));
    }

    /// 要求テスト60: 成功後に重複抑止記録が削除される(再度同じ失敗が起きれば
    /// また通知される)。
    #[test]
    fn success_clears_the_reported_entry_for_that_war() {
        let (
            mut state_reg,
            mut war_reg,
            mut mil_reg,
            mut battle_reg,
            mut dip_reg,
            mut fl_reg,
            mut crisis_reg,
            mut claim_reg,
            mut country_reg,
            mut reported,
            mut army_reg,
            mut justification_reg,
            mut power_reg,
        ) = env();

        reported
            .reported
            .insert((WarId(1), "some_previous_error_key"));

        let outcome = settle_capitulation(
            WarId(1),
            PeaceTerm::CedeWarGoalRegion,
            "Defender Capitulation",
            "1936/06/01",
            &mut state_reg,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(None),
        );

        assert_eq!(outcome, CapitulationSettlementOutcome::Settled);
        assert!(
            !reported.reported.iter().any(|&(w, _)| w == WarId(1)),
            "a successful settlement must clear all previously-reported entries for this war"
        );
    }

    /// P28-007 要求テスト(国境消失): 対象州の所有者は変わらないが、第三国への
    /// 割譲によって当事国間の有効な国境(`border_region_pairs`)が消失した場合、
    /// 次の判定で白紙講和(`FRONTLINE_UNREACHABLE_REASON`)により終結する。
    /// 既存のE1(`peace::invalidate_other_wars_targeting_state`)は戦争目標州
    /// 自体の所有者変更を要求するため、このケース(目標州は不変)では発火せず、
    /// 州所有権移転も発生しないことを確認する。
    #[test]
    fn frontline_lost_without_target_state_transfer_ends_in_white_peace() {
        use crate::state::data::StateData;
        use crate::war::frontline::update_or_create_frontline_for_war;

        // state0: 攻撃側(country0)の境界州。state1: 防御側(country1)の境界州
        // (state0に隣接)。state2: 防御側(country1)の戦争目標州(state1にのみ隣接、
        // state0とは非隣接)。
        let mut states = StateRegistry::build(vec![
            StateData {
                id: StateId(0),
                owner_country_id: CountryId(0),
                neighbors: vec![StateId(1)],
                ..Default::default()
            },
            StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                neighbors: vec![StateId(0), StateId(2)],
                ..Default::default()
            },
            StateData {
                id: StateId(2),
                owner_country_id: CountryId(1),
                neighbors: vec![StateId(1)],
                ..Default::default()
            },
        ]);

        let mut war_reg = WarRegistry::default();
        war_reg.wars.insert(
            WarId(1),
            War {
                id: WarId(1),
                name: "Border Loss Test War".to_string(),
                attackers: [CountryId(0)].into_iter().collect(),
                defenders: [CountryId(1)].into_iter().collect(),
                primary_attacker: None,
                primary_defender: None,
                source_crisis_id: None,
                war_goals: vec![WarGoal {
                    attacker: CountryId(0),
                    defender: CountryId(1),
                    goal_type: WarGoalType::ConquerState,
                    target_states: vec![StateId(2)],
                    base_peace_cost: 20.0,
                    international_concern: 10.0,
                    completion: 0.0,
                    is_primary: true,
                }],
                start_date: "1936/01/01".to_string(),
                end_date: None,
                duration_days: 0,
                war_score: 0.0,
                attacker_war_exhaustion: 0.0,
                defender_war_exhaustion: 0.0,
                occupied_states: std::collections::HashSet::new(),
                status: WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: std::collections::HashSet::new(),
            },
        );

        let mut fl_reg = FrontlineRegistry::default();
        let war_snapshot = war_reg.wars.get(&WarId(1)).unwrap().clone();
        update_or_create_frontline_for_war(&war_snapshot, &states, &mut fl_reg);
        assert!(
            !war_has_no_valid_frontline(WarId(1), &fl_reg),
            "開戦時点ではstate0-state1の国境が存在するはず"
        );

        // 第三国(country2)への割譲により、防御側の境界州(state1)の所有者が
        // 変わる。戦争目標州(state2)自体は防御側(country1)のまま不変。
        states.get_mut(StateId(1)).unwrap().owner_country_id = CountryId(2);
        update_or_create_frontline_for_war(&war_snapshot, &states, &mut fl_reg);

        assert!(
            war_has_no_valid_frontline(WarId(1), &fl_reg),
            "state1喪失により、state0(country0)に隣接するdefender(country1)の州が無くなるはず"
        );
        assert_eq!(
            states.get(StateId(2)).unwrap().owner_country_id,
            CountryId(1),
            "戦争目標州は割譲されていない(E1が発火する条件ではない)"
        );

        let mut mil_reg = MilitaryRegistry::default();
        let mut battle_reg = BattleRegistry::default();
        let mut dip_reg = DiplomacyRegistry::default();
        let mut crisis_reg = CrisisRegistry::default();
        let mut claim_reg = ClaimRegistry::default();
        let mut country_reg = CountryRegistry {
            countries: vec![0, 1, 2]
                .into_iter()
                .map(|id| crate::country::CountryData {
                    id: CountryId(id),
                    capital_state_id: StateId(999), // 首都再配置ロジックを誘発しない
                    ..Default::default()
                })
                .collect(),
        };
        let mut reported = ReportedPeaceSettlementErrors::default();
        let mut army_reg = crate::military::army::ArmyRegistry::default();
        let mut justification_reg = crate::war::justification::WarJustificationRegistry::default();
        let mut power_reg = crate::country::power::CountryPowerRegistry::default();

        // ステップ3.5相当: 国境喪失を検出したWarを、既存の白紙講和経路で終結させる。
        let outcome = settle_capitulation(
            WarId(1),
            PeaceTerm::WhitePeace,
            FRONTLINE_UNREACHABLE_REASON,
            "1936/06/01",
            &mut states,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(None),
        );

        assert_eq!(outcome, CapitulationSettlementOutcome::Settled);
        let war = war_reg.wars.get(&WarId(1)).unwrap();
        assert_eq!(war.status, WarStatus::WhitePeace);
        assert_eq!(war.winner, None);
        assert_eq!(
            war.end_reason.as_deref(),
            Some(FRONTLINE_UNREACHABLE_REASON)
        );
        assert!(
            fl_reg.get_frontline_for_war(WarId(1)).is_none(),
            "終結後は前線が削除されるはず"
        );
        assert_eq!(
            states.get(StateId(2)).unwrap().owner_country_id,
            CountryId(1),
            "白紙講和では戦争目標州の所有権は移転しない"
        );
        assert!(
            dip_reg.is_in_truce(CountryId(0), CountryId(1), "1936/06/01"),
            "白紙講和後は攻撃側・防御側間に休戦が作成されるはず"
        );
    }

    /// P28-007 要求テスト(国境が残る停止): 国境は存在するが、補給・戦力不足で
    /// 長期間攻勢に出られないwar9型の停止では、単なる無戦闘・前線割当不足を
    /// 理由にE2が発火してはならない。E2は`border_region_pairs`の有無のみで
    /// 判定し、`offensive_supply_blocked`や師団配置状況とは独立している。
    #[test]
    fn frontline_with_remaining_border_is_never_terminated_by_e2() {
        use crate::state::data::StateData;
        use crate::war::frontline::update_or_create_frontline_for_war;

        let states = StateRegistry::build(vec![
            StateData {
                id: StateId(0),
                owner_country_id: CountryId(0),
                neighbors: vec![StateId(1)],
                ..Default::default()
            },
            StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                neighbors: vec![StateId(0)],
                ..Default::default()
            },
        ]);

        let mut war_reg = WarRegistry::default();
        war_reg.wars.insert(
            WarId(1),
            War {
                id: WarId(1),
                name: "Stalled War (war9-type)".to_string(),
                attackers: [CountryId(0)].into_iter().collect(),
                defenders: [CountryId(1)].into_iter().collect(),
                primary_attacker: None,
                primary_defender: None,
                source_crisis_id: None,
                war_goals: vec![WarGoal {
                    attacker: CountryId(0),
                    defender: CountryId(1),
                    goal_type: WarGoalType::ConquerState,
                    target_states: vec![StateId(1)],
                    base_peace_cost: 20.0,
                    international_concern: 10.0,
                    completion: 0.0,
                    is_primary: true,
                }],
                start_date: "1936/01/01".to_string(),
                end_date: None,
                duration_days: 0,
                war_score: 0.0,
                attacker_war_exhaustion: 0.0,
                defender_war_exhaustion: 0.0,
                occupied_states: std::collections::HashSet::new(),
                status: WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: std::collections::HashSet::new(),
            },
        );

        let mut fl_reg = FrontlineRegistry::default();
        let war_snapshot = war_reg.wars.get(&WarId(1)).unwrap().clone();
        let fl_id = update_or_create_frontline_for_war(&war_snapshot, &states, &mut fl_reg);

        // war6/war9型の実際の症状(攻勢供給ブロック)を模して、国境は残っているが
        // 長期間攻勢に出られない状態を再現する(師団割当は意図的に空のまま)。
        fl_reg
            .plans
            .get_mut(&(fl_id, CountryId(0)))
            .unwrap()
            .offensive_supply_blocked = true;

        assert!(
            !war_has_no_valid_frontline(WarId(1), &fl_reg),
            "state0-state1の国境は消えていないため、E2は発火してはならない"
        );

        // 実際のステップ3.5と同じ抽出条件で、Activeな戦争からE2対象を探す
        // ――war9型の停止はこのフィルタで除外されるはず。
        let stalled_candidates: Vec<WarId> = war_reg
            .wars
            .values()
            .filter(|w| w.status == WarStatus::Active)
            .map(|w| w.id)
            .filter(|&id| war_has_no_valid_frontline(id, &fl_reg))
            .collect();

        assert!(
            stalled_candidates.is_empty(),
            "国境が残るwar9型の停止はE2の対象候補に含まれてはならない"
        );
        assert_eq!(
            war_reg.wars.get(&WarId(1)).unwrap().status,
            WarStatus::Active,
            "war9型の戦争はActiveのまま継続するはず(単なる無戦闘で終結しない)"
        );
    }

    /// P28-007 要求テスト(優先順位): 同じWarが同日中に降伏条件(ステップ3)と
    /// E2の国境喪失条件(ステップ3.5)の両方を満たし得る場合でも、先に処理される
    /// 降伏判定が正規の講和結果を確定させ、E2がその結果を白紙講和へ誤って
    /// 上書きしないことを確認する。ステップ3.5は「ステップ3完了後にActiveな
    /// War一覧を改めて取得し直す」ため、既に非ActiveになったWarは自然に対象外に
    /// なる。念のため、仮にE2がこのWarへ直接(誤って)適用されても
    /// `execute_peace_settlement`の二重適用防止により結果が変化しないことも
    /// あわせて確認する。
    #[test]
    fn capitulation_outcome_is_not_overwritten_by_e2_same_day() {
        use crate::state::data::StateData;
        use crate::war::frontline::update_or_create_frontline_for_war;

        // state0: 攻撃側(country0)の境界州。state1: 防御側(country1)の唯一の
        // 所有州であり、戦争目標でもある。既に攻撃側に占領されている
        // (owner=country1のまま、controller=country0)。
        let states = StateRegistry::build(vec![
            StateData {
                id: StateId(0),
                owner_country_id: CountryId(0),
                neighbors: vec![StateId(1)],
                ..Default::default()
            },
            StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                controller_country: Some(CountryId(0)),
                neighbors: vec![StateId(0)],
                ..Default::default()
            },
        ]);

        let mut war_reg = WarRegistry::default();
        war_reg.wars.insert(
            WarId(1),
            War {
                id: WarId(1),
                name: "Capitulation Priority Test War".to_string(),
                attackers: [CountryId(0)].into_iter().collect(),
                defenders: [CountryId(1)].into_iter().collect(),
                primary_attacker: None,
                primary_defender: None,
                source_crisis_id: None,
                war_goals: vec![WarGoal {
                    attacker: CountryId(0),
                    defender: CountryId(1),
                    goal_type: WarGoalType::ConquerState,
                    target_states: vec![StateId(1)],
                    base_peace_cost: 20.0,
                    international_concern: 10.0,
                    completion: 0.0,
                    is_primary: true,
                }],
                start_date: "1936/01/01".to_string(),
                end_date: None,
                duration_days: 0,
                war_score: 0.0,
                attacker_war_exhaustion: 0.0,
                defender_war_exhaustion: 0.0,
                occupied_states: std::collections::HashSet::new(),
                status: WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: std::collections::HashSet::new(),
            },
        );

        let mut fl_reg = FrontlineRegistry::default();
        let war_snapshot = war_reg.wars.get(&WarId(1)).unwrap().clone();
        update_or_create_frontline_for_war(&war_snapshot, &states, &mut fl_reg);

        // defenderは既にstate1(唯一の所有州=戦争目標)の実効支配を失っており、
        // 降伏条件(占領率100%)とE2の国境喪失条件の両方が同時に成立し得る。
        assert!(war_has_no_valid_frontline(WarId(1), &fl_reg));

        let mut mil_reg = MilitaryRegistry::default();
        let cap = crate::war::capitulation::evaluate_war_capitulation(
            war_reg.wars.get(&WarId(1)).unwrap(),
            &states,
            &mil_reg,
        );
        assert_eq!(
            cap,
            crate::war::capitulation::CapitulationResult::DefenderCapitulated,
            "占領率100%により、降伏判定(ステップ3)が成立するはず"
        );

        let mut states = states;
        let mut battle_reg = BattleRegistry::default();
        let mut dip_reg = DiplomacyRegistry::default();
        let mut crisis_reg = CrisisRegistry::default();
        let mut claim_reg = ClaimRegistry::default();
        let mut country_reg = CountryRegistry {
            countries: vec![0, 1]
                .into_iter()
                .map(|id| crate::country::CountryData {
                    id: CountryId(id),
                    capital_state_id: StateId(999),
                    ..Default::default()
                })
                .collect(),
        };
        let mut reported = ReportedPeaceSettlementErrors::default();
        let mut army_reg = crate::military::army::ArmyRegistry::default();
        let mut justification_reg = crate::war::justification::WarJustificationRegistry::default();
        let mut power_reg = crate::country::power::CountryPowerRegistry::default();

        // ステップ3: 降伏判定による正規の講和(CedeWarGoalRegion)。
        let outcome = settle_capitulation(
            WarId(1),
            PeaceTerm::CedeWarGoalRegion,
            "Defender Capitulation",
            "1936/06/01",
            &mut states,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(None),
        );
        assert_eq!(outcome, CapitulationSettlementOutcome::Settled);
        assert_eq!(
            war_reg.wars.get(&WarId(1)).unwrap().status,
            WarStatus::AttackerVictory
        );
        assert_eq!(
            war_reg.wars.get(&WarId(1)).unwrap().end_reason.as_deref(),
            Some("Defender Capitulation")
        );

        // ステップ3.5相当: Active War一覧を改めて取得し直す。このWarは既に
        // AttackerVictoryへ遷移済みのため、フィルタで自然に除外される。
        let frontline_stalled_war_ids: Vec<WarId> = war_reg
            .wars
            .values()
            .filter(|w| w.status == WarStatus::Active)
            .map(|w| w.id)
            .collect();
        assert!(
            frontline_stalled_war_ids.is_empty(),
            "降伏判定で既に終結したWarはE2の対象から除外されるはず"
        );

        // 保険として、仮にE2をこのWarへ直接(誤って)適用しても、
        // execute_peace_settlementの二重適用防止により結果が変化しないことを
        // 確認する(end_reasonが白紙講和の理由で上書きされてはならない)。
        let outcome2 = settle_capitulation(
            WarId(1),
            PeaceTerm::WhitePeace,
            FRONTLINE_UNREACHABLE_REASON,
            "1936/06/01",
            &mut states,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
            &mut reported,
            &PlayerCountry(None),
        );
        assert_eq!(outcome2, CapitulationSettlementOutcome::Settled);
        assert_eq!(
            war_reg.wars.get(&WarId(1)).unwrap().status,
            WarStatus::AttackerVictory,
            "二重適用防止により、既存の降伏結果が白紙講和へ上書きされてはならない"
        );
        assert_eq!(
            war_reg.wars.get(&WarId(1)).unwrap().end_reason.as_deref(),
            Some("Defender Capitulation"),
            "end_reasonも上書きされないはず"
        );
    }
}
