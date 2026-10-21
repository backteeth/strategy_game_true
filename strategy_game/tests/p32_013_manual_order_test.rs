//! P32-013: 新ステート(`MapStateId`)の手動建設注文・キャンセル・返金のテスト。
//! 旧経路との比較は、**実際の旧UIのSystem**(`handle_build_buttons`・`handle_cancel_queue_buttons`)と旧の
//! 日次建設System(`handle_daily_construction`)を最小のBevy Appで駆動して行う。旧に仕様が無い箇所
//! (識別子による個別キャンセル、注文主体のラベル、資源設定の前提、他国家の拒否など)は新旧一致を装わず、
//! 新経路だけを手計算の期待値で検証する。入力は検証用シナリオで、正式な配置・バランスではない。
//! 旧`StateId(n+1)`とシナリオ順の`MapStateId`の対応はテスト内で明示する(数値一致で推測しない)。

use bevy::prelude::*;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::building::construction::{REFUND_RATIO, cancel_refund};
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryData, CountryRegistry, PlayerCountry};
use strategy_game::economy::manual_script::{
    ManualAction, ManualLogEntry, RequestOutcome, parse_manual_script, run_manual_script,
};
use strategy_game::economy::map_state::{
    CancelError, CancelTarget, ConstructionOrderId, MapStateEconomyInput, MapStateEconomyRegistry,
    MultiMonthScenario, OrderError, OrderEnd, ScenarioCountry, ScenarioCountryInput, SimDate,
    parse_multi_month_scenario,
};
use strategy_game::country::ai_core::OrderBlock;
use strategy_game::economy::province_resources::{
    ProvinceResourcesFile, derive_state_resources, load_province_resources,
};
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::localization::{CurrentLocale, Locale, TranslationCatalog};
use strategy_game::state::SelectedState;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::ui::notification::GameNotification;
use strategy_game::ui::state_panel::{BuildButton, handle_build_buttons, handle_cancel_queue_buttons};
use strategy_game::world::{MapStateId, WorldModel, load_province_table, load_state_layout};

use BuildingType::{CrystalMine, Factory, Farm, Mine, Railway};

const START: SimDate = SimDate { year: 1800, month: 1, day: 1 };

fn buildings() -> BuildingRegistry {
    let defs: Vec<BuildingDefinition> =
        ron::from_str(&std::fs::read_to_string("assets/data/buildings.ron").unwrap()).unwrap();
    BuildingRegistry { definitions: defs.into_iter().map(|d| (d.building_type, d)).collect() }
}

fn world() -> WorldModel {
    WorldModel::build(
        load_province_table("assets/data/world/preview/provinces.ron").unwrap(),
        load_state_layout("assets/data/world/preview/states.ron").unwrap(),
    )
    .unwrap()
}

fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

/// m4d の資源定義(全プロヴィンス設定済み)。`extra`で上書き、`omit`で未設定にする。
fn resources(extra: &[(u32, Vec<StateResourceDeposit>)], omit: &[u32]) -> ProvinceResourcesFile {
    let mut f = load_province_resources("assets/data/world/preview/province_resources_m4d.ron").unwrap();
    for (p, d) in extra {
        f.provinces.iter_mut().find(|e| e.province == *p).unwrap().deposits = d.clone();
    }
    f.provinces.retain(|e| !omit.contains(&e.province));
    f
}

fn crystal(out: f64) -> StateResourceDeposit {
    StateResourceDeposit {
        resource_type: ResourceType::MagicCrystal,
        base_output: out,
        discovered: true,
        development_level: 1,
    }
}

fn state_input(id: u32, pop: u64, b: &[(BuildingType, u32)]) -> MapStateEconomyInput {
    MapStateEconomyInput {
        state: id,
        population: pop,
        workforce_ratio: 0.5,
        living_standard: 60.0,
        unrest: 10.0,
        base_logistics_capacity: 200.0,
        buildings: b.iter().copied().collect(),
        resource_deposits: vec![],
    }
}

fn country_input(label: u32, treasury: f64, states: &[u32], ai: bool) -> ScenarioCountryInput {
    ScenarioCountryInput {
        label,
        treasury,
        tax_rate: 0.15,
        available_manpower: 0,
        mobilized_manpower: 0,
        stockpile: [
            (ResourceType::Iron, 3000.0),
            (ResourceType::Coal, 3000.0),
            (ResourceType::Food, 900.0),
            (ResourceType::Wood, 900.0),
        ]
        .into_iter()
        .collect(),
        states: states.to_vec(),
        ai_controlled: ai,
        military: None,
    }
}

