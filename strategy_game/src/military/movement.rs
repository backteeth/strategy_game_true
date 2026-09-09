use crate::military::battle::BattleRegistry;
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;

pub fn process_movement(
    military_registry: &mut MilitaryRegistry,
    state_registry: &mut StateRegistry,
    war_registry: &WarRegistry,
    battle_registry: &mut BattleRegistry,
    current_date: &str,
) {
    let base_days = 5.0; // 1セグメントあたりの基本移動日数

    // 処理するユニットのIDリストを事前収集（借用衝突を避けるため）。DivisionId昇順で
    // 決定的に処理する(P26-006-REV-001: 複数国家の師団が同日に同じ州へ到達し得る場合、
    // HashMap反復順[プロセスごとのハッシュシード依存]で「どちらが先に州を占領するか」が
    // 変わり得たため)。
    let mut division_ids: Vec<_> = military_registry.divisions.keys().copied().collect();
    division_ids.sort_by_key(|id| id.0);

    for division_id in division_ids {
        // 戦闘中ユニットは移動しない
        {
            let division = match military_registry.divisions.get(&division_id) {
                Some(a) => a,
                None => continue,
            };
            if division.status == DivisionStatus::Fighting || division.status == DivisionStatus::Retreating {
                continue;
            }
            if division.status != DivisionStatus::Moving {
                continue;
            }
        }

        // target_stateが未設定だが current_path が残っている場合、最初のノードをセット
        {
            let division = match military_registry.divisions.get_mut(&division_id) {
                Some(a) => a,
                None => continue,
            };

            if division.target_state.is_none() {
                if !division.current_path.is_empty() {
                    let next_state = division.current_path.remove(0);
                    division.target_state = Some(next_state);
                    division.movement_progress = 0.0;
                } else {
                    // 目的地に到達
                    division.status = DivisionStatus::Idle;
                    division.destination = None;
                    division.movement_progress = 0.0;
                    continue;
                }
            }
        }

        let (target, current_state, owner) = {
            let division = match military_registry.divisions.get(&division_id) {
                Some(a) => a,
                None => continue,
            };
            match division.target_state {
                Some(t) => (t, division.current_state, division.owner),
                None => continue,
            }
        };

        // 移動先の支配国を確認
        let target_controller = match state_registry.get(target) {
            Some(s) => s.controller(),
            None => {
                // 無効な地域 → 移動停止
                if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                    division.status = DivisionStatus::Idle;
                    division.target_state = None;
                    division.destination = None;
                    division.current_path.clear();
                    division.movement_progress = 0.0;
                }
                continue;
            }
        };

        // 自国領かどうかを判定
        let is_own_territory = target_controller == owner;
        let is_at_war_with_controller = war_registry.are_countries_at_war(owner, target_controller);

        // 通過可能か判定（自国 or 交戦中の敵国）
        if !is_own_territory && !is_at_war_with_controller {
            // 通過不可（中立国・非交戦国）→ 移動停止
            if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                division.status = DivisionStatus::Idle;
                division.target_state = None;
                division.destination = None;
                division.current_path.clear();
                division.movement_progress = 0.0;
            }
            continue;
        }

        // 移動速度計算
        let (def_speed, movement_progress) = {
            let division = match military_registry.divisions.get(&division_id) {
                Some(a) => a,
                None => continue,
            };
            let speed = military_registry
                .definitions
                .get(&division.def_id)
                .map(|d| d.movement_speed)
                .unwrap_or(1.0);
            (speed, division.movement_progress)
        };

        let step_cost = crate::military::pathfinding::calculate_step_cost(
            current_state,
            target,
            state_registry,
        ) as f32;
        let supply_mod = {
            let division = military_registry
                .divisions
                .get(&division_id)
                .map(|a| a.supply_ratio)
                .unwrap_or(0.5);
            division.max(0.5)
        };
        let movement_days = (base_days * step_cost / (def_speed * supply_mod)).max(1.0);
        let daily_progress = 1.0 / movement_days;

        let new_progress = movement_progress + daily_progress;

        if new_progress >= 1.0 {
            // セグメント移動完了 → 到着処理
            // まず現在地を更新
            {
                let division = match military_registry.divisions.get_mut(&division_id) {
                    Some(a) => a,
                    None => continue,
                };
                division.current_state = target;
                division.movement_progress = 0.0;
            }

            // 次のセグメントをセット（または到達完了）
            {
                let division = match military_registry.divisions.get_mut(&division_id) {
                    Some(a) => a,
                    None => continue,
                };
                if !division.current_path.is_empty() {
                    let next = division.current_path.remove(0);
                    division.target_state = Some(next);
                } else {
                    division.target_state = None;
                    division.destination = None;
                    // ステータスはinvasionで決定するため、ここでIdleにしない
                }
            }

            // 侵攻判定（敵領か自国領か）
            if is_own_territory {
                // 自国領への移動完了 → Idle（目的地到着時のみ）
                let arrived_at_destination = {
                    let division = military_registry.divisions.get(&division_id);
                    division.map(|a| a.target_state.is_none() && a.destination.is_none())
                        .unwrap_or(false)
                };
                if arrived_at_destination
                    && let Some(division) = military_registry.divisions.get_mut(&division_id)
                {
                    division.status = DivisionStatus::Idle;
                }
            } else {
                // 敵支配地域への到着 → 侵攻処理
                crate::military::invasion::process_division_arrival(
                    division_id,
                    target,
                    current_state,
                    current_date,
                    military_registry,
                    state_registry,
                    war_registry,
                    battle_registry,
                );
            }
        } else {
            // まだ移動中
            if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                division.movement_progress = new_progress;
            }
        }
    }
}

