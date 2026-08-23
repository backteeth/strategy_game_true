//! P21-011: Crisis最後通牒(ultimatum)への応答処理(受諾/拒否/撤回)と、
//! 宣戦布告処理との接続。既存の`can_X`/`X`検証済み変更パターンに従い、
//! 検証に失敗した場合はRegistryを一切変更しない。
//!
//! 新しい外交ルール(戦力比較によるAI受諾判断・多国間介入・複数請求の同時解決等)は
//! 実装しない。ここは既存の`CrisisPhase`状態機械・`WarJustificationRegistry`・
//! `StateRegistry::transfer_region_ownership`を接続するだけの薄い層。
//!
//! P21-013: 第三国によるCrisis支持表明(`pledge_support`/`withdraw_support`)も追加。
//! ストレージは新しいフィールドを追加せず、既存の
//! `DiplomaticCrisis::third_party_reactions: HashMap<CountryId, ThirdCountryReaction>`
//! (P21-010時点から存在するが本タスクまで一切未使用だったフィールド)をそのまま再利用する
//! (詳細はP21-013完了報告の事前調査結果を参照)。

use crate::app::time::GameDate;
use crate::common::{CountryId, DiplomaticCrisisId, StateId, WarId};
use crate::country::CountryRegistry;
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::{
    CrisisPhase, CrisisRegistry, DiplomaticCrisis, ThirdCountryReaction,
};
use crate::state::data::StateRegistry;
use crate::war::justification::WarJustificationRegistry;

/// P21-013: 第三国が支持する陣営。既存の`ThirdCountryReaction`のうち
/// `SupportsInitiator`/`SupportsTarget`だけを型安全に表現する(呼び出し側から
/// `Neutral`/`CondemnsInitiator`を誤って渡せないようにするための、この機能専用の
/// 狭い公開API層。ストレージ自体は`ThirdCountryReaction`のまま変えない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrisisSupportSide {
    Initiator,
    Target,
}

impl From<CrisisSupportSide> for ThirdCountryReaction {
    fn from(side: CrisisSupportSide) -> Self {
        match side {
            CrisisSupportSide::Initiator => ThirdCountryReaction::SupportsInitiator,
            CrisisSupportSide::Target => ThirdCountryReaction::SupportsTarget,
        }
    }
}

/// このCrisisの唯一のWarGoalが指す対象州を返す。P21-011の範囲では
/// `start_crisis`が常に単一WarGoal・単一target_stateのCrisisしか作らないため、
/// 最初の要素だけを見れば十分(`war/peace.rs::execute_peace_settlement`の
/// `target_state_id`取得と同じ規約)。
fn primary_target_state(crisis: &crate::diplomacy::crisis::DiplomaticCrisis) -> Option<StateId> {
    crisis
        .war_goals
        .first()
        .and_then(|g| g.target_states.first().copied())
}

fn validate_target_responder(
    crisis_registry: &CrisisRegistry,
    country_registry: &CountryRegistry,
    crisis_id: DiplomaticCrisisId,
    responder: CountryId,
) -> Result<(), &'static str> {
    let crisis = crisis_registry
        .crises
        .get(&crisis_id)
        .ok_or("diplomacy_error.crisis.not_found")?;
    if crisis.target != responder {
        return Err("diplomacy_error.crisis.not_your_crisis");
    }
    if crisis.current_phase != CrisisPhase::DemandSent {
        return Err("diplomacy_error.crisis.not_awaiting_response");
    }
    // P21-021: 消滅済み国家は最後通牒に応答できない(通常はapply_country_eliminationが
    // 未解決Crisisを即座にCancelledByCountryEliminationへ遷移させるため構造的にも
    // 起こり得ないが、Domain APIレベルの防御としてここでも明示的に検証する)。
    if country_registry
        .get(responder)
        .is_none_or(|c| !c.is_active())
    {
        return Err("diplomacy_error.crisis.responder_missing");
    }
    Ok(())
}

/// P21-011: 対象国が最後通牒を受諾できるかを検証する(実際には変更しない)。
pub fn can_accept_demand(
    crisis_registry: &CrisisRegistry,
    country_registry: &CountryRegistry,
    crisis_id: DiplomaticCrisisId,
    responder: CountryId,
) -> Result<(), &'static str> {
    validate_target_responder(crisis_registry, country_registry, crisis_id, responder)
}

/// P21-011: 検証込みで最後通牒を受諾する。対象州の所有権をinitiatorへ平和的に
/// 移譲し、根拠となったClaimを消費済みにし、Crisisを`ResolvedPeacefully`へ
/// 遷移させる。検証に失敗した場合、Registry(`crises`・`claims`・`states`・
/// `countries`)は一切変更しない。
///
/// P21-020: 州所有権移転は共通Domain API
/// `state::ownership::preflight_transfer_state_ownership`/`apply_ownership_transfer`
/// を経由する——対象州の現在所有国が`crisis.target`のままであることの検証と、
/// 対象州が`crisis.target`の首都だった場合の決定論的な首都再配置(残存陸州が
/// 無ければ講和自体を安全に拒否)を、War講和(`war::peace::execute_peace_settlement`)
/// と全く同じ規則で行う。所有権移転が失敗した場合はClaimを消費せず、
/// Crisisのフェーズも変更しない(以前は移転結果を無視して常にClaim消費・
/// フェーズ遷移していた既存ギャップの修正)。
///
/// P21-020(ユーザー指摘対応): 対象州を目標とする別のActive War(このCrisisとは
/// 無関係な、通常の正当化経路から始まったWar等)が既に存在する場合は受諾自体を
/// 拒否する。`execute_peace_settlement`側(War講和)は所有権移転の成立時に
/// 競合する他Warを自動的に白紙講和で失効させる設計(§後述)だが、`accept_demand`
/// は外交上の平和的受諾という性質上、無関係な第三者間のWarへ副作用として
/// 介入するのではなく、明示的に受諾を拒否して呼び出し元(プレイヤー/AI)へ
/// 判断を委ねる(Warを終わらせる権限は本来その戦争の当事国にしかない)。
/// この検証は`WarRegistry`を読み取るだけで済むため、既存の重い講和専用
/// Registry群(Military/Battle/Diplomacy/Frontline)を`accept_demand`へ
/// 追加する必要がない。
#[allow(clippy::too_many_arguments)]
pub fn accept_demand(
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &mut ClaimRegistry,
    state_registry: &mut StateRegistry,
    country_registry: &mut CountryRegistry,
    war_registry: &mut crate::war::data::WarRegistry,
    military_registry: &mut crate::military::data::MilitaryRegistry,
    army_registry: &mut crate::military::army::ArmyRegistry,
    frontline_registry: &mut crate::war::frontline::FrontlineRegistry,
    battle_registry: &mut crate::military::battle::BattleRegistry,
    diplomacy_registry: &mut crate::diplomacy::data::DiplomacyRegistry,
    justification_registry: &mut WarJustificationRegistry,
    power_registry: &mut crate::country::power::CountryPowerRegistry,
    current_date_str: &str,
    crisis_id: DiplomaticCrisisId,
    responder: CountryId,
) -> Result<(), &'static str> {
    can_accept_demand(crisis_registry, country_registry, crisis_id, responder)?;

    let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
    let initiator = crisis.initiator;
    let target = crisis.target;
    let target_state = primary_target_state(crisis);
    let related_claim_id = crisis.related_claim_id;

    if let Some(state_id) = target_state
        && war_registry.wars.values().any(|w| {
            w.status == crate::war::data::WarStatus::Active
                && w.war_goals
                    .iter()
                    .any(|g| g.target_states.contains(&state_id))
        })
    {
        return Err("diplomacy_error.crisis.target_state_contested_by_another_war");
    }

    // 実際の変更を一切行う前に、州所有権移転が可能かを検証する(全か無かの原子性)。
    // P21-021: `LastOwnedLandState`(=targetがこの州を失うと陸州0になる)が
    // 返った場合だけ、`country::elimination`の国家消滅Domain APIへフォールバック
    // する(`war::peace::execute_peace_settlement`のCedeWarGoalRegion分岐と
    // 全く同じフォールバック規則)。
    enum CessionPlan {
        Normal(crate::state::ownership::ValidatedOwnershipTransfer),
        Elimination(crate::country::elimination::CountryEliminationPlan),
    }
    let cession_plan = match target_state {
        Some(state_id) => {
            match crate::state::ownership::preflight_transfer_state_ownership(
                state_id,
                target,
                initiator,
                state_registry,
                country_registry,
            ) {
                Ok(plan) => Some(CessionPlan::Normal(plan)),
                Err(crate::state::ownership::OwnershipTransferError::LastOwnedLandState) => {
                    let elim_plan = crate::country::elimination::preflight_country_elimination(
                        state_id,
                        target,
                        initiator,
                        current_date_str,
                        state_registry,
                        country_registry,
                    )
                    .map_err(|e| e.localization_key())?;
                    Some(CessionPlan::Elimination(elim_plan))
                }
                Err(e) => return Err(e.localization_key()),
            }
        }
        None => None,
    };

    match cession_plan {
        Some(CessionPlan::Normal(plan)) => {
            crate::state::ownership::apply_ownership_transfer(
                plan,
                state_registry,
                country_registry,
            )
            .map_err(|e| e.localization_key())?;
        }
        Some(CessionPlan::Elimination(elim_plan)) => {
            // P21-021: targetの最後の所有陸州を移転すると同時に、国家消滅の全清掃を
            // 行う。このCrisis自身(`crisis_id`)は清掃ループから除外され、以下の
            // 最終ステップで通常どおり`ResolvedPeacefully`へ遷移する。
            crate::country::elimination::apply_country_elimination(
                elim_plan,
                None,
                Some(crisis_id),
                current_date_str,
                state_registry,
                country_registry,
                military_registry,
                army_registry,
                frontline_registry,
                war_registry,
                battle_registry,
                diplomacy_registry,
                crisis_registry,
                claim_registry,
                justification_registry,
                power_registry,
            )
            .map_err(|e| e.localization_key())?;
        }
        None => {}
    }
    if let Some(claim_id) = related_claim_id {
        claim_registry.mark_consumed(claim_id);
    }

    let crisis = crisis_registry.crises.get_mut(&crisis_id).unwrap();
    crisis.current_phase = CrisisPhase::ResolvedPeacefully;

    Ok(())
}

