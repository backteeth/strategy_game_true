use crate::app::game_state::GameState;
use crate::app::time::DailySimulationSet;
use crate::building::construction::ConstructionQueueItem;
use crate::common::{CountryId, StateId};
use crate::economy::economic_state::EconomicState;
use crate::economy::resources::CountryStockpile;
use crate::politics::interest_groups::CountryPoliticsData;
use crate::politics::reform::PoliticalReform;
use crate::research::allocation::CountryResearchState;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

// ─── 統治体制 ───────────────────────────────────────────────────────────────

/// 統治体制の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GovernmentType {
    #[default]
    Monarchy,
    Republic,
    Dictatorship,
    Theocracy,
}

impl GovernmentType {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            GovernmentType::Monarchy => "government.monarchy",
            GovernmentType::Republic => "government.republic",
            GovernmentType::Dictatorship => "government.dictatorship",
            GovernmentType::Theocracy => "government.theocracy",
        }
    }
}

// ─── 経済体制 ───────────────────────────────────────────────────────────────

/// 経済体制の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EconomicSystem {
    #[default]
    FreeMarket,
    Mercantilism,
    StateCapitalism,
    Socialism,
    PlannedEconomy,
    TempleEconomy,
    GuildEconomy,
}

impl EconomicSystem {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            EconomicSystem::FreeMarket => "economic_system.free_market",
            EconomicSystem::Mercantilism => "economic_system.mercantilism",
            EconomicSystem::StateCapitalism => "economic_system.state_capitalism",
            EconomicSystem::Socialism => "economic_system.socialism",
            EconomicSystem::PlannedEconomy => "economic_system.planned_economy",
            EconomicSystem::TempleEconomy => "economic_system.temple_economy",
            EconomicSystem::GuildEconomy => "economic_system.guild_economy",
        }
    }
}

// ─── 国家データ ──────────────────────────────────────────────────────────────

fn default_tax_rate() -> f32 {
    0.15
}

/// P21-001: `assets/data/countries.ron`は`available_manpower`を明示しないため、
/// 素の`#[serde(default)]`(=`u64::default()`=0)では募兵が常に不可能になってしまう
/// (募兵導線が接続されるまで顕在化していなかった潜在バグ)。`CountryData::default()`と
/// 同じ値を明示的なデフォルトとして使う。
fn default_available_manpower() -> u64 {
    100_000
}

/// 1国家分のゲームデータ（RONデシリアライズ対応）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountryData {
    pub id: CountryId,
    /// 表示名（ASCII推奨）
    pub name: String,
    /// マップカラー (r, g, b) 0.0〜1.0
    pub map_color: [f32; 3],
    /// 首都州ID
    pub capital_state_id: StateId,
    /// 国庫残高（ゴールド）
    pub treasury: f64,
    /// 統治体制
    pub government_type: GovernmentType,
    /// 経済体制
    pub economic_system: EconomicSystem,

    /// 国家資源備蓄
    #[serde(default)]
    pub stockpile: CountryStockpile,
    /// 税率 (0.0〜1.0)
    #[serde(default = "default_tax_rate")]
    pub tax_rate: f32,
    /// 月次収入
    #[serde(default)]
    pub monthly_income: f64,
    /// 月次支出
    #[serde(default)]
    pub monthly_expenses: f64,
    /// 月次収支
    #[serde(default)]
    pub monthly_balance: f64,
    /// 科学研究力
    #[serde(default)]
    pub science_research_capacity: f64,
    /// 魔法研究力
    #[serde(default)]
    pub magic_research_capacity: f64,
    /// 国家経済状況 (5段階)
    #[serde(default)]
    pub economic_state: EconomicState,
    /// 建設キュー
    #[serde(default)]
    pub construction_queue: Vec<ConstructionQueueItem>,

    /// 国家の研究状態
    #[serde(default)]
    pub research_state: CountryResearchState,
    /// 政治・価値観・利益団体データ
    #[serde(default = "CountryPoliticsData::new_default")]
    pub politics: CountryPoliticsData,
    /// 進行中の価値観改革
    #[serde(default)]
    pub current_reform: Option<PoliticalReform>,

    // ── 軍事データ ────────────────────────────────────────────────────────
    /// 募集可能な人的資源
    #[serde(default = "default_available_manpower")]
    pub available_manpower: u64,
    /// 累積動員負担(P23-006で意味を正式化)。募兵によって民間労働力から離脱し、
    /// まだ民間へ返還されていない人的資源の累積量——訓練中(募兵キュー)・現役
    /// (生存Division)・戦闘等で恒久的に失われた人的資源のいずれも含む。
    ///
    /// **「現在生存して軍に所属している人数」ではない**(このフィールド名が示唆する
    /// 印象に反する、P23-005監査で確認された実際の挙動)。募兵時(`request_recruitment`)
    /// に加算され、募兵キャンセル(`cancel_recruitment`)でのみ減算される。戦闘損耗・
    /// 部隊撃破では意図的に減算しない(`military::manpower`モジュールの説明参照)。
    /// 現在生存する軍事人的資源・恒久軍事損失は`military::manpower::compute_manpower_ledger`
    /// で本フィールドとDivision/募兵キューの実データから導出する。フィールド名は
    /// セーブV1互換性のため変更していない。
    #[serde(default)]
    pub mobilized_manpower: u64,
    /// 必要な軍備品の総量
    #[serde(default)]
    pub total_military_equipment_required: f64,
    /// 利用可能な軍備品の総量（備蓄から割り当てられた分）
    #[serde(default)]
    pub total_military_equipment_available: f64,
    /// 月次の軍維持費
    #[serde(default)]
    pub monthly_military_expenses: f64,
    /// 募集キュー
    #[serde(default)]
    pub recruitment_queue: Vec<crate::military::recruitment::RecruitmentQueueItem>,

    /// P21-021: 国家の活動状態(Active/Eliminated)。旧Save(このフィールドが
    /// 存在しない)は`#[serde(default)]`により`Active`として読み込む。
    #[serde(default)]
    pub status: CountryStatus,
}

