/// P21-SAVE-002B: ランタイムResourceからSaveGameV1への純粋な変換。
///
/// このファイルはBevyの`Res<T>`/`SystemParam`/`Resource`トレイトを一切扱わない
/// (`bevy`を一度もimportしていない)。`SaveGameSources`は各Resourceの型そのものへの
/// 単純な共有参照(`&'a Xxx`)だけを束ねたプレーンなRust構造体であり、Bevy World/Appを
/// 起動せずに直接構築してテストできる。実際にBevyの`Res<T>`からこれを構築する処理
/// (SystemParamバンドル・PostUpdateへの接続)は`src/save/runtime.rs`が担う。
///
/// `build_save_game_v1`は`&SaveGameSources`(共有参照のみ)を受け取り`SaveGameV1`を返す
/// だけの関数であり、`&mut`を一切取らない。ランタイムResourceを変更できないことは
/// この関数シグネチャ自体によってコンパイル時に保証される。
use crate::app::time::{GameDate, GameSpeed};
use crate::country::country_ai::{CountryAiRegistry, CountryAiState};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::military::army::ArmyRegistry;
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::research::world_stage::WorldCivilizationState;
use crate::save::dto::{
    SAVE_FORMAT_VERSION_V1, SaveGameV1, SavedArmyRegistry, SavedBattleRegistry, SavedClaimRegistry,
    SavedCountryAiRegistry, SavedCountryAiState, SavedCrisisRegistry, SavedDiplomacyRegistry,
    SavedFrontlineRegistry, SavedGameDate, SavedMilitaryAiRegistry, SavedMilitaryAiState,
    SavedMilitaryRegistry, SavedWarJustificationRegistry, SavedWarRegistry,
    SavedWorldCivilizationState,
};
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;
use crate::war::frontline::FrontlineRegistry;
use crate::war::justification::WarJustificationRegistry;
use crate::war::military_ai::{MilitaryAiRegistry, MilitaryAiState};

/// セーブに必要な全Resourceへの読み取り専用参照束。
///
/// プレーンなRust構造体であり、Bevyへの依存を持たない(フィールドは全て`&'a Xxx`であり、
/// `Res<'w, Xxx>`ではない)。実際のBevy Systemからは`runtime::SaveGameResourceParams`
/// (`Res<'w, Xxx>`をまとめたSystemParam)が、各`Res`を`&*`で参照解決してこの構造体を
/// 都度組み立てる。`GamePaused`・描画Entity・UI選択状態・カメラは意図的に含まれていない
/// (フィールドとして存在しないこと自体が、保存対象に含めないことの構造的な保証になる)。
pub struct SaveGameSources<'a> {
    pub game_date: &'a GameDate,
    pub game_speed: &'a GameSpeed,
    pub player_country: &'a PlayerCountry,
    pub world_civilization: &'a WorldCivilizationState,
    pub country_registry: &'a CountryRegistry,
    pub state_registry: &'a StateRegistry,
    pub diplomacy_registry: &'a DiplomacyRegistry,
    pub war_justification_registry: &'a WarJustificationRegistry,
    pub war_registry: &'a WarRegistry,
    pub claim_registry: &'a ClaimRegistry,
    pub crisis_registry: &'a CrisisRegistry,
    pub country_ai_registry: &'a CountryAiRegistry,
    pub military_ai_registry: &'a MilitaryAiRegistry,
    pub military_registry: &'a MilitaryRegistry,
    pub battle_registry: &'a BattleRegistry,
    pub army_registry: &'a ArmyRegistry,
    pub frontline_registry: &'a FrontlineRegistry,
}

