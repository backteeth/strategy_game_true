//! P32-035 (M6-b): 新経路のプロヴィンス単位の戦闘(開始・合流・日次の進行・終了・退却)。純粋な Rust(旧経路の戦闘計算は `legacy_combat` を介して再利用する)。
//!
//! 旧経路(`military::invasion`・`military::update`・`military::battle`)から引き継ぐ挙動:
//! - 戦闘の場所は `ProvinceId`(旧は `StateId`)。**同じステート内でも別プロヴィンスの師団を同じ戦闘へ入れない**。1地点に進行中の戦闘は1つだけ。
//! - 敵支配地へ進入した攻撃側に、**その地点にいる**交戦国の師団が全員、防御側として参加する(`MapDivisionId` 昇順。防御側の国は先頭の師団の所属国)。
//! - 同地点で進行中の戦闘があれば、攻撃側の国の師団だけが攻撃側として合流する(`MapDivisionId` 昇順で保持。重複登録なし)。**合流できない師団(第三国・防御側の国)は、進入前に止める**
//!   (旧は到着先へ入ってから待機させ、戦闘に参加しない師団が戦場に居座る。新は進入させない。意図した差)。防御側の増援が戦闘へ合流しない点は旧と同じ(後続の課題)。
//! - 日次: 戦闘の進行(`MapBattleId` 昇順)→ 終了処理(同じく昇順。旧は HashMap 順)。脱落(組織率 0)した師団は退却または撃破、どちらかの陣営の参加者がいなくなれば戦闘が終わる。
//! - 退却は**同じ処理の中で即座に完了**する(旧に `Retreating` の状態は本番で設定されない)。攻撃側は出撃元(支配が自国で、進行中の戦闘が無いとき)、使えなければ戦闘地点の隣接で
//!   自国支配かつ戦闘中でない最小 `ProvinceId`。防御側は戦闘地点の隣接で自国支配かつ戦闘中でない最小 `ProvinceId`。退路がなければ撃破される。
//! - 両陣営の人員損失が 0 の日は、防御側勝利として攻撃側全員を撤退させる(旧 P27-007)。
//!
//! **占領(M6-c。`occupation.rs`)**: 守備隊のいない進入と攻撃側の勝利では、`ControlClaim`(対象プロヴィンス・攻撃国・直前の支配国・理由・関わった師団)を1回だけ発行し、**その場で適用する**
//! (守備隊なしは接触の処理の中、戦闘の勝利は終了処理の中。いずれも当日の組織率回復より前)。その時点の師団は、対象プロヴィンスにいて `Idle`、命令なし(経路・目的地は破棄)。
//! 同じ要求は再発行されない(接触は移動中の師団が到着したときだけ起き、戦闘は終了した日に除去される)。

use super::ids::ProvinceId;
use super::legacy_combat::{self, ParticipantOutcome};
use super::occupation::ClaimRequest;
use super::movement::{BattleOutcome, ClaimReason, FinishReason, MapDivisionId, MapDivisionStatus, MapMilitary, MoveEvent};
use crate::common::CountryId;
use std::collections::{BTreeMap, BTreeSet};

/// 新経路の戦闘ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MapBattleId(pub u32);

impl std::fmt::Display for MapBattleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapBattle#{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapBattleStatus {
    Ongoing,
    AttackerWon,
    DefenderWon,
}

/// プロヴィンス単位の戦闘。
#[derive(Debug, Clone, PartialEq)]
pub struct MapBattle {
    pub id: MapBattleId,
    /// 戦闘の場所。
    pub province: ProvinceId,
    pub attacker_country: CountryId,
    pub defender_country: CountryId,
    /// 参加師団(`MapDivisionId` 昇順・重複なし)。
    pub attackers: Vec<MapDivisionId>,
    pub defenders: Vec<MapDivisionId>,
    /// 攻撃側各師団の出撃元(合流した師団ごとに個別)。敗北時の退路の第一候補。
    pub attacker_origins: BTreeMap<MapDivisionId, ProvinceId>,
    pub elapsed_days: u32,
    pub status: MapBattleStatus,
}