/// P21-021: 国家の活動状態。`CountryRegistry`から国家を物理削除せず、この
/// フィールドだけを明示的に変更することでWar/Claim/Crisis等の履歴参照を
/// 保つ(CountryIdは不変・再利用しない)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum CountryStatus {
    #[default]
    Active,
    /// 最後の所有陸州を失い消滅した(完全併合)。国家復活・解放はP21-021の
    /// 対象外——`Eliminated`から`Active`へ戻す経路は存在しない。
    Eliminated {
        /// 消滅した日付(既存`GameDate::display()`と同じ`"YYYY/MM/DD"`形式)。
        eliminated_date: String,
        /// 併合した国家(常に実在するActive国家であることをelimination
        /// Domain APIが検証してから設定する)。
        annexed_by: CountryId,
    },
}

impl CountryStatus {
    /// 表示用の翻訳キー(P20-009の`display_name`規約)。
    pub fn display_name(&self) -> &'static str {
        match self {
            CountryStatus::Active => "country_status.active",
            CountryStatus::Eliminated { .. } => "country_status.eliminated",
        }
    }
}

impl Default for CountryData {
    fn default() -> Self {
        Self {
            id: CountryId(0),
            name: "Default Country".to_string(),
            map_color: [0.5, 0.5, 0.5],
            capital_state_id: StateId(0),
            treasury: 1000.0,
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::FreeMarket,
            stockpile: CountryStockpile::default(),
            tax_rate: 0.15,
            monthly_income: 0.0,
            monthly_expenses: 0.0,
            monthly_balance: 0.0,
            science_research_capacity: 10.0,
            magic_research_capacity: 10.0,
            economic_state: EconomicState::default(),
            construction_queue: Vec::new(),
            research_state: CountryResearchState::default(),
            politics: CountryPoliticsData::new_default(),
            current_reform: None,
            available_manpower: 100_000,
            mobilized_manpower: 0,
            total_military_equipment_required: 0.0,
            total_military_equipment_available: 0.0,
            monthly_military_expenses: 0.0,
            recruitment_queue: Vec::new(),
            status: CountryStatus::Active,
        }
    }
}

impl CountryData {
    /// bevy::Color として国家色を取得する
    pub fn bevy_color(&self) -> Color {
        Color::srgb(self.map_color[0], self.map_color[1], self.map_color[2])
    }

    /// P21-021: 現在Activeか(`is_eliminated()`の否定と一致する、唯一の判定関数)。
    /// runtimeロジックは`matches!(status, ...)`を直接複製せず、必ずこの
    /// メソッド(または`is_eliminated()`)を経由する。
    pub fn is_active(&self) -> bool {
        matches!(self.status, CountryStatus::Active)
    }

    /// P21-021: 消滅済みか。
    pub fn is_eliminated(&self) -> bool {
        !self.is_active()
    }

