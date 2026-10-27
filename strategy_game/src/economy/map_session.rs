//! P32-019 (S0): 新マップ経済セッションの**組み立て**と、画面(将来の独立バイナリ `map_economy_session`)・CLI が
//! 共有する確認API(Bevy非依存)。画面そのものは含まない(P32-018 の設計の S0)。
//!
//! ## 共有するもの
//! - 静的入力の読み込みと検証: 地理(`HexMap`)・ワールド設定(`WorldModel`)・建物定義・位置施設
//!   ([`SessionPaths`]・[`GeographyData`]・[`EconomySession::load_files`])
//! - シナリオと国家構造から `MapEconomySimulation` を生成([`build_simulation`])。**状態の正本は
//!   `MapEconomySimulation` 1つ**で、ここに経済状態のコピーや別の更新経路は無い
//! - 保存・復元・事前確認・一覧([`EconomySession::save`]/[`load`](EconomySession::load)/[`inspect`](EconomySession::inspect)/
//!   [`list_saves`])。実体は `economy::map_economy_file` の同じ関数(CLI と同じ)
//! - 操作主体の解決・検証([`EconomySession::resolve_actor`])と、受付可否の確認([`EconomySession::check_build`])
//!
//! ## 検証用シナリオの既定
//! 画面の既定は **m4e**(手動国家1・2とAI国家3)+資源 `province_resources_m4d.ron`(U3)。既存CLIの既定は従来どおり
//! **m4d**(全国家AI)で、`--scenario` で明示的に選ぶ。この対応は m4e のシナリオ先頭のコメントと
//! `tests/p32_019_*` が確認している(黙って組み合わせない)。
//!
//! ## 操作主体
//! 国家は検証シナリオのラベル(`u32`)で指す。旧 `CountryId` との対応は数値や列挙順から推測しない(別の型・別の意味)。
//! 状態を変更する操作(`request_build`・`cancel_order`)は、**明示した手動(非AI)国家**でしか受け付けない。
//! 未選択・存在しない国家・AI国家を、別の国家(列挙順の先頭など)へ置き換えることはしない。
//! 参照(`state_economy`・`country`・`check_build` の確認)は未選択でも可能だが、未選択の確認は受理可能とは返さない。
//! 国家とステートの対応は検証シナリオの**経済計算上の対応関係**で、法的所有・占領のモデルではない。

use crate::building::data::{BuildingRegistry, BuildingType};
use crate::economy::map_economy_file::{
    FileError, InspectReport, LoadReceipt, SaveList, SaveLocation, SaveReceipt,
};
use crate::economy::map_state::snapshot::MapEconomySimulation;
use crate::economy::map_state::static_identity::{IdentityError, StaticIdentity, StaticInputs};
use crate::economy::map_state::{
    CancelError, CancelReceipt, CancelTarget, ConstructionOrderId, DayReport, MapStateEconomy,
    MapStateEconomyRegistry, OrderCheck, OrderError, ScenarioCountry, SimDate,
    parse_multi_month_scenario,
};
use crate::economy::province_resources::{derive_state_resources, parse_province_resources};
use crate::hexmap::data::{HexMap, load_from_path};
use crate::world::{
    LocationFacilityRegistry, MapStateId, WorldModel, WorldPreviewData, load_location_facilities,
    load_world_preview,
};

// ---------------------------------------------------------------- シナリオ表

/// 検証用シナリオ(正式な世界設定・ゲーム開始ではない)。経済シナリオと、それが前提にする資源設定の対応を固定する。
#[derive(Debug, PartialEq, Eq)]
pub struct SessionScenario {
    pub id: &'static str,
    pub label: &'static str,
    pub scenario_path: &'static str,
    pub resources_path: &'static str,
    /// 開始日(まだ1日も処理していない日)。
    pub start: SimDate,
}

const START: SimDate = SimDate { year: 1800, month: 1, day: 1 };

