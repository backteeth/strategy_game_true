use crate::common::{CountryId, DivisionDefinitionId, StateId};
use crate::country::CountryRegistry;
use crate::military::data::{Division, DivisionStatus, MilitaryRegistry};
use crate::military::equipment::{EquipmentAccountingError, try_issue_equipment};
use crate::state::data::StateRegistry;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecruitmentQueueItem {
    pub division_id: DivisionDefinitionId,
    pub target_state: StateId,
    pub days_remaining: u32,
    pub total_days: u32,
}

/// キュー項目からDivisionを生成する。
///
/// P27-001: `try_issue_equipment`へ渡す`requested`は、もはや`def.required_
/// equipment`固定(全額一括、all-or-nothing)ではなく、`def.required_equipment.
/// min(現在のMilitaryEquipment備蓄)`——**利用可能な範囲でだけ支給する**
/// (P27-001-PRE-001が確定した設計、候補C)。これにより、備蓄が必要量に届かない
/// 国家でも訓練完了(`days_remaining==0`)と同時にDivisionが生成される。
/// `requested`を事前にこの備蓄以下へ切り詰めるため、`try_issue_equipment`は
/// (備蓄自体が不正[NaN・負値等]でない限り)通常のパスでは失敗しない
/// ——`Err(InsufficientStockpile)`は理論上到達しない防御的分岐になる
/// (呼び出し元の`process_recruitment`はそれでも安全に処理する)。
///
/// `max_equipment`は`result.equipment_after`(実際に支給できた量)ではなく、
/// **常に`def.required_equipment`(満編制上限)をそのまま使う**——支給量を
/// 上限としてしまうと、後日`equipment_reinforcement`(P25-005)が「不足0」と
/// 誤判定し、このDivisionが二度と満編制まで補充されなくなる(P27-001-PRE-001
/// §15で確認した既存の装備補充契約を壊さないための必須の区別)。`equipment`は
/// 実際に支給できた量(`result.equipment_after`、備蓄0なら0.0)のまま。
fn try_spawn_division(
    stockpile: &mut crate::economy::resources::CountryStockpile,
    owner: CountryId,
    item: &RecruitmentQueueItem,
    def: &crate::military::data::DivisionDefinition,
) -> Result<Division, crate::military::equipment::EquipmentAccountingError> {
    let available = stockpile.get(crate::economy::resources::ResourceType::MilitaryEquipment);
    let issued_request = def.required_equipment.min(available);

    let mut equipment = 0.0;
    let result = try_issue_equipment(
        stockpile,
        &mut equipment,
        def.required_equipment,
        issued_request,
    )?;

    Ok(Division {
        id: crate::common::DivisionId(0), // Assigned in add_division
        owner,
        division_type: def.division_type,
        size: def.size,
        current_state: item.target_state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: def.required_manpower,
        max_manpower: def.required_manpower,
        equipment: result.equipment_after,
        max_equipment: def.required_equipment,
        organization: def.organization * 0.1, // Start with low organization
        max_organization: def.organization,
        morale: def.morale * 0.5,
        max_morale: def.morale,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: item.division_id,
        attack_power: def.attack as i32,
        defense_power: def.defense as i32,
        combat_id: None,
    })
}