/// P21-011: 対象国が最後通牒を拒否できるかを検証する(実際には変更しない)。
pub fn can_reject_demand(
    crisis_registry: &CrisisRegistry,
    country_registry: &CountryRegistry,
    crisis_id: DiplomaticCrisisId,
    responder: CountryId,
) -> Result<(), &'static str> {
    validate_target_responder(crisis_registry, country_registry, crisis_id, responder)
}

/// P21-011: 検証込みで最後通牒を拒否する。initiatorへ即座に完成済みの
/// WarJustificationを付与し、Crisisを`Escalating`へ遷移させる。自動宣戦は
/// 行わない(対象外: 既存の宣戦UIから別途宣戦する必要がある)。検証に失敗した
/// 場合、Registryは一切変更しない。
pub fn reject_demand(
    crisis_registry: &mut CrisisRegistry,
    country_registry: &CountryRegistry,
    justification_registry: &mut WarJustificationRegistry,
    crisis_id: DiplomaticCrisisId,
    responder: CountryId,
    current_date: String,
) -> Result<(), &'static str> {
    can_reject_demand(crisis_registry, country_registry, crisis_id, responder)?;
    apply_rejection(
        crisis_registry,
        justification_registry,
        crisis_id,
        current_date,
    );
    Ok(())
}

/// P21-016: Crisisの`third_party_reactions`から、War参加国スナップショット
/// (攻撃側支持者・防御側支持者、CountryId昇順・重複なし)を計算する純粋関数。
/// 要求国・対象国自身は(本来`third_party_reactions`へ入り得ないが、防御的に)
/// 除外する。`Neutral`/`CondemnsInitiator`は含めない。
///
/// `apply_rejection`(プレイヤー拒否・AI拒否・期限切れ自動拒否の全経路が通る
/// 唯一の共有関数)からだけ呼ばれ、支持収集ロジックの複製を避ける。
fn build_support_snapshot(crisis: &DiplomaticCrisis) -> (Vec<CountryId>, Vec<CountryId>) {
    let mut attackers: Vec<CountryId> = Vec::new();
    let mut defenders: Vec<CountryId> = Vec::new();
    for (&supporter, &reaction) in crisis.third_party_reactions.iter() {
        if supporter == crisis.initiator || supporter == crisis.target {
            continue;
        }
        match reaction {
            ThirdCountryReaction::SupportsInitiator => attackers.push(supporter),
            ThirdCountryReaction::SupportsTarget => defenders.push(supporter),
            ThirdCountryReaction::Neutral | ThirdCountryReaction::CondemnsInitiator => {}
        }
    }
    attackers.sort_by_key(|id| id.0);
    defenders.sort_by_key(|id| id.0);
    (attackers, defenders)
}

/// P21-011: 期限切れによる自動拒否。`reject_demand`と同じ状態遷移を行うが、
/// 応答者の検証は行わない(日次進行システムが期限だけを見て呼ぶ)。
/// 呼び出し元(`diplomacy::update::handle_daily_diplomacy`)が`DemandSent`かつ
/// 期限到達済みであることを事前に確認してから呼ぶ。
///
/// P21-016: プレイヤー拒否(`reject_demand`経由)・AI拒否(P21-012、同じく
/// `reject_demand`経由)・期限切れ自動拒否(この関数への直接呼び出し)の
/// **全経路がこの1つの関数を通る**ため、ここでCrisis支持コミットメントの
/// スナップショット(`build_support_snapshot`)を1箇所だけで確定保存する。
pub(crate) fn apply_rejection(
    crisis_registry: &mut CrisisRegistry,
    justification_registry: &mut WarJustificationRegistry,
    crisis_id: DiplomaticCrisisId,
    current_date: String,
) {
    let Some(crisis) = crisis_registry.crises.get(&crisis_id) else {
        return;
    };
    let initiator = crisis.initiator;
    let target = crisis.target;
    let target_state = primary_target_state(crisis);
    let (committed_attackers, committed_defenders) = build_support_snapshot(crisis);

    let justification_id = target_state.map(|state_id| {
        justification_registry.grant_completed_justification(
            initiator,
            target,
            state_id,
            current_date,
            Some(crisis_id),
            committed_attackers,
            committed_defenders,
        )
    });

    let crisis = crisis_registry.crises.get_mut(&crisis_id).unwrap();
    crisis.current_phase = CrisisPhase::Escalating;
    crisis.related_justification_id = justification_id;
}

