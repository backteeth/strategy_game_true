//! P32-037 (M6-d): 新経路の師団への**日次補給**(首都からの到達判定 + 旧の需要・備蓄消費・最大剰余法の配分を共有)。純粋な Rust(旧の計算は `legacy_supply` を介して再利用する)。
//!
//! 設計は `verification_logs/phase-32/p32-033/p32-033_m6_design.md` §6〜§9 と、`p32-036_m6c_occupation_report.md` §11。
//!
//! ## 日次の中での位置(旧の `Economy` Set が `MilitaryAction` より先に走るのと同じ)
//! `advance_day` の**最初**に、その日の移動・戦闘・占領より前に、前日終値の状態(師団の位置・戦闘状態・実効支配)で**1回だけ**計算する(0. 補給 → 1. 経路の検証 → 2. 移動と接触 →
//! 3. 戦闘の進行 → 4. 終了処理(占領)→ 5. 戦闘の除去 → 6. 組織率の回復)。**有効な計画だけを適用**し、備蓄の消費と充足率を一緒に更新する(計画に失敗したら、備蓄も充足率も変えない。他の国には影響しない)。
//! したがって、当日の占領で支配が変わっても、**当日の配分は変わらない**。遮断や再接続は**翌日の補給**に反映される。
//!
//! ## 国家ごとの計算
//! 1. **補給源と到達判定**: 検証用シナリオが国家ごとに首都の `ProvinceId` と初期の軍需物資の備蓄を明示する。**その国の実効支配下にある陸上プロヴィンス**(`ProvinceControl` から導く)だけを通れるものとして、
//!    首都を起点に既存の `reach`(P32-030)で到達判定する。**首都が自国の支配でなくなったら、到達集合を空にする**(旧: 首都が敵支配なら連結集合は空)。`Reach` の**「不明」(範囲外の陸を通る経路があり得る)は
//!    確認済みの到達に含めない**(件数を別に出す)。
//! 2. **需要**: 生存する師団(人員 > 0)ごとに、旧の `division_daily_demand_units`(人員比に比例、補給消費量の検証)で求める。同じ師団の需要は一度だけ計上する。
//! 3. **到達資格**(呼び出し側 = この関数が判定): 現在地が首都と連結している、または**前線補給の例外**(旧 `is_frontline_supply_eligible` が基準): 現在地が連結集合に無く、現在地の支配国が
//!    師団の国でなく、進行中の戦闘に攻撃側として参加していて、**隣接する自国支配のプロヴィンスが首都と連結している**。例外の対象の現在地は連結集合へ入れない(連鎖投射なし)。
//! 4. **配分と消費**: 旧の共通の核(`plan_supply_core`)。連結(資格あり)の需要だけを基準に、備蓄が足りれば全量、足りなければ最大剰余法で配分し(同順位は `MapDivisionId` 昇順)、
//!    資格のない師団は配分 0(需要が正なら充足率 0.0、需要 0 なら 1.0)で備蓄を消費しない。備蓄は `try_consume_military_supplies`(旧の共通処理)で1回だけ消費する。人員 0 の師団の充足率は 1.0 に戻す。
//!
//! ## 充足率の使われ方(旧の更新元・参照点を実コードで確認した上での新経路の扱い)
//! - 旧: `division_supply_fulfillment`(日次補給が `Economy` Set で毎日確定。**戦闘計算にだけ**使われる: `supply_fulfillment_of`)と、`Division.supply_ratio`(初期値 1.0。
//!   **本番で更新する `military::supply::process_supply` は呼ばれない**ので、移動の補給係数 `max(supply_ratio, 0.5)` は本番では常に 1.0)は別の値。
//! - 新: その日の配分結果(`fulfillment`)を、**戦闘計算**(旧と同じ経路)と**移動の補給係数 `max(fulfillment, 0.5)`**(下限 0.5 は旧と同じ)の両方へ反映する。移動への反映は
//!   **旧の本番とは異なる**(旧の本番は補給が移動に効かない)意図した差で、設計書 §7.2 の入力(補給の充足率 → 速度)に沿う。補給の設定がない師団は充足率 1.0 なので、M6-a〜c の挙動は不変。
//!
//! ## 再計算(キャッシュ)
//! キャッシュするのは**国家ごとの `Reach`**(と、そこから導いた確認済みの到達集合)だけ。到達集合を再利用する日にも、需要・前線補給の資格・備蓄配分は**その日の**師団位置と戦闘状態で計算し直す。
//! 無効化: 支配が変わったプロヴィンスの変更前・変更後の支配国(`countries_to_invalidate`。占領 `ControlChanged` と、検証用の `set_controller`)、首都の変更(`set_capital`)。
//! グラフが差し替えられたとき(署名の不一致)は全国家のキャッシュを捨てる。無効化された国家は、**次に補給を計画する時点で**計算し直す(当日の占領による再計算も同じ)。