/// `SaveGameSources`から完全な`SaveGameV1`を構築する。
///
/// 読み取るだけで、いかなるランタイムResourceも変更しない(引数が全て共有参照のため
/// 変更のしようがない)。静的マスターデータ(`MilitaryRegistry.definitions`/
/// `WorldCivilizationState.stage_definitions`/`StateRegistry.index_map`)は読まない。
/// AIの4つの`dirty`フィールド(`CountryAiRegistry.dirty`/`CountryAiState.dirty`/
/// `MilitaryAiRegistry.dirty`/`MilitaryAiState.dirty`、いずれもP21-SAVE-002A1で
/// 派生状態と確定済み)は読み捨てる。
pub fn build_save_game_v1(sources: &SaveGameSources) -> SaveGameV1 {
    SaveGameV1 {
        version: SAVE_FORMAT_VERSION_V1,
        date: SavedGameDate {
            year: sources.game_date.year,
            month: sources.game_date.month,
            day: sources.game_date.day,
            accumulator: sources.game_date.accumulator(),
        },
        game_speed: sources.game_speed.0,
        player_country: sources.player_country.0,
        world_civilization: SavedWorldCivilizationState {
            current_stage: sources.world_civilization.current_stage,
            milestone_countries: sources.world_civilization.milestone_countries.clone(),
            last_advanced_date: sources.world_civilization.last_advanced_date.clone(),
        },
        countries: sources.country_registry.countries.clone(),
        states: sources.state_registry.states.clone(),
        diplomacy: SavedDiplomacyRegistry {
            relations: sources.diplomacy_registry.relations.clone(),
        },
        war_justifications: SavedWarJustificationRegistry {
            justifications: sources.war_justification_registry.justifications.clone(),
            next_id: sources.war_justification_registry.next_id(),
        },
        wars: SavedWarRegistry {
            wars: sources.war_registry.wars.clone(),
            next_id: sources.war_registry.next_id(),
        },
        claims: SavedClaimRegistry {
            claims: sources.claim_registry.claims.clone(),
            next_id: sources.claim_registry.next_id(),
        },
        crises: SavedCrisisRegistry {
            crises: sources.crisis_registry.crises.clone(),
            next_id: sources.crisis_registry.next_id(),
        },
        country_ai: SavedCountryAiRegistry {
            ai_states: sources
                .country_ai_registry
                .ai_states
                .iter()
                .map(|(id, state)| (*id, export_country_ai_state(state)))
                .collect(),
        },
        military_ai: SavedMilitaryAiRegistry {
            ai_states: sources
                .military_ai_registry
                .ai_states
                .iter()
                .map(|(id, state)| (*id, export_military_ai_state(state)))
                .collect(),
        },
        military: SavedMilitaryRegistry {
            divisions: sources.military_registry.divisions.clone(),
            next_division_id: sources.military_registry.next_division_id(),
        },
        battles: SavedBattleRegistry {
            battles: sources.battle_registry.battles.clone(),
            next_id: sources.battle_registry.next_id(),
        },
        armies: SavedArmyRegistry {
            armies: sources.army_registry.armies.clone(),
            division_army_map: sources.army_registry.division_army_map.clone(),
            next_id: sources.army_registry.next_id(),
            next_army_number: sources.army_registry.next_army_number_map().clone(),
        },
        frontlines: SavedFrontlineRegistry {
            frontlines: sources.frontline_registry.frontlines.clone(),
            plans: sources.frontline_registry.plans.clone(),
            next_frontline_id: sources.frontline_registry.next_frontline_id,
            division_frontline_map: sources.frontline_registry.division_frontline_map.clone(),
            frontline_generated_movements: sources
                .frontline_registry
                .frontline_generated_movements
                .clone(),
            army_frontline_map: sources.frontline_registry.army_frontline_map.clone(),
        },
    }
}

/// `CountryAiState`から派生キャッシュ無効化フラグ(`dirty`)を除いた正規状態だけを取り出す
/// (P21-SAVE-002A1で確定した分類に基づく)。
fn export_country_ai_state(state: &CountryAiState) -> SavedCountryAiState {
    SavedCountryAiState {
        country_id: state.country_id,
        mode: state.mode,
        decision_reason: state.decision_reason,
        last_daily_evaluation_day: state.last_daily_evaluation_day,
        last_weekly_evaluation_day: state.last_weekly_evaluation_day,
        last_monthly_evaluation_day: state.last_monthly_evaluation_day,
        cooldown_until_day: state.cooldown_until_day,
    }
}