/// P21-011: initiatorがCrisisを撤回できるかを検証する(実際には変更しない)。
/// 終結済み(ResolvedPeacefully/WarStarted/Cancelled)は撤回できない。
pub fn can_withdraw_crisis(
    crisis_registry: &CrisisRegistry,
    country_registry: &CountryRegistry,
    crisis_id: DiplomaticCrisisId,
    initiator: CountryId,
) -> Result<(), &'static str> {
    let crisis = crisis_registry
        .crises
        .get(&crisis_id)
        .ok_or("diplomacy_error.crisis.not_found")?;
    if crisis.initiator != initiator {
        return Err("diplomacy_error.crisis.not_your_crisis");
    }
    if matches!(
        crisis.current_phase,
        CrisisPhase::ResolvedPeacefully | CrisisPhase::WarStarted | CrisisPhase::Cancelled
    ) {
        return Err("diplomacy_error.crisis.already_resolved");
    }
    // P21-021: 消滅済み国家はCrisisを撤回できない(通常はapply_country_eliminationが
    // 未解決Crisisを即座にCancelledByCountryEliminationへ遷移させるため構造的にも
    // 起こり得ないが、Domain APIレベルの防御としてここでも明示的に検証する)。
    if country_registry
        .get(initiator)
        .is_none_or(|c| !c.is_active())
    {
        return Err("diplomacy_error.crisis.initiator_missing");
    }
    Ok(())
}

/// P21-011: 検証込みでCrisisを撤回する。`DemandSent`(要求中)・`Escalating`
/// (拒否済みで正当化保持中)のどちらからでも撤回でき、`Escalating`から撤回する
/// 場合は付与済みのWarJustificationも取り消す。Crisisを`Cancelled`へ遷移させる。
/// 検証に失敗した場合、Registryは一切変更しない。
pub fn withdraw_crisis(
    crisis_registry: &mut CrisisRegistry,
    country_registry: &CountryRegistry,
    justification_registry: &mut WarJustificationRegistry,
    crisis_id: DiplomaticCrisisId,
    initiator: CountryId,
) -> Result<(), &'static str> {
    can_withdraw_crisis(crisis_registry, country_registry, crisis_id, initiator)?;

    let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
    let related_justification_id = crisis.related_justification_id;

    if let Some(j_id) = related_justification_id {
        justification_registry.cancel_justification(j_id);
    }

    let crisis = crisis_registry.crises.get_mut(&crisis_id).unwrap();
    crisis.current_phase = CrisisPhase::Cancelled;
    crisis.related_justification_id = None;

    Ok(())
}

/// P21-011: 既存の宣戦布告処理(`WarRegistry::declare_war`、UI/AI双方の呼び出し元)から
/// 呼ばれる。`declare_war`が消費する直前のWarJustification idが、`Escalating`
/// フェーズのCrisisに紐づく`related_justification_id`と一致する場合のみ、その
/// Crisisを`WarStarted`へ遷移させる。一致するCrisisがなければ何もしない
/// (Crisis経由でない通常の宣戦布告は対象外)。
pub fn sync_crisis_on_war_declared(
    crisis_registry: &mut CrisisRegistry,
    initiator: CountryId,
    target: CountryId,
    justification_id: usize,
    war_id: WarId,
) {
    if let Some(crisis) = crisis_registry.crises.values_mut().find(|c| {
        c.initiator == initiator
            && c.target == target
            && c.current_phase == CrisisPhase::Escalating
            && c.related_justification_id == Some(justification_id)
    }) {
        crisis.current_phase = CrisisPhase::WarStarted;
        // declare_warが直前にjustification自体をRegistryから消費(削除)済みのため、
        // もはや存在しないidを指したまま残すとsave検証がdangling referenceとして
        // 拒否してしまう。以後は`related_war_id`だけを「戦争へ繋がった」記録として残す。
        crisis.related_justification_id = None;
        crisis.related_war_id = Some(war_id);
    }
}

/// P21-013修正: このCrisisの要求期限(`deadline_date`)に、指定日時点で既に到達しているか。
/// `evaluate_ai_crisis_responses`(`diplomacy::update`)の期限判定と同じ規約: 期限文字列が
/// 存在せず、または解析できない場合は「期限なし」として扱い、`false`(未到達)を返す
/// (`start_crisis`が開始日を解析できなかった場合の保険的`None`と同じ扱い)。
fn crisis_demand_has_expired(
    crisis: &crate::diplomacy::crisis::DiplomaticCrisis,
    current_date: &GameDate,
) -> bool {
    crisis
        .deadline_date
        .as_deref()
        .and_then(GameDate::from_string)
        .is_some_and(|deadline| current_date.is_at_least(&deadline))
}

/// P21-013修正: このCrisisが今、第三国支持の表明/撤回を受け付けられる状態かどうか。
/// `DemandSent`フェーズ**かつ**要求期限に未到達であることの両方を要求する。
///
/// 以前は`current_phase == DemandSent`のみを見ており、「期限到達後は日次タイムアウト処理が
/// 同一tick内で先にフェーズを進める」という前提に依存していた。この前提はロード直後
/// (`deadline_date`が既にロード時点の`GameDate`以前だが、次の`DayChangedMessage`が
/// まだ一度も発火していないためフェーズはまだ`DemandSent`のまま)には成立せず、
/// 期限超過後でも支持/撤回が一時的に可能になってしまう抜けがあった。ドメインAPI
/// (この関数)とUI([`crate::ui::diplomacy_panel`]のボタン表示条件)の両方がこの関数を
/// 直接呼ぶことで、判定基準を1箇所に統一する。
pub(crate) fn crisis_is_awaiting_support_response(
    crisis: &crate::diplomacy::crisis::DiplomaticCrisis,
    current_date: &GameDate,
) -> bool {
    crisis.current_phase == CrisisPhase::DemandSent
        && !crisis_demand_has_expired(crisis, current_date)
}

/// P21-013: 第三国がCrisisへ支持を表明できるかを検証する(実際には変更しない)。
/// 当事国(initiator/target)自身は支持国になれない。存在しない国家も不可。
/// `DemandSent`フェーズかつ要求期限未到達の場合のみ許可する
/// (`crisis_is_awaiting_support_response`参照)。既に同じ側を支持済みなら
/// 冪等に成功扱いとし、反対側を支持済みの場合は直接切り替えず拒否する
/// (先に`withdraw_support`で撤回させる)。
pub fn can_pledge_support(
    crisis_registry: &CrisisRegistry,
    country_registry: &CountryRegistry,
    crisis_id: DiplomaticCrisisId,
    supporter: CountryId,
    side: CrisisSupportSide,
    current_date: &GameDate,
) -> Result<(), &'static str> {
    let crisis = crisis_registry
        .crises
        .get(&crisis_id)
        .ok_or("diplomacy_error.crisis.not_found")?;
    if crisis.initiator == supporter || crisis.target == supporter {
        return Err("diplomacy_error.crisis.support_self");
    }
    // P21-021: 消滅済み(Eliminated)国家は新たにCrisis支持を表明できない
    // (§10: Domain APIレベルでの拒否)。
    if country_registry
        .get(supporter)
        .is_none_or(|c| !c.is_active())
    {
        return Err("diplomacy_error.crisis.support_country_missing");
    }
    if !crisis_is_awaiting_support_response(crisis, current_date) {
        return Err("diplomacy_error.crisis.not_awaiting_response");
    }
    let reaction = ThirdCountryReaction::from(side);
    if let Some(&existing) = crisis.third_party_reactions.get(&supporter)
        && existing != reaction
    {
        return Err("diplomacy_error.crisis.support_side_conflict");
    }
    Ok(())
}

/// P21-013: 検証込みで第三国支持を記録する。ストレージは既存の
/// `DiplomaticCrisis::third_party_reactions`をそのまま使う(新フィールド追加なし)。
/// 同じ側への再表明は冪等(HashMapへの上書き挿入は同値のため実質no-op)。検証に
/// 失敗した場合、Registryは一切変更しない。
pub fn pledge_support(
    crisis_registry: &mut CrisisRegistry,
    country_registry: &CountryRegistry,
    crisis_id: DiplomaticCrisisId,
    supporter: CountryId,
    side: CrisisSupportSide,
    current_date: &GameDate,
) -> Result<(), &'static str> {
    can_pledge_support(
        crisis_registry,
        country_registry,
        crisis_id,
        supporter,
        side,
        current_date,
    )?;
    let crisis = crisis_registry.crises.get_mut(&crisis_id).unwrap();
    crisis.third_party_reactions.insert(supporter, side.into());
    Ok(())
}