    /// P21-021: 「現在の」首都州(Active国家のみ`Some`)。Eliminated国家は
    /// 所有州が無いため`None`——`capital_state_id`フィールド自体は
    /// 「最後の歴史的首都」として保持し続け、履歴表示にはそちらを直接使う。
    /// runtime処理(validation・AI・UI通常表示)はこのメソッド経由で判定する。
    pub fn current_capital_state_id(&self) -> Option<StateId> {
        if self.is_active() {
            Some(self.capital_state_id)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P21-001回帰テスト: `assets/data/countries.ron`は`available_manpower`を
    /// 明示しない(実データで確認済み)。素の`#[serde(default)]`だと`u64::default()`=0に
    /// なり、募兵UIが接続されるまで発覚しなかった「全国家が常に募兵不可能」という
    /// 潜在バグの原因だった。RONデシリアライズ時に`CountryData::default()`と
    /// 同じ100_000が使われることを固定する。
    #[test]
    fn available_manpower_defaults_to_nonzero_when_absent_from_ron() {
        // countries.ron の実際のエントリ形式(available_manpower省略)を再現
        let ron = r#"(
            id: (0),
            name: "Test Country",
            map_color: (0.5, 0.5, 0.5),
            capital_state_id: (0),
            treasury: 5000.0,
            government_type: Monarchy,
            economic_system: Mercantilism,
            stockpile: (amounts: {}),
        )"#;
        let country: CountryData = ron::from_str(ron).expect("minimal country RON must parse");
        assert_eq!(country.available_manpower, 100_000);
    }

    /// P21-021要求: `status`フィールド自体が存在しない旧Save(P21-020以前)は
    /// `#[serde(default)]`により`CountryStatus::Active`として復元される。
    #[test]
    fn old_format_ron_without_status_field_loads_as_active() {
        let ron = r#"(
            id: (0),
            name: "Test Country",
            map_color: (0.5, 0.5, 0.5),
            capital_state_id: (0),
            treasury: 5000.0,
            government_type: Monarchy,
            economic_system: Mercantilism,
            stockpile: (amounts: {}),
        )"#;
        let country: CountryData = ron::from_str(ron).expect("minimal country RON must parse");
        assert_eq!(country.status, CountryStatus::Active);
        assert!(country.is_active());
        assert!(!country.is_eliminated());
    }

    /// P21-021要求: `CountryStatus::Eliminated`(eliminated_date/annexed_by込み)が
    /// RON往復で完全に保持される。
    #[test]
    fn eliminated_status_round_trips_through_ron() {
        let country = CountryData {
            id: CountryId(1),
            status: CountryStatus::Eliminated {
                eliminated_date: "1850/03/15".to_string(),
                annexed_by: CountryId(0),
            },
            ..CountryData::default()
        };
        let ron_text = ron::to_string(&country).unwrap();
        let restored: CountryData = ron::from_str(&ron_text).unwrap();
        assert_eq!(restored.status, country.status);
        assert!(restored.is_eliminated());
        assert_eq!(restored.current_capital_state_id(), None);
    }

    #[test]
    fn current_capital_state_id_is_only_some_for_active_countries() {
        let mut country = CountryData {
            id: CountryId(1),
            capital_state_id: StateId(5),
            ..CountryData::default()
        };
        assert_eq!(country.current_capital_state_id(), Some(StateId(5)));

        country.status = CountryStatus::Eliminated {
            eliminated_date: "1800/01/01".to_string(),
            annexed_by: CountryId(0),
        };
        assert_eq!(
            country.current_capital_state_id(),
            None,
            "an eliminated country's historical capital_state_id must not be reported as current"
        );
        assert_eq!(
            country.capital_state_id,
            StateId(5),
            "the raw field must still keep the last historical capital"
        );
    }

