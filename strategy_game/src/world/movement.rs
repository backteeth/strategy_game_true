//! P32-034/035 (M6-a/b): 新経路の師団の初期配置と、陸上プロヴィンス間の日単位の移動・敵支配地への進入(純粋な Rust。Bevy・経済・保存・軍の旧型に依存しない)。
//!
//! 設計は `verification_logs/phase-32/p32-033/p32-033_m6_design.md`(§5・§6・§10)。日次補給は M6-d(`supply.rs`。その日の移動・戦闘・占領より前に1回計算する)。**前線/AI・UI・保存は含まない**。
//! 占領(守備隊のいない進入・戦闘の勝利による実効支配の更新。D8: 旧本番の**即時占領**)は M6-c(`occupation.rs`)。
//! 戦闘の本体は `battle.rs`、旧経路の戦闘計算の再利用は `legacy_combat.rs`(隔離)。
//!
//! 旧経路(`military::movement`・`military::pathfinding`・`military::invasion`)の挙動のうち、次を引き継ぐ(旧コードを呼ばず、同じ規則を新しい型で実装する。比較は
//! `tests/p32_034_map_movement_test.rs`・`tests/p32_035_map_battle_test.rs`):
//! - 移動命令は冪等: 現在地(Idle)・移動中の同じ目的地への命令は no-op。新しい命令は、経路が確定した時点で `destination`/`path`/`target=None`/`progress=0`/`Moving` を**まとめて**適用し、
//!   その時点では現在地を動かさない。拒否したときは何も変更しない。
//! - 経路: 辺の費用は 1、同費用の候補は `ProvinceId` 昇順。通れるのは**所属国の支配地と、交戦国の支配地**(旧の `allowed`/`hostile`)。河川は費用にも可否にも加えない。
//! - 日次: `MapDivisionId` 昇順。日の初めに経路を検証し(次の移動先が通れなければ進入前に停止、残りの経路に通れない所があれば経路を切って現在のホップだけ続ける)、その後に移動する。
//!   1ホップの日数 = `max(1, 基準日数 × 費用 / (速度 × 補給係数))`(f32、旧と同じ式)。補給係数 = `max(その日の補給充足率, 0.5)`(補給の設定がない師団は充足率 1.0 なので 1.0)。
//!
//! M6-b で加えたもの: 交戦関係(検証用シナリオが明示する国家の組。双方向)、敵支配のプロヴィンスへの進入と戦闘・退却(`battle.rs`)、戦闘中の師団への命令の拒否。
//!
//! **敵支配地に着いたときの扱い(M6-b 単体)**: 師団は到着したプロヴィンスに入り(現在地を更新)、**残りの経路と目的地を破棄**して止まる(占領が成立したものとして敵領の奥へ進まない)。
//! 占領が成立した(実効支配が更新された)あとは、新しい命令で移動できる。実行時状態であり、保存しない(M8 まで)。

use super::battle::{MapBattle, MapBattleId};
use super::connectivity::ProvinceGraph;
use super::ids::ProvinceId;
use crate::common::CountryId;
use std::collections::{BTreeMap, HashMap};

/// 新経路の師団ID(旧 `DivisionId` とは別の型。数値の一致で対応づけない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MapDivisionId(pub u32);

impl std::fmt::Display for MapDivisionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapDivision#{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapDivisionStatus {
    Idle,
    Moving,
    /// 戦闘に参加中(M6-b)。命令は受け付けない。旧の `Retreating` は本番で一度も設定されない(撤退は同じ処理の中で即座に完了する)ので、新経路にも持たない。
    Fighting,
}

/// 師団の戦闘能力(旧 `Division` の戦闘に関わる値。計算は旧の関数を再利用する: `legacy_combat`)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombatStats {
    pub manpower: u64,
    pub max_manpower: u64,
    pub equipment: f64,
    pub max_equipment: f64,
    pub organization: f32,
    pub max_organization: f32,
    pub attack_power: i32,
    pub defense_power: i32,
}

