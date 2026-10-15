//! P29-007: D4(Iron 不足時の MilitaryFactory 優先配分)の段階化(IG 備蓄に応じた線形補間)の
//! 同条件比較。設備・Iron 産出・労働力(t = 1)・初期備蓄を固定し、建設・戦争・人口・財政・
//! 消費を一切動かさずに、`process_country_production` を 120 か月進める。世界の分岐を含まない、
//! 配分方式そのものの効果だけを見る補助実験である(通常シナリオの再現ではない)。
//!
//! 変更前の HEAD(P29-006)でも、公開 API のみを使うため、同じ条件でそのまま実行できる(周期の
//! 検査は変更後の期待値。変更前の平均は `old_mean_ms` に記録)。`SAME` 行(`--nocapture`)で比較する。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::economy::production::process_country_production;
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::state::data::StateData;

fn def(
    b: BuildingType,
    inputs: &[(ResourceType, f64)],
    outputs: &[(ResourceType, f64)],
) -> BuildingDefinition {
    BuildingDefinition {
        building_type: b,
        name: format!("{b:?}"),
        construction_cost: 1000.0,
        required_progress: 90.0,
        required_workforce: 10_000.0,
        logistics_cost: 10.0,
        input_resources: inputs.iter().copied().collect::<HashMap<_, _>>(),
        output_resources: outputs.iter().copied().collect::<HashMap<_, _>>(),
        maintenance_cost: 10.0,
        max_level: 100,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

/// `assets/data/buildings.ron` と同じ入出力。
fn registry() -> BuildingRegistry {
    let mut r = BuildingRegistry::default();
    for d in [
        def(BuildingType::Mine, &[], &[]),
        def(
            BuildingType::Factory,
            &[(ResourceType::Iron, 20.0), (ResourceType::Coal, 20.0)],
            &[(ResourceType::IndustrialGoods, 30.0)],
        ),
        def(
            BuildingType::MilitaryFactory,
            &[(ResourceType::Iron, 30.0), (ResourceType::IndustrialGoods, 15.0)],
            &[(ResourceType::MilitaryEquipment, 20.0), (ResourceType::MilitarySupplies, 100.0)],
        ),
    ] {
        r.definitions.insert(d.building_type, d);
    }
    r
}

struct Scenario {
    name: &'static str,
    /// Mine の月間 Iron 産出(Mine lv1・稼働率 1.0 の鉱床で与える)。
    iron_out: f64,
    factory_lv: u32,
    mf_lv: u32,
    ig0: f64,
    iron0: f64,
    /// 変更前 HEAD(P29-006、二択の D4)で同じ条件を 120 か月進めた月産の平均(参考値)。
    old_mean_ms: f64,
}

struct Metrics {
    ms: Vec<f64>,
    ig: Vec<f64>,
    f_ig_out: Vec<f64>,
    fe_f: Vec<f64>,
    fe_mf: Vec<f64>,
    max_resid: f64,
}

fn run(sc: &Scenario, months: usize) -> Metrics {
    let reg = registry();
    let mut state = StateData {
        id: StateId(1),
        name: "S".to_string(),
        owner_country_id: CountryId(0),
        population: 1_000_000,
        workforce_ratio: 0.5,
        employed_workforce: 500_000, // 労働力比 1.0
        logistics_ratio: 1.0,
        buildings: HashMap::from([
            (BuildingType::Mine, 1),
            (BuildingType::Factory, sc.factory_lv),
            (BuildingType::MilitaryFactory, sc.mf_lv),
        ]),
        resource_deposits: vec![StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: sc.iron_out,
            discovered: true,
            development_level: 1,
        }],
        ..Default::default()
    };
    let mut stock = CountryStockpile::new();
    stock.set(ResourceType::Iron, sc.iron0);
    stock.set(ResourceType::Coal, 1.0e9);
    stock.set(ResourceType::IndustrialGoods, sc.ig0);
    let mut m = Metrics {
        ms: vec![],
        ig: vec![],
        f_ig_out: vec![],
        fe_f: vec![],
        fe_mf: vec![],
        max_resid: 0.0,
    };
    for _ in 0..months {
        let (fe0, ig0, ms0) = (
            stock.get(ResourceType::Iron),
            stock.get(ResourceType::IndustrialGoods),
            stock.get(ResourceType::MilitarySupplies),
        );
        {
            let mut refs: Vec<&mut StateData> = vec![&mut state];
            process_country_production(&mut refs, &mut stock, &reg, false);
        }
        let op = |b: BuildingType| {
            state.building_operation_ratios.get(&b).copied().unwrap_or(0.0) as f64
        };
        let f_lv = sc.factory_lv as f64;
        let m_lv = sc.mf_lv as f64;
        let fe_f = 20.0 * f_lv * op(BuildingType::Factory);
        let fe_mf = 30.0 * m_lv * op(BuildingType::MilitaryFactory);
        let ig_out = 30.0 * f_lv * op(BuildingType::Factory);
        let ig_used = 15.0 * m_lv * op(BuildingType::MilitaryFactory);
        let fe_out = sc.iron_out * op(BuildingType::Mine);
        let ms_out = stock.get(ResourceType::MilitarySupplies) - ms0;
        let r_fe = stock.get(ResourceType::Iron) - (fe0 + fe_out - fe_f - fe_mf);
        let r_ig = stock.get(ResourceType::IndustrialGoods) - (ig0 + ig_out - ig_used);
        m.max_resid = m.max_resid.max(r_fe.abs()).max(r_ig.abs());
        assert!(stock.get(ResourceType::Iron) >= 0.0);
        assert!(stock.get(ResourceType::IndustrialGoods) >= 0.0);
        m.ms.push(ms_out);
        m.ig.push(stock.get(ResourceType::IndustrialGoods));
        m.f_ig_out.push(ig_out);
        m.fe_f.push(fe_f);
        m.fe_mf.push(fe_mf);
    }
    m
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

fn std(v: &[f64]) -> f64 {
    let m = mean(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len().max(1) as f64).sqrt()
}

#[test]
fn same_condition_cycle_comparison() {
    let scenarios = [
        Scenario { name: "elfin1845(Fe124,F46,MF4)", iron_out: 124.0, factory_lv: 46, mf_lv: 4, ig0: 233.0, iron0: 124.0, old_mean_ms: 309.0 },
        Scenario { name: "elfin1817(Fe139,F11,MF4)", iron_out: 139.0, factory_lv: 11, mf_lv: 4, ig0: 154.0, iron0: 136.0, old_mean_ms: 346.8 },
        Scenario { name: "base1817(Fe91,F10,MF3)", iron_out: 91.0, factory_lv: 10, mf_lv: 3, ig0: 218.0, iron0: 56.0, old_mean_ms: 227.9 },
        Scenario { name: "ironrich(Fe600,F10,MF4)", iron_out: 600.0, factory_lv: 10, mf_lv: 4, ig0: 100.0, iron0: 600.0, old_mean_ms: 400.0 },
    ];
    for sc in &scenarios {
        // 最初の 12 か月は初期状態の影響として除く。
        let m = run(sc, 132);
        let ms = &m.ms[12..];
        let ig = &m.ig[12..];
        let mean_ms = mean(ms);
        let std_ms = std(ms);
        // 平均が 0 のケースでは変動係数を算出しない(不正値にしない)。
        let cv = if mean_ms > 1e-9 { format!("{:.3}", std_ms / mean_ms) } else { "n/a".to_string() };
        let min_ms = ms.iter().cloned().fold(f64::MAX, f64::min);
        let max_ms = ms.iter().cloned().fold(f64::MIN, f64::max);
        let max_drop = ms
            .windows(2)
            .map(|w| w[0] - w[1])
            .fold(0.0f64, f64::max);
        let low = ms.iter().filter(|&&x| x < 0.5 * max_ms).count();
        println!(
            "SAME {} MS mean={:.1} min={:.1} max={:.1} std={:.1} cv={} maxdrop={:.1} low(<50%max)={} | IG min={:.0} max={:.0} mean={:.0} | F_IGout mean={:.1} Fe_F mean={:.1} Fe_MF mean={:.1} | resid={:.2e}",
            sc.name, mean_ms, min_ms, max_ms, std_ms, cv, max_drop, low,
            ig.iter().cloned().fold(f64::MAX, f64::min),
            ig.iter().cloned().fold(f64::MIN, f64::max),
            mean(ig),
            mean(&m.f_ig_out[12..]),
            mean(&m.fe_f[12..]),
            mean(&m.fe_mf[12..]),
            m.max_resid,
        );
        assert!(m.max_resid < 1e-6, "{}: conservation residual {}", sc.name, m.max_resid);
        // 平均月産は変更前の 97% 以上(配分方式の変更で平均が大きく下がっていない)。
        assert!(mean_ms >= 0.97 * sc.old_mean_ms, "{}: mean {} vs old {}", sc.name, mean_ms, sc.old_mean_ms);
        // 周期の急落が緩和されている(産出の急落幅が平均の 10% 未満、変動係数 0.05 以下)。
        assert!(max_drop < 0.1 * mean_ms, "{}: max drop {} vs mean {}", sc.name, max_drop, mean_ms);
        assert!(std_ms <= 0.05 * mean_ms.max(1e-9), "{}: cv", sc.name);
    }
}