    #[test]
    fn active_country_ids_excludes_eliminated_countries_and_stays_sorted() {
        let registry = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(2),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(0),
                    status: CountryStatus::Eliminated {
                        eliminated_date: "1800/01/01".to_string(),
                        annexed_by: CountryId(2),
                    },
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
            ],
        };
        assert_eq!(
            registry.active_country_ids(),
            vec![CountryId(1), CountryId(2)]
        );
    }

    // ─── P21-021: プレイヤー国家消滅UX ───────────────────────────────────────

    #[test]
    fn player_elimination_mode_only_allows_commands_when_not_eliminated() {
        assert!(PlayerEliminationMode::NotEliminated.player_can_act());
        assert!(!PlayerEliminationMode::DefeatOverlayPending.player_can_act());
        assert!(!PlayerEliminationMode::Spectating.player_can_act());
    }

    fn detect_test_app(player: CountryId, status: CountryStatus) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(PlayerCountry(Some(player)));
        app.insert_resource(CountryRegistry {
            countries: vec![CountryData {
                id: player,
                status,
                ..CountryData::default()
            }],
        });
        app.insert_resource(PlayerEliminationMode::default());
        app.add_systems(Update, detect_player_elimination);
        app
    }

    #[test]
    fn detect_player_elimination_transitions_to_defeat_overlay_pending() {
        let mut app = detect_test_app(
            CountryId(0),
            CountryStatus::Eliminated {
                eliminated_date: "1800/06/01".to_string(),
                annexed_by: CountryId(1),
            },
        );
        app.update();
        assert_eq!(
            *app.world().resource::<PlayerEliminationMode>(),
            PlayerEliminationMode::DefeatOverlayPending
        );
    }

    #[test]
    fn detect_player_elimination_does_nothing_while_active() {
        let mut app = detect_test_app(CountryId(0), CountryStatus::Active);
        app.update();
        assert_eq!(
            *app.world().resource::<PlayerEliminationMode>(),
            PlayerEliminationMode::NotEliminated
        );
    }

    #[test]
    fn detect_player_elimination_does_not_revert_spectating_back_to_pending() {
        let mut app = detect_test_app(
            CountryId(0),
            CountryStatus::Eliminated {
                eliminated_date: "1800/06/01".to_string(),
                annexed_by: CountryId(1),
            },
        );
        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::Spectating;
        app.update();
        assert_eq!(
            *app.world().resource::<PlayerEliminationMode>(),
            PlayerEliminationMode::Spectating,
            "once the player has dismissed the overlay and is spectating, detection must not \
             revert that back to DefeatOverlayPending on subsequent frames"
        );
    }

    #[test]
    fn player_can_issue_commands_run_condition_reflects_the_mode() {
        #[derive(Resource, Default)]
        struct CallCount(u32);
        fn count_up(mut n: ResMut<CallCount>) {
            n.0 += 1;
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(PlayerEliminationMode::NotEliminated);
        // 直接run conditionを呼ぶのではなく、実際のSystem run_ifとして機能することを
        // 「gateされたSystemが実行された回数」で確認する(実際のBevyスケジューラを
        // 通す検証方法)。
        app.insert_resource(CallCount::default());
        app.add_systems(Update, count_up.run_if(player_can_issue_commands));

        app.update();
        assert_eq!(
            app.world().resource::<CallCount>().0,
            1,
            "NotEliminated must allow the gated system to run"
        );

        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::DefeatOverlayPending;
        app.update();
        assert_eq!(
            app.world().resource::<CallCount>().0,
            1,
            "DefeatOverlayPending must block the gated system"
        );

        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::Spectating;
        app.update();
        assert_eq!(
            app.world().resource::<CallCount>().0,
            1,
            "Spectating must also block the gated system (observing only, no commands)"
        );
    }

    // ─── P22-001: プレイヤー世界征服UX ───────────────────────────────────────

    fn victory_test_app(countries: Vec<CountryData>, player: Option<CountryId>) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(PlayerCountry(player));
        app.insert_resource(CountryRegistry { countries });
        app.insert_resource(PlayerVictoryMode::default());
        app.insert_resource(crate::app::time::GameDate::new(1850, 6, 15));
        app.add_systems(Update, detect_player_victory);
        app
    }

    fn active_country(id: usize) -> CountryData {
        CountryData {
            id: CountryId(id),
            ..CountryData::default()
        }
    }

    fn eliminated_country(id: usize, annexed_by: usize) -> CountryData {
        CountryData {
            id: CountryId(id),
            status: CountryStatus::Eliminated {
                eliminated_date: "1849/01/01".to_string(),
                annexed_by: CountryId(annexed_by),
            },
            ..CountryData::default()
        }
    }

    #[test]
    fn detect_player_victory_transitions_when_player_is_the_last_active_country() {
        let mut app = victory_test_app(
            vec![
                active_country(0),
                eliminated_country(1, 0),
                eliminated_country(2, 0),
            ],
            Some(CountryId(0)),
        );
        app.update();
        assert_eq!(
            *app.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::VictoryOverlayPending {
                victory_date: "1850/06/15".to_string()
            }
        );
    }

    #[test]
    fn detect_player_victory_does_nothing_while_a_rival_is_still_active() {
        let mut app = victory_test_app(
            vec![
                active_country(0),
                active_country(1),
                eliminated_country(2, 0),
            ],
            Some(CountryId(0)),
        );
        app.update();
        assert_eq!(
            *app.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::NotConcluded
        );
    }

    #[test]
    fn detect_player_victory_does_nothing_when_the_player_itself_is_eliminated() {
        // 生き残りの1か国が存在するが、それはプレイヤーではない(=プレイヤーは
        // 消滅済みで観戦中)。「最後の生存国」判定はActive国家だけを見るため、
        // 消滅済みのプレイヤーがここで誤って「勝利」扱いにならないことを確認する。
        let mut app = victory_test_app(
            vec![eliminated_country(0, 1), active_country(1)],
            Some(CountryId(0)),
        );
        app.update();
        assert_eq!(
            *app.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::NotConcluded
        );
    }

    #[test]
    fn detect_player_victory_does_nothing_for_a_degenerate_single_country_world() {
        let mut app = victory_test_app(vec![active_country(0)], Some(CountryId(0)));
        app.update();
        assert_eq!(
            *app.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::NotConcluded
        );
    }

    #[test]
    fn detect_player_victory_does_not_revert_acknowledged_back_to_pending() {
        let mut app = victory_test_app(
            vec![active_country(0), eliminated_country(1, 0)],
            Some(CountryId(0)),
        );
        *app.world_mut().resource_mut::<PlayerVictoryMode>() = PlayerVictoryMode::Acknowledged;
        app.update();
        assert_eq!(
            *app.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::Acknowledged,
            "once the player has dismissed the overlay, detection must not revert that back \
             to VictoryOverlayPending on subsequent frames"
        );
    }

    // ─── P22-002: 観戦中の世界統一検知 ───────────────────────────────────────

    fn conclusion_test_app(countries: Vec<CountryData>, player: Option<CountryId>) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(PlayerCountry(player));
        app.insert_resource(CountryRegistry { countries });
        app.insert_resource(WorldConclusionMode::default());
        app.insert_resource(crate::app::time::GameDate::new(1850, 6, 15));
        app.add_systems(Update, detect_world_conclusion);
        app
    }

    #[test]
    fn detect_world_conclusion_transitions_when_player_is_eliminated_and_one_rival_remains_active()
    {
        let mut app = conclusion_test_app(
            vec![eliminated_country(0, 1), active_country(1)],
            Some(CountryId(0)),
        );
        app.update();
        assert!(matches!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::OverlayPending { .. }
        ));
    }

    #[test]
    fn detect_world_conclusion_records_the_winner_id_and_conclusion_date() {
        let mut app = conclusion_test_app(
            vec![
                eliminated_country(0, 2),
                eliminated_country(1, 2),
                active_country(2),
            ],
            Some(CountryId(0)),
        );
        app.update();
        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::OverlayPending {
                winner_country_id: CountryId(2),
                conclusion_date: "1850/06/15".to_string(),
            }
        );
    }

    #[test]
    fn detect_world_conclusion_does_nothing_while_the_player_is_still_active() {
        let mut app = conclusion_test_app(
            vec![active_country(0), eliminated_country(1, 0)],
            Some(CountryId(0)),
        );
        app.update();
        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::NotConcluded
        );
    }

    #[test]
    fn detect_world_conclusion_does_nothing_when_the_player_itself_is_the_sole_survivor() {
        // これはP22-001(`detect_player_victory`)が扱うべきケースであり、
        // P22-002は「プレイヤーが消滅済み」を前提とするため発火しない。
        let mut app = conclusion_test_app(
            vec![active_country(0), eliminated_country(1, 0)],
            Some(CountryId(0)),
        );
        app.update();
        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::NotConcluded
        );
    }

    #[test]
    fn detect_world_conclusion_does_nothing_when_two_or_more_rivals_remain_active() {
        let mut app = conclusion_test_app(
            vec![
                eliminated_country(0, 1),
                active_country(1),
                active_country(2),
            ],
            Some(CountryId(0)),
        );
        app.update();
        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::NotConcluded
        );
    }

    #[test]
    fn detect_world_conclusion_does_nothing_when_no_country_is_active() {
        let mut app = conclusion_test_app(
            vec![eliminated_country(0, 1), eliminated_country(1, 0)],
            Some(CountryId(0)),
        );
        app.update();
        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::NotConcluded
        );
    }

    #[test]
    fn detect_world_conclusion_does_nothing_for_a_degenerate_single_country_world() {
        let mut app = conclusion_test_app(vec![eliminated_country(0, 0)], Some(CountryId(0)));
        app.update();
        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::NotConcluded
        );
    }

    #[test]
    fn detect_world_conclusion_does_not_revert_acknowledged_back_to_pending() {
        let mut app = conclusion_test_app(
            vec![eliminated_country(0, 1), active_country(1)],
            Some(CountryId(0)),
        );
        *app.world_mut().resource_mut::<WorldConclusionMode>() = WorldConclusionMode::Acknowledged;
        app.update();
        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::Acknowledged,
            "once dismissed, detection must not revert that back to OverlayPending on \
             subsequent frames"
        );
    }

    #[test]
    fn detect_world_conclusion_does_not_update_again_after_multiple_days_once_pending() {
        let mut app = conclusion_test_app(
            vec![eliminated_country(0, 1), active_country(1)],
            Some(CountryId(0)),
        );
        app.update();
        let first = app.world().resource::<WorldConclusionMode>().clone();
        // 日付が進んでも(GameDateを直接書き換えても)、既にPendingになった後は
        // 再評価されず、記録済みの成立日・勝者が上書きされないことを確認する。
        *app.world_mut().resource_mut::<crate::app::time::GameDate>() =
            crate::app::time::GameDate::new(1850, 7, 1);
        app.update();
        app.update();
        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            first,
            "must not re-evaluate or overwrite an already-pending conclusion on later frames"
        );
    }
}