impl CombatStats {
    /// 検証用の標準値(旧経路のテストの標準師団と同じ: 人員 10,000・装備 100・組織率 100・攻撃 10・防御 10)。
    pub fn standard() -> CombatStats {
        CombatStats { manpower: 10_000, max_manpower: 10_000, equipment: 100.0, max_equipment: 100.0, organization: 100.0, max_organization: 100.0, attack_power: 10, defense_power: 10 }
    }

    /// 値が正常か(有限・範囲内)。
    pub fn is_valid(&self) -> bool {
        self.max_manpower > 0
            && self.manpower <= self.max_manpower
            && self.max_equipment.is_finite()
            && self.max_equipment > 0.0
            && self.equipment.is_finite()
            && (0.0..=self.max_equipment).contains(&self.equipment)
            && self.max_organization.is_finite()
            && self.max_organization > 0.0
            && self.organization.is_finite()
            && (0.0..=self.max_organization).contains(&self.organization)
            && self.attack_power >= 0
            && self.defense_power >= 0
    }
}

/// 新経路の師団(旧 `Division` とは並行する別の型で、自動変換は無い)。
#[derive(Debug, Clone, PartialEq)]
pub struct MapDivision {
    pub id: MapDivisionId,
    pub owner: CountryId,
    /// 現在地。
    pub province: ProvinceId,
    /// 最終目的地(移動中だけ)。
    pub destination: Option<ProvinceId>,
    /// 今向かっているホップより先の残りの経路(先頭が次のホップ先)。
    pub path: Vec<ProvinceId>,
    /// 今向かっている1ホップ先。
    pub target: Option<ProvinceId>,
    /// 今のホップの進捗(0.0..1.0)。
    pub progress: f32,
    pub status: MapDivisionStatus,
    pub speed: f32,
    pub combat: CombatStats,
    /// 参加中の戦闘(`Fighting` のときだけ)。
    pub battle: Option<MapBattleId>,
    /// その日の補給充足率(`0.0..=1.0`。M6-d の日次補給が毎日、その日の移動・戦闘・占領より前に確定する。補給の設定がない師団は常に 1.0)。
    /// 戦闘計算(旧の `supply_fulfillment`)と、移動の補給係数(`max(充足率, 0.5)`)の両方が参照する。
    pub fulfillment: f64,
    /// 補給需要の計算に使う補給消費量(旧 `DivisionDefinition::supply_usage`)。補給の設定がある国家の師団は、検証用シナリオが明示する。
    pub supply_usage: f32,
}

impl MapDivision {
    /// 初期配置(Idle・命令なし・標準の戦闘能力)。
    pub fn new(id: MapDivisionId, owner: CountryId, province: ProvinceId, speed: f32) -> MapDivision {
        MapDivision {
            id,
            owner,
            province,
            destination: None,
            path: Vec::new(),
            target: None,
            progress: 0.0,
            status: MapDivisionStatus::Idle,
            speed,
            combat: CombatStats::standard(),
            battle: None,
            fulfillment: 1.0,
            supply_usage: 1.0,
        }
    }

    pub fn with_combat(mut self, combat: CombatStats) -> MapDivision {
        self.combat = combat;
        self
    }

    pub fn with_supply_usage(mut self, supply_usage: f32) -> MapDivision {
        self.supply_usage = supply_usage;
        self
    }
}

/// プロヴィンスごとの実効支配国(検証用の入力から作る実行時状態。**正式な所有・実効支配のデータではない**)。
/// 載っていないプロヴィンスは支配国なし(通れない)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProvinceControl(BTreeMap<ProvinceId, CountryId>);

impl ProvinceControl {
    pub fn new(map: BTreeMap<ProvinceId, CountryId>) -> ProvinceControl {
        ProvinceControl(map)
    }

    pub fn controller(&self, p: ProvinceId) -> Option<CountryId> {
        self.0.get(&p).copied()
    }