fn scenario(countries: Vec<ScenarioCountryInput>, states: Vec<MapStateEconomyInput>) -> MultiMonthScenario {
    MultiMonthScenario {
        format_version: 1,
        description: "P32-013 test scenario (not official data or balance)".to_string(),
        months: 4,
        countries,
        states,
    }
}

fn new_world(
    sc: &MultiMonthScenario,
    f: &ProvinceResourcesFile,
) -> (MapStateEconomyRegistry, Vec<ScenarioCountry>, BuildingRegistry, WorldModel) {
    let (b, w) = (buildings(), world());
    let (mut reg, cs) = MapStateEconomyRegistry::from_multi_scenario(sc, &w, &b).unwrap();
    let d = derive_state_resources(&w, f).unwrap();
    reg.attach_province_resources(&d, &w, f).unwrap();
    (reg, cs, b, w)
}

/// 標準の1国家(国家1・MapState#20、手動のみ)。
fn single(treasury: f64) -> MultiMonthScenario {
    scenario(
        vec![country_input(1, treasury, &[20], false)],
        vec![state_input(20, 600001, &[(Farm, 3), (Mine, 2), (Factory, 1), (Railway, 1)])],
    )
}

// ---------------------------------------------------------------- 旧経路(実際の旧UI System)

struct Old {
    app: App,
}

fn old_world(sc: &MultiMonthScenario, f: &ProvinceResourcesFile, w: &WorldModel) -> Old {
    let d = derive_state_resources(w, f).unwrap();
    // 対応(明示): 旧StateId(n+1) ↔ シナリオ内n番目のステート。旧CountryId(label)は国家ラベルと同じ値で作った旧側の識別。
    let states: Vec<StateData> = sc
        .states
        .iter()
        .enumerate()
        .map(|(n, i)| StateData {
            id: StateId(n + 1),
            owner_country_id: CountryId(1),
            population: i.population,
            workforce_ratio: i.workforce_ratio,
            buildings: i.buildings.clone(),
            resource_deposits: d.state(MapStateId(i.state)).unwrap().deposits.clone(),
            ..StateData::default()
        })
        .collect();
    let c = &sc.countries[0];
    let mut stockpile = CountryStockpile::new();
    for (r, a) in &c.stockpile {
        stockpile.set(*r, *a);
    }
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<GameNotification>()
        .add_message::<DayChangedMessage>()
        .insert_resource(CurrentLocale(Locale::JaJp))
        .insert_resource(TranslationCatalog::load().expect("catalog"))
        .insert_resource(buildings())
        .insert_resource(PlayerCountry(Some(CountryId(1))))
        .insert_resource(SelectedState(Some(StateId(1))))
        .insert_resource(ButtonInput::<KeyCode>::default())
        .insert_resource(CountryRegistry {
            countries: vec![CountryData {
                id: CountryId(1),
                treasury: c.treasury,
                stockpile,
                ..CountryData::default()
            }],
        })
        .insert_resource(StateRegistry::build(states))
        .add_systems(
            Update,
            (
                handle_build_buttons,
                handle_cancel_queue_buttons,
                strategy_game::economy::handle_daily_construction,
            ),
        );
    app.update();
    Old { app }
}

impl Old {
    fn select(&mut self, old_state: usize) {
        self.app.world_mut().insert_resource(SelectedState(Some(StateId(old_state))));
    }
    /// 旧UIの建設ボタンを押す。キューに増えたか(受理)を返す。
    fn build(&mut self, b: BuildingType) -> bool {
        let before = self.app.world().resource::<CountryRegistry>().countries[0].construction_queue.len();
        let e = self.app.world_mut().spawn((BuildButton(b), Interaction::Pressed, BackgroundColor(Color::WHITE))).id();
        self.app.update();
        self.app.world_mut().despawn(e);
        self.app.world().resource::<CountryRegistry>().countries[0].construction_queue.len() > before
    }
    /// 旧UIの「C」キー。
    fn cancel_latest(&mut self) {
        self.app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyC);
        self.app.update();
        self.app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::KeyC);
        self.app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
    }
    fn days(&mut self, n: u32) {
        let mut d = START;
        for _ in 0..n {
            let (nx, _) = d.next();
            d = nx;
            self.app.world_mut().write_message(DayChangedMessage { year: d.year, month: d.month, day: d.day });
            self.app.update();
        }
    }
    fn treasury(&self) -> f64 {
        self.app.world().resource::<CountryRegistry>().countries[0].treasury
    }
    fn queue(&self) -> Vec<(BuildingType, u32, f64, f64, f64)> {
        self.app.world().resource::<CountryRegistry>().countries[0]
            .construction_queue
            .iter()
            .map(|o| (o.building_type, o.target_level, o.progress, o.required_progress, o.paid_cost))
            .collect()
    }
    fn level(&self, old_state: usize, b: BuildingType) -> u32 {
        self.app.world().resource::<StateRegistry>().get(StateId(old_state)).unwrap().building_level(b)
    }
}