/// P21-013: 第三国が自分自身の支持を撤回できるかを検証する(実際には変更しない)。
/// `DemandSent`フェーズかつ要求期限未到達の場合のみ許可する
/// (`can_pledge_support`/`crisis_is_awaiting_support_response`と同じ規約)。撤回できるのは
/// 呼び出し元が渡した`supporter`自身の支持のみ(他国の支持は`supporter`引数として
/// 渡すこと自体を呼び出し元[UI]が行わない、という既存コードベース全体の呼び出し規約に
/// 従う — `withdraw_crisis`の`initiator`引数等と同じモデル)。
pub fn can_withdraw_support(
    crisis_registry: &CrisisRegistry,
    country_registry: &CountryRegistry,
    crisis_id: DiplomaticCrisisId,
    supporter: CountryId,
    current_date: &GameDate,
) -> Result<(), &'static str> {
    let crisis = crisis_registry
        .crises
        .get(&crisis_id)
        .ok_or("diplomacy_error.crisis.not_found")?;
    if !crisis_is_awaiting_support_response(crisis, current_date) {
        return Err("diplomacy_error.crisis.not_awaiting_response");
    }
    // P21-021: 消滅済み国家は支持撤回もできない(通常はapply_country_eliminationが
    // 消滅国家自身のthird_party_reactionsエントリを全Crisisから即座に除去するため
    // 構造的にも起こり得ないが、Domain APIレベルの防御としてここでも明示的に検証する)。
    if country_registry
        .get(supporter)
        .is_none_or(|c| !c.is_active())
    {
        return Err("diplomacy_error.crisis.support_country_missing");
    }
    if !crisis.third_party_reactions.contains_key(&supporter) {
        return Err("diplomacy_error.crisis.support_not_found");
    }
    Ok(())
}

