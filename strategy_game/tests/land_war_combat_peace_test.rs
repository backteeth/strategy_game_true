use strategy_game::common::{ArmyId, CountryId, DivisionId, StateId};
use strategy_game::country::CountryData;
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::combat_calc::resolve_combat_day;
use strategy_game::military::data::{
    ArmyStatus, ArmyUnit, DivisionDefinition, DivisionSize, DivisionType, MilitaryRegistry,
};
use strategy_game::military::invasion::{occupy_state, process_army_arrival};
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::war::capitulation::{evaluate_war_capitulation, CapitulationResult};
use strategy_game::war::combat::sync_battle_results_to_wars;
use strategy_game::war::data::{WarRegistry, WarStatus};
use strategy_game::war::frontline::{update_all_frontlines, FrontlineRegistry};
use strategy_game::war::justification::WarJustificationRegistry;
use strategy_game::war::peace::{execute_peace_settlement, PeaceTerm};
use strategy_game::war::war_score::process_war_score;

/// RONファイルからロードして全4国家が相互に陸続き（直接隣接）になっているかをテストする
#[test]
fn test_all_countries_are_directly_land_connected() {
    let states_ron = std::fs::read_to_string("assets/data/states.ron")
        .expect("assets/data/states.ron should exist");
    let states: Vec<StateData> = ron::from_str(&states_ron)
        .expect("assets/data/states.ron should parse cleanly");
    let state_reg = StateRegistry::build(states);

    let countries_ron = std::fs::read_to_string("assets/data/countries.ron")
        .expect("assets/data/countries.ron should exist");
    let countries: Vec<CountryData> = ron::from_str(&countries_ron)
        .expect("assets/data/countries.ron should parse cleanly");

    let country_ids: Vec<CountryId> = countries.iter().map(|c| c.id).collect();

    // 4国家存在することを確認
    assert_eq!(country_ids.len(), 4, "Expected 4 countries in countries.ron");

    // 全ての国家ペア (c1, c2) について、直接隣接する州が存在するか確認
    for i in 0..country_ids.len() {
        for j in (i + 1)..country_ids.len() {
            let c1 = country_ids[i];
            let c2 = country_ids[j];

            let c1_states = state_reg.get_owned_states(c1);
            let c2_states = state_reg.get_owned_states(c2);

            let mut has_direct_border = false;
            for s1 in &c1_states {
                for s2 in &c2_states {
                    if s1.neighbors.contains(&s2.id) || s2.neighbors.contains(&s1.id) {
                        has_direct_border = true;
                        break;
                    }
                }
                if has_direct_border {
                    break;
                }
            }

            assert!(
                has_direct_border,
                "Country {:?} and Country {:?} should be directly connected by land border",
                c1, c2
            );
        }
    }
}