fn new_queue(c: &ScenarioCountry) -> Vec<(BuildingType, u32, f64, f64, f64)> {
    c.construction_queue
        .iter()
        .map(|o| (o.building_type, o.target_level, o.progress, o.required_progress, o.paid_cost))
        .collect()
}

// ---------------------------------------------------------------- 旧新比較(実際の旧UI System)

#[test]
fn acceptance_payment_progress_and_completion_match_the_old_ui_and_hand_values() {
    let f = resources(&[], &[]);
    let sc = single(5000.0);
    let (mut reg, mut cs, b, w) = new_world(&sc, &f);
    let mut old = old_world(&sc, &f, &w);
    // 受付: Farm(手計算: 費用200・必要進捗30・目標レベル4)と Mine(500・60・3)
    assert!(old.build(Farm));
    assert!(reg.place_order(&mut cs[0], s(20), Farm, &b).is_ok());
    assert!(old.build(Mine));
    assert!(reg.place_order(&mut cs[0], s(20), Mine, &b).is_ok());
    assert_eq!(old.treasury(), 5000.0 - 200.0 - 500.0);
    assert_eq!(cs[0].treasury, old.treasury());
    assert_eq!(new_queue(&cs[0]), old.queue());
    assert_eq!(new_queue(&cs[0]), vec![(Farm, 4, 0.0, 30.0, 200.0), (Mine, 3, 0.0, 60.0, 500.0)]);
    // 29日: 進捗の増え方が新旧で一致し、まだ完成しない
    old.days(29);
    reg.run_days(&mut cs, START, 29, &b);
    assert_eq!(new_queue(&cs[0]), old.queue());
    assert_eq!(new_queue(&cs[0])[0].2, 29.0);
    assert_eq!((old.level(1, Farm), reg.get(s(20)).unwrap().buildings[&Farm]), (3, 3));
    // 30日目: Farmだけが完成する(完成日・反映・キューからの除去が新旧で一致)
    old.days(1);
    reg.run_days(&mut cs, SimDate { year: 1800, month: 1, day: 30 }, 1, &b);
    assert_eq!((old.level(1, Farm), reg.get(s(20)).unwrap().buildings[&Farm]), (4, 4));
    assert_eq!((old.level(1, Mine), reg.get(s(20)).unwrap().buildings[&Mine]), (2, 2));
    assert_eq!(new_queue(&cs[0]), old.queue());
    assert_eq!(cs[0].treasury, old.treasury());
}