/// 経路の各段階で通行可能かを再検証し、問題があれば移動を停止する
/// 戦争終了や支配国変更で無効になった経路を安全に処理する
pub fn validate_and_stop_invalid_movements(
    military_registry: &mut MilitaryRegistry,
    state_registry: &StateRegistry,
    war_registry: &WarRegistry,
) {
    for division in military_registry.divisions.values_mut() {
        if division.status != DivisionStatus::Moving {
            continue;
        }

        // 現在の target_state が通過可能か確認
        if let Some(target) = division.target_state
            && let Some(target_state) = state_registry.get(target)
        {
            let controller = target_state.controller();
            let is_own = controller == division.owner;
            let is_enemy = war_registry.are_countries_at_war(division.owner, controller);

            if !is_own && !is_enemy {
                // 無効になった → 停止
                division.status = DivisionStatus::Idle;
                division.target_state = None;
                division.destination = None;
                division.current_path.clear();
                division.movement_progress = 0.0;
            }
        }

        // 残りの経路を検証
        if division.status == DivisionStatus::Moving {
            let path_valid = division.current_path.iter().all(|&sid| {
                if let Some(s) = state_registry.get(sid) {
                    let controller = s.controller();
                    controller == division.owner
                        || war_registry.are_countries_at_war(division.owner, controller)
                } else {
                    false
                }
            });

            if !path_valid {
                // 経路の一部が無効 → 安全停止（現在地で待機）
                division.current_path.clear();
                division.destination = None;
                // target_stateへの移動は継続（既に進行中の場合）
                // 次の到着時に再判定される
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
    use crate::military::data::{Division, DivisionSize, DivisionStatus, DivisionType};
    use crate::state::data::StateData;
    use crate::war::data::{War, WarStatus};
    use std::collections::HashSet;

    fn make_arriving_division(id: usize, owner: CountryId, origin: StateId, target: StateId) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: origin,
            destination: Some(target),
            current_path: vec![],
            target_state: Some(target),
            manpower: 10_000,
            max_manpower: 10_000,
            equipment: 100.0,
            max_equipment: 100.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 100.0,
            max_morale: 100.0,
            experience: 0.0,
            supply_ratio: 1.0,
            // 1回のprocess_movement呼び出しで確実に到着させる(移動日数の計算式の
            // 具体値に依存せず、new_progressが必ず1.0を超えるようにするため)。
            movement_progress: 0.999_999,
            status: DivisionStatus::Moving,
            def_id: DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn war(id: usize, attacker: CountryId, defender: CountryId) -> War {
        War {
            id: WarId(id),
            name: "Test War".to_string(),
            attackers: [attacker].into_iter().collect(),
            defenders: [defender].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
            source_crisis_id: None,
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: HashSet::new(),
            status: WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: HashSet::new(),
        }
    }

    /// P26-006-REV-001の再現・回帰テスト: 互いに交戦していない2か国(B・C)が、それぞれ
    /// 別々に無防備な国Aと交戦中で、両者の師団が同じ無防備な州へ同日中に到着する場合、
    /// `process_movement`は常にDivisionId昇順で処理し、より小さいDivisionIdを持つ師団が
    /// 占領に成功する――HashMapへの挿入順(country_b/country_cのどちらを先に`insert`
    /// したか)には一切依存しない。
    ///
    /// この根本原因(division_idsがDivisionId昇順にソートされていなかったため、
    /// プロセスごとにランダムなHashMapハッシュシードによって占領国が変わり得た)は、
    /// `tests/p23_010_phase23_manpower_lifecycle_audit_test.rs::a_save_and_reload_
    /// partway_through_matches_an_uninterrupted_run`が間欠的に失敗する直接の原因
    /// だった(P26-006-REV-001の再監査で特定、詳細は検証報告書参照)。
    #[test]
    fn arrival_race_between_two_unrelated_attackers_is_decided_by_division_id_not_insertion_order() {
        fn run(low_id_owner: CountryId, high_id_owner: CountryId, insert_high_first: bool) -> CountryId {
            let country_a = CountryId(0);
            let origin = StateId(10);
            let target = StateId(1);

            let mut war_reg = WarRegistry::default();
            war_reg.wars.insert(WarId(0), war(0, low_id_owner, country_a));
            war_reg.wars.insert(WarId(1), war(1, high_id_owner, country_a));

            let mut state_reg = StateRegistry::build(vec![
                StateData { id: origin, owner_country_id: low_id_owner, ..Default::default() },
                StateData { id: target, owner_country_id: country_a, ..Default::default() },
            ]);

            let low = make_arriving_division(1, low_id_owner, origin, target);
            let high = make_arriving_division(2, high_id_owner, origin, target);

            let mut mil = MilitaryRegistry::default();
            if insert_high_first {
                mil.divisions.insert(high.id, high);
                mil.divisions.insert(low.id, low);
            } else {
                mil.divisions.insert(low.id, low);
                mil.divisions.insert(high.id, high);
            }

            let mut battle_reg = BattleRegistry::default();
            process_movement(&mut mil, &mut state_reg, &war_reg, &mut battle_reg, "1800/01/01");

            state_reg.get(target).unwrap().controller()
        }

        let country_b = CountryId(1);
        let country_c = CountryId(2);

        // DivisionId(1)=country_b(小さい方)は、HashMap挿入順に関わらず常に勝つ。
        assert_eq!(run(country_b, country_c, false), country_b);
        assert_eq!(run(country_b, country_c, true), country_b);

        // 役割を入れ替えても、小さいDivisionIdを持つ側(この場合country_c)が勝つ
        // ――どの国かではなく、DivisionIdの大小だけで決まることを確認する。
        assert_eq!(run(country_c, country_b, false), country_c);
        assert_eq!(run(country_c, country_b, true), country_c);
    }

    /// 上記の占領競合シナリオを負けた側の視点で確認する: 同じ`process_movement`
    /// 呼び出し内で先に処理された師団(country_b)が既に州を占領しているため、
    /// 後から処理される師団(country_c)は、目的州の支配国(country_b)と交戦していない
    /// ことを検知して安全に停止する(既存の`process_movement`の「交戦中でない国家の
    /// 領土には侵入できない」ルール、§86-101行目、セグメント移動完了処理より前で
    /// 判定されるため`current_state`自体は出発州のまま変化しない)。
    #[test]
    fn losing_division_in_the_arrival_race_safely_stops_instead_of_invading_a_non_enemy() {
        let country_a = CountryId(0);
        let country_b = CountryId(1);
        let country_c = CountryId(2);
        let origin = StateId(10);
        let target = StateId(1);

        let mut war_reg = WarRegistry::default();
        war_reg.wars.insert(WarId(0), war(0, country_b, country_a));
        war_reg.wars.insert(WarId(1), war(1, country_c, country_a));

        let mut state_reg = StateRegistry::build(vec![
            StateData { id: origin, owner_country_id: country_b, ..Default::default() },
            StateData { id: target, owner_country_id: country_a, ..Default::default() },
        ]);

        let low = make_arriving_division(1, country_b, origin, target); // 勝つ
        let high = make_arriving_division(2, country_c, origin, target); // 負ける

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(high.id, high);
        mil.divisions.insert(low.id, low);

        let mut battle_reg = BattleRegistry::default();
        process_movement(&mut mil, &mut state_reg, &war_reg, &mut battle_reg, "1800/01/01");

        assert_eq!(state_reg.get(target).unwrap().controller(), country_b);
        let loser = mil.divisions.get(&DivisionId(2)).unwrap();
        assert_eq!(loser.status, DivisionStatus::Idle, "自国と交戦していない国が支配する州へは侵入できず、安全に停止するはず");
        assert_eq!(loser.current_state, origin, "既にcountry_bへ占領された州とは交戦していないため、移動完了処理前に停止し出発州のままのはず");
        assert!(loser.target_state.is_none());
        assert!(loser.destination.is_none());
    }
}