    /// 支配国を設定する(`None` で支配国なし)。M6-c の占領の更新口でもある。
    pub fn set(&mut self, p: ProvinceId, c: Option<CountryId>) {
        match c {
            Some(c) => {
                self.0.insert(p, c);
            }
            None => {
                self.0.remove(&p);
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (ProvinceId, CountryId)> + '_ {
        self.0.iter().map(|(p, c)| (*p, *c))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// 移動の検証用パラメータ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementParams {
    /// 1ホップの基準日数(旧経路は 5.0)。
    pub base_days: f32,
}

/// 旧経路の既定の基準日数。
pub const DEFAULT_BASE_DAYS: f32 = 5.0;

/// 移動の補給係数の下限(旧経路の `max(supply_ratio, 0.5)` の 0.5)。補給係数 = `max(その日の補給充足率, この値)`。
pub const MIN_SUPPLY_MODIFIER: f32 = 0.5;

/// 辺の費用(旧経路の `calculate_step_cost` と同じく常に 1。河川・地形は加えない)。
pub const STEP_COST: f32 = 1.0;

/// 構築の失敗(シナリオの検証を通った入力では生じない。直接構築の防御)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapMilitaryError {
    DuplicateDivision(MapDivisionId),
    InvalidSpeed(MapDivisionId),
    InvalidCombat(MapDivisionId),
    InvalidBaseDays,
    /// 同じ国家の組(自国との交戦)。
    SelfWar(CountryId),
    /// 同じ組(逆順を含む)が重複している。
    DuplicateWar(CountryId, CountryId),
    /// 補給の設定で、同じ国家が複数回ある。
    DuplicateSupplyCountry(CountryId),
    /// 補給の設定の備蓄が不正(有限でない・負・値域外)。
    InvalidSupplyStockpile(CountryId),
    /// 補給の設定がある国家の師団の補給消費量が不正。
    InvalidSupplyUsage(MapDivisionId),
}

impl std::fmt::Display for MapMilitaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateDivision(d) => write!(f, "{d} is defined more than once"),
            Self::InvalidSpeed(d) => write!(f, "{d} has a non-finite or non-positive speed"),
            Self::InvalidCombat(d) => write!(f, "{d} has invalid combat stats"),
            Self::InvalidBaseDays => write!(f, "base_days must be finite and positive"),
            Self::SelfWar(c) => write!(f, "country {} cannot be at war with itself", c.0),
            Self::DuplicateWar(a, b) => write!(f, "the war between countries {} and {} is listed more than once", a.0, b.0),
            Self::DuplicateSupplyCountry(c) => write!(f, "country {} has more than one supply setup", c.0),
            Self::InvalidSupplyStockpile(c) => write!(f, "country {}: the military supplies stockpile is invalid", c.0),
            Self::InvalidSupplyUsage(d) => write!(f, "{d}: supply_usage is invalid"),
        }
    }
}

impl std::error::Error for MapMilitaryError {}

/// 命令の結果(受理)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderOutcome {
    /// 新しい移動を開始した(`hops` は経路のホップ数)。
    Started { hops: usize },
    /// 既に目的地にいる(Idle)。何も変更していない。
    AlreadyThere,
    /// 既に同じ目的地へ移動中。何も変更していない。
    AlreadyMoving,
}

/// 命令の拒否(正常な結果。拒否しても師団の状態は一切変わらない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderRejection {
    UnknownDivision(MapDivisionId),
    /// グラフのノードでない(水域・範囲外の陸のセルには ID が無い)。
    UnknownProvince(ProvinceId),
    /// 戦闘に参加中の師団(M6-b)。現在地への命令を含め、すべて拒否する。
    DivisionInCombat(MapDivisionId),
    /// 目的地が所属国の支配でも、交戦国の支配でもない。`controller` が `Some` なら非交戦国の支配、`None` なら支配国なし。
    DestinationNotPassable { province: ProvinceId, controller: Option<CountryId> },
    /// 通れるプロヴィンス(自国・交戦国の支配地)だけでは経路が無い。
    NoPath,
}

impl std::fmt::Display for OrderRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownDivision(d) => write!(f, "unknown-division: {d} does not exist"),
            Self::UnknownProvince(p) => write!(f, "unknown-province: {p} is not a province of the graph"),
            Self::DivisionInCombat(d) => write!(f, "division-in-combat: {d} is fighting and takes no orders"),
            Self::DestinationNotPassable { province, controller: Some(c) } => {
                write!(f, "destination-not-passable: {province} is controlled by country {} (not the division's country, and not at war with it)", c.0)
            }
            Self::DestinationNotPassable { province, controller: None } => {
                write!(f, "destination-not-passable: {province} has no controller")
            }
            Self::NoPath => write!(f, "no-path: no route through provinces controlled by the division's country or a country at war with it"),
        }
    }
}