// ─── レジストリ ──────────────────────────────────────────────────────────────

/// ゲーム内全国家データを保持するリソース
#[derive(Resource, Default)]
pub struct CountryRegistry {
    pub countries: Vec<CountryData>,
}

impl CountryRegistry {
    /// ID で国家を検索する
    pub fn get(&self, id: CountryId) -> Option<&CountryData> {
        self.countries.iter().find(|c| c.id == id)
    }

    /// ID で国家を可変検索する
    pub fn get_mut(&mut self, id: CountryId) -> Option<&mut CountryData> {
        self.countries.iter_mut().find(|c| c.id == id)
    }

    /// P21-021: 現在Activeな国家のCountryId一覧を`CountryId`昇順で返す
    /// (HashMap/Vec走査順に依存しない決定論)。AI・PowerTier等、候補国を
    /// 列挙する全箇所がこのメソッドを共有し、`is_active()`判定を個別に
    /// 複製しない。
    pub fn active_country_ids(&self) -> Vec<CountryId> {
        let mut ids: Vec<CountryId> = self
            .countries
            .iter()
            .filter(|c| c.is_active())
            .map(|c| c.id)
            .collect();
        ids.sort_by_key(|id| id.0);
        ids
    }
}

pub mod country_ai;
pub mod elimination;
pub mod power;