/// P25-003/P27-001: `days_remaining`が0に達したキュー項目を、人的資源(既に
/// `request_recruitment`時点で`available_manpower`/`mobilized_manpower`へ確定済み)
/// に加え、国家備蓄(`MilitaryEquipment`)からの装備支給によりDivisionへ変換する。
///
/// P27-001: `try_spawn_division`が`def.required_equipment`を国家備蓄以下へ
/// 事前に切り詰めて要求するため(P27-001-PRE-001の候補C)、**装備が0でも
/// 完成する**——訓練完了(`days_remaining==0`)は常にその日のうちにDivision生成
/// まで進む。不足分は既存の日次装備補充(`equipment_reinforcement`、P25-005)が
/// 翌日以降、通常のDivisionと同列に補う。DivisionDefinition自体が解決できない
/// 場合は、既存の(P23-009で確認済みの)挙動どおり項目を静かに完了させて破棄する
/// (本タスクの対象外)。
///
/// `blocked_by_equipment_shortage`(P25-005由来)は、`try_spawn_division`が
/// `Err(InsufficientStockpile)`を返した場合にのみそのCountryIdを含める。
/// P27-001以降、この分岐は備蓄が正の値である限り理論上到達しない(要求量を
/// 事前に備蓄以下へ切り詰めているため)——備蓄自体が不正(NaN・負値等)で
/// `try_spawn_division`が他のエラー種別を返す場合は、この保護対象に含めない
/// 既存の方針(§6.2相当)をそのまま維持する。呼び出し元(`handle_daily_military`)
/// は引き続きこの戻り値を`apply_daily_equipment_reinforcement`へそのまま渡す
/// (`equipment_reinforcement.rs`自体は無変更)。
pub fn process_recruitment(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
) -> HashSet<CountryId> {
    let mut new_divisions = Vec::new();
    let mut blocked_by_equipment_shortage: HashSet<CountryId> = HashSet::new();

    for country in country_registry.countries.iter_mut() {
        let mut completed_indices = Vec::new();

        for (i, item) in country.recruitment_queue.iter_mut().enumerate() {
            if item.days_remaining > 0 {
                item.days_remaining -= 1;
            }

            if item.days_remaining == 0 {
                let Some(def) = military_registry.definitions.get(&item.division_id) else {
                    // DivisionDefinition未解決(既存の挙動、P25-003の対象外)。
                    completed_indices.push(i);
                    continue;
                };

                match try_spawn_division(&mut country.stockpile, country.id, item, def) {
                    Ok(division) => {
                        completed_indices.push(i);
                        new_divisions.push(division);
                    }
                    Err(EquipmentAccountingError::InsufficientStockpile) => {
                        // P27-001以降、`try_spawn_division`は要求量を備蓄以下へ
                        // 事前に切り詰めるため、備蓄が有効な非負値である限りこの
                        // 分岐へは到達しない(理論上の防御的分岐)。万一到達した
                        // 場合は、項目を完了させずキューに残し、当日の既存
                        // Division向け装備補充を止めて備蓄を保護する既存の方針
                        // (P25-005)をそのまま維持する。
                        blocked_by_equipment_shortage.insert(country.id);
                    }
                    Err(_) => {
                        // 不正なdefinition・不正数量等(現在の正常経路では発生
                        // しないはずの理論上のエラー)。備蓄保護の対象とはせず
                        // (§6.2)、項目を完了させずキューに残すだけに留める。
                        // manpower・stockpileはtry_issue_equipmentが失敗時に
                        // 一切変更しないため、ここでの追加のロールバックは不要。
                    }
                }
            }
        }

        for i in completed_indices.iter().rev() {
            country.recruitment_queue.remove(*i);
        }
    }

    for division in new_divisions {
        military_registry.add_division(division);
    }

    blocked_by_equipment_shortage
}

/// キューへ投入する(P25-003)。`required_equipment`は物理的なMilitaryEquipment
/// 必要量として扱う——ここでは有効性(finite・非負・固定小数点正規範囲内)だけを
/// `validate_required_equipment`で検証し、実際の国家備蓄からの支給はDivision
/// 生成時(`process_recruitment`)まで行わない(§8: キュー投入時に装備は消費しない、
/// 消費するのは生成成功時のみ)。旧仮実装(`treasury`を装備の代金として即座に
/// 減算していた"Simplified"実装)は削除した——`required_equipment`はもはや
/// 通貨ではないため、treasuryは募兵によって変化しない。
pub fn request_recruitment(
    country: &mut crate::country::CountryData,
    military_registry: &MilitaryRegistry,
    division_id: DivisionDefinitionId,
    target_state: StateId,
) -> Result<(), &'static str> {
    if !country.is_active() {
        return Err("Country is eliminated");
    }

    let def = military_registry
        .definitions
        .get(&division_id)
        .ok_or("Unknown division type")?;

    if country.available_manpower < def.required_manpower {
        return Err("Insufficient manpower");
    }

    if crate::military::equipment::validate_required_equipment(def.required_equipment).is_err() {
        return Err("Invalid required_equipment on division definition");
    }

    country.available_manpower -= def.required_manpower;
    country.mobilized_manpower += def.required_manpower;

    country.recruitment_queue.push(RecruitmentQueueItem {
        division_id,
        target_state,
        days_remaining: def.recruitment_days,
        total_days: def.recruitment_days,
    });

    Ok(())
}