/// 停止の理由(進入前に止まる場合)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// 次の移動先が、自国でも交戦国でもない支配になった(または支配国なしになった)。
    NotPassable,
    /// 次の移動先が敵支配地で、既に別の国が攻撃側の戦闘が進行中(合流できない)。
    ContestedByOthers,
}

/// 戦闘の結末。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleOutcome {
    AttackerWon,
    DefenderWon,
}

/// 戦闘が終わった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    /// 防御側の参加者がいなくなった(撤退・撃破)。
    DefenderGone,
    /// 攻撃側の参加者がいなくなった(撤退・撃破)。
    AttackerGone,
    /// 両陣営とも参加者がいなくなった(旧: 防御側勝利、支配は変わらない)。
    BothGone,
    /// その日、どちらの陣営も人員損失が 0(旧 P27-007: 防御側勝利、攻撃側全員を撤退)。
    ZeroDamage,
}

/// 実効支配の更新要求の理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimReason {
    /// 守備隊のいない敵支配地に進入した。
    Undefended,
    /// 戦闘で攻撃側が勝った。
    AttackerWonBattle(MapBattleId),
}

/// 日次処理で起きたこと(表示・テスト・M6-d への通知用)。実効支配の更新は `ControlClaim` → `ControlChanged` / `ClaimNotApplied`(`occupation.rs`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveEvent {
    /// 1ホップを終えて `province` に着いた(最終目的地ではない場合を含む)。
    HopCompleted { division: MapDivisionId, province: ProvinceId },
    /// 最終目的地(または経路の切れた所)に着いて Idle になった。
    Arrived { division: MapDivisionId, province: ProvinceId },
    /// 次の移動先 `blocked` が通れなくなったので、`at` で進入前に停止した(旧経路の互換: 理由が通行不可の場合)。
    Stopped { division: MapDivisionId, at: ProvinceId, blocked: ProvinceId },
    /// 進入前の停止で、理由が `NotPassable` 以外(`ContestedByOthers`)。
    StoppedBefore { division: MapDivisionId, at: ProvinceId, blocked: ProvinceId, reason: StopReason },
    /// 残りの経路に通れない所ができたので、経路を切った(今のホップだけ続ける)。
    PathTruncated { division: MapDivisionId },
    /// 敵支配地へ進入して戦闘を開始した。
    BattleStarted { battle: MapBattleId, province: ProvinceId, attacker: CountryId, defender: CountryId, attackers: Vec<MapDivisionId>, defenders: Vec<MapDivisionId> },
    /// 進行中の戦闘へ攻撃側として合流した。
    BattleJoined { battle: MapBattleId, division: MapDivisionId, province: ProvinceId },
    /// 戦闘から脱落した師団が `from` から `to` へ撤退した(追加の人員損失・装備損失を適用済み。即座に完了する)。
    Retreated { division: MapDivisionId, battle: MapBattleId, from: ProvinceId, to: ProvinceId },
    /// 戦闘から脱落した師団が撃破された(人員が尽きた、または退路がない)。
    Destroyed { division: MapDivisionId, battle: MapBattleId, at: ProvinceId },
    /// 戦闘が終わった。
    BattleFinished { battle: MapBattleId, province: ProvinceId, attacker: CountryId, defender: CountryId, outcome: BattleOutcome, reason: FinishReason },
    /// 実効支配の更新要求。守備隊のいない進入、または戦闘での攻撃側の勝利で、1回だけ発行され、**その場で適用される**(直後に `ControlChanged` または `ClaimNotApplied`)。
    ControlClaim { province: ProvinceId, claimant: CountryId, previous_controller: Option<CountryId>, reason: ClaimReason, divisions: Vec<MapDivisionId> },
    /// 実効支配が実際に変わった(対象プロヴィンスだけ)。M6-d が `previous`・`new` の到達結果を無効化できる(`countries_to_invalidate`)。
    ControlChanged { province: ProvinceId, previous: Option<CountryId>, new: CountryId, reason: ClaimReason, divisions: Vec<MapDivisionId> },
    /// 更新要求を適用しなかった(支配は上書きしない。見送りは翌日に自動で再適用しない)。
    ClaimNotApplied { province: ProvinceId, claimant: CountryId, reason: ClaimReason, rejection: super::occupation::ClaimRejection },
    /// 国家1か国分のその日の日次補給の結果(M6-d。その日の移動・戦闘・占領より前に確定した値)。
    DailySupply(SupplyReport),
    /// 日次補給の計画または適用に失敗した(備蓄と師団の充足率は変更しない。他国には影響しない)。
    SupplyFailed { country: CountryId, reason: SupplyFailure },
}