use super::connectivity::{ExplicitPassable, GraphSignature, ProvinceGraph, Reach, reach};
use super::ids::ProvinceId;
use super::legacy_supply::{self, DemandEntry};
use super::movement::{MapDivision, MapDivisionId, MapMilitary, MapMilitaryError, MoveEvent, SupplyFailure, SupplyReport};
use crate::common::CountryId;
use std::collections::{BTreeMap, HashSet};

/// 国家ごとの補給の設定(検証用シナリオが明示する。正式な世界データではない)。
#[derive(Debug, Clone, PartialEq)]
pub struct SupplyCountryConfig {
    pub country: CountryId,
    /// 補給源(首都)の `ProvinceId`。
    pub capital: ProvinceId,
    /// 初期の軍需物資の備蓄(旧の `ResourceType::MilitarySupplies` と同じ値の単位)。
    pub stockpile: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct CountrySupply {
    pub country: CountryId,
    pub capital: ProvinceId,
    pub stockpile: f64,
}

/// 国家ごとのキャッシュ(到達結果)。`reach` は首都が `graph` の師団で到達判定できたときだけ `Some`。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CachedReach {
    pub capital: ProvinceId,
    pub reach: Option<Reach>,
    /// 確認済みの到達(`Reachable`)だけ。「不明」は含まない。
    pub connected: HashSet<ProvinceId>,
    pub unknown: usize,
}

/// 到達結果の再利用の統計(累計。測定・テスト用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SupplyStats {
    /// 国家ごとの到達結果を計算した回数。
    pub reach_computations: u64,
    /// キャッシュした到達結果を再利用した回数。
    pub reach_reuses: u64,
}

/// 日次補給の実行時状態(設定がなければ `MapMilitary` は持たない)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SupplyRuntime {
    pub(super) countries: BTreeMap<usize, CountrySupply>,
    pub(super) cache: BTreeMap<usize, CachedReach>,
    pub(super) signature: Option<GraphSignature>,
    pub(super) stats: SupplyStats,
}

impl MapMilitary {
    /// 国家ごとの補給を設定する(検証用シナリオが明示する首都と初期の備蓄)。空なら補給なし(従来の挙動)。
    /// 重複する国家・不正な備蓄・設定のある国家の師団の不正な補給消費量は `Err`。
    pub fn with_supply(mut self, configs: Vec<SupplyCountryConfig>) -> Result<MapMilitary, MapMilitaryError> {
        if configs.is_empty() {
            self.supply = None;
            return Ok(self);
        }
        let mut countries: BTreeMap<usize, CountrySupply> = BTreeMap::new();
        for c in configs {
            if legacy_supply::stockpile_units(c.stockpile).is_none() {
                return Err(MapMilitaryError::InvalidSupplyStockpile(c.country));
            }
            let entry = CountrySupply { country: c.country, capital: c.capital, stockpile: c.stockpile };
            if countries.insert(c.country.0, entry).is_some() {
                return Err(MapMilitaryError::DuplicateSupplyCountry(c.country));
            }
        }
        for d in self.divisions.values() {
            if countries.contains_key(&d.owner.0) && !legacy_supply::usage_is_valid(d.supply_usage) {
                return Err(MapMilitaryError::InvalidSupplyUsage(d.id));
            }
        }
        self.supply = Some(SupplyRuntime { countries, ..SupplyRuntime::default() });
        Ok(self)
    }

