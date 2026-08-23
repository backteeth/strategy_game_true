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
/// 日次dirty化のみを行う独立System。P21-020でエラー通知系の引数(4件)が
/// `handle_daily_war_resolution`へ追加され、Bevyのfunction-system引数上限
/// (`update_recruit_button_ui`等が既に同じ理由で分離済み)へ達したため分離した。
/// `MessageReader<DayChangedMessage>`は各Systemが自分専用の読み取りカーソルを
/// 持つため、同じMessageを複数Systemで独立に読んでも問題ない(既存の
/// `handle_daily_war_prep`/`handle_daily_war_resolution`の分割と同じ前提)。
pub fn mark_ai_registries_dirty_daily(
    mut day_events: MessageReader<DayChangedMessage>,
    mut ai_registry: ResMut<crate::war::military_ai::MilitaryAiRegistry>,
    mut country_ai_registry: ResMut<crate::country::country_ai::CountryAiRegistry>,
) {
    for _event in day_events.read() {
        ai_registry.mark_all_dirty();
        country_ai_registry.mark_all_dirty();
    }
}

/// 戦争解決・後処理システム (WarResolution Set)
/// 戦闘結果の集計、戦勝点更新、降伏判定・自動講和、戦後整理を行う
/// (AI dirty化は`mark_ai_registries_dirty_daily`へ分離済み、上記コメント参照)。
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
                    match outcome {
                        CapitulationSettlementOutcome::Settled
                        | CapitulationSettlementOutcome::FailedAlreadyReported => {}
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
                                "[AutoCapitulation] {war_id:?} settlement failed (AI-only, no \
                                 player notification, will not repeat until the error or war \
                                 changes): {err_key}"
                            );
                        }
                    }
                }
            }
        }

        // 4. 無効前線・陸軍参照の整理と前線の最新状態再計算
        crate::war::frontline::update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
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
}