#[test]
fn rejections_match_the_old_ui_and_change_nothing() {
    let w = world();
    // MapState#22 を MagicCrystal だけの州(クリスタル専用)にする。MapState#21 にはクリスタル鉱床なし(Iron)。
    let f = resources(&[(17266, vec![crystal(30.0)])], &[]);
    let sc = scenario(
        vec![country_input(1, 5000.0, &[20, 21, 22], false)],
        vec![
            state_input(20, 600001, &[(Farm, 10), (Mine, 2)]),
            state_input(21, 400000, &[(Farm, 6)]),
            state_input(22, 250000, &[(Farm, 2)]),
        ],
    );
    let (reg, cs0, b, _) = new_world(&sc, &f);
    let mut old = old_world(&sc, &f, &w);
    // 旧StateId(n+1) ↔ シナリオ内n番目: 1=MapState#20, 2=#21, 3=#22
    let cases: [(usize, u32, BuildingType, &str); 5] = [
        (1, 20, Farm, "上限(Farmは最大10)"),
        (2, 21, CrystalMine, "発見済みクリスタル鉱床なし"),
        (3, 22, Mine, "クリスタル専用州への通常Mine"),
        (3, 22, CrystalMine, "(受理される対照: クリスタル鉱床あり)"),
        (2, 21, Farm, "(受理される対照)"),
    ];
    let mut cs = cs0.clone();
    for (old_state, new_state, building, why) in cases {
        old.select(old_state);
        let before = (reg.clone(), cs.clone());
        let old_ok = old.build(building);
        let new_ok = reg.place_order(&mut cs[0], s(new_state), building, &b).is_ok();
        assert_eq!(old_ok, new_ok, "{why}");
        if !new_ok {
            assert_eq!((reg.clone(), cs.clone()), before, "rejection must change nothing: {why}");
        }
        assert_eq!(cs[0].treasury, old.treasury(), "{why}");
    }
    // 重複(同じ州の同じ施設が既にキュー)・資金不足
    old.select(2);
    assert_eq!(old.build(Farm), reg.place_order(&mut cs[0], s(21), Farm, &b).is_ok());
    let mut poor_sc = sc.clone();
    poor_sc.countries[0].treasury = 150.0;
    let (reg_p, mut cs_p, _, _) = new_world(&poor_sc, &f);
    let mut old_p = old_world(&poor_sc, &f, &w);
    assert!(!old_p.build(Farm));
    assert_eq!(
        reg_p.place_order(&mut cs_p[0], s(20), Farm, &b),
        Err(OrderError::MaxLevel { level: 10, max: 10 }),
        "limit is checked before funds, as in the old UI"
    );
    // 資金不足(上限に達していないFarm): 150 < 200
    assert!(matches!(
        reg_p.place_order(&mut cs_p[0], s(21), Farm, &b),
        Err(OrderError::InsufficientFunds { cost, treasury }) if cost == 200.0 && treasury == 150.0
    ));
    old_p.select(2);
    assert!(!old_p.build(Farm));
}

#[test]
fn cancel_latest_and_refund_match_the_old_ui_and_hand_values() {
    let f = resources(&[], &[]);
    let sc = single(5000.0);
    let (reg, mut cs, b, w) = new_world(&sc, &f);
    let mut old = old_world(&sc, &f, &w);
    for bt in [Farm, Mine, Factory] {
        assert!(old.build(bt));
        reg.place_order(&mut cs[0], s(20), bt, &b).unwrap();
    }
    assert_eq!(old.treasury(), 5000.0 - 200.0 - 500.0 - 1000.0);
    // 旧UIは最後の注文(Factory)を取り消し、費用の50%を返す(手計算: 1000 × 0.5 = 500)
    old.cancel_latest();
    let r = reg.cancel_order(&mut cs, 1, CancelTarget::Latest).unwrap();
    assert_eq!(r.refund, 500.0);
    assert_eq!(cs[0].treasury, old.treasury());
    assert_eq!(cs[0].treasury, 5000.0 - 200.0 - 500.0 - 1000.0 + 500.0);
    assert_eq!(new_queue(&cs[0]), old.queue());
    // 進捗に依存しない: 45日進めた後のMine(Farmは完成済み)の取消しも 500 × 0.5
    old.days(45);
    let (mut reg2, mut cs2, _, _) = new_world(&sc, &f);
    for bt in [Farm, Mine] {
        reg2.place_order(&mut cs2[0], s(20), bt, &b).unwrap();
    }
    reg2.run_days(&mut cs2, START, 30, &b);
    let before = cs2[0].treasury;
    // (旧は45日、新は30日: 取消し時点の進捗は違っても返金は同じ規則であることを、別に確認する)
    let r2 = reg2.cancel_order(&mut cs2, 1, CancelTarget::Latest).unwrap();
    assert_eq!((r2.progress, r2.refund), (30.0, 250.0));
    assert_eq!(cs2[0].treasury, before + 250.0);
    old.cancel_latest();
    assert_eq!(old.treasury(), 5000.0 - 200.0 - 500.0 - 1000.0 + 500.0 + 250.0);
    assert_eq!(cancel_refund(1000.0), 1000.0 * REFUND_RATIO);
}

// ---------------------------------------------------------------- 新経路のみ(旧に仕様が無い箇所)