/// 日次補給が失敗した理由(旧の `DailySupplyPlanError` / `DailySupplyApplyError` を、`Eq` を保てる形に分類したもの)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyFailure {
    /// 師団の需要を求められない(不正な戦闘能力・補給消費量)。
    Demand,
    /// 国家の備蓄が不正で、正規の単位へ変換できない。
    InvalidStockpile,
    /// 単位の演算が溢れた。
    Overflow,
    /// 備蓄の消費の確定に失敗した。
    ApplyFailed,
}

/// 国家1か国分のその日の日次補給の要約(整数だけ。備蓄は旧と同じ固定小数点の単位)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyReport {
    pub country: CountryId,
    /// 首都が自国の支配下にあるか(偽なら到達集合は空)。
    pub capital_controlled: bool,
    /// 首都から確認済みの到達(`Reachable`)の件数。「不明」は含まない。
    pub connected_provinces: usize,
    /// 「不明」(範囲外の陸を通る経路があり得る)の件数。確認済みの到達には含めない。
    pub unknown_provinces: usize,
    /// その日に到達結果を計算し直したか(偽なら、キャッシュした国家ごとの `Reach` を再利用した)。
    pub reach_recomputed: bool,
    pub total_demand_units: u128,
    pub eligible_demand_units: u128,
    pub consumed_units: u128,
    /// 消費後の備蓄。
    pub stockpile_units: u64,
    /// 前線補給の例外で資格を得た師団(昇順)。
    pub frontline_divisions: Vec<MapDivisionId>,
}

/// 新経路の師団・支配・交戦関係・戦闘の集合(実行時状態)。
#[derive(Debug, Clone, PartialEq)]
pub struct MapMilitary {
    pub(super) divisions: BTreeMap<MapDivisionId, MapDivision>,
    pub(super) control: ProvinceControl,
    pub(super) params: MovementParams,
    /// 交戦する国の組(双方向。`(小さい CountryId の数値, 大きい数値)` に正規化して保持)。
    pub(super) wars: Vec<(usize, usize)>,
    pub(super) battles: BTreeMap<MapBattleId, MapBattle>,
    pub(super) next_battle: u32,
    /// 日次補給(M6-d。設定がなければ `None`で、従来と同じ挙動)。
    pub(super) supply: Option<super::supply::SupplyRuntime>,
}

/// 通れるプロヴィンスだけを通る最短経路(費用 1、同費用は `ProvinceId` 昇順)。終点は `start` を含まず `end` を含む。`start == end` は空の経路。
/// 経路が無ければ `None`。`passable` は中継点・終点の通行可否(始点は問わない)。
///
/// 同費用の tie-break: 各階層の候補を `ProvinceId` 昇順に展開し、最初に見つけた親を採る(旧 `find_path` の `(費用, ID)` 順のヒープと同じ結果)。
pub fn find_path_where(graph: &ProvinceGraph, start: ProvinceId, end: ProvinceId, passable: &dyn Fn(ProvinceId) -> bool) -> Option<Vec<ProvinceId>> {
    if start == end {
        return Some(Vec::new());
    }
    let mut pred: HashMap<ProvinceId, ProvinceId> = HashMap::new();
    let mut frontier = vec![start];
    while !frontier.is_empty() {
        let mut next: Vec<ProvinceId> = Vec::new();
        for &x in &frontier {
            for n in graph.neighbor_ids(x) {
                if n == start || pred.contains_key(&n) || !passable(n) {
                    continue;
                }
                pred.insert(n, x);
                if n == end {
                    let mut path = vec![end];
                    let mut cur = end;
                    while let Some(&p) = pred.get(&cur) {
                        if p == start {
                            break;
                        }
                        path.push(p);
                        cur = p;
                    }
                    path.reverse();
                    return Some(path);
                }
                next.push(n);
            }
        }
        next.sort_unstable();
        frontier = next;
    }
    None
}