/// P25-003: キュー投入時に国家備蓄からMilitaryEquipmentを消費しないため
/// (`request_recruitment`参照)、キャンセル時にも装備の返却は不要——人的資源
/// (`available_manpower`/`mobilized_manpower`)のみを返却する。
pub fn cancel_recruitment(
    country: &mut crate::country::CountryData,
    military_registry: &MilitaryRegistry,
    queue_index: usize,
) -> Result<(), &'static str> {
    if !country.is_active() {
        return Err("Country is eliminated");
    }
    if queue_index >= country.recruitment_queue.len() {
        return Err("Invalid queue index");
    }

    let item = country.recruitment_queue.remove(queue_index);
    if let Some(def) = military_registry.definitions.get(&item.division_id) {
        country.available_manpower += def.required_manpower;
        country.mobilized_manpower = country
            .mobilized_manpower
            .saturating_sub(def.required_manpower);
    }

    Ok(())
}

/// P21-001: UIが募兵ボタンの表示・押下可否を判定するための副作用のない事前評価結果。
///
/// `request_recruitment`自体は対象州の所有権を検証しない(呼び出し元の責務)ため、
/// この関数がUI層向けに所有権チェックを含めた一段階手前の判定として提供する。
/// 実際の人的資源消費は`request_recruitment`が改めて原子的に検証・実行する
/// (この関数の結果が`Ready`でも、呼び出し間に状態が変化した場合は
/// `request_recruitment`側の再検証で安全に拒否される)。P25-003: 装備
/// (`MilitaryEquipment`備蓄)はキュー投入時の条件ではない(§8)ため、この判定には
/// 含めない——不足していてもキュー投入は`Ready`のまま成功し、実際の支給可否は
/// Division生成時(`process_recruitment`)まで確定しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecruitFeasibility {
    Ready,
    NoStateSelected,
    NotOwnState,
    DefinitionUnavailable,
    InsufficientManpower,
}

impl RecruitFeasibility {
    pub fn is_ready(self) -> bool {
        matches!(self, RecruitFeasibility::Ready)
    }
}

/// 指定した州・部隊定義に対して募兵操作が実行可能かを判定する(副作用なし)。
#[allow(clippy::too_many_arguments)]
pub fn evaluate_recruit_feasibility(
    selected_state: Option<StateId>,
    player_country_id: CountryId,
    state_registry: &StateRegistry,
    country: &crate::country::CountryData,
    military_registry: &MilitaryRegistry,
    division_id: DivisionDefinitionId,
) -> RecruitFeasibility {
    let Some(state_id) = selected_state else {
        return RecruitFeasibility::NoStateSelected;
    };
    let Some(state) = state_registry.get(state_id) else {
        return RecruitFeasibility::NoStateSelected;
    };
    if state.owner_country_id != player_country_id {
        return RecruitFeasibility::NotOwnState;
    }

    let Some(def) = military_registry.definitions.get(&division_id) else {
        return RecruitFeasibility::DefinitionUnavailable;
    };

    if country.available_manpower < def.required_manpower {
        return RecruitFeasibility::InsufficientManpower;
    }

    RecruitFeasibility::Ready
}