/// 既知の検証用シナリオ。`m4e` も資源は `province_resources_m4d.ron`(検証地域の全プロヴィンスを設定済み)を使う。
pub const SCENARIOS: [SessionScenario; 2] = [
    SessionScenario {
        id: "m4d",
        label: "M4-D 軍事状況を参照する経済AI(全国家AI)",
        scenario_path: "assets/data/world/preview/economy_scenario_m4d.ron",
        resources_path: "assets/data/world/preview/province_resources_m4d.ron",
        start: START,
    },
    SessionScenario {
        id: "m4e",
        label: "M4-E 手動建設の検証(国家1・2が手動、国家3がAI)",
        scenario_path: "assets/data/world/preview/economy_scenario_m4e.ron",
        resources_path: "assets/data/world/preview/province_resources_m4d.ron",
        start: START,
    },
];

/// 新しい画面の既定シナリオ(U3)。
pub const SCREEN_DEFAULT_SCENARIO: &str = "m4e";
/// 既存CLI(`world_economy_check --economy-*`)の既定シナリオ。従来どおり変えない。
pub const CLI_DEFAULT_SCENARIO: &str = "m4d";

pub fn scenario_by_id(id: &str) -> Option<&'static SessionScenario> {
    SCENARIOS.iter().find(|s| s.id == id)
}

// ---------------------------------------------------------------- エラー