/// 所属国が支配するプロヴィンスだけを通る最短経路(M6-a の形。交戦関係なし)。
pub fn find_land_path(graph: &ProvinceGraph, control: &ProvinceControl, owner: CountryId, start: ProvinceId, end: ProvinceId) -> Option<Vec<ProvinceId>> {
    find_path_where(graph, start, end, &|n| control.controller(n) == Some(owner))
}

impl MapMilitary {
    pub fn new(divisions: Vec<MapDivision>, control: ProvinceControl, params: MovementParams) -> Result<MapMilitary, MapMilitaryError> {
        if !(params.base_days.is_finite() && params.base_days > 0.0) {
            return Err(MapMilitaryError::InvalidBaseDays);
        }
        let mut map = BTreeMap::new();
        for d in divisions {
            if !(d.speed.is_finite() && d.speed > 0.0) {
                return Err(MapMilitaryError::InvalidSpeed(d.id));
            }
            if !d.combat.is_valid() {
                return Err(MapMilitaryError::InvalidCombat(d.id));
            }
            let id = d.id;
            if map.insert(id, d).is_some() {
                return Err(MapMilitaryError::DuplicateDivision(id));
            }
        }
        Ok(MapMilitary { divisions: map, control, params, wars: Vec::new(), battles: BTreeMap::new(), next_battle: 0, supply: None })
    }

    /// 交戦関係を設定する(検証用シナリオが明示する組。双方向)。自国との組・重複(逆順を含む)は `Err`。
    pub fn with_wars(mut self, wars: Vec<(CountryId, CountryId)>) -> Result<MapMilitary, MapMilitaryError> {
        let mut out: Vec<(usize, usize)> = Vec::new();
        for (a, b) in wars {
            if a == b {
                return Err(MapMilitaryError::SelfWar(a));
            }
            let key = (a.0.min(b.0), a.0.max(b.0));
            if out.contains(&key) {
                return Err(MapMilitaryError::DuplicateWar(a, b));
            }
            out.push(key);
        }
        out.sort_unstable();
        self.wars = out;
        Ok(self)
    }

    /// 2国が交戦中か(双方向。同一国は常に false)。
    pub fn at_war(&self, a: CountryId, b: CountryId) -> bool {
        a != b && self.wars.contains(&(a.0.min(b.0), a.0.max(b.0)))
    }

    pub fn params(&self) -> MovementParams {
        self.params
    }

    pub fn control(&self) -> &ProvinceControl {
        &self.control
    }

    /// 支配国を設定する(命令後の通行条件の変更の検証用。**占領の更新は M6-c**。ここから呼ばれて戦闘・進行中の経路が自動で直ることはない)。
    pub fn set_controller(&mut self, p: ProvinceId, c: Option<CountryId>) {
        let previous = self.control.controller(p);
        self.control.set(p, c);
        // 支配が変わると、変更前と変更後の支配国の到達結果(キャッシュした `Reach`)は古くなる(M6-d)。
        let affected: Vec<CountryId> = previous.into_iter().chain(c).collect();
        self.invalidate_reach(&affected);
    }

    pub fn division(&self, id: MapDivisionId) -> Option<&MapDivision> {
        self.divisions.get(&id)
    }