use country_ai::{
    CountryAiRegistry, PendingAiWarDeclarations, emit_ai_war_declaration_notifications,
    handle_daily_country_ai,
};
use power::{CountryPowerRegistry, rebuild_country_power_monthly};

// ─── プレイヤー国家 ──────────────────────────────────────────────────────────

/// プレイヤーが選択した国家ID（Playing 中は必ず Some）
#[derive(Resource, Default, Debug)]
pub struct PlayerCountry(pub Option<CountryId>);

/// P21-021: プレイヤー国家の消滅UXの進行状態。`CountryData.status`自体は
/// Saveされるが、この列挙体自体はSaveへ含めない(Load直後は常に`NotEliminated`
/// から始まり、`detect_player_elimination`が同じフレーム/日でロード後の
/// `CountryStatus`を見て即座に`DefeatOverlayPending`へ再遷移する——「観戦を
/// 既に選んでいた」という細かい経緯までは復元しないが、「消滅している」という
/// 事実自体は`CountryStatus`から毎回正しく再導出されるため、Load後に
/// プレイヤー操作が誤って許可されたままになることはない)。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerEliminationMode {
    #[default]
    NotEliminated,
    /// 消滅直後、まだ敗北オーバーレイを閉じていない。
    DefeatOverlayPending,
    /// 敗北オーバーレイを閉じ、観戦モードで継続中(情報閲覧・ゲーム進行は可能)。
    Spectating,
}

impl PlayerEliminationMode {
    /// プレイヤーが自国家として新しい行動(募兵・建設・移動・外交・Claim・
    /// Crisis・War等)を起こせるか。`DefeatOverlayPending`/`Spectating`の
    /// いずれでも`false`(観戦モードでも自国コマンドは発行できない——
    /// 消滅している以上、当然どちらの状態でもコマンドの主体になり得ない)。
    pub fn player_can_act(self) -> bool {
        matches!(self, PlayerEliminationMode::NotEliminated)
    }
}

/// P21-021: プレイヤー国家が消滅した瞬間を検知し、`PlayerEliminationMode`を
/// `DefeatOverlayPending`へ遷移させる。`CountryStatus`(正規のソース)を
/// 毎フレーム参照するため、消滅の実際の発生経路(War講和・Crisis要求受諾の
/// いずれでも)を問わず、また通常プレイでもLoad直後でも同じ1箇所で検知する。
/// 既に`DefeatOverlayPending`/`Spectating`であれば何もしない(冪等)。
pub fn detect_player_elimination(
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    mut mode: ResMut<PlayerEliminationMode>,
) {
    if *mode != PlayerEliminationMode::NotEliminated {
        return;
    }
    let Some(player_id) = player_country.0 else {
        return;
    };
    if country_registry
        .get(player_id)
        .is_some_and(|c| c.is_eliminated())
    {
        *mode = PlayerEliminationMode::DefeatOverlayPending;
    }
}