    /// 日次補給が設定されているか。
    pub fn supply_enabled(&self) -> bool {
        self.supply.is_some()
    }

    /// 国家の現在の備蓄(旧の固定小数点の単位)。補給の設定がない国家は `None`。
    pub fn supply_stockpile_units(&self, country: CountryId) -> Option<u64> {
        self.supply.as_ref()?.countries.get(&country.0).and_then(|c| legacy_supply::stockpile_units(c.stockpile))
    }

    /// 国家の首都(補給の設定がある国家だけ)。
    pub fn supply_capital(&self, country: CountryId) -> Option<ProvinceId> {
        self.supply.as_ref()?.countries.get(&country.0).map(|c| c.capital)
    }

    /// 首都を変更する(検証用。その国家の到達結果を無効化する)。補給の設定がない国家なら `false`。
    pub fn set_capital(&mut self, country: CountryId, capital: ProvinceId) -> bool {
        let Some(s) = self.supply.as_mut() else { return false };
        let Some(c) = s.countries.get_mut(&country.0) else { return false };
        c.capital = capital;
        s.cache.remove(&country.0);
        true
    }

    /// 国家の補給の設定(首都・備蓄)と到達結果を取り除く(国家消滅。P32-041)。設定があった場合だけ `true`。
    pub(super) fn remove_supply_country(&mut self, country: CountryId) -> bool {
        let Some(s) = self.supply.as_mut() else { return false };
        s.cache.remove(&country.0);
        s.countries.remove(&country.0).is_some()
    }

    /// 国家の到達結果(キャッシュ)を無効化する。再計算は、次に補給を計画する時点で行う。
    pub fn invalidate_reach(&mut self, countries: &[CountryId]) {
        if let Some(s) = self.supply.as_mut() {
            for c in countries {
                s.cache.remove(&c.0);
            }
        }
    }

    /// 国家の到達結果がキャッシュされているか(次の補給でそのまま再利用される状態か)。
    pub fn reach_is_cached(&self, country: CountryId) -> bool {
        self.supply.as_ref().is_some_and(|s| s.cache.contains_key(&country.0))
    }

    /// 到達結果の再利用の統計(累計)。
    pub fn supply_stats(&self) -> Option<SupplyStats> {
        self.supply.as_ref().map(|s| s.stats)
    }

    /// 前線補給の例外(旧 `is_frontline_supply_eligible` が基準)。
    fn frontline_eligible(&self, graph: &ProvinceGraph, d: &MapDivision, connected: &HashSet<ProvinceId>) -> bool {
        if connected.contains(&d.province) {
            return false;
        }
        if self.control.controller(d.province) == Some(d.owner) {
            return false;
        }
        let attacking = d.battle.and_then(|b| self.battles.get(&b)).is_some_and(|b| {
            b.status == super::battle::MapBattleStatus::Ongoing && b.attacker_country == d.owner && b.province == d.province && b.attackers.contains(&d.id)
        });
        if !attacking {
            return false;
        }
        graph.neighbor_ids(d.province).any(|n| connected.contains(&n) && self.control.controller(n) == Some(d.owner))
    }