#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    UnknownScenario(String),
    /// ファイルの読み込みに失敗した。
    Io { path: String, message: String },
    /// 地図・ワールド設定・位置施設・建物定義・資源設定の読み込み/検証に失敗した。
    Data { what: &'static str, message: String },
    /// シナリオの解析・検証・経済の組み立てに失敗した。
    Scenario(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::UnknownScenario(id) => write!(
                f,
                "unknown scenario \"{id}\" (known: {})",
                SCENARIOS.iter().map(|s| s.id).collect::<Vec<_>>().join(", ")
            ),
            SessionError::Io { path, message } => write!(f, "cannot read {path}: {message}"),
            SessionError::Data { what, message } => write!(f, "{what}: {message}"),
            SessionError::Scenario(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for SessionError {}

fn read_text(path: &str) -> Result<String, SessionError> {
    std::fs::read_to_string(path)
        .map_err(|e| SessionError::Io { path: path.to_string(), message: e.to_string() })
}

// ---------------------------------------------------------------- 静的入力の場所と読み込み

/// 静的入力ファイルの場所。既定は通常のアセット。場所を選ぶだけで、検証を無効にするものではない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPaths {
    pub map: String,
    pub provinces: String,
    pub states: String,
    pub buildings: String,
    pub facilities: String,
}

impl Default for SessionPaths {
    fn default() -> Self {
        SessionPaths {
            map: "assets/data/hex_atlas_map.json".to_string(),
            provinces: "assets/data/world/preview/provinces.ron".to_string(),
            states: "assets/data/world/preview/states.ron".to_string(),
            buildings: "assets/data/buildings.ron".to_string(),
            facilities: "assets/data/world/preview/province_facilities.ron".to_string(),
        }
    }
}

/// 読み込み済みの地理とワールド設定(CLI・テスト用の所有版)。画面は既存のリソース
/// (`hexmap::view::LoadedHexMap`・`hexmap::world_view::LoadedWorld`)を使い、これを二重に読まない。
pub struct GeographyData {
    pub map: HexMap,
    pub preview: WorldPreviewData,
}

impl GeographyData {
    /// 地図とワールド設定を読み込む。ワールド設定は地図との整合を含めて検証される(エラーは理由つきで返す)。
    pub fn load(paths: &SessionPaths) -> Result<GeographyData, SessionError> {
        let map = load_from_path(&paths.map)
            .map_err(|e| SessionError::Data { what: "hex map", message: e.to_string() })?;
        let preview = load_world_preview(&paths.provinces, &paths.states, &map)
            .map_err(|message| SessionError::Data { what: "world preview", message })?;
        Ok(GeographyData { map, preview })
    }

    pub fn model(&self) -> &WorldModel {
        &self.preview.model
    }
}

/// 位置施設の定義を読み込んで検証する。
pub fn load_facilities(path: &str, world: &WorldModel) -> Result<LocationFacilityRegistry, SessionError> {
    let file = load_location_facilities(path)
        .map_err(|message| SessionError::Data { what: "location facilities", message })?;
    LocationFacilityRegistry::build(&file, world)
        .map_err(|issues| SessionError::Data { what: "location facilities", message: format!("{issues:?}") })
}

/// 建物定義を読み込んで検証する(旧ゲームの読み込みと同じ検証: `BuildingRegistry::from_ron`)。
pub fn load_buildings(path: &str) -> Result<BuildingRegistry, SessionError> {
    BuildingRegistry::from_ron(&read_text(path)?)
        .map_err(|message| SessionError::Data { what: "building definitions", message })
}

// ---------------------------------------------------------------- シミュレーションの生成

fn fmt_issues(issues: Vec<crate::economy::map_state::EconomyDataIssue>) -> SessionError {
    SessionError::Scenario(issues.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n"))
}

/// シナリオ・資源設定のテキストから `MapEconomySimulation` を生成する(ファイルは読まない。テストで入力を差し替えられる)。
pub fn build_simulation_from_text(
    scenario_text: &str,
    resources_text: &str,
    start: SimDate,
    world: &WorldModel,
    buildings: &BuildingRegistry,
) -> Result<MapEconomySimulation, SessionError> {
    let scenario = parse_multi_month_scenario(scenario_text).map_err(SessionError::Scenario)?;
    let resources = parse_province_resources(resources_text)
        .map_err(|message| SessionError::Data { what: "province resources", message })?;
    let (mut registry, countries) =
        MapStateEconomyRegistry::from_multi_scenario(&scenario, world, buildings).map_err(fmt_issues)?;
    let derived = derive_state_resources(world, &resources).map_err(|issues| SessionError::Data {
        what: "province resources",
        message: issues.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n"),
    })?;
    registry.attach_province_resources(&derived, world, &resources).map_err(fmt_issues)?;
    Ok(MapEconomySimulation::new(start, registry, countries))
}

/// 既知のシナリオから `MapEconomySimulation` を生成する(P32-016 以降のCLIの `m4d_simulation` と同じ組み立て)。
pub fn build_simulation(
    scenario: &SessionScenario,
    world: &WorldModel,
    buildings: &BuildingRegistry,
) -> Result<MapEconomySimulation, SessionError> {
    build_simulation_from_text(
        &read_text(scenario.scenario_path)?,
        &read_text(scenario.resources_path)?,
        scenario.start,
        world,
        buildings,
    )
}

// ---------------------------------------------------------------- 操作主体

/// 操作主体の解決・検証の失敗。黙って別の国家へ置き換えない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorError {
    /// 操作主体が選択されていない(状態を変更する操作は受け付けない)。
    NotSelected,
    /// この検証シナリオに無い国家ラベル。
    UnknownCountry(u32),
    /// AI国家(手動操作の対象ではない)。
    NotManual(u32),
    /// 操作主体の経済計算に含まれないステート(別国家のステートなど)。
    StateNotMapped { actor: u32, state: MapStateId },
}

impl std::fmt::Display for ActorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorError::NotSelected => write!(f, "no acting country is selected"),
            ActorError::UnknownCountry(l) => write!(f, "country {l} is not in this scenario"),
            ActorError::NotManual(l) => write!(f, "country {l} is AI-controlled (not a manual actor)"),
            ActorError::StateNotMapped { actor, state } => {
                write!(f, "{state} is not part of country {actor}'s economy")
            }
        }
    }
}