/// P21-021: プレイヤーが自国コマンドを発行できるかを表すBevy `run_if`条件。
/// 募兵・建設・移動・外交・Claim・Crisis・War・講和の各ボタンハンドラSystemに
/// `.run_if(player_can_issue_commands)`として付与し、Domain APIレベルの
/// `is_active()`検証(§10)に加えて、System自体を実行させない形の防御を
/// 二重にする(UIボタンの非表示/無効化漏れがあっても、この条件がSystem
/// レベルで最終防衛線になる)。
pub fn player_can_issue_commands(mode: Res<PlayerEliminationMode>) -> bool {
    mode.player_can_act()
}

/// P22-001: プレイヤーが世界征服(残る唯一のActive国家)を達成したかのUX進行状態。
/// `PlayerEliminationMode`と対を成す――どちらも`CountryRegistry`(正規のソース)を
/// 毎フレーム参照して再導出する一時的なUI状態であり、Saveへは含めない
/// (Load直後は常に`NotConcluded`から始まり、`detect_player_victory`が同じフレームで
/// ロード後の`CountryRegistry`を見て即座に`VictoryOverlayPending`へ再遷移する)。
/// プレイヤーが消滅済みなら`detect_player_victory`はこの状態を`NotConcluded`から
/// 動かさない(消滅している国家が同時に「最後の生存国」になることはあり得ない
/// ――`active_country_ids()`は`is_active()`な国家しか含まないため)。
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub enum PlayerVictoryMode {
    #[default]
    NotConcluded,
    /// 世界征服達成直後、まだ勝利オーバーレイを閉じていない。到達した日付を
    /// 検知した瞬間の値として保持する(`CountryStatus::Eliminated.eliminated_date`と
    /// 同じ理由――遷移後もゲーム内時間は進み続けるため、表示時点ではなく
    /// 達成時点の日付を固定表示する)。
    VictoryOverlayPending { victory_date: String },
    /// 勝利オーバーレイを閉じ、プレイを継続中。
    Acknowledged,
}

/// P22-001: プレイヤーが世界で唯一のActive国家になった瞬間を検知し、
/// `PlayerVictoryMode`を`VictoryOverlayPending`へ遷移させる。
/// `CountryRegistry::active_country_ids()`(P21-021由来の唯一の列挙経路)を
/// 毎フレーム参照するため、最後の敵国がWar講和・Crisis要求受諾のいずれで
/// 消滅した場合でも同じ1箇所で検知する。既に`VictoryOverlayPending`/
/// `Acknowledged`であれば何もしない(冪等)。1か国しか存在しない世界
/// (征服する相手が最初から存在しない縮退ケース)では発火しない。
pub fn detect_player_victory(
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    game_date: Res<crate::app::time::GameDate>,
    mut mode: ResMut<PlayerVictoryMode>,
) {
    if *mode != PlayerVictoryMode::NotConcluded {
        return;
    }
    if country_registry.countries.len() <= 1 {
        return;
    }
    let Some(player_id) = player_country.0 else {
        return;
    };
    let active_ids = country_registry.active_country_ids();
    if active_ids.len() == 1 && active_ids[0] == player_id {
        *mode = PlayerVictoryMode::VictoryOverlayPending {
            victory_date: game_date.display(),
        };
    }
}

/// P22-002: 観戦中(プレイヤー消滅後)に他国同士の決着で世界が統一された際の
/// 終戦通知UX進行状態。`PlayerVictoryMode`の「プレイヤー自身が最後の生存国」
/// と対称な、「プレイヤー以外の1か国が最後の生存国」ケースを扱う。
/// `PlayerEliminationMode`/`PlayerVictoryMode`と同じ理由でSaveへは含めない
/// (`CountryRegistry`から毎フレーム再導出するランタイムUI状態)。
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub enum WorldConclusionMode {
    #[default]
    NotConcluded,
    /// 世界統一成立直後、まだ観戦者向けオーバーレイを閉じていない。
    /// 到達した日付を検知した瞬間の値として保持する
    /// (`PlayerVictoryMode::VictoryOverlayPending`と同じ理由――敗北オーバーレイを
    /// 閉じた日ではなく、実際にActive国家が1か国になった日を固定表示する)。
    OverlayPending {
        winner_country_id: CountryId,
        conclusion_date: String,
    },
    /// オーバーレイを閉じ、観戦を継続中。
    Acknowledged,
}