#[test]
fn cancel_by_stable_id_removes_only_that_order_and_other_ids_do_not_shift() {
    let f = resources(&[], &[]);
    let (reg, mut cs, b, _) = new_world(&single(9000.0), &f);
    let a = reg.place_order(&mut cs[0], s(20), Farm, &b).unwrap();
    let m = reg.place_order(&mut cs[0], s(20), Mine, &b).unwrap();
    let fa = reg.place_order(&mut cs[0], s(20), Factory, &b).unwrap();
    assert_eq!((a.seq, m.seq, fa.seq), (1, 2, 3));
    let t0 = cs[0].treasury;
    // 中央の注文だけを取り消す(旧UIは最後の注文しか取り消せない: この動作は新しい要件で、返金の式は同じ)
    let r = reg.cancel_order(&mut cs, 1, CancelTarget::Order(m)).unwrap();
    assert_eq!((r.id, r.refund, r.paid_cost), (m, 250.0, 500.0));
    assert_eq!(cs[0].treasury, t0 + 250.0);
    let ids: Vec<ConstructionOrderId> = cs[0].construction_queue.iter().map(|o| o.id).collect();
    assert_eq!(ids, vec![a, fa], "remaining orders keep their ids even though their positions moved");
    // 残りの注文も識別子で取り消せる(位置ではなく識別子で解決される)
    let r2 = reg.cancel_order(&mut cs, 1, CancelTarget::Order(fa)).unwrap();
    assert_eq!(r2.refund, 500.0);
    assert_eq!(cs[0].construction_queue.len(), 1);
    // 再注文には新しい識別子が発行され、取り消した識別子は再利用されない
    let m2 = reg.place_order(&mut cs[0], s(20), Mine, &b).unwrap();
    assert_eq!(m2.seq, 4);
    assert_eq!(cs[0].order_log.iter().filter(|(_, e)| *e == OrderEnd::Cancelled).count(), 2);
}

#[test]
fn cancel_rejections_change_nothing_and_never_refund_twice_or_to_another_country() {
    let f = resources(&[], &[]);
    let sc = scenario(
        vec![country_input(1, 5000.0, &[20], false), country_input(2, 3000.0, &[21], false)],
        vec![
            state_input(20, 600001, &[(Farm, 3), (Mine, 2)]),
            state_input(21, 400000, &[(Farm, 6)]),
        ],
    );
    let (mut reg, mut cs, b, _) = new_world(&sc, &f);
    let a = reg.place_order(&mut cs[0], s(20), Farm, &b).unwrap();
    let c2 = reg.place_order(&mut cs[1], s(21), Farm, &b).unwrap();
    let snap = (reg.clone(), cs.clone());
    // 別国家の注文・存在しない注文・存在しない国家
    assert_eq!(reg.cancel_order(&mut cs, 1, CancelTarget::Order(c2)), Err(CancelError::NotIssuersOrder(c2)));
    let ghost = ConstructionOrderId { country: 1, seq: 99 };
    assert_eq!(reg.cancel_order(&mut cs, 1, CancelTarget::Order(ghost)), Err(CancelError::NoSuchOrder(ghost)));
    assert_eq!(reg.cancel_order(&mut cs, 9, CancelTarget::Latest), Err(CancelError::UnknownCountry(9)));
    assert_eq!((reg.clone(), cs.clone()), snap, "rejections change nothing");
    // 取消し → 二度目は拒否され、国庫は増えない
    let t = cs[0].treasury;
    reg.cancel_order(&mut cs, 1, CancelTarget::Order(a)).unwrap();
    assert_eq!(cs[0].treasury, t + 100.0);
    assert_eq!(cs[1].treasury, 3000.0 - 200.0, "no refund reached another country");
    assert_eq!(reg.cancel_order(&mut cs, 1, CancelTarget::Order(a)), Err(CancelError::AlreadyCancelled(a)));
    assert_eq!(reg.cancel_order(&mut cs, 1, CancelTarget::Latest), Err(CancelError::EmptyQueue));
    assert_eq!(cs[0].treasury, t + 100.0);
    // 取り消した注文は後から完成しない
    reg.run_days(&mut cs, START, 40, &b);
    assert_eq!(reg.get(s(20)).unwrap().buildings[&Farm], 3);
    assert_eq!(reg.get(s(21)).unwrap().buildings[&Farm], 7, "the other country's order completes");
    // 完成済みの注文は未完成として返金しない
    let t2 = cs[1].treasury;
    assert_eq!(reg.cancel_order(&mut cs, 2, CancelTarget::Order(c2)), Err(CancelError::AlreadyCompleted(c2)));
    assert_eq!(cs[1].treasury, t2);
}