    /// その日の日次補給(`advance_day` の最初。設定がなければ何もしない)。
    pub(super) fn plan_daily_supply(&mut self, graph: &ProvinceGraph, events: &mut Vec<MoveEvent>) {
        let Some(mut sup) = self.supply.take() else { return };
        // グラフが差し替えられたら、全国家の古い到達結果を捨てる。
        if sup.signature.as_ref() != Some(graph.signature()) {
            sup.cache.clear();
            sup.signature = Some(graph.signature().clone());
        }
        let keys: Vec<usize> = sup.countries.keys().copied().collect();
        for key in keys {
            let (country, capital, stockpile) = {
                let c = &sup.countries[&key];
                (c.country, c.capital, c.stockpile)
            };
            // 1. 補給源(首都)と、国家ごとの到達結果(キャッシュ)
            let capital_controlled = self.control.controller(capital) == Some(country);
            let mut recomputed = false;
            if capital_controlled {
                if sup.cache.get(&key).is_some_and(|c| c.capital == capital) {
                    sup.stats.reach_reuses += 1;
                } else {
                    let ids: Vec<ProvinceId> = self.control.iter().filter(|(p, c)| *c == country && graph.contains(*p)).map(|(p, _)| p).collect();
                    let result: Option<Reach> = ExplicitPassable::new(graph, ids).ok().and_then(|pass| reach(graph, &[capital], &pass).ok());
                    let connected: HashSet<ProvinceId> = result.as_ref().map(|r| r.reached_ids().into_iter().collect()).unwrap_or_default();
                    let unknown = result.as_ref().map(|r| r.unknown_count()).unwrap_or(0);
                    sup.cache.insert(key, CachedReach { capital, reach: result, connected, unknown });
                    sup.stats.reach_computations += 1;
                    recomputed = true;
                }
            }
            let empty: HashSet<ProvinceId> = HashSet::new();
            let (connected, unknown) = if capital_controlled {
                let c = &sup.cache[&key];
                (&c.connected, c.unknown)
            } else {
                (&empty, 0)
            };
            // 2. 生存する師団の需要と、到達資格(その日の位置・戦闘状態で判定。キャッシュしない)
            let living: Vec<MapDivisionId> = self.divisions.values().filter(|d| d.owner == country && d.combat.manpower > 0).map(|d| d.id).collect();
            let mut entries: Vec<DemandEntry> = Vec::with_capacity(living.len());
            let mut frontline: Vec<MapDivisionId> = Vec::new();
            let mut failure: Option<SupplyFailure> = None;
            for id in &living {
                let d = &self.divisions[id];
                match legacy_supply::demand_units(d.id, d.owner, &d.combat, d.supply_usage) {
                    Ok(demand) => {
                        let fl = self.frontline_eligible(graph, d, connected);
                        if fl {
                            frontline.push(d.id);
                        }
                        entries.push(DemandEntry { key: d.id.0 as usize, demand_units: demand, eligible: connected.contains(&d.province) || fl });
                    }
                    Err(_) => {
                        failure = Some(SupplyFailure::Demand);
                        break;
                    }
                }
            }
            if let Some(reason) = failure {
                events.push(MoveEvent::SupplyFailed { country, reason });
                continue;
            }
            // 3. 配分(旧の共通の核)
            let plan = match legacy_supply::plan(stockpile, &entries) {
                Ok(p) => p,
                Err(e) => {
                    events.push(MoveEvent::SupplyFailed { country, reason: legacy_supply::plan_failure(&e) });
                    continue;
                }
            };
            // 4. 適用: 備蓄の消費と充足率を一緒に更新する(消費に失敗したら何も変えない)
            let new_stockpile = match legacy_supply::consume(stockpile, plan.consumed_units) {
                Ok(v) => v,
                Err(_) => {
                    events.push(MoveEvent::SupplyFailed { country, reason: SupplyFailure::ApplyFailed });
                    continue;
                }
            };
            for a in &plan.allocations {
                if let Some(d) = self.divisions.get_mut(&MapDivisionId(a.key as u32)) {
                    d.fulfillment = a.fulfillment;
                }
            }
            // 計画に含まれない(人員 0 の)師団の充足率は 1.0 に戻す(旧: 前日の不足率を持ち越さない)
            for d in self.divisions.values_mut().filter(|d| d.owner == country && d.combat.manpower == 0) {
                d.fulfillment = 1.0;
            }
            sup.countries.get_mut(&key).expect("exists").stockpile = new_stockpile;
            events.push(MoveEvent::DailySupply(SupplyReport {
                country,
                capital_controlled,
                connected_provinces: connected.len(),
                unknown_provinces: unknown,
                reach_recomputed: recomputed,
                total_demand_units: plan.total_demand_units,
                eligible_demand_units: plan.eligible_demand_units,
                consumed_units: plan.consumed_units,
                stockpile_units: legacy_supply::stockpile_units(new_stockpile).unwrap_or(0),
                frontline_divisions: frontline,
            }));
        }
        self.supply = Some(sup);
    }
}