/// P21-013: 検証込みで第三国支持を撤回する。検証に失敗した場合、Registryは
/// 一切変更しない。撤回後、同じCrisisへ反対側を新たに支持できる
/// (`can_pledge_support`はエントリが存在しなければ`support_side_conflict`を
/// 返さないため)。
pub fn withdraw_support(
    crisis_registry: &mut CrisisRegistry,
    country_registry: &CountryRegistry,
    crisis_id: DiplomaticCrisisId,
    supporter: CountryId,
    current_date: &GameDate,
) -> Result<(), &'static str> {
    can_withdraw_support(
        crisis_registry,
        country_registry,
        crisis_id,
        supporter,
        current_date,
    )?;
    let crisis = crisis_registry.crises.get_mut(&crisis_id).unwrap();
    crisis.third_party_reactions.remove(&supporter);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diplomacy::claims::{ClaimSource, ClaimStatus, TerritorialClaim};
    use crate::diplomacy::crisis::WarGoal;
    use crate::state::data::StateData;
    use crate::war::data::WarRegistry;

    fn make_state_registry(state_id: usize, owner: usize) -> StateRegistry {
        StateRegistry::build(vec![StateData {
            id: StateId(state_id),
            owner_country_id: CountryId(owner),
            ..Default::default()
        }])
    }

    fn seed_claim(
        claim_registry: &mut ClaimRegistry,
        claimant: usize,
        target_state: usize,
    ) -> crate::common::ClaimId {
        claim_registry.add_claim(TerritorialClaim {
            id: crate::common::ClaimId(0),
            claimant_country: CountryId(claimant),
            target_state: StateId(target_state),
            strength: 50.0,
            created_date: "1800/01/01".to_string(),
            is_permanent: false,
            source: ClaimSource::Strategic,
            status: ClaimStatus::Active,
        })
    }

    fn seed_demand_sent_crisis(
        crisis_registry: &mut CrisisRegistry,
        initiator: usize,
        target: usize,
        target_state: usize,
        related_claim_id: Option<crate::common::ClaimId>,
    ) -> DiplomaticCrisisId {
        crisis_registry.add_crisis(crate::diplomacy::crisis::DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(initiator),
            target: CountryId(target),
            war_goals: vec![WarGoal {
                attacker: CountryId(initiator),
                defender: CountryId(target),
                goal_type: crate::diplomacy::crisis::WarGoalType::ConquerState,
                target_states: vec![StateId(target_state)],
                base_peace_cost: 0.0,
                international_concern: 0.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: "1800/01/01".to_string(),
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1800/01/31".to_string()),
            international_concern: 0.0,
            third_party_reactions: Default::default(),
            related_claim_id,
            related_justification_id: None,
            related_war_id: None,
        })
    }

    /// 要求テスト: 受諾により州所有権が移転し、Claimが消費され、フェーズが遷移する。
    #[test]
    fn accept_demand_transfers_state_consumes_claim_and_resolves() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut claim_registry = ClaimRegistry::default();
        let mut state_registry = make_state_registry(1, 1);
        let mut country_registry = countries_with(&[0, 1]);

        let claim_id = seed_claim(&mut claim_registry, 0, 1);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, Some(claim_id));

        accept_demand(
            &mut crisis_registry,
            &mut claim_registry,
            &mut state_registry,
            &mut country_registry,
            &mut WarRegistry::default(),
            &mut crate::military::data::MilitaryRegistry::default(),
            &mut crate::military::army::ArmyRegistry::default(),
            &mut crate::war::frontline::FrontlineRegistry::default(),
            &mut crate::military::battle::BattleRegistry::default(),
            &mut crate::diplomacy::data::DiplomacyRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut crate::country::power::CountryPowerRegistry::default(),
            "1800/01/01",
            crisis_id,
            CountryId(1),
        )
        .expect("target must be able to accept an awaiting demand");

        assert_eq!(
            state_registry.get(StateId(1)).unwrap().owner_country_id,
            CountryId(0)
        );
        assert_eq!(
            claim_registry.claims.get(&claim_id).unwrap().status,
            ClaimStatus::Consumed
        );
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::ResolvedPeacefully
        );
    }

    /// P21-020(ユーザー指摘対応): 対象州を目標とする別のActive Warが既に
    /// 存在する場合、要求受諾自体を拒否する(複数Warの同一州競合対策の
    /// うち`accept_demand`側の選択)。Registryは一切変更しない。
    #[test]
    fn accept_demand_rejects_when_target_state_is_contested_by_another_active_war() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut claim_registry = ClaimRegistry::default();
        let mut state_registry = make_state_registry(1, 1);
        let mut country_registry = countries_with(&[0, 1]);

        let claim_id = seed_claim(&mut claim_registry, 0, 1);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, Some(claim_id));

        // StateId(1)を目標とする、このCrisisとは無関係な別のActive Warが
        // 既に存在する。
        let mut war_registry = WarRegistry::default();
        war_registry.wars.insert(
            crate::common::WarId(0),
            crate::war::data::War {
                id: crate::common::WarId(0),
                name: "Unrelated War".to_string(),
                attackers: [CountryId(9)].into_iter().collect(),
                defenders: [CountryId(1)].into_iter().collect(),
                primary_attacker: None,
                primary_defender: None,
                source_crisis_id: None,
                war_goals: vec![crate::diplomacy::crisis::WarGoal {
                    attacker: CountryId(9),
                    defender: CountryId(1),
                    goal_type: crate::diplomacy::crisis::WarGoalType::ConquerState,
                    target_states: vec![StateId(1)],
                    base_peace_cost: 20.0,
                    international_concern: 10.0,
                    completion: 0.0,
                    is_primary: true,
                }],
                start_date: "1800/01/01".to_string(),
                end_date: None,
                duration_days: 0,
                war_score: 0.0,
                attacker_war_exhaustion: 0.0,
                defender_war_exhaustion: 0.0,
                occupied_states: std::collections::HashSet::new(),
                status: crate::war::data::WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: std::collections::HashSet::new(),
            },
        );

        let result = accept_demand(
            &mut crisis_registry,
            &mut claim_registry,
            &mut state_registry,
            &mut country_registry,
            &mut war_registry,
            &mut crate::military::data::MilitaryRegistry::default(),
            &mut crate::military::army::ArmyRegistry::default(),
            &mut crate::war::frontline::FrontlineRegistry::default(),
            &mut crate::military::battle::BattleRegistry::default(),
            &mut crate::diplomacy::data::DiplomacyRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut crate::country::power::CountryPowerRegistry::default(),
            "1800/01/01",
            crisis_id,
            CountryId(1),
        );

        assert_eq!(
            result,
            Err("diplomacy_error.crisis.target_state_contested_by_another_war")
        );
        assert_eq!(
            state_registry.get(StateId(1)).unwrap().owner_country_id,
            CountryId(1),
            "failed accept must not mutate state ownership"
        );
        assert_eq!(
            claim_registry.claims.get(&claim_id).unwrap().status,
            ClaimStatus::Active,
            "failed accept must not consume the claim"
        );
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::DemandSent,
            "failed accept must not transition the crisis phase"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-020: accept_demandが共通所有権移転Domain APIを経由することの検証
    // (§14要求18,32,33,34,43)。
    // ─────────────────────────────────────────────────────────────────────

    /// 要求テスト33: 要求受諾で対象州がtargetの首都だった場合、決定論的に
    /// 残存陸州(最小StateId)へ再配置される。
    #[test]
    fn accept_demand_reassigns_capital_when_ceding_the_targets_capital() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut claim_registry = ClaimRegistry::default();
        let mut state_registry = StateRegistry::build(vec![
            StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                ..Default::default()
            },
            StateData {
                id: StateId(2),
                owner_country_id: CountryId(1),
                ..Default::default()
            },
        ]);
        let mut country_registry = CountryRegistry {
            countries: vec![
                crate::country::CountryData {
                    id: CountryId(0),
                    ..Default::default()
                },
                crate::country::CountryData {
                    id: CountryId(1),
                    capital_state_id: StateId(1), // 割譲対象州が首都
                    ..Default::default()
                },
            ],
        };

        let claim_id = seed_claim(&mut claim_registry, 0, 1);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, Some(claim_id));

        accept_demand(
            &mut crisis_registry,
            &mut claim_registry,
            &mut state_registry,
            &mut country_registry,
            &mut WarRegistry::default(),
            &mut crate::military::data::MilitaryRegistry::default(),
            &mut crate::military::army::ArmyRegistry::default(),
            &mut crate::war::frontline::FrontlineRegistry::default(),
            &mut crate::military::battle::BattleRegistry::default(),
            &mut crate::diplomacy::data::DiplomacyRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut crate::country::power::CountryPowerRegistry::default(),
            "1800/01/01",
            crisis_id,
            CountryId(1),
        )
        .expect("ceding the capital must still succeed when another land state remains");

        assert_eq!(
            state_registry.get(StateId(1)).unwrap().owner_country_id,
            CountryId(0)
        );
        assert_eq!(
            country_registry.get(CountryId(1)).unwrap().capital_state_id,
            StateId(2),
            "the target's capital must be reassigned to its remaining land state"
        );
        assert_eq!(
            claim_registry.claims.get(&claim_id).unwrap().status,
            ClaimStatus::Consumed
        );
    }

    /// P21-021要求: targetが対象州以外に陸州を一切持たない場合、要求受諾は
    /// 「安全に拒否」ではなく国家消滅(完全併合)をトリガーする——対象州は
    /// initiatorへ移転し、target(CountryId(1))は`CountryStatus::Eliminated`
    /// へ遷移し(`CountryRegistry`からは物理削除されない)、Claimは消費され、
    /// Crisis自身は`ResolvedPeacefully`(`CancelledByCountryElimination`では
    /// ない——受諾成功というこのCrisis自身の結末を、国家消滅清掃が上書きしない)。
    /// P21-020時点の旧挙動("safely rejected")はこのテストが意図的に反転させる
    /// (旧テスト名・旧アサーションは本タスクの対象外仕様だったため置き換える)。
    #[test]
    fn accept_demand_with_no_remaining_land_state_triggers_country_elimination() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut claim_registry = ClaimRegistry::default();
        let mut state_registry = StateRegistry::build(vec![StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            ..Default::default()
        }]);
        let mut country_registry = CountryRegistry {
            countries: vec![
                crate::country::CountryData {
                    id: CountryId(0),
                    ..Default::default()
                },
                crate::country::CountryData {
                    id: CountryId(1),
                    capital_state_id: StateId(1), // 唯一の所有陸州かつ首都
                    ..Default::default()
                },
            ],
        };

        let claim_id = seed_claim(&mut claim_registry, 0, 1);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, Some(claim_id));

        accept_demand(
            &mut crisis_registry,
            &mut claim_registry,
            &mut state_registry,
            &mut country_registry,
            &mut WarRegistry::default(),
            &mut crate::military::data::MilitaryRegistry::default(),
            &mut crate::military::army::ArmyRegistry::default(),
            &mut crate::war::frontline::FrontlineRegistry::default(),
            &mut crate::military::battle::BattleRegistry::default(),
            &mut crate::diplomacy::data::DiplomacyRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut crate::country::power::CountryPowerRegistry::default(),
            "1800/06/15",
            crisis_id,
            CountryId(1),
        )
        .expect("accepting a demand for the target's last land state must trigger elimination, not fail");

        assert_eq!(
            state_registry.get(StateId(1)).unwrap().owner_country_id,
            CountryId(0),
            "the final state must transfer to the initiator"
        );
        let eliminated = country_registry.get(CountryId(1)).unwrap();
        assert!(
            eliminated.is_eliminated(),
            "the target must transition to Eliminated, not be dropped from the registry"
        );
        assert_eq!(
            eliminated.status,
            crate::country::CountryStatus::Eliminated {
                eliminated_date: "1800/06/15".to_string(),
                annexed_by: CountryId(0),
            }
        );
        assert_eq!(
            claim_registry.claims.get(&claim_id).unwrap().status,
            ClaimStatus::Consumed,
            "the fulfilling claim must be marked Consumed (not Invalidated, which is for the eliminated country's own outgoing claims)"
        );
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::ResolvedPeacefully,
            "this crisis resolved peacefully via acceptance; elimination cleanup must not overwrite it to CancelledByCountryElimination"
        );
    }

    /// 要求テスト: initiator以外(第三者)は受諾できない。
    #[test]
    fn accept_demand_rejects_non_target_responder() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut claim_registry = ClaimRegistry::default();
        let mut state_registry = make_state_registry(1, 1);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        let result = accept_demand(
            &mut crisis_registry,
            &mut claim_registry,
            &mut state_registry,
            &mut CountryRegistry::default(),
            &mut WarRegistry::default(),
            &mut crate::military::data::MilitaryRegistry::default(),
            &mut crate::military::army::ArmyRegistry::default(),
            &mut crate::war::frontline::FrontlineRegistry::default(),
            &mut crate::military::battle::BattleRegistry::default(),
            &mut crate::diplomacy::data::DiplomacyRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut crate::country::power::CountryPowerRegistry::default(),
            "1800/01/01",
            crisis_id,
            CountryId(0), // initiator自身、targetではない
        );

        assert_eq!(result, Err("diplomacy_error.crisis.not_your_crisis"));
        assert_eq!(
            state_registry.get(StateId(1)).unwrap().owner_country_id,
            CountryId(1),
            "failed accept must not mutate state ownership"
        );
    }

    /// 要求テスト: DemandSent以外(例: Escalating)は受諾できない。
    #[test]
    fn accept_demand_rejects_when_not_awaiting_response() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut claim_registry = ClaimRegistry::default();
        let mut state_registry = make_state_registry(1, 1);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        crisis_registry
            .crises
            .get_mut(&crisis_id)
            .unwrap()
            .current_phase = CrisisPhase::Escalating;

        let result = accept_demand(
            &mut crisis_registry,
            &mut claim_registry,
            &mut state_registry,
            &mut CountryRegistry::default(),
            &mut WarRegistry::default(),
            &mut crate::military::data::MilitaryRegistry::default(),
            &mut crate::military::army::ArmyRegistry::default(),
            &mut crate::war::frontline::FrontlineRegistry::default(),
            &mut crate::military::battle::BattleRegistry::default(),
            &mut crate::diplomacy::data::DiplomacyRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut crate::country::power::CountryPowerRegistry::default(),
            "1800/01/01",
            crisis_id,
            CountryId(1),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.not_awaiting_response"));
    }

    /// 要求テスト: 拒否によりinitiatorへ完成済みJustificationが付与され、Escalatingへ遷移する。
    #[test]
    fn reject_demand_grants_completed_justification_and_escalates() {
        let mut crisis_registry = CrisisRegistry::default();
        let country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        reject_demand(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(1),
            "1800/01/15".to_string(),
        )
        .expect("target must be able to reject an awaiting demand");

        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        assert_eq!(crisis.current_phase, CrisisPhase::Escalating);
        let j_id = crisis
            .related_justification_id
            .expect("rejection must grant a justification id");
        let justification = justification_registry.justifications.get(&j_id).unwrap();
        assert!(
            justification.is_ready,
            "granted justification must be immediately ready"
        );
        assert_eq!(justification.initiator, CountryId(0));
        assert_eq!(justification.target, CountryId(1));
    }

    /// P21-016要求テスト: 拒否より前に第三国が表明していた支持が、付与される
    /// WarJustificationのスナップショット(`source_crisis_id`/`committed_attackers`/
    /// `committed_defenders`)へ正しく引き継がれる。
    #[test]
    fn reject_demand_captures_support_snapshot_into_granted_justification() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut justification_registry = WarJustificationRegistry::default();
        let countries = countries_with(&[0, 1, 2, 3]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        )
        .unwrap();
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(3),
            CrisisSupportSide::Target,
            &before_deadline(),
        )
        .unwrap();

        reject_demand(
            &mut crisis_registry,
            &countries,
            &mut justification_registry,
            crisis_id,
            CountryId(1),
            "1800/01/15".to_string(),
        )
        .unwrap();

        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        let j_id = crisis.related_justification_id.unwrap();
        let justification = justification_registry.justifications.get(&j_id).unwrap();
        assert_eq!(justification.source_crisis_id, Some(crisis_id));
        assert_eq!(justification.committed_attackers, vec![CountryId(2)]);
        assert_eq!(justification.committed_defenders, vec![CountryId(3)]);
    }

    /// P21-016要求テスト: `Neutral`/`CondemnsInitiator`はスナップショットへ含まれない。
    #[test]
    fn reject_demand_snapshot_excludes_neutral_and_condemning_reactions() {
        let mut crisis_registry = CrisisRegistry::default();
        let country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        crisis_registry
            .crises
            .get_mut(&crisis_id)
            .unwrap()
            .third_party_reactions
            .insert(CountryId(2), ThirdCountryReaction::Neutral);
        crisis_registry
            .crises
            .get_mut(&crisis_id)
            .unwrap()
            .third_party_reactions
            .insert(CountryId(3), ThirdCountryReaction::CondemnsInitiator);

        reject_demand(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(1),
            "1800/01/15".to_string(),
        )
        .unwrap();

        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        let j_id = crisis.related_justification_id.unwrap();
        let justification = justification_registry.justifications.get(&j_id).unwrap();
        assert!(justification.committed_attackers.is_empty());
        assert!(justification.committed_defenders.is_empty());
    }

    /// P21-016要求テスト: スナップショットは`CountryId`昇順・重複なしで格納される
    /// (`HashMap`の走査順に依存しない決定的な出力を保証する)。
    #[test]
    fn reject_demand_snapshot_is_sorted_by_country_id_ascending() {
        let mut crisis_registry = CrisisRegistry::default();
        let country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        // 挿入順をわざと降順にして、出力が挿入順ではなくID順であることを確認する。
        for id in [9usize, 4, 7] {
            crisis_registry
                .crises
                .get_mut(&crisis_id)
                .unwrap()
                .third_party_reactions
                .insert(CountryId(id), ThirdCountryReaction::SupportsInitiator);
        }

        reject_demand(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(1),
            "1800/01/15".to_string(),
        )
        .unwrap();

        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        let j_id = crisis.related_justification_id.unwrap();
        let justification = justification_registry.justifications.get(&j_id).unwrap();
        assert_eq!(
            justification.committed_attackers,
            vec![CountryId(4), CountryId(7), CountryId(9)]
        );
    }

    /// P21-021: 消滅済み国家はDomain APIレベルで最後通牒に応答(拒否)できない
    /// (UIのrun_ifだけに依存しない直接検証)。
    #[test]
    fn reject_demand_rejects_an_eliminated_responder() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        country_registry.get_mut(CountryId(1)).unwrap().status =
            crate::country::CountryStatus::Eliminated {
                eliminated_date: "1799/12/01".to_string(),
                annexed_by: CountryId(9),
            };

        let result = reject_demand(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(1),
            "1800/01/15".to_string(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.responder_missing"));
        assert!(justification_registry.justifications.is_empty());
    }

    /// P21-021: 消滅済み国家はDomain APIレベルで最後通牒を受諾もできない。
    #[test]
    fn can_accept_demand_rejects_an_eliminated_responder() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut country_registry = countries_with(&[0, 1]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        country_registry.get_mut(CountryId(1)).unwrap().status =
            crate::country::CountryStatus::Eliminated {
                eliminated_date: "1799/12/01".to_string(),
                annexed_by: CountryId(9),
            };

        let result =
            can_accept_demand(&crisis_registry, &country_registry, crisis_id, CountryId(1));

        assert_eq!(result, Err("diplomacy_error.crisis.responder_missing"));
    }

    /// 要求テスト: 拒否は対象国以外からは行えない。
    #[test]
    fn reject_demand_rejects_non_target_responder() {
        let mut crisis_registry = CrisisRegistry::default();
        let country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        let result = reject_demand(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(0),
            "1800/01/15".to_string(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.not_your_crisis"));
        assert!(justification_registry.justifications.is_empty());
    }

    /// 要求テスト: DemandSent中の撤回はCancelledへ遷移し、Justificationは存在しない(まだ拒否されていない)。
    #[test]
    fn withdraw_during_demand_sent_cancels_without_justification() {
        let mut crisis_registry = CrisisRegistry::default();
        let country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        withdraw_crisis(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(0),
        )
        .expect("initiator must be able to withdraw an awaiting demand");

        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::Cancelled
        );
    }

    /// 要求テスト: Escalating中の撤回は付与済みJustificationを取り消す。
    #[test]
    fn withdraw_during_escalating_cancels_granted_justification() {
        let mut crisis_registry = CrisisRegistry::default();
        let country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        reject_demand(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(1),
            "1800/01/15".to_string(),
        )
        .unwrap();
        let j_id = crisis_registry
            .crises
            .get(&crisis_id)
            .unwrap()
            .related_justification_id
            .unwrap();
        assert!(justification_registry.justifications.contains_key(&j_id));

        withdraw_crisis(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(0),
        )
        .unwrap();

        assert!(
            !justification_registry.justifications.contains_key(&j_id),
            "withdraw must cancel the justification granted by the rejection"
        );
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .related_justification_id,
            None
        );
    }

    /// 要求テスト: 終結済み(ResolvedPeacefully等)のCrisisは撤回できない。
    #[test]
    fn withdraw_rejects_already_resolved_crisis() {
        let mut crisis_registry = CrisisRegistry::default();
        let country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        crisis_registry
            .crises
            .get_mut(&crisis_id)
            .unwrap()
            .current_phase = CrisisPhase::ResolvedPeacefully;

        let result = withdraw_crisis(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(0),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.already_resolved"));
    }

    /// P21-021: 消滅済み国家はDomain APIレベルでCrisisを撤回できない
    /// (UIのrun_ifだけに依存しない直接検証)。
    #[test]
    fn withdraw_crisis_rejects_an_eliminated_initiator() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        country_registry.get_mut(CountryId(0)).unwrap().status =
            crate::country::CountryStatus::Eliminated {
                eliminated_date: "1799/12/01".to_string(),
                annexed_by: CountryId(9),
            };

        let result = withdraw_crisis(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(0),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.initiator_missing"));
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::DemandSent,
            "the crisis must survive rejection unchanged"
        );
    }

    /// 要求テスト: 撤回は当事国(initiator)以外からは行えない。
    #[test]
    fn withdraw_rejects_non_initiator() {
        let mut crisis_registry = CrisisRegistry::default();
        let country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        let result = withdraw_crisis(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(1),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.not_your_crisis"));
    }

    /// 要求テスト: 宣戦布告成功時、一致するEscalating中のCrisisがWarStartedへ遷移する。
    #[test]
    fn sync_crisis_on_war_declared_transitions_matching_crisis() {
        let mut crisis_registry = CrisisRegistry::default();
        let country_registry = countries_with(&[0, 1]);
        let mut justification_registry = WarJustificationRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        reject_demand(
            &mut crisis_registry,
            &country_registry,
            &mut justification_registry,
            crisis_id,
            CountryId(1),
            "1800/01/15".to_string(),
        )
        .unwrap();
        let j_id = crisis_registry
            .crises
            .get(&crisis_id)
            .unwrap()
            .related_justification_id
            .unwrap();

        sync_crisis_on_war_declared(
            &mut crisis_registry,
            CountryId(0),
            CountryId(1),
            j_id,
            WarId(7),
        );

        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        assert_eq!(crisis.current_phase, CrisisPhase::WarStarted);
        assert_eq!(crisis.related_war_id, Some(WarId(7)));
        assert_eq!(
            crisis.related_justification_id, None,
            "the justification id must be cleared once war starts (declare_war already consumed it)"
        );
    }

    /// 要求テスト: 一致するCrisisがない場合は何も変更しない(Crisis経由でない通常宣戦)。
    #[test]
    fn sync_crisis_on_war_declared_is_noop_when_no_matching_crisis() {
        let mut crisis_registry = CrisisRegistry::default();
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        sync_crisis_on_war_declared(
            &mut crisis_registry,
            CountryId(0),
            CountryId(1),
            999,
            WarId(7),
        );

        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        assert_eq!(
            crisis.current_phase,
            CrisisPhase::DemandSent,
            "an unrelated war declaration must not touch an unrelated crisis"
        );
        assert_eq!(crisis.related_war_id, None);
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-013: 第三国Crisis支持のテスト
    // ─────────────────────────────────────────────────────────────────────

    fn countries_with(ids: &[usize]) -> CountryRegistry {
        CountryRegistry {
            countries: ids
                .iter()
                .map(|&id| crate::country::CountryData {
                    id: CountryId(id),
                    ..Default::default()
                })
                .collect(),
        }
    }

    /// `seed_demand_sent_crisis`が付与する`deadline_date`("1800/01/31")より前の日付。
    fn before_deadline() -> GameDate {
        GameDate::new(1800, 1, 15)
    }

    /// `seed_demand_sent_crisis`が付与する`deadline_date`("1800/01/31")そのものの日付。
    fn on_deadline_day() -> GameDate {
        GameDate::new(1800, 1, 31)
    }

    /// `seed_demand_sent_crisis`が付与する`deadline_date`("1800/01/31")より後の日付。
    fn after_deadline() -> GameDate {
        GameDate::new(1800, 2, 5)
    }

    /// 要求テスト: 第三国が要求国側を支持できる。
    #[test]
    fn pledge_support_allows_third_country_to_support_initiator() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1, 2]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        )
        .expect("a genuine third country must be able to support the initiator");

        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        assert_eq!(
            crisis.third_party_reactions.get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsInitiator)
        );
    }

    /// 要求テスト: 第三国が対象国側を支持できる。
    #[test]
    fn pledge_support_allows_third_country_to_support_target() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1, 2]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Target,
            &before_deadline(),
        )
        .expect("a genuine third country must be able to support the target");

        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        assert_eq!(
            crisis.third_party_reactions.get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsTarget)
        );
    }

    /// 要求テスト: initiatorは自身のCrisisで支持国になれない。
    #[test]
    fn pledge_support_rejects_initiator_supporting_itself() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        let result = pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(0),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.support_self"));
        assert!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .third_party_reactions
                .is_empty()
        );
    }

    /// 要求テスト: targetは自身のCrisisで支持国になれない。
    #[test]
    fn pledge_support_rejects_target_supporting_itself() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        let result = pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(1),
            CrisisSupportSide::Target,
            &before_deadline(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.support_self"));
    }

    /// 要求テスト: 存在しない国家は支持できない。
    #[test]
    fn pledge_support_rejects_nonexistent_country() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1]); // CountryId(2)は存在しない
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        let result = pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        );

        assert_eq!(
            result,
            Err("diplomacy_error.crisis.support_country_missing")
        );
    }

    /// 要求テスト: 同じ側への再登録は冪等(エラーにならず、状態も変わらない)。
    #[test]
    fn pledge_support_to_the_same_side_again_is_idempotent() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1, 2]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        )
        .unwrap();

        let result = pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        );

        assert!(
            result.is_ok(),
            "re-pledging the same side must be idempotent"
        );
        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        assert_eq!(crisis.third_party_reactions.len(), 1);
        assert_eq!(
            crisis.third_party_reactions.get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsInitiator)
        );
    }

    /// 要求テスト: 反対側への直接切り替えは拒否される(先に撤回が必要)。
    #[test]
    fn pledge_support_rejects_direct_switch_to_the_opposite_side() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1, 2]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        )
        .unwrap();

        let result = pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Target,
            &before_deadline(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.support_side_conflict"));
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .third_party_reactions
                .get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsInitiator),
            "a rejected side-switch must not mutate the existing pledge"
        );
    }

    /// 要求テスト: 支持撤回後は反対側を支持できる。
    #[test]
    fn withdraw_support_then_pledge_opposite_side_succeeds() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1, 2]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        )
        .unwrap();

        withdraw_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            &before_deadline(),
        )
        .unwrap();
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Target,
            &before_deadline(),
        )
        .expect("after withdrawing, the opposite side must be pledgeable");

        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .third_party_reactions
                .get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsTarget)
        );
    }

    /// 要求テスト: 支持していない国家の撤回はエラーになる(=他国の支持を撤回できない、
    /// の裏付け: `supporter`引数に実際の支持者以外を渡すと必ず失敗する)。
    #[test]
    fn withdraw_support_fails_for_a_country_that_never_pledged() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1, 2]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);

        let result = withdraw_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            &before_deadline(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.support_not_found"));
    }

    /// P21-021: 消滅済み国家はDomain APIレベルで支持撤回もできない
    /// (UIのrun_ifだけに依存しない直接検証)。
    #[test]
    fn withdraw_support_rejects_an_eliminated_supporter() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut countries = countries_with(&[0, 1, 2]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        )
        .unwrap();
        countries.get_mut(CountryId(2)).unwrap().status =
            crate::country::CountryStatus::Eliminated {
                eliminated_date: "1799/12/01".to_string(),
                annexed_by: CountryId(9),
            };

        let result = withdraw_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            &before_deadline(),
        );

        assert_eq!(
            result,
            Err("diplomacy_error.crisis.support_country_missing")
        );
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .third_party_reactions
                .get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsInitiator),
            "the pledge must survive rejection unchanged"
        );
    }

    /// 要求テスト: 期限到達後(=タイムアウト処理により既にEscalatingへ遷移済み)は
    /// 支持・撤回とも不可。
    #[test]
    fn pledge_and_withdraw_support_are_rejected_once_past_deadline() {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1, 2, 3]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        )
        .unwrap();
        // 既存のP21-011タイムアウト処理(`apply_rejection`)が行うのと同じ遷移を模擬する。
        crisis_registry
            .crises
            .get_mut(&crisis_id)
            .unwrap()
            .current_phase = CrisisPhase::Escalating;

        let pledge_result = pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(3), // まだ支持していない別の第三国(initiator/targetではない)
            CrisisSupportSide::Target,
            &before_deadline(),
        );
        let withdraw_result = withdraw_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            &before_deadline(),
        );

        assert_eq!(
            pledge_result,
            Err("diplomacy_error.crisis.not_awaiting_response")
        );
        assert_eq!(
            withdraw_result,
            Err("diplomacy_error.crisis.not_awaiting_response")
        );
    }

    /// 修正確認テスト: フェーズが`DemandSent`のまま(=日次タイムアウト処理がまだ
    /// 一度も走っていない)でも、`current_date`が期限日当日に達していれば支持・撤回とも
    /// 拒否される。以前は`current_phase == DemandSent`のみを見ていたため、この状態
    /// (期限到達済みだがフェーズ未更新)では誤って許可されてしまっていた抜けの再発防止。
    #[test]
    fn pledge_and_withdraw_support_are_rejected_exactly_on_the_deadline_day_while_phase_is_still_demand_sent()
     {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1, 2, 3]);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        )
        .unwrap();
        // フェーズは意図的に変更しない — DemandSentのまま、期限日当日を迎えた状態を模擬する。
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::DemandSent,
            "precondition: phase must still be DemandSent"
        );

        let pledge_result = pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(3),
            CrisisSupportSide::Target,
            &on_deadline_day(),
        );
        let withdraw_result = withdraw_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            &on_deadline_day(),
        );

        assert_eq!(
            pledge_result,
            Err("diplomacy_error.crisis.not_awaiting_response"),
            "reaching the deadline itself must reject new pledges even if the phase hasn't transitioned yet"
        );
        assert_eq!(
            withdraw_result,
            Err("diplomacy_error.crisis.not_awaiting_response"),
            "reaching the deadline itself must reject withdrawal even if the phase hasn't transitioned yet"
        );
    }

    /// 修正確認テスト: 期限を超過した日付(=セーブロード直後、次の日次進行が一度も
    /// 走っていない状況を模擬)でも、フェーズがまだ`DemandSent`のままなら支持・撤回とも
    /// 拒否される。ロード直後に一瞬だけ操作可能になってしまう抜けの再発防止。
    #[test]
    fn pledge_and_withdraw_support_are_rejected_after_deadline_while_phase_is_still_demand_sent_like_right_after_a_stale_load()
     {
        let mut crisis_registry = CrisisRegistry::default();
        let countries = countries_with(&[0, 1, 2, 3]);
        // ロード直後を模擬: `seed_demand_sent_crisis`はロードされたセーブと同じ状態
        // (期限"1800/01/31"を過ぎているが、まだ`DemandSent`のまま)を再現する。
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Initiator,
            &before_deadline(),
        )
        .unwrap();

        let pledge_result = pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(3),
            CrisisSupportSide::Target,
            &after_deadline(),
        );
        let withdraw_result = withdraw_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            &after_deadline(),
        );

        assert_eq!(
            pledge_result,
            Err("diplomacy_error.crisis.not_awaiting_response")
        );
        assert_eq!(
            withdraw_result,
            Err("diplomacy_error.crisis.not_awaiting_response")
        );
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .third_party_reactions
                .get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsInitiator),
            "a rejected withdrawal must not mutate the pre-existing pledge"
        );
    }

    /// 要求テスト: terminal phase(ResolvedPeacefully/WarStarted/Cancelled)では
    /// 支持・撤回とも不可。
    #[test]
    fn pledge_and_withdraw_support_are_rejected_on_terminal_phases() {
        for phase in [
            CrisisPhase::ResolvedPeacefully,
            CrisisPhase::WarStarted,
            CrisisPhase::Cancelled,
        ] {
            let mut crisis_registry = CrisisRegistry::default();
            let countries = countries_with(&[0, 1, 2]);
            let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, None);
            crisis_registry
                .crises
                .get_mut(&crisis_id)
                .unwrap()
                .current_phase = phase;

            let result = pledge_support(
                &mut crisis_registry,
                &countries,
                crisis_id,
                CountryId(2),
                CrisisSupportSide::Initiator,
                &before_deadline(),
            );

            assert_eq!(
                result,
                Err("diplomacy_error.crisis.not_awaiting_response"),
                "phase {phase:?} must reject new pledges"
            );
        }
    }

    /// 要求テスト: 支持履歴はterminal遷移後も保持される(既存の受諾処理が
    /// `third_party_reactions`へ一切触れないことの確認)。
    #[test]
    fn support_history_survives_crisis_resolution() {
        let mut crisis_registry = CrisisRegistry::default();
        let mut claim_registry = ClaimRegistry::default();
        let mut state_registry = make_state_registry(1, 1);
        let mut countries = countries_with(&[0, 1, 2]);
        let claim_id = seed_claim(&mut claim_registry, 0, 1);
        let crisis_id = seed_demand_sent_crisis(&mut crisis_registry, 0, 1, 1, Some(claim_id));
        pledge_support(
            &mut crisis_registry,
            &countries,
            crisis_id,
            CountryId(2),
            CrisisSupportSide::Target,
            &before_deadline(),
        )
        .unwrap();

        accept_demand(
            &mut crisis_registry,
            &mut claim_registry,
            &mut state_registry,
            &mut countries,
            &mut WarRegistry::default(),
            &mut crate::military::data::MilitaryRegistry::default(),
            &mut crate::military::army::ArmyRegistry::default(),
            &mut crate::war::frontline::FrontlineRegistry::default(),
            &mut crate::military::battle::BattleRegistry::default(),
            &mut crate::diplomacy::data::DiplomacyRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut crate::country::power::CountryPowerRegistry::default(),
            "1800/01/01",
            crisis_id,
            CountryId(1),
        )
        .unwrap();

        let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
        assert_eq!(crisis.current_phase, CrisisPhase::ResolvedPeacefully);
        assert_eq!(
            crisis.third_party_reactions.get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsTarget),
            "support history must survive past crisis resolution"
        );
    }
}