#[test]
fn request_validation_covers_issuer_state_facility_and_mine_resources() {
    let f = resources(&[], &[17268]); // MapState#22 の1プロヴィンスが未設定
    let sc = scenario(
        vec![country_input(1, 5000.0, &[20], false), country_input(2, 5000.0, &[21, 22], false)],
        vec![
            state_input(20, 600001, &[(Farm, 3), (Mine, 2)]),
            state_input(21, 400000, &[(Farm, 6)]),
            state_input(22, 250000, &[(Farm, 2)]),
        ],
    );
    let (reg, mut cs, b, _) = new_world(&sc, &f);
    let snap = (reg.clone(), cs.clone());
    assert_eq!(reg.request_build(&mut cs, 7, s(20), Farm, &b), Err(OrderError::UnknownCountry(7)));
    assert_eq!(reg.request_build(&mut cs, 1, s(99), Farm, &b), Err(OrderError::UnknownState(s(99))));
    assert_eq!(reg.request_build(&mut cs, 1, s(21), Farm, &b), Err(OrderError::NotMappedToCountry(s(21))));
    // 未設定のMine(MapState#22)は受付時に拒否。設定済みのMapState#21は受理
    assert_eq!(
        reg.request_build(&mut cs, 2, s(22), Mine, &b),
        Err(OrderError::Precondition(OrderBlock::ResourcesNotConfigured))
    );
    assert_eq!((reg.clone(), cs.clone()), snap);
    assert!(reg.request_build(&mut cs, 2, s(21), Mine, &b).is_ok());
    // 位置施設は型に存在せず、BuildingTypeの全種別がステート単位の経済施設
    assert!(BuildingType::ALL.iter().all(|x| strategy_game::economy::map_state::is_state_level_economic_facility(*x)));
}

#[test]
fn cost_and_duration_do_not_depend_on_province_count() {
    // MapState#20(7プロヴィンス)と#21(13プロヴィンス)で、同じ施設の費用・必要進捗・返金は同じ
    let f = resources(&[], &[]);
    let sc = scenario(
        vec![country_input(1, 9000.0, &[20, 21], false)],
        vec![state_input(20, 600001, &[(Farm, 3)]), state_input(21, 400000, &[(Farm, 3)])],
    );
    let (reg, mut cs, b, w) = new_world(&sc, &f);
    assert_ne!(w.provinces_of(s(20)).unwrap().len(), w.provinces_of(s(21)).unwrap().len());
    reg.place_order(&mut cs[0], s(20), Railway, &b).unwrap();
    reg.place_order(&mut cs[0], s(21), Railway, &b).unwrap();
    let q = new_queue(&cs[0]);
    assert_eq!(q[0], q[1]);
    assert_eq!(q[0], (Railway, 1, 0.0, 60.0, 800.0)); // 手計算: 800 / 60日
    assert_eq!(cs[0].treasury, 9000.0 - 1600.0);
}

#[test]
fn blocked_mine_is_cancellable_by_the_existing_rule_and_nothing_is_lost_before() {
    let b = buildings();
    // 国家2のMapState#21(Mineなし)でMineを注文 → 注文後に資源の設定を外す → 完成保留 → キャンセル
    let sc = scenario(
        vec![country_input(2, 3000.0, &[21], false)],
        vec![state_input(21, 400000, &[(Farm, 6)])],
    );
    let good = resources(&[], &[]);
    let (reg, mut cs, _, _) = new_world(&sc, &good);
    let id = reg.place_order(&mut cs[0], s(21), Mine, &b).unwrap();
    let bad = resources(&[], &[17696]);
    let (mut reg2, _, _, _) = new_world(&sc, &bad);
    let reports = reg2.run_days(&mut cs, START, 70, &b);
    assert!(reports.iter().any(|(_, r)| r.blocked.iter().any(|x| x.3 == OrderBlock::ResourcesNotConfigured)));
    assert_eq!(reg2.get(s(21)).unwrap().buildings.get(&Mine), None, "no facility without valid inputs");
    assert_eq!(cs[0].construction_queue.len(), 1);
    let t = cs[0].treasury;
    // 保留中でも既存のキャンセル規則(支払済み費用の50%)。保留による追加のペナルティや別の返金率は無い
    let r = reg2.cancel_order(&mut cs, 2, CancelTarget::Order(id)).unwrap();
    assert_eq!((r.refund, r.progress >= r.required_progress), (250.0, true));
    assert!((cs[0].treasury - (t + 250.0)).abs() < 1e-9);
    // 取消し後はキューが空で、以後完成しない
    let after = reg2.run_days(&mut cs, SimDate { year: 1800, month: 3, day: 12 }, 10, &b);
    assert!(after.iter().all(|(_, r)| r.completed.is_empty() && r.blocked.is_empty()));
}

