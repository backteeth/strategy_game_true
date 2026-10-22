//! P32-007 (M4-A): 新ステート(`MapStateId`)の経済データを、既存の中核計算(雇用・物流・生産)へ
//! 通して結果を表示する、**読み取り専用の検証実行経路**。
//!
//! 検証用シナリオ(`assets/data/world/preview/economy_scenario.ron`)の固定入力を使う。
//! 正式な人口分布・施設配置・ゲームバランスではない。通常の地理プレビューや旧ゲームへは接続せず、
//! ファイルへの書き込み・セーブ形式の変更は行わない。
//!
//! 実行: `cargo run --bin world_economy_check`(`strategy_game/`で)。

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::economy::map_state::{
    MapStateEconomyRegistry, parse_economy_scenario, stockpile_from_scenario,
};
use strategy_game::economy::resources::ResourceType;
use strategy_game::hexmap::data::load_from_path;
use strategy_game::world::load_world_preview;

fn main() {
    if let Err(e) = run() {
        eprintln!("world_economy_check failed: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a.starts_with("--economy-")) {
        return run_economy_state_cli(&args);
    }
    let map = load_from_path("assets/data/hex_atlas_map.json").map_err(|e| e.to_string())?;
    let world = load_world_preview(
        "assets/data/world/preview/provinces.ron",
        "assets/data/world/preview/states.ron",
        &map,
    )?;
    let text = std::fs::read_to_string("assets/data/world/preview/economy_scenario.ron")
        .map_err(|e| e.to_string())?;
    let scenario = parse_economy_scenario(&text)?;
    let defs: Vec<BuildingDefinition> = ron::from_str(
        &std::fs::read_to_string("assets/data/buildings.ron").map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let buildings = BuildingRegistry {
        definitions: defs.into_iter().map(|d| (d.building_type, d)).collect(),
    };

    println!("=== 検証用経済シナリオ(正式な配置・バランスではない) ===");
    println!("{}", scenario.description);
    let mut reg = MapStateEconomyRegistry::from_scenario(&scenario, &world.model, &buildings)
        .map_err(|issues| {
            issues
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })?;
    let mut stock = stockpile_from_scenario(&scenario);
    println!(
        "国家(固定入力): 動員済み人的資源 {} / 破産 {} / 備蓄 Iron {} Coal {} Food {}",
        scenario.country_mobilized_manpower,
        scenario.is_bankrupt,
        stock.get(ResourceType::Iron),
        stock.get(ResourceType::Coal),
        stock.get(ResourceType::Food)
    );
    let out = reg.run_month(
        scenario.country_mobilized_manpower,
        &mut stock,
        &buildings,
        scenario.is_bankrupt,
    );
    for s in reg.iter() {
        let provinces = world
            .model
            .provinces_of(s.state)
            .map(|p| p.len())
            .unwrap_or(0);
        println!(
            "\n[MapState#{}] 所属プロヴィンス {}(経済データはステートごとに1回だけ保持)",
            s.state.0, provinces
        );
        println!(
            "  入力: 人口 {} / 労働力比率 {} / 潜在労働力 {} / 施設 {}",
            s.population,
            s.workforce_ratio,
            strategy_game::economy::state_access::StateEconomyAccess::total_workforce(s),
            BuildingType::ALL
                .iter()
                .filter(|b| s.buildings.get(b).copied().unwrap_or(0) > 0)
                .map(|b| format!("{b:?}={}", s.buildings[b]))
                .collect::<Vec<_>>()
                .join(" ")
        );
        println!(
            "  結果: 雇用 {} / 失業 {} / 物流 容量 {} 使用 {} 充足率 {:.3}",
            s.employed_workforce,
            s.unemployed_workforce,
            s.logistics_capacity,
            s.logistics_usage,
            s.logistics_ratio
        );
        let mut ops: Vec<_> = s.building_operation_ratios.iter().collect();
        ops.sort_by_key(|(b, _)| BuildingType::ALL.iter().position(|x| x == *b));
        println!(
            "  稼働率: {}",
            if ops.is_empty() {
                "なし".to_string()
            } else {
                ops.iter()
                    .map(|(b, r)| format!("{b:?}={r:.3}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        );
    }
    println!(
        "\n国家の生産結果: 研究力 science {} magic {} / 備蓄 Iron {} Coal {} Food {} IndustrialGoods {}",
        out.science,
        out.magic,
        stock.get(ResourceType::Iron),
        stock.get(ResourceType::Coal),
        stock.get(ResourceType::Food),
        stock.get(ResourceType::IndustrialGoods)
    );
    run_multi_month(&world.model, &buildings)?;
    run_province_resources(&world.model, &buildings)?;
    run_m4c(&world.model, &buildings)?;
    run_m4d(&world.model, &buildings)?;
    run_manual_orders(&world.model, &buildings)?;
    run_snapshot_resume(&world.model, &buildings)?;
    run_location_facilities(&world.model, &map)?;
    Ok(())
}

/// M4-B: 複数国家・複数月の検証用シナリオ(`economy_scenario_m4b.ron`)を新経路で実行して表示する。
/// 旧経路(実際の旧System)との月ごとの一致は`tests/p32_008_map_state_finance_test.rs`で検証する。
fn run_multi_month(
    world: &strategy_game::world::WorldModel,
    buildings: &BuildingRegistry,
) -> Result<(), String> {
    use strategy_game::economy::map_state::{MapStateEconomyRegistry, parse_multi_month_scenario};
    let text = std::fs::read_to_string("assets/data/world/preview/economy_scenario_m4b.ron")
        .map_err(|e| e.to_string())?;
    let sc = parse_multi_month_scenario(&text)?;
    println!(
        "

=== M4-B 複数国家・複数月の検証用シナリオ(正式な配置・バランスではない) ==="
    );
    println!("{} / {} か月", sc.description, sc.months);
    println!("国家への対応づけは経済計算上のもので、法的所有・占領モデルではない。");
    let (mut reg, mut countries) =
        MapStateEconomyRegistry::from_multi_scenario(&sc, world, buildings).map_err(|issues| {
            issues
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(
                    "
",
                )
        })?;
    println!(
        "
[初期状態]"
    );
    for s in reg.iter() {
        println!(
            "  MapState#{}: 人口 {} 生活水準 {} 不満度 {}",
            s.state.0, s.population, s.living_standard, s.unrest
        );
    }
    for c in &countries {
        println!(
            "  国家{}: 国庫 {} 税率 {} 人的資源 {} 動員 {} ステート {:?}",
            c.label,
            c.treasury,
            c.tax_rate,
            c.available_manpower,
            c.mobilized_manpower,
            c.states.iter().map(|s| s.0).collect::<Vec<_>>()
        );
    }
    for m in 1..=sc.months {
        println!(
            "
[{m}か月目]"
        );
        for c in countries.iter_mut() {
            let r = reg.run_country_month(c, buildings);
            println!(
                "  国家{}: 所有人口 {} / 破産 {} / 収入 {:.4} 支出 {:.4} 収支 {:.4} / 国庫 {:.4} / 人的資源 {}",
                r.label,
                r.owned_population,
                r.is_bankrupt,
                r.finance.income,
                r.finance.expenses,
                r.finance.balance,
                c.treasury,
                c.available_manpower
            );
            for id in &c.states {
                let s = reg.get(*id).unwrap();
                println!(
                    "    MapState#{}: 人口 {} 雇用 {} 失業 {} 物流比 {:.3} 生活水準 {:.2} 不満度 {:.2}",
                    s.state.0,
                    s.population,
                    s.employed_workforce,
                    s.unemployed_workforce,
                    s.logistics_ratio,
                    s.living_standard,
                    s.unrest
                );
            }
        }
    }
    Ok(())
}

/// P32-009: プロヴィンス別の天然資源(入力の正本)→ステート集計 → 生産計算への入力と結果を表示する。
fn run_province_resources(
    world: &strategy_game::world::WorldModel,
    buildings: &BuildingRegistry,
) -> Result<(), String> {
    use strategy_game::economy::province_resources::{
        StateResourceSetup, derive_state_resources, load_province_resources,
    };
    let file = load_province_resources("assets/data/world/preview/province_resources.ron")?;
    let derived = derive_state_resources(world, &file).map_err(|i| {
        i.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(
            "
",
        )
    })?;
    println!(
        "

=== P32-009 プロヴィンス別の天然資源(検証用。正式な分布・バランスではない) ==="
    );
    println!(
        "scope = {:?}(Partial は全世界の資源配置が完了したことを意味しない)。base_output = Mine 1レベルあたりの月間産出能力(埋蔵量・在庫ではない)",
        derived.scope()
    );
    for st in derived.states() {
        let setup = match st.setup {
            StateResourceSetup::Complete => "設定済み(全プロヴィンス)",
            StateResourceSetup::Incomplete => "一部未設定",
            StateResourceSetup::Unconfigured => "未設定",
        };
        println!(
            "
[MapState#{}] {setup}: 設定 {} (うち資源なし明示 {}) / 未設定 {}",
            st.state.0,
            st.configured_provinces,
            st.explicit_none_provinces,
            st.unset_provinces.len()
        );
        for src in &st.sources {
            println!(
                "  出典 Province#{}: {:?} base_output {} discovered {} development_level {}",
                src.province.0,
                src.deposit.resource_type,
                src.deposit.base_output,
                src.deposit.discovered,
                src.deposit.development_level
            );
        }
        for d in &st.deposits {
            println!(
                "  集計(経済への入力) {:?}: base_output {} discovered {}",
                d.resource_type, d.base_output, d.discovered
            );
        }
    }
    println!(
        "
どのステートにも属さない設定済みプロヴィンス(集計しない): {:?}",
        derived
            .unassigned_configured()
            .iter()
            .map(|p| p.0)
            .collect::<Vec<_>>()
    );

    // 生産計算: ステート単位の鉱床入力を空にしたシナリオへ、プロヴィンス別の資源を接続して1か月実行する。
    let text = std::fs::read_to_string("assets/data/world/preview/economy_scenario.ron")
        .map_err(|e| e.to_string())?;
    let legacy = parse_economy_scenario(&text)?;
    let mut province_based = legacy.clone();
    for s in &mut province_based.states {
        s.resource_deposits.clear();
    }
    let fmt = |e: Vec<strategy_game::economy::map_state::EconomyDataIssue>| {
        e.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(
            "
",
        )
    };
    let mut reg_legacy =
        MapStateEconomyRegistry::from_scenario(&legacy, world, buildings).map_err(fmt)?;
    let mut reg_new =
        MapStateEconomyRegistry::from_scenario(&province_based, world, buildings).map_err(fmt)?;
    reg_new
        .attach_province_resources(&derived, world, &file)
        .map_err(fmt)?;
    let (mut s_old, mut s_new) = (
        stockpile_from_scenario(&legacy),
        stockpile_from_scenario(&legacy),
    );
    reg_legacy.run_month(
        legacy.country_mobilized_manpower,
        &mut s_old,
        buildings,
        legacy.is_bankrupt,
    );
    reg_new.run_month(
        legacy.country_mobilized_manpower,
        &mut s_new,
        buildings,
        legacy.is_bankrupt,
    );
    println!(
        "
[生産計算への入力と結果(MapState#20のMine 2レベル)]"
    );
    let m = reg_new.get(strategy_game::world::MapStateId(20)).unwrap();
    println!(
        "  入力: 鉱床 {:?} / Mine稼働率 {:.4}",
        m.resource_deposits
            .iter()
            .map(|d| format!("{:?}={}", d.resource_type, d.base_output))
            .collect::<Vec<_>>(),
        m.building_operation_ratios
            .get(&BuildingType::Mine)
            .copied()
            .unwrap_or(0.0)
    );
    println!(
        "  結果(備蓄): Iron {} Coal {} (旧入力=ステート単位の鉱床 との一致: {})",
        s_new.get(ResourceType::Iron),
        s_new.get(ResourceType::Coal),
        ResourceType::ALL
            .iter()
            .all(|r| s_old.get(*r) == s_new.get(*r))
    );
    Ok(())
}

/// P32-010: 経済状態・建設キュー・経済AIの検証用シナリオ(`economy_scenario_m4c.ron`)を、
/// プロヴィンス別の資源入力(P32-009)を接続して新経路で実行し、判断・注文・進捗・完成・
/// 経済の変化を表示する。旧経路との一致は`tests/p32_010_map_state_ai_test.rs`で検証する。
fn run_m4c(
    world: &strategy_game::world::WorldModel,
    buildings: &BuildingRegistry,
) -> Result<(), String> {
    use strategy_game::economy::economic_state::EconomicState;
    use strategy_game::economy::map_state::{
        MapStateEconomyRegistry, SimDate, parse_multi_month_scenario,
    };
    use strategy_game::economy::province_resources::{derive_state_resources, load_province_resources};
    let text = std::fs::read_to_string("assets/data/world/preview/economy_scenario_m4c.ron")
        .map_err(|e| e.to_string())?;
    let sc = parse_multi_month_scenario(&text)?;
    let fmt = |e: Vec<strategy_game::economy::map_state::EconomyDataIssue>| {
        e.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n")
    };
    println!("\n\n=== P32-010 M4-C 経済状態・建設キュー・経済AI(検証用。正式な配置・バランスではない) ===");
    println!("{} / {} か月 = {} 日(建設期間は通常どおり)", sc.description, sc.months, sc.months * 30);
    let res_file = load_province_resources("assets/data/world/preview/province_resources.ron")?;
    let derived = derive_state_resources(world, &res_file)
        .map_err(|i| i.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("\n"))?;
    let (mut reg, mut countries) =
        MapStateEconomyRegistry::from_multi_scenario(&sc, world, buildings).map_err(fmt)?;
    reg.attach_province_resources(&derived, world, &res_file).map_err(fmt)?;
    println!("鉱床入力: プロヴィンス別の資源定義(P32-009)。Mineの注文は、ステートの資源が設定済みであることを要求する。");

    let start = SimDate { year: 1800, month: 1, day: 1 };
    let name = |s: EconomicState| match s {
        EconomicState::Depression => "Depression",
        EconomicState::Recession => "Recession",
        EconomicState::Stable => "Stable",
        EconomicState::Boom => "Boom",
        EconomicState::Prosperity => "Prosperity",
    };
    let mut date = start;
    for _ in 0..sc.months * 30 {
        let (d, changed) = date.next();
        date = d;
        let r = reg.run_day(&mut countries, date, changed, buildings);
        for (label, state, b, cost) in &r.ordered {
            println!("{}/{:02}/{:02} 国家{label} AIが注文: MapState#{} {:?} (費用 {cost})", date.year, date.month, date.day, state.0, b);
        }
        for (label, from, to) in &r.tax_changes {
            println!("{}/{:02}/{:02} 国家{label} AIが税率を調整: {from} -> {to}", date.year, date.month, date.day);
        }
        for (label, state, b, level) in &r.completed {
            println!("{}/{:02}/{:02} 国家{label} 建設完成: MapState#{} {:?} -> Lv{level}", date.year, date.month, date.day, state.0, b);
        }
        for (label, state, b, why) in &r.blocked {
            println!("{}/{:02}/{:02} 国家{label} 完成を保留: MapState#{} {:?} ({why:?})", date.year, date.month, date.day, state.0, b);
        }
        if r.monthly_ran {
            println!("{}/{:02}/{:02} --- 月次経済 ---", date.year, date.month, date.day);
            for c in &countries {
                println!(
                    "  国家{}: 国庫 {:.2} 収支 {:.2} 税率 {:.2} 経済状態 {} 建設キュー {}",
                    c.label, c.treasury, c.monthly_balance, c.tax_rate, name(c.economic_state), c.construction_queue.len()
                );
                for id in &c.states {
                    let s = reg.get(*id).unwrap();
                    let mut lv: Vec<String> = BuildingType::ALL
                        .iter()
                        .filter(|b| s.buildings.get(b).copied().unwrap_or(0) > 0)
                        .map(|b| format!("{b:?}{}", s.buildings[b]))
                        .collect();
                    lv.sort();
                    println!(
                        "    MapState#{}: 雇用 {} 失業 {} 施設 [{}] 稼働率 Mine {:.3}",
                        s.state.0, s.employed_workforce, s.unemployed_workforce, lv.join(" "),
                        s.building_operation_ratios.get(&BuildingType::Mine).copied().unwrap_or(0.0)
                    );
                }
            }
        }
    }
    Ok(())
}

/// P32-011: 位置施設(港・要塞・鉄道駅)の所在地・所属・索引・港の周辺水域を表示する(検証用)。
fn run_location_facilities(
    world: &strategy_game::world::WorldModel,
    map: &strategy_game::hexmap::data::HexMap,
) -> Result<(), String> {
    use strategy_game::world::{
        LocationFacilityKind, LocationFacilityRegistry, ProvinceFacilitySetup, StateFacilitySetup,
        ProvinceId, load_location_facilities, water_adjacency,
    };
    let file = load_location_facilities("assets/data/world/preview/province_facilities.ron")?;
    let reg = LocationFacilityRegistry::build(&file, world).map_err(|i| {
        i.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("\n")
    })?;
    println!();
    println!();
    println!("=== P32-011 位置施設の入力層(検証用。正式な配置ではなく、施設効果は未接続) ===");
    println!(
        "scope = {:?}(Partial は全世界の施設配置が完了したことを意味しない)。施設 {} 件。補給・貿易・防御・鉄道接続・費用・維持費・雇用は未接続。",
        reg.scope(),
        reg.len()
    );
    for d in reg.iter() {
        let p = world.province(ProvinceId(d.province)).ok_or("province vanished")?;
        let state = match reg.state_of_facility(world, strategy_game::world::LocationFacilityId(d.id)) {
            Some(s) => format!("MapState#{}", s.0),
            None => "ステート未割当".to_string(),
        };
        let mut line = format!(
            "Facility#{} {:?} 所在地 Province#{} {} 所属 {state}",
            d.id, d.kind, d.province, p.cell
        );
        if d.kind == LocationFacilityKind::Port {
            let w = water_adjacency(map, p.cell);
            line += &format!(
                " / 周辺水域: 海 {} 湖 {} 範囲外 {}(地理上の事実。建設可否の判定ではない)",
                w.sea, w.lake, w.unmapped
            );
        }
        println!("{line}");
    }
    println!("-- 索引(所在地と現在の所属から導出) --");
    for s in world.states() {
        let ids: Vec<String> = reg
            .facilities_in_state(world, s.id)
            .unwrap_or_default()
            .iter()
            .map(|d| format!("#{}({:?})", d.id, d.kind))
            .collect();
        let setup = match reg.state_setup(world, s.id) {
            Some(StateFacilitySetup::Complete) => "全プロヴィンス設定済み",
            Some(StateFacilitySetup::Incomplete) => "一部未設定",
            _ => "未設定",
        };
        println!("MapState#{}: {setup} / 施設 [{}]", s.id.0, ids.join(" "));
    }
    let un: Vec<String> = reg
        .unassigned_facilities(world)
        .iter()
        .map(|d| format!("#{}({:?})", d.id, d.kind))
        .collect();
    println!("ステート未割当の施設 [{}](架空のステートへは集計しない)", un.join(" "));
    for k in LocationFacilityKind::ALL {
        println!("種別 {k:?}: {} 件", reg.facilities_of_kind(k).len());
    }
    let (mut has, mut empty, mut unset) = (0, 0, 0);
    for p in world.provinces() {
        match reg.province_setup(p.id) {
            ProvinceFacilitySetup::HasFacilities => has += 1,
            ProvinceFacilitySetup::ConfirmedEmpty => empty += 1,
            ProvinceFacilitySetup::Unset => unset += 1,
        }
    }
    println!("プロヴィンス設定: 施設あり {has} / 設定済みで施設なし {empty} / 未設定 {unset}");
    Ok(())
}

/// P32-012 (M4-D): 軍事状況を参照する経済AI(優先度2.5 / 2.8 / 2.9)と履歴に基づく判断(Factory増設の抑止)を、
/// 明示した軍事状況の入力で新経路に通し、入力・判断理由・行動・履歴の更新を表示する(検証用)。
fn run_m4d(
    world: &strategy_game::world::WorldModel,
    buildings: &BuildingRegistry,
) -> Result<(), String> {
    use strategy_game::economy::map_state::{
        MapStateEconomyRegistry, SimDate, parse_multi_month_scenario,
    };
    use strategy_game::economy::province_resources::{derive_state_resources, load_province_resources};
    let text = std::fs::read_to_string("assets/data/world/preview/economy_scenario_m4d.ron")
        .map_err(|e| e.to_string())?;
    let sc = parse_multi_month_scenario(&text)?;
    let fmt = |e: Vec<strategy_game::economy::map_state::EconomyDataIssue>| {
        e.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n")
    };
    println!();
    println!();
    println!("=== P32-012 M4-D 軍事状況を参照する経済AIと履歴(検証用。正式な配置・バランスではない) ===");
    println!(
        "{} / {} か月 = {} 日(建設期間は通常どおり)。軍事状況は明示した入力で、新マップ上の軍隊・戦争・前線ではない。",
        sc.description,
        sc.months,
        sc.months * 30
    );
    for c in &sc.countries {
        match &c.military {
            Some(m) => println!(
                "入力 国家{}: at_war={} 日次需要={:?} 生存師団={} 募兵キュー={} 標準歩兵の必要人的資源={:?}",
                c.label, m.at_war, m.daily_demand_units, m.living_divisions, m.recruitment_queue_len, m.infantry_required_manpower
            ),
            None => println!("入力 国家{}: 軍事状況の入力なし(平和とは扱わない。優先度2.5・2.9は評価されない)", c.label),
        }
    }
    let res_file = load_province_resources("assets/data/world/preview/province_resources_m4d.ron")?;
    let derived = derive_state_resources(world, &res_file)
        .map_err(|i| i.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("\n"))?;
    let (mut reg, mut countries) =
        MapStateEconomyRegistry::from_multi_scenario(&sc, world, buildings).map_err(fmt)?;
    reg.attach_province_resources(&derived, world, &res_file).map_err(fmt)?;

    let mut date = SimDate { year: 1800, month: 1, day: 1 };
    for _ in 0..sc.months * 30 {
        let (d, changed) = date.next();
        date = d;
        let r = reg.run_day(&mut countries, date, changed, buildings);
        let stamp = format!("{}/{:02}/{:02}", date.year, date.month, date.day);
        for e in &r.evaluations {
            if e.fired.is_some() || e.iron_shortage_average.is_some() || e.factory_blocked {
                println!(
                    "{stamp} 国家{} 判断: 戦時文脈={} Iron不足(月平均)={:?} 再建文脈={} Factory増設抑止={} 着工した軍事優先度={:?} 判断理由={:?}",
                    e.label, e.wartime_context, e.iron_shortage_average, e.rebuild_context, e.factory_blocked, e.fired, e.decision_reason
                );
            }
        }
        for (label, state, b, cost) in &r.ordered {
            println!("{stamp} 国家{label} AIが注文: MapState#{} {:?} (費用 {cost})", state.0, b);
        }
        for (label, state, b, level) in &r.completed {
            println!("{stamp} 国家{label} 建設完成: MapState#{} {:?} -> Lv{level}", state.0, b);
        }
        if r.monthly_ran {
            for c in &countries {
                if let Some(a) = &c.ai {
                    println!(
                        "{stamp} 履歴更新 国家{}: 軍需生産 {} 件 / Iron未充足 {} 件 / Factory飢餓 {} 件",
                        c.label,
                        a.history.supply.samples().len(),
                        a.history.iron.samples().len(),
                        a.history.factory.samples().len()
                    );
                }
            }
        }
    }
    Ok(())
}

/// P32-013: 新ステート(`MapStateId`)の手動建設注文・キャンセル・返金を、明示した要求の列で実行して表示する(検証用)。
fn run_manual_orders(
    world: &strategy_game::world::WorldModel,
    buildings: &BuildingRegistry,
) -> Result<(), String> {
    use strategy_game::economy::manual_script::{
        ManualAction, ManualLogEntry, RequestOutcome, parse_manual_script, run_manual_script,
    };
    use strategy_game::economy::map_state::parse_multi_month_scenario;
    use strategy_game::economy::province_resources::load_province_resources;
    let scenario = parse_multi_month_scenario(
        &std::fs::read_to_string("assets/data/world/preview/economy_scenario_m4e.ron")
            .map_err(|e| e.to_string())?,
    )?;
    let script = parse_manual_script(
        &std::fs::read_to_string("assets/data/world/preview/manual_orders_m4e.ron")
            .map_err(|e| e.to_string())?,
    )?;
    let resources = load_province_resources("assets/data/world/preview/province_resources_m4d.ron")?;
    println!();
    println!();
    println!("=== P32-013 手動建設注文・キャンセル・返金(検証用。プレイヤーUIへは未接続) ===");
    println!("{} / {} 日。入力ファイルは読み取りのみ。", script.description, script.days);
    let run = run_manual_script(&scenario, world, buildings, &resources, &script)?;
    let mut seen_blocked = std::collections::HashSet::new();
    for entry in &run.log {
        match entry {
            ManualLogEntry::Request { elapsed_days, index, action, outcome, treasury } => {
                let t = treasury
                    .map(|(a, b)| format!(" 国庫 {a:.1} -> {b:.1}"))
                    .unwrap_or_default();
                let what = match action {
                    ManualAction::Build { country, state, building } => {
                        format!("国家{country} 注文 MapState#{state} {building:?}")
                    }
                    ManualAction::Cancel { country, target } => format!("国家{country} キャンセル {target:?}"),
                    ManualAction::OmitResourceProvince(p) => format!("資源変更 Province#{p} の設定を外す"),
                };
                let result = match outcome {
                    RequestOutcome::BuildAccepted { id, cost } => format!("受理 {id} 費用 {cost}"),
                    RequestOutcome::BuildRejected(e) => format!("拒否 {e:?}"),
                    RequestOutcome::CancelAccepted(r) => format!(
                        "受理 {} {:?} MapState#{} 進捗 {}/{} 支払済み {} 返金 {}",
                        r.id, r.building_type, r.state.0, r.progress, r.required_progress, r.paid_cost, r.refund
                    ),
                    RequestOutcome::CancelRejected(e) => format!("拒否 {e:?}"),
                    RequestOutcome::CancelTargetNotIssued(i) => format!("拒否 要求{i}は注文を発行していない"),
                    RequestOutcome::ResourcesReplaced => "資源の接続をやり直した".to_string(),
                };
                println!("[{elapsed_days}日経過] 要求{index}: {what} => {result}{t}");
            }
            ManualLogEntry::Day { date, report } => {
                for (label, state, b, level) in &report.completed {
                    println!("{}/{:02}/{:02} 国家{label} 建設完成: MapState#{} {:?} -> Lv{level}", date.year, date.month, date.day, state.0, b);
                }
                for (label, state, b, why) in &report.blocked {
                    if seen_blocked.insert((*label, state.0)) {
                        println!("{}/{:02}/{:02} 国家{label} 完成を保留: MapState#{} {:?} ({why:?})", date.year, date.month, date.day, state.0, b);
                    }
                }
                for (label, state, b, cost) in &report.ordered {
                    println!("{}/{:02}/{:02} 国家{label} AIが注文: MapState#{} {:?} (費用 {cost})", date.year, date.month, date.day, state.0, b);
                }
            }
        }
    }
    for c in &run.countries {
        println!(
            "終了時 国家{}: 国庫 {:.1} キュー [{}]",
            c.label,
            c.treasury,
            c.construction_queue
                .iter()
                .map(|o| format!("{} {:?}@MapState#{} {}/{}", o.id, o.building_type, o.state.0, o.progress, o.required_progress))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

/// P32-015: 新ステート経路の実行時状態のメモリ上のスナップショットを、別の初期状態の集約へ復元して再開し、
/// 連続実行と各日の結果・状態が一致することを表示する(検証用。ファイルI/Oなし・通常起動へは注入しない)。
fn run_snapshot_resume(
    world: &strategy_game::world::WorldModel,
    buildings: &BuildingRegistry,
) -> Result<(), String> {
    use strategy_game::economy::map_state::snapshot::MapEconomySimulation;
    use strategy_game::economy::map_state::{MapStateEconomyRegistry, SimDate, parse_multi_month_scenario};
    use strategy_game::economy::province_resources::{derive_state_resources, load_province_resources};
    let scenario = parse_multi_month_scenario(
        &std::fs::read_to_string("assets/data/world/preview/economy_scenario_m4d.ron")
            .map_err(|e| e.to_string())?,
    )?;
    let resources = load_province_resources("assets/data/world/preview/province_resources_m4d.ron")?;
    let make = || -> Result<MapEconomySimulation, String> {
        let fmt = |e: Vec<strategy_game::economy::map_state::EconomyDataIssue>| {
            e.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n")
        };
        let (mut reg, cs) =
            MapStateEconomyRegistry::from_multi_scenario(&scenario, world, buildings).map_err(fmt)?;
        let d = derive_state_resources(world, &resources)
            .map_err(|i| i.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("\n"))?;
        reg.attach_province_resources(&d, world, &resources).map_err(fmt)?;
        Ok(MapEconomySimulation::new(SimDate { year: 1800, month: 1, day: 1 }, reg, cs))
    };
    println!();
    println!();
    println!("=== P32-015 スナップショット取得と検証付き復元(メモリ上。ファイルI/Oなし) ===");
    println!("m4d(AI3国家・軍事入力あり)を300日。k日完了時点で取得 → 別の初期状態の集約へ復元 → 残りを再開し、連続実行と比較。");
    const DAYS: usize = 300;
    let mut cont = make()?;
    let mut snaps = vec![cont.snapshot(world, buildings)];
    let mut reports = Vec::new();
    for _ in 0..DAYS {
        reports.push(cont.advance_day(buildings));
        snaps.push(cont.snapshot(world, buildings));
    }
    let first_monthly = reports.iter().position(|(_, r)| r.monthly_ran).unwrap_or(0);
    let first_done = reports.iter().position(|(_, r)| !r.completed.is_empty());
    let mut points = vec![0, first_monthly, first_monthly + 1, 60, 100, 150, 200, DAYS];
    if let Some(d) = first_done {
        points.push(d);
        points.push(d + 1);
    }
    points.sort_unstable();
    points.dedup();
    for k in points {
        let mut r = make()?;
        if let Err(e) = r.restore(&snaps[k], world, buildings) {
            println!("k={k}: 復元に失敗: {e}");
            return Err("snapshot restore failed".to_string());
        }
        let mut ok = r.snapshot(world, buildings) == snaps[k];
        for d in k + 1..=DAYS {
            let rep = r.advance_day(buildings);
            ok &= rep == reports[d - 1] && r.snapshot(world, buildings) == snaps[d];
        }
        ok &= r == cont;
        let date = snaps[k].date;
        println!(
            "k={k:3} ({}/{:02}/{:02} 完了時点): 再開 {} 日を連続実行と比較 => {}",
            date.year,
            date.month,
            date.day,
            DAYS - k,
            if ok { "一致" } else { "不一致" }
        );
        if !ok {
            return Err(format!("resume at day {k} differs from the continuous run"));
        }
    }
    // 不正入力: 別コンテキスト・不正な値は拒否され、現在の状態は変わらない。
    let mut target = make()?;
    for _ in 0..30 {
        target.advance_day(buildings);
    }
    let before = target.clone();
    let mut bad = snaps[100].clone();
    bad.context ^= 1;
    let e1 = target.restore(&bad, world, buildings).err().map(|e| e.to_string());
    let mut bad = snaps[100].clone();
    bad.countries[0].treasury = f64::NAN;
    let e2 = target.restore(&bad, world, buildings).err().map(|e| e.to_string());
    println!("不正入力(別コンテキスト): 拒否 = {:?}", e1);
    println!("不正入力(国庫がNaN): 拒否 = {:?}", e2);
    println!("失敗後の現在状態は変更なし: {}", target == before);
    if e1.is_none() || e2.is_none() || target != before {
        return Err("invalid snapshot handling failed".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------- P32-016: 新経路用ファイルの保存・再開(明示的な操作のみ)

fn m4d_simulation(
    world: &strategy_game::world::WorldModel,
    buildings: &BuildingRegistry,
) -> Result<strategy_game::economy::map_state::snapshot::MapEconomySimulation, String> {
    use strategy_game::economy::map_state::snapshot::MapEconomySimulation;
    use strategy_game::economy::map_state::{MapStateEconomyRegistry, SimDate, parse_multi_month_scenario};
    use strategy_game::economy::province_resources::{derive_state_resources, load_province_resources};
    let scenario = parse_multi_month_scenario(
        &std::fs::read_to_string("assets/data/world/preview/economy_scenario_m4d.ron")
            .map_err(|e| e.to_string())?,
    )?;
    let resources = load_province_resources("assets/data/world/preview/province_resources_m4d.ron")?;
    let fmt = |e: Vec<strategy_game::economy::map_state::EconomyDataIssue>| {
        e.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n")
    };
    let (mut reg, cs) =
        MapStateEconomyRegistry::from_multi_scenario(&scenario, world, buildings).map_err(fmt)?;
    let d = derive_state_resources(world, &resources)
        .map_err(|i| i.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("\n"))?;
    reg.attach_province_resources(&d, world, &resources).map_err(fmt)?;
    Ok(MapEconomySimulation::new(SimDate { year: 1800, month: 1, day: 1 }, reg, cs))
}

fn state_digest(
    sim: &strategy_game::economy::map_state::snapshot::MapEconomySimulation,
    world: &strategy_game::world::WorldModel,
    buildings: &BuildingRegistry,
) -> u64 {
    let text = ron::to_string(&sim.snapshot(world, buildings)).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn print_day(
    sim: &strategy_game::economy::map_state::snapshot::MapEconomySimulation,
    report: &strategy_game::economy::map_state::DayReport,
    world: &strategy_game::world::WorldModel,
    buildings: &BuildingRegistry,
) {
    let d = sim.date;
    let treasuries: Vec<String> = sim.countries.iter().map(|c| format!("{}:{:.2}", c.label, c.treasury)).collect();
    println!(
        "day {}/{:02}/{:02} digest={:016x} treasuries=[{}] completed={} ordered={} monthly={}",
        d.year,
        d.month,
        d.day,
        state_digest(sim, world, buildings),
        treasuries.join(" "),
        report.completed.len(),
        report.ordered.len(),
        report.monthly_ran
    );
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

/// `--economy-run <日数> [--digest-from <日数>]`(連続実行・保存なし)/
/// `--economy-save <日数> --save-base <dir> --name <名前>`(進めて、日と日の間で保存)/
/// `--economy-resume <日数> --save-base <dir> --name <名前>`(同じ静的入力を用意し、ファイルから復元して翌日から再開)。
/// 通常起動(引数なし)ではファイルを作らない。保存先は明示した `--save-base` 配下の `map_economy/` だけ。
fn run_economy_state_cli(args: &[String]) -> Result<(), String> {
    use strategy_game::economy::map_economy_file::{SaveLocation, load, save};
    let map = load_from_path("assets/data/hex_atlas_map.json").map_err(|e| e.to_string())?;
    let world = load_world_preview(
        "assets/data/world/preview/provinces.ron",
        "assets/data/world/preview/states.ron",
        &map,
    )?;
    let defs: Vec<BuildingDefinition> = ron::from_str(
        &std::fs::read_to_string("assets/data/buildings.ron").map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let buildings = BuildingRegistry { definitions: defs.into_iter().map(|d| (d.building_type, d)).collect() };
    let days_of = |flag: &str| -> Result<u32, String> {
        flag_value(args, flag)
            .ok_or_else(|| format!("{flag} needs a day count"))?
            .parse::<u32>()
            .map_err(|e| format!("{flag}: {e}"))
    };
    let location = || -> Result<(SaveLocation, String), String> {
        let base = flag_value(args, "--save-base").ok_or("--save-base <dir> is required (no default save location)")?;
        let name = flag_value(args, "--name").ok_or("--name <name> is required")?;
        Ok((SaveLocation::new(base), name))
    };
    let mut sim = m4d_simulation(&world.model, &buildings)?;
    if args.iter().any(|a| a == "--economy-run") {
        let days = days_of("--economy-run")?;
        let from = flag_value(args, "--digest-from").map(|s| s.parse::<u32>().map_err(|e| e.to_string())).transpose()?.unwrap_or(0);
        println!("連続実行: {days} 日(出力は {from} 日より後)");
        for n in 1..=days {
            let (_, report) = sim.advance_day(&buildings);
            if n > from {
                print_day(&sim, &report, &world.model, &buildings);
            }
        }
    } else if args.iter().any(|a| a == "--economy-save") {
        let days = days_of("--economy-save")?;
        let (loc, name) = location()?;
        for _ in 0..days {
            sim.advance_day(&buildings);
        }
        println!("{days} 日進めた(最後に処理を完了した日 {}/{:02}/{:02})。日と日の間で保存する。", sim.date.year, sim.date.month, sim.date.day);
        match save(&sim, &world.model, &buildings, &loc, &name) {
            Ok(r) => println!(
                "保存成功: {} ({} バイト、日付 {}/{:02}/{:02}、既存ファイルの置換 {})",
                r.path.display(),
                r.bytes,
                r.date.year,
                r.date.month,
                r.date.day,
                r.replaced_existing
            ),
            Err(e) => return Err(format!("保存失敗(成功ではない): {e}")),
        }
    } else if args.iter().any(|a| a == "--economy-resume") {
        let days = days_of("--economy-resume")?;
        let (loc, name) = location()?;
        println!("同じ静的入力(m4d)から初期状態を用意し、ファイルから復元する(初期注文や検証用操作の再適用はしない)。");
        match load(&mut sim, &world.model, &buildings, &loc, &name) {
            Ok(r) => println!(
                "復元成功: {} (復元した日付 = 最後に処理を完了した日 {}/{:02}/{:02}。再開後の最初の処理日はその翌日)",
                r.path.display(),
                r.date.year,
                r.date.month,
                r.date.day
            ),
            Err(e) => return Err(format!("復元失敗(現在の状態は変更されていない): {e}")),
        }
        for _ in 0..days {
            let (_, report) = sim.advance_day(&buildings);
            print_day(&sim, &report, &world.model, &buildings);
        }
    } else {
        return Err("unknown --economy-* operation".to_string());
    }
    Ok(())
}