/// `MilitaryAiState`から派生キャッシュ無効化フラグ(`dirty`)を除いた正規状態だけを取り出す。
fn export_military_ai_state(state: &MilitaryAiState) -> SavedMilitaryAiState {
    SavedMilitaryAiState {
        country_id: state.country_id,
        last_evaluated_day: state.last_evaluated_day,
        last_decision_reason: state.last_decision_reason,
        estimated_own_power: state.estimated_own_power,
        estimated_enemy_power: state.estimated_enemy_power,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{
        ArmyId, BattleId, ClaimId, CountryId, DiplomaticCrisisId, DivisionDefinitionId, DivisionId,
        FrontlineId, StateId, WarId,
    };
    use crate::country::country_ai::{CountryAiDecisionReason, CountryAiMode};
    use crate::country::{CountryData, EconomicSystem, GovernmentType};
    use crate::diplomacy::claims::{ClaimSource, TerritorialClaim};
    use crate::diplomacy::crisis::{CrisisPhase, DiplomaticCrisis};
    use crate::military::data::{Division, DivisionSize, DivisionStatus, DivisionType};
    use crate::research::data::WorldStage;
    use crate::state::data::StateData;
    use crate::war::military_ai::MilitaryAiDecisionReason;
    use std::collections::HashMap;

    fn make_division() -> Division {
        Division {
            id: DivisionId(7),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(3),
            destination: Some(StateId(9)),
            current_path: vec![StateId(4), StateId(9)],
            target_state: None,
            manpower: 8_500,
            max_manpower: 10_000,
            equipment: 90.0,
            max_equipment: 100.0,
            organization: 42.5,
            max_organization: 100.0,
            morale: 0.8,
            max_morale: 1.0,
            experience: 3.5,
            supply_ratio: 0.95,
            movement_progress: 0.42,
            status: DivisionStatus::Moving,
            def_id: DivisionDefinitionId(1),
            attack_power: 12,
            defense_power: 8,
            combat_id: None,
        }
    }

    /// テスト用に代表的な全Resourceを構築する(往復テスト共通のfixture)。
    /// `#[allow(clippy::type_complexity)]`は不要なほど単純なタプル返却に留める。
    struct RepresentativeResources {
        game_date: GameDate,
        game_speed: GameSpeed,
        player_country: PlayerCountry,
        world_civilization: WorldCivilizationState,
        country_registry: CountryRegistry,
        state_registry: StateRegistry,
        diplomacy_registry: DiplomacyRegistry,
        war_justification_registry: WarJustificationRegistry,
        war_registry: WarRegistry,
        claim_registry: ClaimRegistry,
        crisis_registry: CrisisRegistry,
        country_ai_registry: CountryAiRegistry,
        military_ai_registry: MilitaryAiRegistry,
        military_registry: MilitaryRegistry,
        battle_registry: BattleRegistry,
        army_registry: ArmyRegistry,
        frontline_registry: FrontlineRegistry,
    }

    impl RepresentativeResources {
        fn sources(&self) -> SaveGameSources<'_> {
            SaveGameSources {
                game_date: &self.game_date,
                game_speed: &self.game_speed,
                player_country: &self.player_country,
                world_civilization: &self.world_civilization,
                country_registry: &self.country_registry,
                state_registry: &self.state_registry,
                diplomacy_registry: &self.diplomacy_registry,
                war_justification_registry: &self.war_justification_registry,
                war_registry: &self.war_registry,
                claim_registry: &self.claim_registry,
                crisis_registry: &self.crisis_registry,
                country_ai_registry: &self.country_ai_registry,
                military_ai_registry: &self.military_ai_registry,
                military_registry: &self.military_registry,
                battle_registry: &self.battle_registry,
                army_registry: &self.army_registry,
                frontline_registry: &self.frontline_registry,
            }
        }
    }

    fn representative_resources() -> RepresentativeResources {
        let mut game_date = GameDate::new(1801, 6, 15);
        game_date.add_accumulator(0.37);

        let mut military_registry = MilitaryRegistry::default();
        // 静的マスターデータ(師団定義)も併せて投入し、divisionsとは別枠であることを
        // 変換結果側(SavedMilitaryRegistryにdefinitionsフィールドが存在しないこと)で検証する。
        military_registry.definitions.insert(
            DivisionDefinitionId(1),
            crate::military::data::DivisionDefinition {
                id: DivisionDefinitionId(1),
                name: "Infantry".to_string(),
                division_type: DivisionType::Infantry,
                size: DivisionSize::Standard,
                required_manpower: 1000,
                required_equipment: 10.0,
                recruitment_days: 10,
                movement_speed: 1.0,
                attack: 10.0,
                defense: 10.0,
                breakthrough: 5.0,
                organization: 100.0,
                morale: 1.0,
                supply_usage: 1.0,
                maintenance_cost: 1.0,
            },
        );
        let division = make_division();
        military_registry.divisions.insert(division.id, division);

        let mut army_registry = ArmyRegistry::default();
        // next_id/next_army_numberはprivateのため、create_armyの本番経路で間接的に
        // 埋める(直接コンストラクトできない=privateフィールドを迂回していないことの証拠)。
        let created_id = army_registry
            .create_army(CountryId(1), &[DivisionId(7)], &military_registry)
            .expect("division must be usable");
        assert_eq!(
            created_id,
            ArmyId(0),
            "first army created in a fresh registry must get ArmyId(0)"
        );

        // ClaimRegistry/CrisisRegistryのnext_idもprivateのため、add_claim/add_crisisの
        // 本番経路で埋める(struct更新構文{ ..Default::default() }はprivateフィールドを
        // 暗黙にコピーする場合でも呼び出し元モジュールからの可視性を要求するため使えない)。
        let mut claim_registry = ClaimRegistry::default();
        let claim_id = claim_registry.add_claim(TerritorialClaim {
            id: ClaimId(0), // add_claimが上書きするため実際の値はダミーでよい
            claimant_country: CountryId(1),
            target_state: StateId(9),
            strength: 42.0,
            created_date: "1800/03/01".to_string(),
            is_permanent: false,
            source: ClaimSource::BorderDispute,
            status: crate::diplomacy::claims::ClaimStatus::Active,
        });
        assert_eq!(claim_id, ClaimId(0));

        let mut crisis_registry = CrisisRegistry::default();
        let crisis_id = crisis_registry.add_crisis(DiplomaticCrisis {
            id: DiplomaticCrisisId(0), // add_crisisが上書きするため実際の値はダミーでよい
            initiator: CountryId(1),
            target: CountryId(2),
            war_goals: Vec::new(),
            start_date: "1800/02/01".to_string(),
            current_phase: CrisisPhase::Negotiating,
            escalation: 15.0,
            initiator_support: 60.0,
            target_resistance: 40.0,
            days_in_phase: 3,
            deadline_date: Some("1800/03/01".to_string()),
            international_concern: 5.0,
            third_party_reactions: HashMap::new(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        });
        assert_eq!(crisis_id, DiplomaticCrisisId(0));

        let mut country_ai_registry = CountryAiRegistry::default();
        country_ai_registry.ai_states.insert(
            CountryId(2),
            CountryAiState {
                country_id: CountryId(2),
                mode: CountryAiMode::AtWar,
                decision_reason: CountryAiDecisionReason::WarInProgress,
                last_daily_evaluation_day: 10,
                last_weekly_evaluation_day: 7,
                last_monthly_evaluation_day: 0,
                cooldown_until_day: 15,
                dirty: true,
            },
        );

        let mut military_ai_registry = MilitaryAiRegistry::default();
        military_ai_registry.ai_states.insert(
            CountryId(2),
            MilitaryAiState {
                country_id: CountryId(2),
                last_evaluated_day: 10,
                last_decision_reason: MilitaryAiDecisionReason::SufficientAdvantage,
                estimated_own_power: 5_000,
                estimated_enemy_power: 3_000,
                dirty: true,
            },
        );

        let country = CountryData {
            id: CountryId(1),
            name: "Test Country".to_string(),
            map_color: [0.2, 0.4, 0.6],
            capital_state_id: StateId(0),
            treasury: 5_000.0,
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::Mercantilism,
            ..CountryData::default()
        };
        let country_registry = CountryRegistry {
            countries: vec![country],
        };

        let state = StateData {
            id: StateId(9),
            name: "Contested State".to_string(),
            owner_country_id: CountryId(2),
            controller_country: Some(CountryId(1)),
            occupation_progress: 55.0,
            ..StateData::default()
        };
        let state_registry = StateRegistry::build(vec![state]);

        let mut world_civilization = WorldCivilizationState {
            current_stage: WorldStage::IndustrialRevolution,
            last_advanced_date: "1801/01/01".to_string(),
            ..WorldCivilizationState::default()
        };
        // 静的マスターデータ(stage_definitions)も投入し、変換結果に含まれないことを
        // (SavedWorldCivilizationStateにそのフィールドが存在しないことで)検証する。
        world_civilization.stage_definitions.insert(
            WorldStage::IndustrialRevolution,
            crate::research::world_stage::WorldStageDefinition {
                stage: WorldStage::IndustrialRevolution,
                display_name: "Industrial".to_string(),
                description: "desc".to_string(),
                required_previous_stage: Some(WorldStage::PreIndustrial),
                milestone_technologies: Vec::new(),
                required_country_count: 1,
            },
        );

        let mut battle_registry = BattleRegistry::default();
        let _ = battle_registry.start_battle(crate::military::battle::Battle {
            id: BattleId(0),
            war_id: WarId(0),
            state_id: StateId(9),
            attacker_country: CountryId(1),
            defender_country: CountryId(2),
            attacker_division_ids: vec![DivisionId(7)],
            defender_division_ids: vec![],
            attacker_origins: HashMap::new(),
            start_date: "1801/06/10".to_string(),
            elapsed_days: 2,
            status: crate::military::battle::BattleStatus::Ongoing,
        });

        RepresentativeResources {
            game_date,
            game_speed: GameSpeed(2),
            player_country: PlayerCountry(Some(CountryId(1))),
            world_civilization,
            country_registry,
            state_registry,
            diplomacy_registry: DiplomacyRegistry::default(),
            war_justification_registry: WarJustificationRegistry::default(),
            war_registry: WarRegistry::default(),
            claim_registry,
            crisis_registry,
            country_ai_registry,
            military_ai_registry,
            military_registry,
            battle_registry,
            army_registry,
            frontline_registry: FrontlineRegistry::default(),
        }
    }

    #[test]
    fn builds_full_save_game_v1_with_correct_version() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());
        assert_eq!(save.version, SAVE_FORMAT_VERSION_V1);
    }

    #[test]
    fn game_date_including_accumulator_is_converted() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());
        assert_eq!(save.date.year, 1801);
        assert_eq!(save.date.month, 6);
        assert_eq!(save.date.day, 15);
        assert!((save.date.accumulator - 0.37).abs() < f64::EPSILON);
    }

    #[test]
    fn game_speed_and_player_country_are_converted() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());
        assert_eq!(save.game_speed, 2);
        assert_eq!(save.player_country, Some(CountryId(1)));
    }

    #[test]
    fn country_and_state_normative_state_is_converted() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());

        assert_eq!(save.countries.len(), 1);
        assert_eq!(save.countries[0].id, CountryId(1));
        assert_eq!(save.countries[0].treasury, 5_000.0);

        assert_eq!(save.states.len(), 1);
        assert_eq!(save.states[0].id, StateId(9));
        assert_eq!(save.states[0].controller_country, Some(CountryId(1)));
    }

    #[test]
    fn diplomacy_war_and_justification_are_converted() {
        let mut resources = representative_resources();

        let war = crate::war::data::War {
            id: WarId(0),
            name: "Test War".to_string(),
            attackers: [CountryId(1)].into_iter().collect(),
            defenders: [CountryId(2)].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
            war_goals: Vec::new(),
            start_date: "1801/01/01".to_string(),
            end_date: None,
            duration_days: 5,
            war_score: 10.0,
            attacker_war_exhaustion: 1.0,
            defender_war_exhaustion: 2.0,
            occupied_states: [StateId(9)].into_iter().collect(),
            status: crate::war::data::WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 1,
            won_defender_battles: 0,
            processed_battle_ids: [BattleId(0)].into_iter().collect(),
        };
        resources.war_registry.wars.insert(war.id, war);

        let save = build_save_game_v1(&resources.sources());
        let saved_war = save.wars.wars.get(&WarId(0)).unwrap();
        assert_eq!(saved_war.status, crate::war::data::WarStatus::Active);
        assert_eq!(saved_war.occupied_states.len(), 1);
    }

    #[test]
    fn claim_and_crisis_are_present_as_fields_even_when_empty() {
        let mut resources = representative_resources();
        resources.claim_registry = ClaimRegistry::default();
        resources.crisis_registry = CrisisRegistry::default();

        let save = build_save_game_v1(&resources.sources());
        assert!(save.claims.claims.is_empty());
        assert_eq!(save.claims.next_id, 0);
        assert!(save.crises.crises.is_empty());
        assert_eq!(save.crises.next_id, 0);
    }

    #[test]
    fn claim_and_crisis_contents_and_next_id_are_converted_when_nonempty() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());

        assert_eq!(save.claims.claims.len(), 1);
        let claim = save.claims.claims.get(&ClaimId(0)).unwrap();
        assert_eq!(claim.strength, 42.0);
        assert_eq!(save.claims.next_id, resources.claim_registry.next_id());

        assert_eq!(save.crises.crises.len(), 1);
        let crisis = save.crises.crises.get(&DiplomaticCrisisId(0)).unwrap();
        assert_eq!(crisis.current_phase, CrisisPhase::Negotiating);
        assert_eq!(save.crises.next_id, resources.crisis_registry.next_id());
    }

    #[test]
    fn division_and_next_division_id_are_converted() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());

        let division = save.military.divisions.get(&DivisionId(7)).unwrap();
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(
            save.military.next_division_id,
            resources.military_registry.next_division_id()
        );
    }

    #[test]
    fn battle_and_next_id_are_converted() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());

        assert_eq!(save.battles.battles.len(), 1);
        let battle = save.battles.battles.get(&BattleId(0)).unwrap();
        assert_eq!(battle.state_id, StateId(9));
        assert_eq!(save.battles.next_id, resources.battle_registry.next_id());
    }

    #[test]
    fn army_membership_reverse_map_and_naming_counter_are_converted() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());

        assert_eq!(save.armies.armies.len(), 1);
        let army = save.armies.armies.get(&ArmyId(0)).unwrap();
        assert_eq!(army.member_division_ids, vec![DivisionId(7)]);
        assert_eq!(
            save.armies.division_army_map.get(&DivisionId(7)),
            Some(&ArmyId(0))
        );
        assert_eq!(save.armies.next_id, resources.army_registry.next_id());
        assert_eq!(
            save.armies.next_army_number.get(&CountryId(1)),
            resources
                .army_registry
                .next_army_number_map()
                .get(&CountryId(1))
        );
    }

    #[test]
    fn frontline_registry_all_six_fields_are_converted() {
        let mut resources = representative_resources();
        let frontline = crate::war::frontline::Frontline {
            frontline_id: FrontlineId(0),
            war_id: WarId(0),
            attacker_country_id: CountryId(1),
            defender_country_id: CountryId(2),
            attacker_front_regions: vec![StateId(3)],
            defender_front_regions: vec![StateId(4)],
            border_region_pairs: vec![(StateId(3), StateId(4))],
        };
        resources
            .frontline_registry
            .frontlines
            .insert(frontline.frontline_id, frontline);
        let plan = crate::war::frontline::FrontlinePlan {
            frontline_id: FrontlineId(0),
            commanding_country_id: CountryId(1),
            stance: crate::war::frontline::FrontlineStance::Offensive,
            objective_region_id: Some(StateId(4)),
            assigned_division_ids: vec![DivisionId(7)],
            // ArmyId(0)はrepresentative_resources()自体がCountryId(1)所有として用意する既存Army。
            assigned_army_ids: vec![ArmyId(0)],
            offensive_line_region_ids: None,
        };
        resources
            .frontline_registry
            .plans
            .insert((FrontlineId(0), CountryId(1)), plan);
        resources
            .frontline_registry
            .division_frontline_map
            .insert(DivisionId(7), FrontlineId(0));
        resources
            .frontline_registry
            .frontline_generated_movements
            .insert(DivisionId(7));
        resources
            .frontline_registry
            .army_frontline_map
            .insert(ArmyId(0), FrontlineId(0));
        resources.frontline_registry.next_frontline_id = 1;

        let save = build_save_game_v1(&resources.sources());
        assert_eq!(save.frontlines.frontlines.len(), 1);
        assert_eq!(save.frontlines.plans.len(), 1);
        assert_eq!(save.frontlines.next_frontline_id, 1);
        assert_eq!(
            save.frontlines.division_frontline_map.get(&DivisionId(7)),
            Some(&FrontlineId(0))
        );
        assert!(
            save.frontlines
                .frontline_generated_movements
                .contains(&DivisionId(7))
        );
        assert_eq!(
            save.frontlines.army_frontline_map.get(&ArmyId(0)),
            Some(&FrontlineId(0)),
            "P21-005: army_frontline_map must be converted from the runtime FrontlineRegistry"
        );
        let saved_plan = save
            .frontlines
            .plans
            .get(&(FrontlineId(0), CountryId(1)))
            .unwrap();
        assert_eq!(saved_plan.assigned_army_ids, vec![ArmyId(0)]);
    }

    #[test]
    fn only_mutable_part_of_world_civilization_state_is_converted() {
        let resources = representative_resources();
        // 元のResourceには静的マスターデータ(stage_definitions)が入っていることを確認
        assert!(!resources.world_civilization.stage_definitions.is_empty());

        let save = build_save_game_v1(&resources.sources());
        assert_eq!(
            save.world_civilization.current_stage,
            WorldStage::IndustrialRevolution
        );
        assert_eq!(save.world_civilization.last_advanced_date, "1801/01/01");
        // SavedWorldCivilizationStateはそもそもstage_definitionsフィールドを持たない
        // (dto.rsの型定義を参照)。構造的に保存されない。
    }

    #[test]
    fn military_registry_definitions_are_not_converted() {
        let resources = representative_resources();
        // 元のResourceには静的マスターデータ(師団定義)が入っていることを確認
        assert!(!resources.military_registry.definitions.is_empty());

        let save = build_save_game_v1(&resources.sources());
        assert_eq!(save.military.divisions.len(), 1);
        // SavedMilitaryRegistryはそもそもdefinitionsフィールドを持たない
        // (dto.rsの型定義を参照)。構造的に保存されない。
    }

    #[test]
    fn state_registry_index_map_is_not_converted() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());
        assert_eq!(save.states.len(), 1);
        // SavedGameV1.statesはVec<StateData>であり、StateRegistry.index_mapに相当する
        // フィールドは構造的に存在しない(dto.rsの型定義を参照)。
    }

    #[test]
    fn conversion_does_not_require_gamepaused_entity_ui_or_camera() {
        // このファイル(export.rs)はbevyを一度もimportしていない。build_save_game_v1の
        // シグネチャは`&SaveGameSources`のみを取り、GamePausedを一切受け取らない。
        let resources = representative_resources();
        let _save = build_save_game_v1(&resources.sources());
    }

    #[test]
    fn conversion_does_not_mutate_source_resources() {
        let resources = representative_resources();

        let before_treasury = resources.country_registry.countries[0].treasury;
        let before_next_division_id = resources.military_registry.next_division_id();
        let before_next_army_id = resources.army_registry.next_id();
        let before_country_ai_dirty = resources
            .country_ai_registry
            .ai_states
            .get(&CountryId(2))
            .unwrap()
            .dirty;
        let before_military_ai_dirty = resources
            .military_ai_registry
            .ai_states
            .get(&CountryId(2))
            .unwrap()
            .dirty;

        // build_save_game_v1は`&SaveGameSources`(共有参照の束)しか受け取れないため、
        // 変更はコンパイルエラーになる(構造的な保証)。念のため実行後も値を再確認する。
        let _save = build_save_game_v1(&resources.sources());

        assert_eq!(
            resources.country_registry.countries[0].treasury,
            before_treasury
        );
        assert_eq!(
            resources.military_registry.next_division_id(),
            before_next_division_id
        );
        assert_eq!(resources.army_registry.next_id(), before_next_army_id);
        assert_eq!(
            resources
                .country_ai_registry
                .ai_states
                .get(&CountryId(2))
                .unwrap()
                .dirty,
            before_country_ai_dirty
        );
        assert_eq!(
            resources
                .military_ai_registry
                .ai_states
                .get(&CountryId(2))
                .unwrap()
                .dirty,
            before_military_ai_dirty
        );
    }

    #[test]
    fn country_ai_normative_state_is_converted_and_dirty_is_dropped() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());

        let saved_state = save.country_ai.ai_states.get(&CountryId(2)).unwrap();
        assert_eq!(saved_state.mode, CountryAiMode::AtWar);
        assert_eq!(
            saved_state.decision_reason,
            CountryAiDecisionReason::WarInProgress
        );
        assert_eq!(saved_state.cooldown_until_day, 15);
        // SavedCountryAiStateはそもそもdirtyフィールドを持たない(dto.rsの型定義を参照)。
    }

    #[test]
    fn military_ai_normative_state_is_converted_and_dirty_is_dropped() {
        let resources = representative_resources();
        let save = build_save_game_v1(&resources.sources());

        let saved_state = save.military_ai.ai_states.get(&CountryId(2)).unwrap();
        assert_eq!(
            saved_state.last_decision_reason,
            MilitaryAiDecisionReason::SufficientAdvantage
        );
        assert_eq!(saved_state.estimated_own_power, 5_000);
        // SavedMilitaryAiStateはそもそもdirtyフィールドを持たない(dto.rsの型定義を参照)。
    }

    /// 要求テスト項目: dirtyだけ異なるランタイムAI状態から同一のSaved AI DTOが生成される。
    #[test]
    fn dirty_only_difference_produces_semantically_identical_saved_country_ai_dto() {
        let mut resources_a = representative_resources();
        let mut resources_b = representative_resources();
        resources_a
            .country_ai_registry
            .ai_states
            .get_mut(&CountryId(2))
            .unwrap()
            .dirty = true;
        resources_b
            .country_ai_registry
            .ai_states
            .get_mut(&CountryId(2))
            .unwrap()
            .dirty = false;

        let save_a = build_save_game_v1(&resources_a.sources());
        let save_b = build_save_game_v1(&resources_b.sources());

        assert_eq!(
            save_a.country_ai.ai_states.get(&CountryId(2)),
            save_b.country_ai.ai_states.get(&CountryId(2)),
            "dirty以外が同一なら、変換結果のSavedCountryAiStateも意味的に同一であること"
        );
    }

    #[test]
    fn dirty_only_difference_produces_semantically_identical_saved_military_ai_dto() {
        let mut resources_a = representative_resources();
        let mut resources_b = representative_resources();
        resources_a
            .military_ai_registry
            .ai_states
            .get_mut(&CountryId(2))
            .unwrap()
            .dirty = true;
        resources_b
            .military_ai_registry
            .ai_states
            .get_mut(&CountryId(2))
            .unwrap()
            .dirty = false;

        let save_a = build_save_game_v1(&resources_a.sources());
        let save_b = build_save_game_v1(&resources_b.sources());

        assert_eq!(
            save_a.military_ai.ai_states.get(&CountryId(2)),
            save_b.military_ai.ai_states.get(&CountryId(2)),
            "dirty以外が同一なら、変換結果のSavedMilitaryAiStateも意味的に同一であること"
        );
    }
}