#[test]
fn manual_and_ai_orders_share_one_treasury_and_one_queue() {
    // 国家3(AI): 国庫1500。手動でFactory(1000)を注文すると、AIは残り500で動く(同じ予算を重複利用しない)。
    let f = resources(&[], &[]);
    let sc = scenario(
        vec![country_input(3, 1500.0, &[22], true)],
        vec![state_input(22, 250000, &[(Farm, 2), (Factory, 1), (Railway, 1)])],
    );
    let (reg, mut cs, b, _) = new_world(&sc, &f);
    let manual = reg.place_order(&mut cs[0], s(22), Factory, &b).unwrap();
    assert_eq!(cs[0].treasury, 500.0);
    // 手動注文は同じキューにあり、AIの注文の支払いは評価時点の国庫を超えない(同じ予算を二重に使えない。月次の収支による国庫の増減は別)。
    let mut reg = reg;
    let mut date = START;
    for _ in 0..60 {
        let (n, ch) = date.next();
        date = n;
        let before = cs[0].treasury;
        let r = reg.run_day(&mut cs, date, ch, &b);
        let spent: f64 = r.ordered.iter().map(|o| o.3).sum();
        if !r.monthly_ran && !r.ordered.is_empty() {
            assert!(spent <= before + 1e-9, "day {date:?} spent {spent} before {before} orders {:?}", r.ordered);
            assert!(cs[0].treasury >= 0.0, "paying for AI orders never overdraws the shared treasury");
        }
    }
    assert!(cs[0].construction_queue.iter().any(|o| o.id == manual) || cs[0].order_log.iter().any(|(i, e)| *i == manual && *e == OrderEnd::Completed));
    // AIの注文費用は手動注文の有無で変わらない(同じ建設費の表を使う)
    let (reg_b, mut cs_b, _, _) = new_world(&sc, &f);
    let rb = reg_b.clone_and_run(&mut cs_b, &b, 60);
    let ai_costs: Vec<f64> = rb.iter().flat_map(|(_, r)| r.ordered.iter().map(|o| o.3)).collect();
    assert!(ai_costs.iter().all(|c| [200.0, 500.0, 1000.0, 800.0].contains(c)));
}

/// テスト用の補助: レジストリを複製せずに指定日数進める。
trait RunHelper {
    fn clone_and_run(
        &self,
        cs: &mut [ScenarioCountry],
        b: &BuildingRegistry,
        days: u32,
    ) -> Vec<(SimDate, strategy_game::economy::map_state::DayReport)>;
}
impl RunHelper for MapStateEconomyRegistry {
    fn clone_and_run(
        &self,
        cs: &mut [ScenarioCountry],
        b: &BuildingRegistry,
        days: u32,
    ) -> Vec<(SimDate, strategy_game::economy::map_state::DayReport)> {
        let mut r = self.clone();
        r.run_days(cs, START, days, b)
    }
}

#[test]
fn completion_flows_into_the_monthly_economy_by_hand_computed_employment() {
    // Farm(必要労働力5000)の完成後の最初の月次経済で、雇用が必要労働力の合計(施設レベル × 定義)になる。
    let f = resources(&[], &[]);
    let (mut reg, mut cs, b, _) = new_world(&single(5000.0), &f);
    reg.place_order(&mut cs[0], s(20), Farm, &b).unwrap();
    reg.run_days(&mut cs, START, 31, &b); // 30日で完成 → 31日目(2/01)に月次経済
    let expected: u64 = [(Farm, 4u32), (Mine, 2), (Factory, 1), (Railway, 1)]
        .iter()
        .map(|(bt, lv)| b.get(*bt).unwrap().required_workforce as u64 * u64::from(*lv))
        .sum();
    assert_eq!(reg.get(s(20)).unwrap().employed_workforce, expected);
}

#[test]
fn re_order_after_cancel_and_after_completion_is_accepted() {
    let f = resources(&[], &[]);
    let (mut reg, mut cs, b, _) = new_world(&single(9000.0), &f);
    let a = reg.place_order(&mut cs[0], s(20), Farm, &b).unwrap();
    reg.cancel_order(&mut cs, 1, CancelTarget::Order(a)).unwrap();
    let a2 = reg.place_order(&mut cs[0], s(20), Farm, &b).unwrap();
    assert_ne!(a, a2);
    reg.run_days(&mut cs, START, 30, &b);
    assert_eq!(reg.get(s(20)).unwrap().buildings[&Farm], 4);
    let a3 = reg.place_order(&mut cs[0], s(20), Farm, &b).unwrap();
    assert_eq!(a3.seq, 3);
    assert_eq!(cs[0].construction_queue[0].target_level, 5);
}

