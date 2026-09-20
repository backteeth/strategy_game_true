use crate::common::{CountryId, StateId};
use crate::military::battle::BattleRegistry;
use crate::military::data::{Division, DivisionStatus, MilitaryRegistry};
use crate::military::pathfinding::find_path;
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;

/// P27-004: 移動の開始・維持を扱う唯一の冪等API。プレイヤー手動移動
/// (`map::division_selection`)・Defend AI(`war::frontline::process_defensive_plan`)・
/// Offensive AI(`war::frontline::process_offensive_plan`)の3経路が、それぞれ
/// `destination`/`current_path`/`target_state`/`movement_progress`/`status`を
/// 個別にインライン設定していたことがP27-004-PRE-001の根本原因だった——特に
/// Defend AIが「前日と全く同じ目的地」への命令を毎日再発行し、`movement_progress`を
/// 無条件に`0.0`へリセットしていたため、`FrontlineOrders`(リセット)→
/// `MilitaryAction`(1日分加算)の日次サイクルにより進捗が恒久的に1日分の値へ
/// 固定される(P27-004-PRE-001 §14で動的に再現・確認済み)。
///
/// 冪等性契約(P27-004 spec §4・§5・§6):
/// - 既に`requested_destination`へ到達済み(`current_state == requested_destination`)、
///   または既に`requested_destination`へ向けて`Moving`中(`status == Moving &&
///   destination == Some(requested_destination)`)の場合は**no-op**——
///   `status`・`current_state`・`destination`・`current_path`・`target_state`・
///   `movement_progress`のいずれも変更しない(bit-for-bit維持)。`find_path`も
///   再実行しない。
/// - それ以外(異なる目的地、または現在Moving中でない)の場合は、既存どおり
///   `find_path`を実行し、成功時のみ`destination`/`current_path`/
///   `target_state=None`/`movement_progress=0.0`/`status=Moving`を原子的に
///   一括更新する。経路探索失敗時は既存状態を一切変更しない(部分更新なし)。
///
/// 経路探索アルゴリズム・通行判定・tie-break・地形コストは`find_path`
/// (`military::pathfinding`、無変更)をそのまま呼ぶだけであり、本関数はこれらを
/// 複製しない。州到着処理・経路無効化(`validate_and_stop_invalid_movements`)・
/// save/load適用は本関数の責務外のまま(呼び出し元・既存の別関数が引き続き担当)。
///
/// 戻り値: `requested_destination`へ向けた移動が(既存維持または新規開始により)
/// 成立していれば`true`、経路が見つからず何もしなかった場合は`false`。
pub fn start_or_maintain_movement(
    division: &mut Division,
    requested_destination: StateId,
    state_registry: &StateRegistry,
    own_side: &[CountryId],
    enemy_side: &[CountryId],
) -> bool {
    if division.status == DivisionStatus::Moving {
        if division.destination == Some(requested_destination) {
            // 既に同じ目的地へ移動中 → no-op(P27-004の中核契約)。current_stateが
            // たまたま要求先と一致する[出発直後で未離脱]場合でも、Moving中に別の
            // destinationへ切り替える正規命令(spec §5)まで巻き込んで無視しては
            // いけないため、current_stateではなくdestinationの一致だけで判定する。
            return true;
        }
    } else if division.current_state == requested_destination {
        // Idle状態で既に目的地にいる → no-op(旧`division_selection.rs`の
        // 到着済みガードの一般化)。
        return true;
    }

    let Some(path) = find_path(
        division.current_state,
        requested_destination,
        state_registry,
        own_side,
        enemy_side,
    ) else {
        return false;
    };

    division.destination = Some(requested_destination);
    division.current_path = path;
    division.target_state = None;
    division.movement_progress = 0.0;
    division.status = DivisionStatus::Moving;
    true
}

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

    // ─── P27-004: start_or_maintain_movement(移動命令の冪等化) ─────────────

    fn idle_division(id: usize, owner: CountryId, at: StateId) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: at,
            destination: None,
            current_path: Vec::new(),
            target_state: None,
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
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    /// P27-004 spec §9・§18: State1↔State2↔State3(全て同一国家owner、隣接)。
    fn linear_three_state_registry(owner: CountryId) -> StateRegistry {
        StateRegistry::build(vec![
            StateData { id: StateId(1), owner_country_id: owner, neighbors: vec![StateId(2)], ..Default::default() },
            StateData {
                id: StateId(2),
                owner_country_id: owner,
                neighbors: vec![StateId(1), StateId(3)],
                ..Default::default()
            },
            StateData { id: StateId(3), owner_country_id: owner, neighbors: vec![StateId(2)], ..Default::default() },
        ])
    }

    #[test]
    fn start_or_maintain_movement_begins_movement_from_idle() {
        let owner = CountryId(1);
        let state_reg = linear_three_state_registry(owner);
        let mut division = idle_division(0, owner, StateId(1));

        let started = start_or_maintain_movement(&mut division, StateId(2), &state_reg, &[owner], &[]);

        assert!(started);
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(2)));
        assert_eq!(division.current_path, vec![StateId(2)]);
        assert_eq!(division.movement_progress, 0.0);
        assert!(division.target_state.is_none());
    }

    #[test]
    fn start_or_maintain_movement_is_a_no_op_when_already_moving_to_the_same_destination() {
        let owner = CountryId(1);
        let state_reg = linear_three_state_registry(owner);
        let mut division = idle_division(0, owner, StateId(1));
        assert!(start_or_maintain_movement(&mut division, StateId(3), &state_reg, &[owner], &[]));

        // 1日分の移動が進行した状態を模擬する(実際にはprocess_movementが行う)。
        division.movement_progress = 0.4;
        let path_before = division.current_path.clone();
        let target_before = division.target_state;

        // P27-004の核心: 全く同じ目的地への命令が再発行されても、進捗・経路・
        // target_stateはbit-for-bitで維持される(process_defensive_plan由来の
        // 日次再発行によるリセットを防ぐ)。
        let started = start_or_maintain_movement(&mut division, StateId(3), &state_reg, &[owner], &[]);

        assert!(started);
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(3)));
        assert_eq!(division.movement_progress, 0.4, "同一目的地の再命令はprogressを維持するはず");
        assert_eq!(division.current_path, path_before, "同一目的地の再命令はrouteを維持するはず");
        assert_eq!(division.target_state, target_before, "同一目的地の再命令はtarget_stateを維持するはず");
    }

    #[test]
    fn start_or_maintain_movement_applies_a_redirect_to_the_current_position_while_still_departing() {
        // 境界ケース: current_stateが要求先とたまたま一致していても(例: 出発直後で
        // まだ物理的に出発州を離れていない)、Moving中に別のdestinationへの正規な
        // 切替命令(spec §5)まで「既に到着済み」とno-op判定して無視してはいけない。
        // 判定の真実源はcurrent_stateではなくdestinationの一致であるべき。
        let owner = CountryId(1);
        let state_reg = linear_three_state_registry(owner);
        let mut division = idle_division(0, owner, StateId(1));
        division.status = DivisionStatus::Moving;
        division.destination = Some(StateId(2)); // StateId(2)へ向けて出発直後
        division.current_path = vec![StateId(2)];
        division.movement_progress = 0.1;
        // current_stateはまだStateId(1)(出発州)のまま。

        let started = start_or_maintain_movement(&mut division, StateId(1), &state_reg, &[owner], &[]);

        assert!(started);
        assert_eq!(
            division.destination,
            Some(StateId(1)),
            "destinationがrequested先と異なる(Some(2)≠1)ため、current_state一致に\
             関わらず正規の切替命令として適用されるはず"
        );
        assert_eq!(division.movement_progress, 0.0, "正規の切替はprogressを0から開始するはず");
    }

    #[test]
    fn start_or_maintain_movement_preserves_equipment_and_manpower_on_no_op() {
        let owner = CountryId(1);
        let state_reg = linear_three_state_registry(owner);
        let mut division = idle_division(0, owner, StateId(1));
        division.status = DivisionStatus::Moving;
        division.destination = Some(StateId(2));
        division.current_path = vec![StateId(2)];
        division.movement_progress = 0.6;
        division.equipment = 42.5;
        division.manpower = 7_777;

        start_or_maintain_movement(&mut division, StateId(2), &state_reg, &[owner], &[]);

        assert_eq!(division.equipment, 42.5);
        assert_eq!(division.manpower, 7_777);
    }

    #[test]
    fn start_or_maintain_movement_progress_accumulates_across_repeated_identical_daily_orders() {
        // spec §4: 毎日同じ命令が再発行されても、実際の移動進行(process_movement)と
        // 交互に実行すればprogressは単調に累積し、有限日数で到着する。
        let owner = CountryId(1);
        let mut state_reg = linear_three_state_registry(owner);
        let mut mil = MilitaryRegistry::default();
        let division = idle_division(0, owner, StateId(1));
        let division_id = mil.add_division(division);
        let war_registry = WarRegistry::default();
        let mut battle_registry = BattleRegistry::default();

        let mut days = 0u32;
        loop {
            days += 1;
            assert!(days <= 100, "現行式から導出される有限日数内に到着するはず(暴走検出)");
            {
                let d = mil.divisions.get_mut(&division_id).unwrap();
                start_or_maintain_movement(d, StateId(2), &state_reg, &[owner], &[]);
            }
            process_movement(&mut mil, &mut state_reg, &war_registry, &mut battle_registry, "1800/01/02");
            let d = mil.divisions.get(&division_id).unwrap();
            if d.status == DivisionStatus::Idle && d.current_state == StateId(2) {
                break;
            }
        }
        assert!(days < 100, "到着したはず");
        let d = mil.divisions.get(&division_id).unwrap();
        assert_eq!(d.destination, None);
        assert_eq!(d.movement_progress, 0.0);
    }

    #[test]
    fn start_or_maintain_movement_overwrites_when_destination_changes() {
        let owner = CountryId(1);
        let state_reg = linear_three_state_registry(owner);
        let mut division = idle_division(0, owner, StateId(1));
        division.status = DivisionStatus::Moving;
        division.destination = Some(StateId(2));
        division.current_path = vec![StateId(2)];
        division.movement_progress = 0.7;

        let started = start_or_maintain_movement(&mut division, StateId(3), &state_reg, &[owner], &[]);

        assert!(started);
        assert_eq!(division.destination, Some(StateId(3)), "異なる目的地は正規に上書きされるはず");
        assert_eq!(division.current_path, vec![StateId(2), StateId(3)]);
        assert_eq!(division.movement_progress, 0.0, "異なる目的地への上書きはprogressを0から開始するはず");
        assert_eq!(division.current_state, StateId(1), "current_stateは命令時点の実位置のまま変化しないはず");
    }

    #[test]
    fn start_or_maintain_movement_does_nothing_when_no_path_exists() {
        // State3は隣接していない孤立州(neighbors未設定)を用意し、到達不能にする。
        let owner = CountryId(1);
        let hostile = CountryId(2);
        let state_reg = StateRegistry::build(vec![
            StateData { id: StateId(1), owner_country_id: owner, neighbors: vec![], ..Default::default() },
            StateData { id: StateId(2), owner_country_id: hostile, neighbors: vec![], ..Default::default() },
        ]);
        let mut division = idle_division(0, owner, StateId(1));

        let started = start_or_maintain_movement(&mut division, StateId(2), &state_reg, &[owner], &[]);

        assert!(!started, "到達不能な目的地には経路が見つからないはず");
        assert_eq!(division.status, DivisionStatus::Idle, "経路なしなら既存状態(Idle)を維持するはず");
        assert!(division.destination.is_none());
        assert!(division.current_path.is_empty());
        assert_eq!(division.movement_progress, 0.0);
    }

    #[test]
    fn start_or_maintain_movement_preserves_existing_movement_when_a_different_destination_has_no_path() {
        let owner = CountryId(1);
        let state_reg = linear_three_state_registry(owner);
        let mut division = idle_division(0, owner, StateId(1));
        division.status = DivisionStatus::Moving;
        division.destination = Some(StateId(2));
        division.current_path = vec![StateId(2)];
        division.movement_progress = 0.3;

        // StateId(99)は存在しない(経路探索は失敗する)。
        let started =
            start_or_maintain_movement(&mut division, StateId(99), &state_reg, &[owner], &[]);

        assert!(!started);
        assert_eq!(
            division.destination,
            Some(StateId(2)),
            "経路探索失敗時は既存の進行中移動を一切変更しないはず(部分更新なし)"
        );
        assert_eq!(division.movement_progress, 0.3);
        assert_eq!(division.status, DivisionStatus::Moving);
    }

    #[test]
    fn start_or_maintain_movement_is_a_no_op_when_already_at_the_destination() {
        let owner = CountryId(1);
        let state_reg = linear_three_state_registry(owner);
        let mut division = idle_division(0, owner, StateId(2));

        let started = start_or_maintain_movement(&mut division, StateId(2), &state_reg, &[owner], &[]);

        assert!(started);
        assert_eq!(division.status, DivisionStatus::Idle, "既に到達済みならMovingへ入らないはず");
        assert!(division.destination.is_none());
        assert!(division.current_path.is_empty());
    }

    #[test]
    fn start_or_maintain_movement_does_not_re_run_bfs_for_an_identical_in_progress_order() {
        // 冪等パスではfind_pathを再実行しない(同一経路オブジェクトを再構築しない)
        // ことを、意図的に「もう存在しない」経路条件下でも既存の有効な進行中移動が
        // 壊れないことによって間接的に確認する: hostile国のCountryIdを空にして
        // 本来なら再探索が失敗するはずの状況でも、同一目的地への再命令はno-opの
        // ままprogressを維持する。
        let owner = CountryId(1);
        let state_reg = linear_three_state_registry(owner);
        let mut division = idle_division(0, owner, StateId(1));
        division.status = DivisionStatus::Moving;
        division.destination = Some(StateId(3));
        division.current_path = vec![StateId(2), StateId(3)];
        division.movement_progress = 0.55;

        // own_side/enemy_sideを両方空にする(通常ならfind_pathが失敗する条件)。
        let started = start_or_maintain_movement(&mut division, StateId(3), &state_reg, &[], &[]);

        assert!(started, "同一目的地への再命令はfind_pathを再実行しないためown_side/enemy_sideに依存しないはず");
        assert_eq!(division.movement_progress, 0.55);
        assert_eq!(division.current_path, vec![StateId(2), StateId(3)]);
    }
}