/// 陸続きの国家間で「正当化 -> 開戦 -> 前線構築 -> 侵攻・戦闘 -> ダメージ発生 -> 占領 -> 戦勝点積算 -> 講和成立」の一連のフローをテストする
#[test]
fn test_land_war_declaration_combat_and_peace_flow() {
    // 1. 環境セットアップ（RONファイルからの実データ読み込み）
    let states_ron = std::fs::read_to_string("assets/data/states.ron").unwrap();
    let states: Vec<StateData> = ron::from_str(&states_ron).unwrap();
    let mut state_reg = StateRegistry::build(states);

    let countries_ron = std::fs::read_to_string("assets/data/countries.ron").unwrap();
    let countries: Vec<CountryData> = ron::from_str(&countries_ron).unwrap();
    let country_reg = strategy_game::country::CountryRegistry { countries };

    let mut diplomacy_reg = DiplomacyRegistry::default();
    let mut justification_reg = WarJustificationRegistry::default();
    let mut war_reg = WarRegistry::default();
    let mut frontline_reg = FrontlineRegistry::default();
    let mut military_reg = MilitaryRegistry::default();
    let mut battle_reg = BattleRegistry::default();

    // 師団定義の追加
    let div_def = DivisionDefinition {
        id: DivisionId(1),
        name: "Standard Infantry".to_string(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower: 10000,
        required_equipment: 100.0,
        recruitment_days: 30,
        movement_speed: 2.0,
        attack: 15.0,
        defense: 12.0,
        breakthrough: 5.0,
        organization: 100.0,
        morale: 100.0,
        supply_usage: 1.0,
        maintenance_cost: 1.0,
    };
    military_reg.definitions.insert(DivisionId(1), div_def);

    let attacker_country = CountryId(0); // Kingdom of Arcadia
    let defender_country = CountryId(3); // Oceanic Magic Empire (今回直接接続した国家)
    let target_state = StateId(8);      // Oceanic の首都州 (State 8)
    let attacker_border_state = StateId(1); // Arcadia の State 1 (State 8 と直接隣接)

    // 隣接の直接確認
    let s1 = state_reg.get(attacker_border_state).unwrap();
    assert!(s1.neighbors.contains(&target_state), "State 1 must border State 8");

    // 2. 正当化プロセスと開戦 (War Declaration)
    let start_date = "1936/01/01".to_string();
    justification_reg
        .start_justification(
            attacker_country,
            defender_country,
            target_state,
            start_date,
            &country_reg,
            &state_reg,
            &diplomacy_reg,
        )
        .expect("Justification start should succeed");

    // 30日経過させて正当化を完成
    for _ in 0..30 {
        justification_reg.process_daily_justifications(&state_reg);
    }

    let war_id = war_reg
        .declare_war(
            attacker_country,
            defender_country,
            target_state,
            "1936/01/31".to_string(),
            &country_reg,
            &state_reg,
            &mut diplomacy_reg,
            &mut justification_reg,
        )
        .expect("War declaration should succeed");

    assert!(war_reg.are_countries_at_war(attacker_country, defender_country));

    // 3. 前線の生成確認 (Frontline Generation)
    update_all_frontlines(&war_reg, &state_reg, &military_reg, &mut frontline_reg);
    let frontline = frontline_reg.get_frontline_for_war(war_id);
    assert!(frontline.is_some(), "Frontline should be created for war");
    let fl = frontline.unwrap();
    assert!(fl.border_region_pairs.contains(&(attacker_border_state, target_state)));

    // 4. 部隊配置と戦闘の開始 (Army Placement & Combat)
    let atk_army = ArmyUnit {
        id: ArmyId(1),
        owner: attacker_country,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: attacker_border_state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 10000,
        max_manpower: 10000,
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: ArmyStatus::Idle,
        def_id: DivisionId(1),
        attack_power: 15,
        defense_power: 12,
        combat_id: None,
    };

    let def_army = ArmyUnit {
        id: ArmyId(2),
        owner: defender_country,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: target_state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 8000,
        max_manpower: 8000,
        equipment: 80.0,
        max_equipment: 80.0,
        organization: 80.0,
        max_organization: 80.0,
        morale: 80.0,
        max_morale: 80.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: ArmyStatus::Idle,
        def_id: DivisionId(1),
        attack_power: 15,
        defense_power: 12,
        combat_id: None,
    };

    military_reg.armies.insert(atk_army.id, atk_army.clone());
    military_reg.armies.insert(def_army.id, def_army.clone());

    // 攻撃部隊が防御側のいる目標州に到着し、戦闘が自動開始されることを検証
    process_army_arrival(
        atk_army.id,
        target_state,
        attacker_border_state,
        "1936/01/31",
        &mut military_reg,
        &mut state_reg,
        &war_reg,
        &mut battle_reg,
    );

    let ongoing_battle = battle_reg.get_ongoing_battle_in_state(target_state);
    assert!(ongoing_battle.is_some(), "Battle should be successfully started on target state");

    // 戦闘計算 (Resolve Combat Day)
    let (atk_loss, _, def_loss, _) = resolve_combat_day(&atk_army, &def_army, 0);
    assert!(atk_loss > 0 || def_loss > 0, "Combat should deal damage to forces");

    // 戦闘勝利のシミュレーションと戦争ログ集計
    sync_battle_results_to_wars(&battle_reg, &mut war_reg);

    // 5. 占領と戦勝点算定 (Occupation & War Score)
    occupy_state(target_state, attacker_country, &mut state_reg);
    occupy_state(StateId(9), attacker_country, &mut state_reg);
    let state_8 = state_reg.get(target_state).unwrap();
    assert_eq!(state_8.controller_country, Some(attacker_country), "Target state should now be controlled by attacker");

    process_war_score(&state_reg, &mut war_reg);
    let active_war = war_reg.wars.get(&war_id).unwrap();
    assert!(active_war.war_score > 0.0, "Attacker war score should be positive after occupying target state");

    // 6. 降伏判定・講和 (Capitulation & Peace Treaty Settlement)
    let cap_result = evaluate_war_capitulation(active_war, &state_reg, &military_reg);
    // 防御側の全地域（60%以上）が占領されたため DefenderCapitulated が評価される
    assert_eq!(cap_result, CapitulationResult::DefenderCapitulated);

    // 講和手続きの実行
    let peace_res = execute_peace_settlement(
        war_id,
        PeaceTerm::CedeWarGoalRegion,
        "Defender Capitulation Settlement",
        "1936/02/15",
        &mut state_reg,
        &mut war_reg,
        &mut military_reg,
        &mut battle_reg,
        &mut diplomacy_reg,
        &mut frontline_reg,
    );

    assert!(peace_res.is_ok(), "Peace settlement execution should succeed");

    // 講和完了の検証
    let ended_war = war_reg.wars.get(&war_id).unwrap();
    assert_eq!(ended_war.status, WarStatus::AttackerVictory, "War status should be AttackerVictory after peace");
    assert!(ended_war.end_date.is_some(), "War end date should be set");
    assert_eq!(ended_war.end_reason.as_deref(), Some("Defender Capitulation Settlement"));

    // 講和により領土割譲（Target State の所有権が攻撃側に移動）したことを確認
    let cceded_state = state_reg.get(target_state).unwrap();
    assert_eq!(cceded_state.owner_country_id, attacker_country, "Target state ownership should be ceded to attacker");

    // 前線が削除・整理されたことを確認
    let fl_after = frontline_reg.get_frontline_for_war(war_id);
    assert!(fl_after.is_none(), "Frontline should be removed after peace settlement");
}