// ---------------------------------------------------------------- 検証用の要求ファイル

#[test]
fn manual_script_file_produces_the_expected_outcomes() {
    let (b, w) = (buildings(), world());
    let sc = parse_multi_month_scenario(&std::fs::read_to_string("assets/data/world/preview/economy_scenario_m4e.ron").unwrap()).unwrap();
    let f = load_province_resources("assets/data/world/preview/province_resources_m4d.ron").unwrap();
    let text = std::fs::read_to_string("assets/data/world/preview/manual_orders_m4e.ron").unwrap();
    let script = parse_manual_script(&text).unwrap();
    let before = text.clone();
    let run = run_manual_script(&sc, &w, &b, &f, &script).unwrap();
    assert_eq!(std::fs::read_to_string("assets/data/world/preview/manual_orders_m4e.ron").unwrap(), before, "input is never modified");
    let outcomes: Vec<&RequestOutcome> = run
        .log
        .iter()
        .filter_map(|e| match e {
            ManualLogEntry::Request { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect();
    assert_eq!(outcomes.len(), script.steps.len());
    // 0 受理(200) 1 予算不足 2 他国家の対象 3 受理(500) 4 資源未設定 5 受理(1000) 6 受理(500)
    assert!(matches!(outcomes[0], RequestOutcome::BuildAccepted { cost, .. } if *cost == 200.0));
    assert!(matches!(outcomes[1], RequestOutcome::BuildRejected(OrderError::InsufficientFunds { .. })));
    assert!(matches!(outcomes[2], RequestOutcome::BuildRejected(OrderError::NotMappedToCountry(_))));
    assert!(matches!(outcomes[3], RequestOutcome::BuildAccepted { cost, .. } if *cost == 500.0));
    assert!(matches!(outcomes[4], RequestOutcome::BuildRejected(OrderError::Precondition(OrderBlock::ResourcesNotConfigured))));
    assert!(matches!(outcomes[5], RequestOutcome::BuildAccepted { cost, .. } if *cost == 1000.0));
    assert!(matches!(outcomes[6], RequestOutcome::BuildAccepted { cost, .. } if *cost == 500.0));
    // 7 建設途中(10/60)のキャンセル: 500 × 0.5 = 250 / 8 二度目 / 9 他国家 / 10 再注文
    assert!(matches!(outcomes[7], RequestOutcome::CancelAccepted(r) if r.refund == 250.0 && r.progress == 10.0 && r.required_progress == 60.0));
    assert!(matches!(outcomes[8], RequestOutcome::CancelRejected(CancelError::AlreadyCancelled(_))));
    assert!(matches!(outcomes[9], RequestOutcome::CancelRejected(CancelError::NotIssuersOrder(_))));
    assert!(matches!(outcomes[10], RequestOutcome::BuildAccepted { .. }));
    assert!(matches!(outcomes[11], RequestOutcome::ResourcesReplaced));
    // 12 完成後のキャンセル要求 / 13 完成保留中のMineのキャンセル(250)
    assert!(matches!(outcomes[12], RequestOutcome::CancelRejected(CancelError::AlreadyCompleted(_))));
    assert!(matches!(outcomes[13], RequestOutcome::CancelAccepted(r) if r.refund == 250.0 && r.progress > r.required_progress - 1.0));
    // 国家2のMineは保留されたまま完成せず、国家1のFarmとMine(再注文)は完成した
    let completed: Vec<_> = run
        .log
        .iter()
        .filter_map(|e| match e {
            ManualLogEntry::Day { report, .. } => Some(report.completed.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(completed.iter().any(|c| c.0 == 1 && c.2 == Farm));
    assert!(completed.iter().any(|c| c.0 == 1 && c.2 == Mine));
    assert!(!completed.iter().any(|c| c.0 == 2));
    assert!(matches!(&script.steps[0].action, ManualAction::Build { country: 1, state: 20, building: Farm }));
    // 取り消した注文(Order#1-2)は完成していない: 国家1のMineの最終レベルは 2 + 再注文の1 = 3
    assert_eq!(run.registry.get(s(20)).unwrap().buildings[&Mine], 3);
}