    /// 師団(`MapDivisionId` 昇順)。
    pub fn divisions(&self) -> impl Iterator<Item = &MapDivision> + '_ {
        self.divisions.values()
    }

    /// `owner` の師団が、`p` を通れるか(自国の支配地、または交戦国の支配地)。
    pub(super) fn passable_for(&self, owner: CountryId, p: ProvinceId) -> bool {
        match self.control.controller(p) {
            Some(c) => c == owner || self.at_war(owner, c),
            None => false,
        }
    }

    /// 移動命令(旧 `start_or_maintain_movement` の新経路版)。受理されたときだけ状態を変える。
    pub fn issue_move_order(&mut self, graph: &ProvinceGraph, id: MapDivisionId, destination: ProvinceId) -> Result<OrderOutcome, OrderRejection> {
        let Some(d) = self.divisions.get(&id) else {
            return Err(OrderRejection::UnknownDivision(id));
        };
        if !graph.contains(destination) {
            return Err(OrderRejection::UnknownProvince(destination));
        }
        // 戦闘中は、現在地への命令も含めてすべて拒否する(旧経路は呼び出し側 UI/AI がこの確認を持つ。新経路は API に持たせる)。
        if d.status == MapDivisionStatus::Fighting {
            return Err(OrderRejection::DivisionInCombat(id));
        }
        // 旧の冪等契約: 移動中は「同じ目的地」だけで判定する(現在地ではなく)。Idle は現在地で判定する。
        if d.status == MapDivisionStatus::Moving {
            if d.destination == Some(destination) {
                return Ok(OrderOutcome::AlreadyMoving);
            }
        } else if d.province == destination {
            return Ok(OrderOutcome::AlreadyThere);
        }
        let owner = d.owner;
        if !self.passable_for(owner, destination) {
            return Err(OrderRejection::DestinationNotPassable { province: destination, controller: self.control.controller(destination) });
        }
        let start = d.province;
        let Some(path) = find_path_where(graph, start, destination, &|n| self.passable_for(owner, n)) else {
            return Err(OrderRejection::NoPath);
        };
        let hops = path.len();
        let d = self.divisions.get_mut(&id).expect("checked");
        d.destination = Some(destination);
        d.path = path;
        d.target = None;
        d.progress = 0.0;
        d.status = MapDivisionStatus::Moving;
        Ok(OrderOutcome::Started { hops })
    }

    /// 1日分の処理。順序(旧 `handle_daily_military` の経路検証〜組織率回復に対応。**安定した ID 昇順**):
    /// 1. 経路の事前検証(`MapDivisionId` 昇順)→ 2. 移動と接触(`MapDivisionId` 昇順。敵支配地への進入で戦闘の開始・合流・守備隊なし)→
    /// 3. 戦闘の進行(`MapBattleId` 昇順。旧 `process_daily_battles`)→ 4. 終了処理(`MapBattleId` 昇順。旧 `resolve_finished_battles`。旧は HashMap 順)→
    /// 5. 終了した戦闘の除去 → 6. 組織率の回復(`MapDivisionId` 昇順)。
    ///
    /// 占領(`ControlClaim` の適用)は、守備隊のいない進入では接触の処理の中、戦闘の勝利では終了処理の中で、その場で行う(当日の組織率回復より前)。`graph` は退路の探索に使う。
    pub fn advance_day(&mut self, graph: &ProvinceGraph) -> Vec<MoveEvent> {
        let mut events = Vec::new();
        // 0. 日次補給(M6-d): その日の移動・戦闘・占領より前に、前日終値の状態(師団の位置・戦闘状態・支配)で1回だけ計算し、備蓄の消費と充足率を一緒に更新する。
        self.plan_daily_supply(graph, &mut events);
        let ids: Vec<MapDivisionId> = self.divisions.keys().copied().collect();
        // 1. 経路の事前検証(旧 `validate_and_stop_invalid_movements`)
        for id in &ids {
            let (status, target, owner) = {
                let d = &self.divisions[id];
                (d.status, d.target, d.owner)
            };
            if status != MapDivisionStatus::Moving {
                continue;
            }
            if let Some(t) = target
                && !self.passable_for(owner, t)
            {
                let d = self.divisions.get_mut(id).expect("id from keys");
                stop(d, t, &mut events);
                continue;
            }
            let bad = self.divisions[id].path.iter().any(|p| !self.passable_for(owner, *p));
            if bad {
                let d = self.divisions.get_mut(id).expect("id from keys");
                d.path.clear();
                d.destination = None;
                events.push(MoveEvent::PathTruncated { division: d.id });
            }
        }
        // 2. 移動と接触(旧 `process_movement` + `process_division_arrival`)
        for id in &ids {
            self.move_one(*id, &mut events);
        }
        // 3.〜5. 戦闘(battle.rs)
        let zero = self.run_daily_battles();
        self.resolve_battles(graph, &zero, &mut events);
        self.cleanup_finished_battles();
        // 6. 組織率の回復
        self.recover_organizations();
        // 当日の占領で支配が変わった国家(変更前と変更後)の到達結果を無効化する。再計算は、次に補給を計画する時点で行う(M6-d)。
        let targets = super::occupation::countries_to_invalidate(&events);
        self.invalidate_reach(&targets);
        events
    }

    fn move_one(&mut self, id: MapDivisionId, events: &mut Vec<MoveEvent>) {
        let params = self.params;
        let (owner, speed) = {
            let d = &self.divisions[&id];
            if d.status != MapDivisionStatus::Moving {
                return;
            }
            (d.owner, d.speed)
        };
        {
            let d = self.divisions.get_mut(&id).expect("exists");
            if d.target.is_none() {
                if !d.path.is_empty() {
                    d.target = Some(d.path.remove(0));
                    d.progress = 0.0;
                } else {
                    d.status = MapDivisionStatus::Idle;
                    d.destination = None;
                    d.progress = 0.0;
                    return;
                }
            }
        }
        let target = self.divisions[&id].target.expect("set above");
        if !self.passable_for(owner, target) {
            let d = self.divisions.get_mut(&id).expect("exists");
            stop(d, target, events);
            return;
        }
        // 補給係数 = max(その日の補給充足率, 0.5)(旧の `max(supply_ratio, 0.5)` と同じ下限。補給の設定がない師団は充足率 1.0 なので M6-a〜c と同じ)。
        let supply_mod = (self.divisions[&id].fulfillment as f32).max(MIN_SUPPLY_MODIFIER);
        let movement_days = (params.base_days * STEP_COST / (speed * supply_mod)).max(1.0);
        let new_progress = self.divisions[&id].progress + 1.0 / movement_days;
        if new_progress < 1.0 {
            self.divisions.get_mut(&id).expect("exists").progress = new_progress;
            return;
        }
        // 到着。敵支配地で、別の国が攻撃側の戦闘が進行中なら、進入前に停止する(合流できない)。
        let hostile = self.control.controller(target).is_some_and(|c| c != owner);
        if hostile && self.ongoing_battle_at(target).is_some_and(|b| b.attacker_country != owner) {
            let d = self.divisions.get_mut(&id).expect("exists");
            let at = d.province;
            d.status = MapDivisionStatus::Idle;
            d.target = None;
            d.destination = None;
            d.path.clear();
            d.progress = 0.0;
            events.push(MoveEvent::StoppedBefore { division: id, at, blocked: target, reason: StopReason::ContestedByOthers });
            return;
        }
        // 出撃元 = このホップの出発地(到着前の現在地)。攻撃側が敗れたときの退路の第一候補。
        let from = {
            let d = self.divisions.get_mut(&id).expect("exists");
            let from = d.province;
            d.province = target;
            d.progress = 0.0;
            events.push(MoveEvent::HopCompleted { division: id, province: target });
            from
        };
        if hostile {
            // 敵支配地: 残りの経路と目的地を破棄して止まり(M6-b 単体では占領しない)、接触を処理する。
            self.contact(id, from, events);
        } else {
            let d = self.divisions.get_mut(&id).expect("exists");
            if !d.path.is_empty() {
                d.target = Some(d.path.remove(0));
            } else {
                d.target = None;
                d.destination = None;
                d.status = MapDivisionStatus::Idle;
                events.push(MoveEvent::Arrived { division: id, province: target });
            }
        }
    }
}

fn stop(d: &mut MapDivision, blocked: ProvinceId, events: &mut Vec<MoveEvent>) {
    d.status = MapDivisionStatus::Idle;
    d.target = None;
    d.destination = None;
    d.path.clear();
    d.progress = 0.0;
    events.push(MoveEvent::Stopped { division: d.id, at: d.province, blocked });
}
