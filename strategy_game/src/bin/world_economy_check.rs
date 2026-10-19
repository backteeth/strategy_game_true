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