/// 操作を伴う要求の失敗(操作主体の問題と、既存の受付・取消しの拒否理由を区別する)。
#[derive(Debug, Clone, PartialEq)]
pub enum BuildRequestError {
    Actor(ActorError),
    Order(OrderError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CancelRequestError {
    Actor(ActorError),
    Cancel(CancelError),
}

// ---------------------------------------------------------------- セッション

/// 経済セッション: 状態の正本 [`MapEconomySimulation`] と、それを動かす静的入力(建物定義・位置施設)。
/// 地図とワールド設定は呼び出し側が持つ(画面は既存のリソース、CLI は [`GeographyData`])ので、保存・復元・事前確認では引数で受ける。
/// 操作主体の選択はこの構造体に持たない(画面・CLI の選択状態。保存対象でもない)。
pub struct EconomySession {
    pub sim: MapEconomySimulation,
    pub buildings: BuildingRegistry,
    pub facilities: Option<LocationFacilityRegistry>,
    scenario: &'static SessionScenario,
}

impl EconomySession {
    pub fn new(
        scenario: &'static SessionScenario,
        world: &WorldModel,
        buildings: BuildingRegistry,
        facilities: Option<LocationFacilityRegistry>,
    ) -> Result<EconomySession, SessionError> {
        let sim = build_simulation(scenario, world, &buildings)?;
        Ok(EconomySession { sim, buildings, facilities, scenario })
    }

    /// 建物定義と位置施設を `paths` から読み込んで組み立てる(CLI・画面の起動処理)。
    pub fn load_files(
        scenario: &'static SessionScenario,
        world: &WorldModel,
        paths: &SessionPaths,
    ) -> Result<EconomySession, SessionError> {
        let buildings = load_buildings(&paths.buildings)?;
        let facilities = load_facilities(&paths.facilities, world)?;
        EconomySession::new(scenario, world, buildings, Some(facilities))
    }

    pub fn scenario(&self) -> &'static SessionScenario {
        self.scenario
    }

    /// 保存・照合に使う静的入力(`self` の建物定義・位置施設と、呼び出し側の地図・ワールド設定)。
    pub fn inputs<'a>(&'a self, map: &'a HexMap, world: &'a WorldModel) -> StaticInputs<'a> {
        StaticInputs { map, world, buildings: &self.buildings, facilities: self.facilities.as_ref() }
    }

    /// 現在の静的入力の項目別の識別情報(保存時と同じ生成)。
    pub fn identity(&self, map: &HexMap, world: &WorldModel) -> Result<StaticIdentity, IdentityError> {
        StaticIdentity::compute(&self.sim, &self.inputs(map, world))
    }

    // ------------------------------------------------------------ 参照(操作主体なしで可能)

    pub fn state_economy(&self, state: MapStateId) -> Option<&MapStateEconomy> {
        self.sim.registry.get(state)
    }

    pub fn country(&self, label: u32) -> Option<&ScenarioCountry> {
        self.sim.countries.iter().find(|c| c.label == label)
    }

    /// 手動操作の対象(非AI)の国家ラベル(昇順)。**候補の一覧であり、既定の選択ではない**。
    pub fn manual_actors(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.sim.countries.iter().filter(|c| c.ai.is_none()).map(|c| c.label).collect();
        v.sort_unstable();
        v
    }

    /// 操作主体の選択を検証する。成功時は国家ラベル(選択と同じ値)を返す。未選択・存在しない・AI国家は
    /// それぞれ別のエラーで、別の国家に置き換えない。
    pub fn resolve_actor(&self, selected: Option<u32>) -> Result<u32, ActorError> {
        let label = selected.ok_or(ActorError::NotSelected)?;
        let country = self.country(label).ok_or(ActorError::UnknownCountry(label))?;
        if country.ai.is_some() {
            return Err(ActorError::NotManual(label));
        }
        Ok(label)
    }

    /// 操作主体とステートの経済計算上の対応を確認する(表示用の参照。建設の受付判定は `check_build`/`request_build` が行う)。
    pub fn resolve_actor_for_state(&self, selected: Option<u32>, state: MapStateId) -> Result<u32, ActorError> {
        let label = self.resolve_actor(selected)?;
        let country = self.country(label).ok_or(ActorError::UnknownCountry(label))?;
        if !country.states.contains(&state) {
            return Err(ActorError::StateNotMapped { actor: label, state });
        }
        Ok(label)
    }

    // ------------------------------------------------------------ 受付可否の確認(状態を変更しない)

    /// 建設の受付可否を確認する(**状態を一切変更しない**。実際の注文では現在の状態で再検証される)。
    /// 判定は `MapStateEconomyRegistry::check_build` で、`request_build` と同じ中核検証。未選択・AI国家・
    /// 存在しない国家は受理可能とは返さない。
    pub fn check_build(
        &self,
        selected: Option<u32>,
        state: MapStateId,
        building: BuildingType,
    ) -> Result<OrderCheck, BuildRequestError> {
        let actor = self.resolve_actor(selected).map_err(BuildRequestError::Actor)?;
        self.sim
            .registry
            .check_build(&self.sim.countries, actor, state, building, &self.buildings)
            .map_err(BuildRequestError::Order)
    }

    // ------------------------------------------------------------ 状態を変更する操作(日と日の間に呼ぶ)

    /// 翌日を処理する(`MapEconomySimulation::advance_day`)。
    pub fn advance_day(&mut self) -> (SimDate, DayReport) {
        self.sim.advance_day(&self.buildings)
    }

    /// 手動の建設注文。操作主体を検証してから既存の `request_build` を呼ぶ(判定の複製なし)。
    pub fn request_build(
        &mut self,
        selected: Option<u32>,
        state: MapStateId,
        building: BuildingType,
    ) -> Result<ConstructionOrderId, BuildRequestError> {
        let actor = self.resolve_actor(selected).map_err(BuildRequestError::Actor)?;
        self.sim
            .registry
            .request_build(&mut self.sim.countries, actor, state, building, &self.buildings)
            .map_err(BuildRequestError::Order)
    }

    /// 注文の識別子を指定したキャンセル(`CancelTarget::Latest` は提供しない)。
    pub fn cancel_order(
        &mut self,
        selected: Option<u32>,
        id: ConstructionOrderId,
    ) -> Result<CancelReceipt, CancelRequestError> {
        let actor = self.resolve_actor(selected).map_err(CancelRequestError::Actor)?;
        self.sim
            .registry
            .cancel_order(&mut self.sim.countries, actor, CancelTarget::Order(id))
            .map_err(CancelRequestError::Cancel)
    }

    // ------------------------------------------------------------ 保存・復元・事前確認・一覧(`map_economy_file` の同じ関数)

    pub fn save(
        &self,
        map: &HexMap,
        world: &WorldModel,
        location: &SaveLocation,
        name: &str,
    ) -> Result<SaveReceipt, FileError> {
        crate::economy::map_economy_file::save(&self.sim, &self.inputs(map, world), location, name)
    }

    /// 復元せずに確認する(`map_economy_file::inspect`)。
    pub fn inspect(
        &self,
        map: &HexMap,
        world: &WorldModel,
        location: &SaveLocation,
        name: &str,
    ) -> Result<InspectReport, FileError> {
        crate::economy::map_economy_file::inspect(&self.sim, &self.inputs(map, world), location, name)
    }

    /// 復元する(`map_economy_file::load`。常に最初から全検証をやり直し、失敗時は `sim` を変更しない)。
    pub fn load(
        &mut self,
        map: &HexMap,
        world: &WorldModel,
        location: &SaveLocation,
        name: &str,
    ) -> Result<LoadReceipt, FileError> {
        let inputs = StaticInputs {
            map,
            world,
            buildings: &self.buildings,
            facilities: self.facilities.as_ref(),
        };
        crate::economy::map_economy_file::load(&mut self.sim, &inputs, location, name)
    }

    pub fn list_saves(location: &SaveLocation) -> SaveList {
        crate::economy::map_economy_file::list_saves(location)
    }
}

impl Clone for EconomySession {
    /// 状態の複製(試行・比較用)。`BuildingRegistry` は `Clone` を持たないので、定義を複製して作る。
    fn clone(&self) -> Self {
        EconomySession {
            sim: self.sim.clone(),
            buildings: BuildingRegistry { definitions: self.buildings.definitions.clone() },
            facilities: self.facilities.clone(),
            scenario: self.scenario,
        }
    }
}
