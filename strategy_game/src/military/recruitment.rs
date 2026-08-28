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

/// キュー項目からDivisionを生成する。`try_issue_equipment`を、
/// 装備0・上限`def.required_equipment`のDivisionへ`def.required_equipment`を
/// 全額支給する呼び出しとして使うことで、支給と同時に「新規Divisionの装備上限」
/// (P25-003 §10: `max_equipment = canonical(required_equipment)`,
/// `equipment = max_equipment`)を導出する——正規APIの支給結果をそのまま
/// 両フィールドへ使うため、rawな`required_equipment`とcanonical値が食い違う
/// ことはない。国家備蓄(`MilitaryEquipment`)が不足している場合は`Err`を返し、
/// 呼び出し元(`process_recruitment`)は当日のこの項目を未完了のまま(`days_
/// remaining`を0に留めたまま)キューに残す。
fn try_spawn_division(
    stockpile: &mut crate::economy::resources::CountryStockpile,
    owner: CountryId,
    item: &RecruitmentQueueItem,
    def: &crate::military::data::DivisionDefinition,
) -> Result<Division, crate::military::equipment::EquipmentAccountingError> {
    let mut equipment = 0.0;
    let result = try_issue_equipment(
        stockpile,
        &mut equipment,
        def.required_equipment,
        def.required_equipment,
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
        max_equipment: result.equipment_after,
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

/// P25-003: `days_remaining`が0に達したキュー項目を、人的資源(既に`request_
/// recruitment`時点で`available_manpower`/`mobilized_manpower`へ確定済み)に加え、
/// 国家備蓄(`MilitaryEquipment`)からの装備支給も完了した場合にのみDivisionへ
/// 変換する。装備不足の場合は項目を完了させず(`completed_indices`へ追加しない)、
/// `days_remaining`を0に留めたままキューに残す——翌日以降、備蓄が回復すれば
/// 自動的に再判定される(既存の日次呼び出し構造がそのまま「毎日再試行」を実現する)。
/// DivisionDefinition自体が解決できない場合は、既存の(P23-009で確認済みの)
/// 挙動どおり項目を静かに完了させて破棄する(本タスクの対象外、§16)。
///
/// P25-005: ある国の完了済みキュー項目が純粋な`EquipmentAccountingError::
/// InsufficientStockpile`(=備蓄不足そのもの。不正なdefinitionや不正数量による
/// 他のエラー種別は含まない、§6.2)によって生成失敗した場合、そのCountryIdを
/// 戻り値へ含める。呼び出し元(`handle_daily_military`)はこれを使って、当日の
/// 既存Division向け装備補充予算をその国だけ0にし、翌日以降に備蓄を持ち越して
/// 待機中のキューが必要量へ到達できるようにする(新しい永続予約フィールドは
/// 追加しない、当日の処理結果から都度決定する一時的な判定)。
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
                        // 純粋な備蓄不足。項目を完了させずキューに残し、当日の
                        // 既存Division向け装備補充を止めて備蓄を保護する(§6)。
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