/// P22-002: プレイヤーが消滅済みの状態で、プレイヤー以外の1か国だけが
/// 世界で唯一のActive国家になった瞬間を検知し、`WorldConclusionMode`を
/// `OverlayPending`へ遷移させる。`detect_player_victory`(プレイヤー自身が
/// 唯一の生存国のケース)とは、プレイヤーが消滅済みかどうかで判定条件が
/// 完全に排他的なため、同時に両方のPendingへ遷移することはあり得ない。
/// 既に`OverlayPending`/`Acknowledged`であれば何もしない(冪等)。1か国しか
/// 存在しない世界(縮退ケース)、およびActive国家が0か国の場合は発火しない。
///
/// System実行順序: `DailySimulationSet::UiUpdate`(最終Set)に置くことで、
/// 同じフレーム内で先行するWarResolution等による州所有権移転・国家消滅の
/// 反映後に判定する。`detect_player_elimination`/`detect_player_victory`の
/// 後に`.after()`で固定しているが、これはResourceの読み書きが競合するためではなく
/// (`WorldConclusionMode`はどちらとも独立)、「消滅判定・勝利判定が確定した後の
/// 状態を読む」という一貫したフレーム内順序を明示するための保険であり、
/// UI側の表示抑制(`ui::world_conclusion::sync_world_conclusion_overlay`が
/// `PlayerEliminationMode`を見て敗北オーバーレイより先に表示しないよう制御する)
/// と対になる設計判断である。
pub fn detect_world_conclusion(
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    game_date: Res<crate::app::time::GameDate>,
    mut mode: ResMut<WorldConclusionMode>,
) {
    if *mode != WorldConclusionMode::NotConcluded {
        return;
    }
    if country_registry.countries.len() <= 1 {
        return;
    }
    let Some(player_id) = player_country.0 else {
        return;
    };
    if !country_registry
        .get(player_id)
        .is_some_and(|c| c.is_eliminated())
    {
        return;
    }
    let active_ids = country_registry.active_country_ids();
    if active_ids.len() == 1 && active_ids[0] != player_id {
        *mode = WorldConclusionMode::OverlayPending {
            winner_country_id: active_ids[0],
            conclusion_date: game_date.display(),
        };
    }
}

// ─── プラグイン ──────────────────────────────────────────────────────────────

/// 国家関連プラグイン
pub struct CountryPlugin;

impl Plugin for CountryPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CountryRegistry::default())
            .insert_resource(PlayerCountry::default())
            .insert_resource(CountryAiRegistry::default())
            .insert_resource(PendingAiWarDeclarations::default())
            .insert_resource(CountryPowerRegistry::default())
            .insert_resource(PlayerEliminationMode::default())
            .insert_resource(PlayerVictoryMode::default())
            .insert_resource(WorldConclusionMode::default())
            .add_systems(
                Update,
                (
                    handle_daily_country_ai,
                    emit_ai_war_declaration_notifications.after(handle_daily_country_ai),
                )
                    .in_set(DailySimulationSet::CountryAi)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                // P21-014: 月次進行(MonthChangedMessage)のたびに国家総合力を再評価する。
                // 最終SetのUiUpdateに置き、同一フレーム内の他Setの変更を必ず反映する。
                rebuild_country_power_monthly
                    .in_set(DailySimulationSet::UiUpdate)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                // P21-021: 消滅検知は毎フレーム軽量(Option比較+HashMap 1回引き)なため
                // 専用Setを設けず、他の日次進行とは独立にUiUpdateへ置く
                // (Load直後の最初のフレームでも即座に検知できるよう、
                // DayChangedMessage待ちにはしない)。
                detect_player_elimination
                    .in_set(DailySimulationSet::UiUpdate)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                // P22-001: 消滅検知と同じ理由でUiUpdateへ置く(毎フレーム軽量、
                // Load直後の最初のフレームでも即座に検知できるようDayChangedMessage
                // 待ちにしない)。`detect_player_elimination`とは読み書きするResourceが
                // 完全に独立している(前者はPlayerEliminationMode、後者は
                // PlayerVictoryMode)ため、明示的な順序制約は不要。
                detect_player_victory
                    .in_set(DailySimulationSet::UiUpdate)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                // P22-002: 消滅検知・勝利検知と同じ理由でUiUpdateへ置く。
                // `.after()`は排他性の根拠として上のdocコメントに明記した理由で
                // 付与している(Resource競合の回避ではなく、フレーム内で消滅/勝利判定が
                // 確定した後の状態を読むという順序の明示)。
                detect_world_conclusion
                    .in_set(DailySimulationSet::UiUpdate)
                    .after(detect_player_elimination)
                    .after(detect_player_victory)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}