impl MapMilitary {
    /// 戦闘(`MapBattleId` 昇順)。
    pub fn battles(&self) -> impl Iterator<Item = &MapBattle> + '_ {
        self.battles.values()
    }

    pub fn battle(&self, id: MapBattleId) -> Option<&MapBattle> {
        self.battles.get(&id)
    }

    /// `p` で進行中の戦闘(あれば1つ)。
    pub fn ongoing_battle_at(&self, p: ProvinceId) -> Option<&MapBattle> {
        self.battles.values().find(|b| b.province == p && b.status == MapBattleStatus::Ongoing)
    }

    /// 敵支配地へ入った攻撃側 `id`(現在地は既に対象プロヴィンス)の接触処理(旧 `process_division_arrival`)。`from` は出撃元。
    pub(super) fn contact(&mut self, id: MapDivisionId, from: ProvinceId, events: &mut Vec<MoveEvent>) {
        let (owner, province) = {
            let d = &mut self.divisions.get_mut(&id).expect("exists");
            // 残りの経路と目的地を破棄して止まる(M6-b 単体では、占領が成立したものとして奥へ進まない)。
            d.target = None;
            d.destination = None;
            d.path.clear();
            d.progress = 0.0;
            d.status = MapDivisionStatus::Idle;
            (d.owner, d.province)
        };
        // 既存の進行中の戦闘: 攻撃側の国なら合流(合流できない国は移動の段階で進入前に止めてある)。
        if let Some(b) = self.ongoing_battle_at(province) {
            debug_assert_eq!(b.attacker_country, owner, "other countries are stopped before entering");
            if b.attacker_country == owner {
                let bid = b.id;
                self.join_as_attacker(bid, id, from);
                events.push(MoveEvent::BattleJoined { battle: bid, division: id, province });
            }
            return;
        }
        // 地点にいる交戦国の師団(`MapDivisionId` 昇順)
        let defenders: Vec<MapDivisionId> = self.divisions.values().filter(|d| d.province == province && d.id != id && self.at_war(owner, d.owner)).map(|d| d.id).collect();
        if defenders.is_empty() {
            // 守備隊なし: 更新要求を1回だけ発行し、その場で適用する(旧の即時占領。師団は Idle のまま対象プロヴィンスにいる)。
            let req = ClaimRequest { province, claimant: owner, previous_controller: self.control.controller(province), reason: ClaimReason::Undefended, divisions: vec![id] };
            self.claim_and_report(req, events);
            return;
        }
        let defender_country = self.divisions[&defenders[0]].owner;
        let bid = MapBattleId(self.next_battle);
        self.next_battle += 1;
        let battle = MapBattle {
            id: bid,
            province,
            attacker_country: owner,
            defender_country,
            attackers: vec![id],
            defenders: defenders.clone(),
            attacker_origins: [(id, from)].into_iter().collect(),
            elapsed_days: 0,
            status: MapBattleStatus::Ongoing,
        };
        self.battles.insert(bid, battle);
        for did in std::iter::once(&id).chain(defenders.iter()) {
            let d = self.divisions.get_mut(did).expect("participant exists");
            d.status = MapDivisionStatus::Fighting;
            d.battle = Some(bid);
            // 旧は防御側の命令を戦闘開始後も残す(停止した古い命令が残る)。新は命令を消す。
            d.target = None;
            d.destination = None;
            d.path.clear();
            d.progress = 0.0;
        }
        events.push(MoveEvent::BattleStarted { battle: bid, province, attacker: owner, defender: defender_country, attackers: vec![id], defenders });
    }

    fn join_as_attacker(&mut self, bid: MapBattleId, id: MapDivisionId, from: ProvinceId) {
        let b = self.battles.get_mut(&bid).expect("battle exists");
        if !b.attackers.contains(&id) {
            b.attackers.push(id);
            b.attackers.sort_unstable();
        }
        b.attacker_origins.insert(id, from);
        let d = self.divisions.get_mut(&id).expect("exists");
        d.status = MapDivisionStatus::Fighting;
        d.battle = Some(bid);
    }

    /// 日次の戦闘の進行(旧 `process_daily_battles`。`MapBattleId` 昇順)。その日ゼロ損害だった戦闘の ID を返す。
    pub(super) fn run_daily_battles(&mut self) -> BTreeSet<MapBattleId> {
        let mut zero = BTreeSet::new();
        let ids: Vec<MapBattleId> = self.battles.keys().copied().collect();
        for bid in ids {
            let (attackers, defenders) = {
                let b = &self.battles[&bid];
                if b.status != MapBattleStatus::Ongoing {
                    continue;
                }
                // 消滅済みの参加者を除く
                let a: Vec<MapDivisionId> = b.attackers.iter().copied().filter(|d| self.divisions.contains_key(d)).collect();
                let d: Vec<MapDivisionId> = b.defenders.iter().copied().filter(|d| self.divisions.contains_key(d)).collect();
                (a, d)
            };
            {
                let b = self.battles.get_mut(&bid).expect("exists");
                b.attackers = attackers.clone();
                b.defenders = defenders.clone();
            }
            if attackers.is_empty() || defenders.is_empty() {
                continue; // 終了処理に任せる
            }
            let view = |ids: &[MapDivisionId], this: &MapMilitary| -> Vec<(MapDivisionId, CountryId, super::movement::CombatStats)> {
                ids.iter().map(|i| (*i, this.divisions[i].owner, this.divisions[i].combat)).collect()
            };
            let a = view(&attackers, self);
            let d = view(&defenders, self);
            let a_refs: Vec<(MapDivisionId, CountryId, &super::movement::CombatStats)> = a.iter().map(|(i, o, c)| (*i, *o, c)).collect();
            let d_refs: Vec<(MapDivisionId, CountryId, &super::movement::CombatStats)> = d.iter().map(|(i, o, c)| (*i, *o, c)).collect();
            let Some(damage) = legacy_combat::resolve_day(&a_refs, &d_refs) else {
                continue; // 参加者の準備率が不正: その日の解決を見送る(旧と同じ)
            };
            if damage.zero_damage {
                zero.insert(bid);
            }
            for (id, (m, o)) in damage.attackers.iter().chain(damage.defenders.iter()) {
                if let Some(div) = self.divisions.get_mut(id) {
                    legacy_combat::apply_casualty(&mut div.combat, *m, *o);
                }
            }
            self.battles.get_mut(&bid).expect("exists").elapsed_days += 1;
        }
        zero
    }

    /// 防御側の退路: 戦闘地点の隣接で、`owner` の支配かつ進行中の戦闘が無い最小の `ProvinceId`。
    fn defender_retreat(&self, graph: &super::connectivity::ProvinceGraph, battle_province: ProvinceId, owner: CountryId) -> Option<ProvinceId> {
        graph.neighbor_ids(battle_province).find(|n| self.control.controller(*n) == Some(owner) && self.ongoing_battle_at(*n).is_none())
    }

    /// 攻撃側の退路: 出撃元が有効(`owner` の支配で、進行中の戦闘が無い)ならそこ。無効・欠落なら、防御側と同じ条件の代替先。
    fn attacker_retreat(&self, graph: &super::connectivity::ProvinceGraph, battle_province: ProvinceId, owner: CountryId, origin: Option<ProvinceId>) -> Option<ProvinceId> {
        if let Some(o) = origin
            && self.control.controller(o) == Some(owner)
            && self.ongoing_battle_at(o).is_none()
        {
            return Some(o);
        }
        self.defender_retreat(graph, battle_province, owner)
    }

    /// 脱落した参加師団の結末を適用する。戦闘に残るなら `true`。
    fn apply_outcome(&mut self, id: MapDivisionId, bid: MapBattleId, province: ProvinceId, dest: Option<ProvinceId>, force: bool, events: &mut Vec<MoveEvent>) -> bool {
        let Some(d) = self.divisions.get_mut(&id) else {
            return false; // 既に消滅済み
        };
        let outcome = if force { legacy_combat::force_retreat_or_destroy(&mut d.combat, dest.is_some()) } else { legacy_combat::participant_outcome(&mut d.combat, dest.is_some()) };
        match outcome {
            ParticipantOutcome::Stays => true,
            ParticipantOutcome::Retreats => {
                let to = dest.expect("retreat needs a destination");
                d.province = to;
                d.status = MapDivisionStatus::Idle;
                d.battle = None;
                d.target = None;
                d.destination = None;
                d.path.clear();
                d.progress = 0.0;
                events.push(MoveEvent::Retreated { division: id, battle: bid, from: province, to });
                false
            }
            ParticipantOutcome::Destroyed => {
                self.divisions.remove(&id);
                events.push(MoveEvent::Destroyed { division: id, battle: bid, at: province });
                false
            }
        }
    }

    /// 戦闘の終了処理(旧 `resolve_finished_battles`。`MapBattleId` 昇順)。
    pub(super) fn resolve_battles(&mut self, graph: &super::connectivity::ProvinceGraph, zero: &BTreeSet<MapBattleId>, events: &mut Vec<MoveEvent>) {
        let ids: Vec<MapBattleId> = self.battles.keys().copied().collect();
        for bid in ids {
            let battle = self.battles[&bid].clone();
            if battle.status != MapBattleStatus::Ongoing {
                continue;
            }
            let province = battle.province;
            let finish = |this: &mut MapMilitary, outcome: MapBattleStatus| {
                this.battles.get_mut(&bid).expect("exists").status = outcome;
            };
            // その日ゼロ損害: 防御側勝利、攻撃側全員が出撃元(無効なら代替先)へ撤退(防御側は Idle へ)
            if zero.contains(&bid) {
                finish(self, MapBattleStatus::DefenderWon);
                for id in &battle.attackers {
                    let dest = self.attacker_retreat(graph, province, battle.attacker_country, battle.attacker_origins.get(id).copied());
                    self.apply_outcome(*id, bid, province, dest, true, events);
                }
                for id in &battle.defenders {
                    if let Some(d) = self.divisions.get_mut(id) {
                        d.status = MapDivisionStatus::Idle;
                        d.battle = None;
                    }
                }
                events.push(MoveEvent::BattleFinished {
                    battle: bid,
                    province,
                    attacker: battle.attacker_country,
                    defender: battle.defender_country,
                    outcome: BattleOutcome::DefenderWon,
                    reason: FinishReason::ZeroDamage,
                });
                continue;
            }
            let mut remaining_attackers = Vec::new();
            for id in &battle.attackers {
                let dest = self.attacker_retreat(graph, province, battle.attacker_country, battle.attacker_origins.get(id).copied());
                if self.apply_outcome(*id, bid, province, dest, false, events) {
                    remaining_attackers.push(*id);
                }
            }
            let mut remaining_defenders = Vec::new();
            for id in &battle.defenders {
                let Some(owner) = self.divisions.get(id).map(|d| d.owner) else { continue };
                let dest = self.defender_retreat(graph, province, owner);
                if self.apply_outcome(*id, bid, province, dest, false, events) {
                    remaining_defenders.push(*id);
                }
            }
            {
                let b = self.battles.get_mut(&bid).expect("exists");
                b.attackers = remaining_attackers.clone();
                b.defenders = remaining_defenders.clone();
            }
            match (remaining_attackers.is_empty(), remaining_defenders.is_empty()) {
                (false, false) => {} // 両陣営とも生存: 継続
                (false, true) => {
                    finish(self, MapBattleStatus::AttackerWon);
                    for id in &remaining_attackers {
                        let d = self.divisions.get_mut(id).expect("alive");
                        d.status = MapDivisionStatus::Idle;
                        d.battle = None;
                    }
                    events.push(MoveEvent::BattleFinished {
                        battle: bid,
                        province,
                        attacker: battle.attacker_country,
                        defender: battle.defender_country,
                        outcome: BattleOutcome::AttackerWon,
                        reason: FinishReason::DefenderGone,
                    });
                    // 攻撃側の勝利: 終了処理の中で、その場で適用する(旧の即時占領。後続の戦闘の退路判定は更新後の支配国を参照する)。
                    let req = ClaimRequest {
                        province,
                        claimant: battle.attacker_country,
                        previous_controller: self.control.controller(province),
                        reason: ClaimReason::AttackerWonBattle(bid),
                        divisions: remaining_attackers.clone(),
                    };
                    self.claim_and_report(req, events);
                }
                (true, false) => {
                    finish(self, MapBattleStatus::DefenderWon);
                    for id in &remaining_defenders {
                        let d = self.divisions.get_mut(id).expect("alive");
                        d.status = MapDivisionStatus::Idle;
                        d.battle = None;
                    }
                    events.push(MoveEvent::BattleFinished {
                        battle: bid,
                        province,
                        attacker: battle.attacker_country,
                        defender: battle.defender_country,
                        outcome: BattleOutcome::DefenderWon,
                        reason: FinishReason::AttackerGone,
                    });
                }
                (true, true) => {
                    finish(self, MapBattleStatus::DefenderWon);
                    events.push(MoveEvent::BattleFinished {
                        battle: bid,
                        province,
                        attacker: battle.attacker_country,
                        defender: battle.defender_country,
                        outcome: BattleOutcome::DefenderWon,
                        reason: FinishReason::BothGone,
                    });
                }
            }
        }
    }

    /// 終了した戦闘を除去する(旧 `cleanup_finished_battles`)。進行中だけが残る。
    pub(super) fn cleanup_finished_battles(&mut self) {
        self.battles.retain(|_, b| b.status == MapBattleStatus::Ongoing);
    }

    /// 組織率の回復(旧 `process_org_recovery`): 戦闘中でも移動中でもなく、自国支配のプロヴィンスにいる師団。
    pub(super) fn recover_organizations(&mut self) {
        let ids: Vec<MapDivisionId> = self.divisions.keys().copied().collect();
        for id in ids {
            let (status, owner, province) = {
                let d = &self.divisions[&id];
                (d.status, d.owner, d.province)
            };
            if status != MapDivisionStatus::Idle || self.control.controller(province) != Some(owner) {
                continue;
            }
            legacy_combat::recover_organization(&mut self.divisions.get_mut(&id).expect("exists").combat);
        }
    }
}
