/// P21-SAVE-002C: `SaveGameV1`内部のID・参照整合性、および静的マスターデータ参照の検証。
///
/// このファイルは`bevy`を一切importしない(`dto.rs`/`export.rs`/`write.rs`と同じ方針)。
/// `SaveValidationContext`は起動時にRONから構築済みの静的マスターデータへの読み取り専用
/// 参照だけを束ねたプレーンなRust構造体であり、動的な国家・州・軍事状態を現在の
/// ランタイムResourceと比較する用途には使わない(動的データはセーブ自身を正とする)。
///
/// 検証に失敗したデータを部分的に適用してはいけない、という方針に基づき、
/// `validate_save_game_v1`は「1件でも問題があれば`SaveGameV1`全体を返さない」
/// (`Result<ValidatedSaveGameV1, SaveValidationErrors>`)。複数の問題は可能な限り
/// 1回の呼び出しでまとめて収集する(最初の1件で打ち切らない)。
///
/// HashMap/HashSetの走査順序はプロセスごとにハッシュシードが異なり得るため、
/// 同一の不正データへ毎回同じ順序で問題を報告できるよう、走査前に必ずキーで
/// ソートしてから処理する(`sorted_by`ヘルパー、および各所の`.sort()`/`.sort_by_key()`)。
use crate::app::time;
use crate::building::data::{BuildingDefinition, BuildingType};
use crate::common::{
    ArmyId, BattleId, ClaimId, CountryId, DiplomaticCrisisId, DivisionDefinitionId, DivisionId,
    FrontlineId, StateId, WarId,
};
use crate::country::CountryData;
use crate::diplomacy::data::DiplomaticPairKey;
use crate::military::battle::BattleStatus;
use crate::military::data::DivisionDefinition;
use crate::research::data::{TechnologyDefinition, TechnologyField, WorldStage};
use crate::research::world_stage::WorldStageDefinition;
use crate::save::dto::{SAVE_FORMAT_VERSION_V1, SaveGameV1};
use std::collections::{HashMap, HashSet};

/// 静的マスターデータ(起動時にRONから構築され、以降変化しない)への読み取り専用参照束。
///
/// 動的な国家・州・軍事・外交状態はここに含めない(セーブ自身が正であり、現在の
/// ランタイムResourceと比較しない)。`Entity`は含まない。各フィールドは既に`pub`な
/// 既存Resourceのフィールドをそのまま借用するだけであり、この検証のために
/// 新しいpublicアクセサをコアモジュールへ追加する必要はなかった。
pub struct SaveValidationContext<'a> {
    pub building_definitions: &'a HashMap<BuildingType, BuildingDefinition>,
    pub technology_definitions: &'a HashMap<String, TechnologyDefinition>,
    pub division_definitions: &'a HashMap<DivisionDefinitionId, DivisionDefinition>,
    pub world_stage_definitions: &'a HashMap<WorldStage, WorldStageDefinition>,
}

/// 参照整合性検証を通過した`SaveGameV1`。
///
/// フィールドは非公開で、`validate_save_game_v1`を通過した場合だけ生成できる。
/// 任意の`SaveGameV1`から直接構築する経路は存在しない。`DerefMut`や可変参照は公開しない。
/// タスクD(検証済みDTOのResource適用)がこの型を消費できるよう`pub(crate) into_inner`を
/// 提供するが、それ以外の書き込み経路は用意しない。
#[derive(Debug)]
pub struct ValidatedSaveGameV1 {
    save: SaveGameV1,
}

impl ValidatedSaveGameV1 {
    /// 検証済みデータへの読み取り専用アクセス。
    pub fn save(&self) -> &SaveGameV1 {
        &self.save
    }

    /// タスクD以降がResourceへ適用するために内部の`SaveGameV1`を取り出す。
    /// `pub(crate)`に限定し、crate外や本ラウンドの他コードから未検証のまま
    /// 取り出せないようにする。
    #[allow(dead_code)]
    pub(crate) fn into_inner(self) -> SaveGameV1 {
        self.save
    }
}

/// 検証で見つかった問題を分類するコード。
///
/// UIが将来これをローカライズできるよう、技術詳細だけのString1種類には潰さない。
/// 同種の問題はカテゴリ単位でコードを共有し、具体的な箇所・詳細は`path`/`detail`で表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveValidationCode {
    /// Vec内で同じ主キー(CountryId/StateId等)が重複している。
    DuplicateId,
    /// HashMapのキーと要素内部が保持する自己IDが一致しない。
    MapKeyMismatch,
    /// 必須の値が欠落している(例: `player_country`がNone)。
    MissingValue,
    /// IDが存在しない対象を参照している(ダングリング参照、静的マスター参照を含む)。
    DanglingReference,
    /// 数値が既存ロジックの許容範囲外(日付・速度・レベル等)。
    InvalidRange,
    /// 既存ロジックが有限値を前提とする箇所でNaN/Infinityが入っている。
    NonFiniteValue,
    /// 州の隣接関係が双方向でない。
    AsymmetricAdjacency,
    /// 州が自分自身を隣接として持つ。
    SelfAdjacency,
    /// 同じ隣接州IDが重複して列挙されている。
    DuplicateAdjacency,
    /// 次回発行IDが既存の最大IDと衝突し得る。
    NextIdCollision,
    /// 所有者・国籍の不一致(例: 首都州のowner、Army所属師団のowner)。
    OwnershipMismatch,
    /// 前線/戦争の参加国と、命令主体・攻撃側防御側の対応が一致しない。
    ParticipantMismatch,
    /// 双方向マッピング(division_army_map等)がどちらか一方からしか成立しない。
    ReverseMapInconsistent,
    /// 空であってはならないコレクションが空(例: 空Army)。
    EmptyCollection,
    /// 同じID(Division等)が複数の排他的な所属先に同時に含まれている。
    DuplicateMembership,
    /// 互いに素であるべき集合が重なっている(例: War攻撃側と防御側)。
    SetOverlap,
    /// 陸地であるべき箇所が海の州を参照している(例: 攻勢線の地点)。
    NonLandRegion,
    /// 隣接グラフ上で連結しているべき州の集合が非連結になっている(例: 攻勢線)。
    DisconnectedRegionSet,
}

/// 1件の検証問題。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveValidationIssue {
    pub code: SaveValidationCode,
    /// 問題箇所を特定するパス表現(例: `armies.armies[ArmyId(2)].member_division_ids[0]`)。
    pub path: String,
    /// 技術的な詳細説明(診断用、ローカライズ対象ではない)。
    pub detail: String,
}

/// 検証で見つかった問題の集合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveValidationErrors {
    pub issues: Vec<SaveValidationIssue>,
}

fn push(
    issues: &mut Vec<SaveValidationIssue>,
    code: SaveValidationCode,
    path: impl Into<String>,
    detail: impl Into<String>,
) {
    issues.push(SaveValidationIssue {
        code,
        path: path.into(),
        detail: detail.into(),
    });
}

/// セーブ内部のID集合(ダングリング参照検証を高速化・単純化するための事前計算)。
///
/// `ArmyId`は含めない: Army自体への参照(`division_army_map`の値等)は常に
/// `save.armies.armies.get(...)`で直接引き、その要素の有無自体を検証結果として扱う
/// (Army1件ごとに専用のOwnership/ReverseMap検証が必要なため、単純な存在集合だけでは
/// 検証ロジックを表現できない)。
struct RefIndex<'a> {
    countries: &'a HashSet<CountryId>,
    states: &'a HashSet<StateId>,
    wars: &'a HashSet<WarId>,
    divisions: &'a HashSet<DivisionId>,
    frontlines: &'a HashSet<FrontlineId>,
    battles: &'a HashSet<BattleId>,
}

fn technology_field_rank(field: TechnologyField) -> usize {
    TechnologyField::ALL
        .iter()
        .position(|f| *f == field)
        .unwrap_or(usize::MAX)
}

/// `SaveGameV1`の参照整合性・静的マスター参照・不変条件を検証する。
///
/// 1件でも問題があれば`SaveGameV1`全体を返さない(部分適用を防ぐ)。複数の問題は
/// 1回の呼び出しでまとめて収集する。検証専用であり、`save`/`context`を変更しない。
pub fn validate_save_game_v1(
    save: SaveGameV1,
    context: &SaveValidationContext,
) -> Result<ValidatedSaveGameV1, SaveValidationErrors> {
    let mut issues = Vec::new();

    if save.version != SAVE_FORMAT_VERSION_V1 {
        push(
            &mut issues,
            SaveValidationCode::InvalidRange,
            "version",
            format!(
                "unsupported save format version {} (expected {})",
                save.version, SAVE_FORMAT_VERSION_V1
            ),
        );
    }

    let country_ids: HashSet<CountryId> = save.countries.iter().map(|c| c.id).collect();
    let state_ids: HashSet<StateId> = save.states.iter().map(|s| s.id).collect();
    let war_ids: HashSet<WarId> = save.wars.wars.keys().copied().collect();
    let division_ids: HashSet<DivisionId> = save.military.divisions.keys().copied().collect();
    let frontline_ids: HashSet<FrontlineId> = save.frontlines.frontlines.keys().copied().collect();
    let battle_ids: HashSet<BattleId> = save.battles.battles.keys().copied().collect();

    let idx = RefIndex {
        countries: &country_ids,
        states: &state_ids,
        wars: &war_ids,
        divisions: &division_ids,
        frontlines: &frontline_ids,
        battles: &battle_ids,
    };

    validate_world_state(&save, context, &idx, &mut issues);
    validate_countries(&save, context, &idx, &mut issues);
    validate_states(&save, context, &idx, &mut issues);
    validate_registry_keys_and_next_ids(&save, &mut issues);
    validate_diplomacy(&save, &idx, &mut issues);
    validate_war_justifications(&save, &idx, &mut issues);
    validate_wars(&save, &idx, &mut issues);
    validate_claims_and_crises(&save, &idx, &mut issues);
    validate_divisions(&save, context, &idx, &mut issues);
    validate_battles(&save, &idx, &mut issues);
    validate_armies(&save, &idx, &mut issues);
    validate_ai(&save, &idx, &mut issues);
    validate_frontlines(&save, &idx, &mut issues);

    if issues.is_empty() {
        Ok(ValidatedSaveGameV1 { save })
    } else {
        Err(SaveValidationErrors { issues })
    }
}

// ─── 世界・基本状態 ──────────────────────────────────────────────────────────

fn validate_world_state(
    save: &SaveGameV1,
    context: &SaveValidationContext,
    idx: &RefIndex,
    issues: &mut Vec<SaveValidationIssue>,
) {
    if save.countries.is_empty() {
        push(
            issues,
            SaveValidationCode::EmptyCollection,
            "countries",
            "save must contain at least one country",
        );
    }
    if save.states.is_empty() {
        push(
            issues,
            SaveValidationCode::EmptyCollection,
            "states",
            "save must contain at least one state",
        );
    }

    match save.player_country {
        None => push(
            issues,
            SaveValidationCode::MissingValue,
            "player_country",
            "player_country must be Some for a Playing-state save",
        ),
        Some(pid) => {
            if !idx.countries.contains(&pid) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    "player_country",
                    format!("references unknown {pid:?}"),
                );
            }
        }
    }

    let date = &save.date;
    match time::GameDate::days_in_month(date.month) {
        None => push(
            issues,
            SaveValidationCode::InvalidRange,
            "date.month",
            format!("month {} is out of range 1..=12", date.month),
        ),
        Some(max_day) => {
            if date.day == 0 || date.day > max_day {
                push(
                    issues,
                    SaveValidationCode::InvalidRange,
                    "date.day",
                    format!(
                        "day {} is out of range 1..={max_day} for month {}",
                        date.day, date.month
                    ),
                );
            }
        }
    }
    if !date.accumulator.is_finite() || date.accumulator < 0.0 || date.accumulator >= 1.0 {
        push(
            issues,
            SaveValidationCode::InvalidRange,
            "date.accumulator",
            format!(
                "accumulator {} must be finite and within [0.0, 1.0)",
                date.accumulator
            ),
        );
    }

    if !(1..=4).contains(&save.game_speed) {
        push(
            issues,
            SaveValidationCode::InvalidRange,
            "game_speed",
            format!("game_speed {} is out of range 1..=4", save.game_speed),
        );
    }

    // `WorldStage::PreIndustrial`はゲーム開始時点の既定段階であり、他の段階と異なり
    // `assets/data/world_stages.ron`に定義エントリを持たない(それ自身へ「到達する」
    // という遷移が存在しないため)。よって`world_stage_definitions`に存在しなくても
    // 参照切れとして扱わない。
    if save.world_civilization.current_stage != WorldStage::PreIndustrial
        && !context
            .world_stage_definitions
            .contains_key(&save.world_civilization.current_stage)
    {
        push(
            issues,
            SaveValidationCode::DanglingReference,
            "world_civilization.current_stage",
            format!(
                "{:?} is not a defined WorldStage",
                save.world_civilization.current_stage
            ),
        );
    }

    let mut stage_entries: Vec<(&WorldStage, &HashSet<usize>)> =
        save.world_civilization.milestone_countries.iter().collect();
    stage_entries.sort_by_key(|(stage, _)| **stage);
    for (stage, countries) in stage_entries {
        let path = format!("world_civilization.milestone_countries[{stage:?}]");
        if !context.world_stage_definitions.contains_key(stage) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                path.clone(),
                format!("{stage:?} is not a defined WorldStage"),
            );
        }
        let mut raw_ids: Vec<usize> = countries.iter().copied().collect();
        raw_ids.sort_unstable();
        for raw_id in raw_ids {
            if !idx.countries.contains(&CountryId(raw_id)) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    path.clone(),
                    format!("references unknown CountryId({raw_id})"),
                );
            }
        }
    }
}

// ─── 国家 ────────────────────────────────────────────────────────────────────

fn validate_countries(
    save: &SaveGameV1,
    context: &SaveValidationContext,
    idx: &RefIndex,
    issues: &mut Vec<SaveValidationIssue>,
) {
    let mut seen: HashSet<CountryId> = HashSet::new();
    for (i, c) in save.countries.iter().enumerate() {
        let base = format!("countries[{i}]");
        if !seen.insert(c.id) {
            push(
                issues,
                SaveValidationCode::DuplicateId,
                format!("{base}.id"),
                format!("duplicate {:?}", c.id),
            );
        }

        validate_capital(save, c, &base, idx, issues);

        if !c.treasury.is_finite() {
            push(
                issues,
                SaveValidationCode::NonFiniteValue,
                format!("{base}.treasury"),
                "treasury must be finite",
            );
        }

        let mut stockpile: Vec<(&crate::economy::resources::ResourceType, &f64)> =
            c.stockpile.amounts.iter().collect();
        stockpile.sort_by_key(|(res, _)| format!("{res:?}"));
        for (res, &amount) in stockpile {
            if !amount.is_finite() {
                push(
                    issues,
                    SaveValidationCode::NonFiniteValue,
                    format!("{base}.stockpile.amounts[{res:?}]"),
                    format!("stockpile amount {amount} must be finite"),
                );
            } else if amount < 0.0 {
                push(
                    issues,
                    SaveValidationCode::InvalidRange,
                    format!("{base}.stockpile.amounts[{res:?}]"),
                    format!("stockpile amount {amount} must not be negative"),
                );
            }
        }

        for (qi, item) in c.construction_queue.iter().enumerate() {
            let qpath = format!("{base}.construction_queue[{qi}]");
            if !idx.states.contains(&item.state_id) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{qpath}.state_id"),
                    format!("references unknown {:?}", item.state_id),
                );
            }
            match context.building_definitions.get(&item.building_type) {
                None => push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{qpath}.building_type"),
                    format!("{:?} is not a defined BuildingType", item.building_type),
                ),
                Some(def) => {
                    if item.target_level > def.max_level {
                        push(
                            issues,
                            SaveValidationCode::InvalidRange,
                            format!("{qpath}.target_level"),
                            format!(
                                "target_level {} exceeds max_level {}",
                                item.target_level, def.max_level
                            ),
                        );
                    }
                }
            }
        }

        for (qi, item) in c.recruitment_queue.iter().enumerate() {
            let qpath = format!("{base}.recruitment_queue[{qi}]");
            if !context.division_definitions.contains_key(&item.division_id) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{qpath}.division_id"),
                    format!(
                        "{:?} is not a defined DivisionDefinitionId",
                        item.division_id
                    ),
                );
            }
            if !idx.states.contains(&item.target_state) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{qpath}.target_state"),
                    format!("references unknown {:?}", item.target_state),
                );
            }
        }

        let mut completed: Vec<&String> = c.research_state.completed_technologies.iter().collect();
        completed.sort();
        for tech_id in completed {
            if !context.technology_definitions.contains_key(tech_id) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{base}.research_state.completed_technologies"),
                    format!("references unknown technology id '{tech_id}'"),
                );
            }
        }

        let mut in_progress: Vec<(
            &TechnologyField,
            &crate::research::allocation::InProgressTech,
        )> = c.research_state.in_progress.iter().collect();
        in_progress.sort_by_key(|(field, _)| technology_field_rank(**field));
        for (field, tech) in in_progress {
            if !context.technology_definitions.contains_key(&tech.tech_id) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{base}.research_state.in_progress[{field:?}]"),
                    format!("references unknown technology id '{}'", tech.tech_id),
                );
            }
        }
    }
}

fn validate_capital(
    save: &SaveGameV1,
    c: &CountryData,
    base: &str,
    idx: &RefIndex,
    issues: &mut Vec<SaveValidationIssue>,
) {
    if !idx.states.contains(&c.capital_state_id) {
        push(
            issues,
            SaveValidationCode::DanglingReference,
            format!("{base}.capital_state_id"),
            format!("references unknown {:?}", c.capital_state_id),
        );
        return;
    }
    if let Some(capital) = save.states.iter().find(|s| s.id == c.capital_state_id)
        && capital.owner_country_id != c.id
    {
        push(
            issues,
            SaveValidationCode::OwnershipMismatch,
            format!("{base}.capital_state_id"),
            format!(
                "capital {:?} is owned by {:?}, not {:?}",
                c.capital_state_id, capital.owner_country_id, c.id
            ),
        );
    }
}

// ─── 州 ──────────────────────────────────────────────────────────────────────

fn validate_states(
    save: &SaveGameV1,
    context: &SaveValidationContext,
    idx: &RefIndex,
    issues: &mut Vec<SaveValidationIssue>,
) {
    let mut seen: HashSet<StateId> = HashSet::new();
    for (i, s) in save.states.iter().enumerate() {
        let base = format!("states[{i}]");
        if !seen.insert(s.id) {
            push(
                issues,
                SaveValidationCode::DuplicateId,
                format!("{base}.id"),
                format!("duplicate {:?}", s.id),
            );
        }

        if !idx.countries.contains(&s.owner_country_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{base}.owner_country_id"),
                format!("references unknown {:?}", s.owner_country_id),
            );
        }
        if let Some(controller) = s.controller_country
            && !idx.countries.contains(&controller)
        {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{base}.controller_country"),
                format!("references unknown {controller:?}"),
            );
        }
        if let Some(original_owner) = s.original_owner
            && !idx.countries.contains(&original_owner)
        {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{base}.original_owner"),
                format!("references unknown {original_owner:?}"),
            );
        }
        if let Some(war_id) = s.war_id
            && !idx.wars.contains(&war_id)
        {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{base}.war_id"),
                format!("references unknown {war_id:?}"),
            );
        }

        let mut buildings: Vec<(&BuildingType, &u32)> = s.buildings.iter().collect();
        buildings.sort_by_key(|(bt, _)| format!("{bt:?}"));
        for (b_type, &level) in buildings {
            match context.building_definitions.get(b_type) {
                None => push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{base}.buildings[{b_type:?}]"),
                    format!("{b_type:?} is not a defined BuildingType"),
                ),
                Some(def) => {
                    if level > def.max_level {
                        push(
                            issues,
                            SaveValidationCode::InvalidRange,
                            format!("{base}.buildings[{b_type:?}]"),
                            format!("level {level} exceeds max_level {}", def.max_level),
                        );
                    }
                }
            }
        }

        let mut seen_neighbors: HashSet<StateId> = HashSet::new();
        for (ni, &neighbor_id) in s.neighbors.iter().enumerate() {
            let npath = format!("{base}.neighbors[{ni}]");
            if neighbor_id == s.id {
                push(
                    issues,
                    SaveValidationCode::SelfAdjacency,
                    npath.clone(),
                    format!("{:?} lists itself as a neighbor", s.id),
                );
            }
            if !seen_neighbors.insert(neighbor_id) {
                push(
                    issues,
                    SaveValidationCode::DuplicateAdjacency,
                    npath.clone(),
                    format!("{neighbor_id:?} is listed more than once"),
                );
            }
            match save.states.iter().find(|other| other.id == neighbor_id) {
                None => push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    npath,
                    format!("references unknown {neighbor_id:?}"),
                ),
                Some(neighbor) => {
                    if !neighbor.neighbors.contains(&s.id) {
                        push(
                            issues,
                            SaveValidationCode::AsymmetricAdjacency,
                            npath,
                            format!(
                                "adjacency between {:?} and {:?} is not bidirectional",
                                s.id, neighbor_id
                            ),
                        );
                    }
                }
            }
        }

        if !s.occupation_progress.is_finite() {
            push(
                issues,
                SaveValidationCode::NonFiniteValue,
                format!("{base}.occupation_progress"),
                "occupation_progress must be finite",
            );
        }
        if !s.living_standard.is_finite() || !s.unrest.is_finite() || !s.logistics_ratio.is_finite()
        {
            push(
                issues,
                SaveValidationCode::NonFiniteValue,
                base.clone(),
                "living_standard/unrest/logistics_ratio must be finite",
            );
        }
    }
}

// ─── Registryのキー・次回ID ───────────────────────────────────────────────────

fn validate_registry_keys_and_next_ids(save: &SaveGameV1, issues: &mut Vec<SaveValidationIssue>) {
    // WarJustificationRegistry: key(usize) == justification.id
    let mut max_justification_id: Option<usize> = None;
    let mut entries: Vec<(&usize, &crate::war::justification::WarJustification)> =
        save.war_justifications.justifications.iter().collect();
    entries.sort_by_key(|(k, _)| **k);
    for (&key, j) in entries {
        if j.id != key {
            push(
                issues,
                SaveValidationCode::MapKeyMismatch,
                format!("war_justifications.justifications[{key}]"),
                format!("map key {key} does not match element id {}", j.id),
            );
        }
        max_justification_id = Some(max_justification_id.map_or(j.id, |m| m.max(j.id)));
    }
    if let Some(max_id) = max_justification_id
        && save.war_justifications.next_id <= max_id
    {
        push(
            issues,
            SaveValidationCode::NextIdCollision,
            "war_justifications.next_id",
            format!(
                "next_id {} must be greater than max existing id {max_id}",
                save.war_justifications.next_id
            ),
        );
    }

    check_id_registry(
        "wars.wars",
        &save.wars.wars,
        |k: &WarId| k.0,
        |w| w.id.0,
        save.wars.next_id,
        issues,
    );
    check_id_registry(
        "claims.claims",
        &save.claims.claims,
        |k: &ClaimId| k.0,
        |c| c.id.0,
        save.claims.next_id,
        issues,
    );
    check_id_registry(
        "crises.crises",
        &save.crises.crises,
        |k: &DiplomaticCrisisId| k.0,
        |c| c.id.0,
        save.crises.next_id,
        issues,
    );
    check_id_registry(
        "military.divisions",
        &save.military.divisions,
        |k: &DivisionId| k.0,
        |d| d.id.0,
        save.military.next_division_id,
        issues,
    );
    check_id_registry(
        "battles.battles",
        &save.battles.battles,
        |k: &BattleId| k.0,
        |b| b.id.0,
        save.battles.next_id,
        issues,
    );
    check_id_registry(
        "armies.armies",
        &save.armies.armies,
        |k: &ArmyId| k.0,
        |a| a.id.0,
        save.armies.next_id,
        issues,
    );
    check_id_registry(
        "frontlines.frontlines",
        &save.frontlines.frontlines,
        |k: &FrontlineId| k.0,
        |f| f.frontline_id.0,
        save.frontlines.next_frontline_id,
        issues,
    );

    // CountryAiRegistry / MilitaryAiRegistry: map key == state.country_id
    let mut country_ai: Vec<(&CountryId, &crate::save::dto::SavedCountryAiState)> =
        save.country_ai.ai_states.iter().collect();
    country_ai.sort_by_key(|(k, _)| k.0);
    for (&key, state) in country_ai {
        if state.country_id != key {
            push(
                issues,
                SaveValidationCode::MapKeyMismatch,
                format!("country_ai.ai_states[{key:?}]"),
                format!(
                    "map key {key:?} does not match element country_id {:?}",
                    state.country_id
                ),
            );
        }
    }
    let mut military_ai: Vec<(&CountryId, &crate::save::dto::SavedMilitaryAiState)> =
        save.military_ai.ai_states.iter().collect();
    military_ai.sort_by_key(|(k, _)| k.0);
    for (&key, state) in military_ai {
        if state.country_id != key {
            push(
                issues,
                SaveValidationCode::MapKeyMismatch,
                format!("military_ai.ai_states[{key:?}]"),
                format!(
                    "map key {key:?} does not match element country_id {:?}",
                    state.country_id
                ),
            );
        }
    }

    // FrontlineRegistry.plans: tuple key == (plan.frontline_id, plan.commanding_country_id)
    let mut plans: Vec<(
        &(FrontlineId, CountryId),
        &crate::war::frontline::FrontlinePlan,
    )> = save.frontlines.plans.iter().collect();
    plans.sort_by_key(|(k, _)| (k.0.0, k.1.0));
    for (&(fl_key, c_key), plan) in plans {
        if plan.frontline_id != fl_key || plan.commanding_country_id != c_key {
            push(
                issues,
                SaveValidationCode::MapKeyMismatch,
                format!("frontlines.plans[({fl_key:?}, {c_key:?})]"),
                format!(
                    "map key does not match element ({:?}, {:?})",
                    plan.frontline_id, plan.commanding_country_id
                ),
            );
        }
    }
}

/// `HashMap<K, V>`のキーと要素内部の自己IDが一致すること、および付随する`next_id`が
/// 既存の最大IDと衝突しないことを検証する共通ヘルパー。`key_id`/`element_id`は
/// それぞれキー・要素からIDの数値表現(usize)を取り出す(ソート・比較を型に依存せず
/// 統一的に行うため)。
fn check_id_registry<K, V>(
    path_base: &str,
    map: &HashMap<K, V>,
    key_id: impl Fn(&K) -> usize,
    element_id: impl Fn(&V) -> usize,
    next_id: usize,
    issues: &mut Vec<SaveValidationIssue>,
) where
    K: std::fmt::Debug,
{
    let mut entries: Vec<(&K, &V)> = map.iter().collect();
    entries.sort_by_key(|(k, _)| key_id(k));

    let mut max_id: Option<usize> = None;
    for (key, value) in entries {
        let id = element_id(value);
        max_id = Some(max_id.map_or(id, |m: usize| m.max(id)));
        if key_id(key) != id {
            push(
                issues,
                SaveValidationCode::MapKeyMismatch,
                format!("{path_base}[{key:?}]"),
                format!("map key {key:?} does not match element id {id}"),
            );
        }
    }

    if let Some(max_id) = max_id
        && next_id <= max_id
    {
        push(
            issues,
            SaveValidationCode::NextIdCollision,
            format!("{path_base} next_id"),
            format!("next_id {next_id} must be greater than max existing id {max_id}"),
        );
    }
}

// ─── 外交 ────────────────────────────────────────────────────────────────────

fn validate_diplomacy(save: &SaveGameV1, idx: &RefIndex, issues: &mut Vec<SaveValidationIssue>) {
    let mut relations: Vec<(
        &DiplomaticPairKey,
        &crate::diplomacy::data::DiplomaticRelation,
    )> = save.diplomacy.relations.iter().collect();
    relations.sort_by_key(|(k, _)| (k.0.0, k.1.0));

    for (key, relation) in relations {
        let path = format!("diplomacy.relations[({:?}, {:?})]", key.0, key.1);
        let (a, b) = (key.0, key.1);

        if a == b {
            push(
                issues,
                SaveValidationCode::SetOverlap,
                path.clone(),
                format!("DiplomaticPairKey must reference two distinct countries, got {a:?} twice"),
            );
        } else if a.0 >= b.0 {
            push(
                issues,
                SaveValidationCode::InvalidRange,
                path.clone(),
                format!(
                    "DiplomaticPairKey must be normalized (first.0 < second.0), got ({a:?}, {b:?})"
                ),
            );
        }

        if !idx.countries.contains(&a) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                path.clone(),
                format!("references unknown {a:?}"),
            );
        }
        if !idx.countries.contains(&b) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                path.clone(),
                format!("references unknown {b:?}"),
            );
        }

        if !relation.opinion.is_finite()
            || !relation.tension.is_finite()
            || !relation.trust.is_finite()
            || !relation.threat.is_finite()
        {
            push(
                issues,
                SaveValidationCode::NonFiniteValue,
                path.clone(),
                "opinion/tension/trust/threat must be finite",
            );
        }

        let mut cooldowns: Vec<(&CountryId, &u32)> = relation.cooldowns.iter().collect();
        cooldowns.sort_by_key(|(k, _)| k.0);
        for (cid, _) in cooldowns {
            if !idx.countries.contains(cid) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{path}.cooldowns[{cid:?}]"),
                    format!("references unknown {cid:?}"),
                );
            }
        }

        for (ti, treaty) in relation.treaties.iter().enumerate() {
            let (ta, tb) = treaty.countries;
            if ta == tb || !((ta == a && tb == b) || (ta == b && tb == a)) {
                push(
                    issues,
                    SaveValidationCode::ParticipantMismatch,
                    format!("{path}.treaties[{ti}]"),
                    format!(
                        "treaty countries ({ta:?}, {tb:?}) must be exactly the relation's pair ({a:?}, {b:?})"
                    ),
                );
            }
        }

        if let Some(activity) = &relation.active_activity {
            let participants_ok = (activity.initiator == a && activity.target == b)
                || (activity.initiator == b && activity.target == a);
            if activity.initiator == activity.target || !participants_ok {
                push(
                    issues,
                    SaveValidationCode::ParticipantMismatch,
                    format!("{path}.active_activity"),
                    format!(
                        "activity initiator/target ({:?}, {:?}) must be exactly the relation's pair ({a:?}, {b:?})",
                        activity.initiator, activity.target
                    ),
                );
            }
        }
    }
}

// ─── 戦争正当化 ──────────────────────────────────────────────────────────────

fn validate_war_justifications(
    save: &SaveGameV1,
    idx: &RefIndex,
    issues: &mut Vec<SaveValidationIssue>,
) {
    let mut entries: Vec<(&usize, &crate::war::justification::WarJustification)> =
        save.war_justifications.justifications.iter().collect();
    entries.sort_by_key(|(k, _)| **k);

    for (id, j) in entries {
        let path = format!("war_justifications.justifications[{id}]");
        if j.initiator == j.target {
            push(
                issues,
                SaveValidationCode::SetOverlap,
                path.clone(),
                format!(
                    "initiator and target must differ, both are {:?}",
                    j.initiator
                ),
            );
        }
        if !idx.countries.contains(&j.initiator) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.initiator"),
                format!("references unknown {:?}", j.initiator),
            );
        }
        if !idx.countries.contains(&j.target) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.target"),
                format!("references unknown {:?}", j.target),
            );
        }
        if !idx.states.contains(&j.target_state) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.target_state"),
                format!("references unknown {:?}", j.target_state),
            );
        }
    }
}

// ─── 戦争 ────────────────────────────────────────────────────────────────────

fn validate_wars(save: &SaveGameV1, idx: &RefIndex, issues: &mut Vec<SaveValidationIssue>) {
    let mut wars: Vec<(&WarId, &crate::war::data::War)> = save.wars.wars.iter().collect();
    wars.sort_by_key(|(k, _)| k.0);

    for (war_id, war) in wars {
        let path = format!("wars.wars[{war_id:?}]");

        if war.attackers.is_empty() {
            push(
                issues,
                SaveValidationCode::EmptyCollection,
                format!("{path}.attackers"),
                "attackers must not be empty (existing frontline code unwraps the first attacker)",
            );
        }
        if war.defenders.is_empty() {
            push(
                issues,
                SaveValidationCode::EmptyCollection,
                format!("{path}.defenders"),
                "defenders must not be empty (existing frontline code unwraps the first defender)",
            );
        }
        let overlap: Vec<&CountryId> = war.attackers.intersection(&war.defenders).collect();
        if !overlap.is_empty() {
            push(
                issues,
                SaveValidationCode::SetOverlap,
                format!("{path}.attackers"),
                format!("attackers and defenders overlap: {overlap:?}"),
            );
        }

        let mut participants: Vec<CountryId> = war
            .attackers
            .iter()
            .chain(war.defenders.iter())
            .copied()
            .collect();
        participants.sort_by_key(|c| c.0);
        for cid in participants {
            if !idx.countries.contains(&cid) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{path} participants"),
                    format!("references unknown {cid:?}"),
                );
            }
        }

        let mut occupied: Vec<StateId> = war.occupied_states.iter().copied().collect();
        occupied.sort_by_key(|s| s.0);
        for sid in occupied {
            if !idx.states.contains(&sid) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{path}.occupied_states"),
                    format!("references unknown {sid:?}"),
                );
            }
        }

        if let Some(winner) = war.winner
            && !war.attackers.contains(&winner)
            && !war.defenders.contains(&winner)
        {
            push(
                issues,
                SaveValidationCode::ParticipantMismatch,
                format!("{path}.winner"),
                format!("winner {winner:?} must be one of the war's participants"),
            );
        }

        for (gi, goal) in war.war_goals.iter().enumerate() {
            let gpath = format!("{path}.war_goals[{gi}]");
            if !idx.countries.contains(&goal.attacker) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{gpath}.attacker"),
                    format!("references unknown {:?}", goal.attacker),
                );
            }
            if !idx.countries.contains(&goal.defender) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{gpath}.defender"),
                    format!("references unknown {:?}", goal.defender),
                );
            }
            for (si, sid) in goal.target_states.iter().enumerate() {
                if !idx.states.contains(sid) {
                    push(
                        issues,
                        SaveValidationCode::DanglingReference,
                        format!("{gpath}.target_states[{si}]"),
                        format!("references unknown {sid:?}"),
                    );
                }
            }
        }

        if !war.war_score.is_finite()
            || !war.attacker_war_exhaustion.is_finite()
            || !war.defender_war_exhaustion.is_finite()
        {
            push(
                issues,
                SaveValidationCode::NonFiniteValue,
                path,
                "war_score/attacker_war_exhaustion/defender_war_exhaustion must be finite",
            );
        }

        // `processed_battle_ids`は意図的に検証しない: `BattleRegistry::cleanup_finished_battles`
        // (`military::update`から日次で呼ばれる)は完了・中止した戦闘を`battles`から完全に
        // 削除するため、`processed_battle_ids`はその後も「既に集計済み」を示す恒久的な
        // 履歴集合として残る仕様である(存在しないBattleIdを指していても正常)。
    }
}

// ─── Claim / Crisis ──────────────────────────────────────────────────────────

fn validate_claims_and_crises(
    save: &SaveGameV1,
    idx: &RefIndex,
    issues: &mut Vec<SaveValidationIssue>,
) {
    let mut claims: Vec<(&ClaimId, &crate::diplomacy::claims::TerritorialClaim)> =
        save.claims.claims.iter().collect();
    claims.sort_by_key(|(k, _)| k.0);
    for (claim_id, claim) in claims {
        let path = format!("claims.claims[{claim_id:?}]");
        if !idx.countries.contains(&claim.claimant_country) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.claimant_country"),
                format!("references unknown {:?}", claim.claimant_country),
            );
        }
        if !idx.states.contains(&claim.target_state) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.target_state"),
                format!("references unknown {:?}", claim.target_state),
            );
        }
        if !claim.strength.is_finite() {
            push(
                issues,
                SaveValidationCode::NonFiniteValue,
                format!("{path}.strength"),
                "strength must be finite",
            );
        }
    }

    let mut crises: Vec<(
        &DiplomaticCrisisId,
        &crate::diplomacy::crisis::DiplomaticCrisis,
    )> = save.crises.crises.iter().collect();
    crises.sort_by_key(|(k, _)| k.0);
    for (crisis_id, crisis) in crises {
        let path = format!("crises.crises[{crisis_id:?}]");
        if crisis.initiator == crisis.target {
            push(
                issues,
                SaveValidationCode::SetOverlap,
                path.clone(),
                format!(
                    "initiator and target must differ, both are {:?}",
                    crisis.initiator
                ),
            );
        }
        if !idx.countries.contains(&crisis.initiator) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.initiator"),
                format!("references unknown {:?}", crisis.initiator),
            );
        }
        if !idx.countries.contains(&crisis.target) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.target"),
                format!("references unknown {:?}", crisis.target),
            );
        }

        let mut reactions: Vec<(&CountryId, _)> = crisis.third_party_reactions.iter().collect();
        reactions.sort_by_key(|(k, _)| k.0);
        for (cid, _) in reactions {
            if !idx.countries.contains(cid) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{path}.third_party_reactions[{cid:?}]"),
                    format!("references unknown {cid:?}"),
                );
            }
        }

        for (gi, goal) in crisis.war_goals.iter().enumerate() {
            let gpath = format!("{path}.war_goals[{gi}]");
            if !idx.countries.contains(&goal.attacker) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{gpath}.attacker"),
                    format!("references unknown {:?}", goal.attacker),
                );
            }
            if !idx.countries.contains(&goal.defender) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{gpath}.defender"),
                    format!("references unknown {:?}", goal.defender),
                );
            }
            for (si, sid) in goal.target_states.iter().enumerate() {
                if !idx.states.contains(sid) {
                    push(
                        issues,
                        SaveValidationCode::DanglingReference,
                        format!("{gpath}.target_states[{si}]"),
                        format!("references unknown {sid:?}"),
                    );
                }
            }
        }

        if !crisis.escalation.is_finite()
            || !crisis.initiator_support.is_finite()
            || !crisis.target_resistance.is_finite()
            || !crisis.international_concern.is_finite()
        {
            push(
                issues,
                SaveValidationCode::NonFiniteValue,
                path,
                "escalation/initiator_support/target_resistance/international_concern must be finite",
            );
        }
    }
}

// ─── Division ────────────────────────────────────────────────────────────────

fn validate_divisions(
    save: &SaveGameV1,
    context: &SaveValidationContext,
    idx: &RefIndex,
    issues: &mut Vec<SaveValidationIssue>,
) {
    let mut divisions: Vec<(&DivisionId, &crate::military::data::Division)> =
        save.military.divisions.iter().collect();
    divisions.sort_by_key(|(k, _)| k.0);

    for (division_id, division) in divisions {
        let path = format!("military.divisions[{division_id:?}]");

        if !idx.countries.contains(&division.owner) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.owner"),
                format!("references unknown {:?}", division.owner),
            );
        }
        if !idx.states.contains(&division.current_state) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.current_state"),
                format!("references unknown {:?}", division.current_state),
            );
        }
        if let Some(dest) = division.destination
            && !idx.states.contains(&dest)
        {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.destination"),
                format!("references unknown {dest:?}"),
            );
        }
        if let Some(target) = division.target_state
            && !idx.states.contains(&target)
        {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.target_state"),
                format!("references unknown {target:?}"),
            );
        }
        for (pi, sid) in division.current_path.iter().enumerate() {
            if !idx.states.contains(sid) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{path}.current_path[{pi}]"),
                    format!("references unknown {sid:?}"),
                );
            }
        }
        if !context.division_definitions.contains_key(&division.def_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.def_id"),
                format!(
                    "{:?} is not a defined DivisionDefinitionId",
                    division.def_id
                ),
            );
        }
        if let Some(combat_id) = division.combat_id
            && !idx.battles.contains(&combat_id)
        {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.combat_id"),
                format!("references unknown {combat_id:?}"),
            );
        }

        let finite_ok = division.equipment.is_finite()
            && division.max_equipment.is_finite()
            && division.organization.is_finite()
            && division.max_organization.is_finite()
            && division.morale.is_finite()
            && division.max_morale.is_finite()
            && division.experience.is_finite()
            && division.supply_ratio.is_finite()
            && division.movement_progress.is_finite();
        if !finite_ok {
            push(
                issues,
                SaveValidationCode::NonFiniteValue,
                path,
                "equipment/organization/morale/experience/supply_ratio/movement_progress must be finite",
            );
        }
    }
}

// ─── Battle ──────────────────────────────────────────────────────────────────

fn validate_battles(save: &SaveGameV1, idx: &RefIndex, issues: &mut Vec<SaveValidationIssue>) {
    let mut battles: Vec<(&BattleId, &crate::military::battle::Battle)> =
        save.battles.battles.iter().collect();
    battles.sort_by_key(|(k, _)| k.0);

    let mut ongoing_participants: HashMap<DivisionId, BattleId> = HashMap::new();

    for (battle_id, battle) in &battles {
        let path = format!("battles.battles[{battle_id:?}]");
        if !idx.wars.contains(&battle.war_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.war_id"),
                format!("references unknown {:?}", battle.war_id),
            );
        }
        if !idx.states.contains(&battle.state_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.state_id"),
                format!("references unknown {:?}", battle.state_id),
            );
        }
        if !idx.countries.contains(&battle.attacker_country) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.attacker_country"),
                format!("references unknown {:?}", battle.attacker_country),
            );
        }
        if !idx.countries.contains(&battle.defender_country) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.defender_country"),
                format!("references unknown {:?}", battle.defender_country),
            );
        }

        if battle.status != BattleStatus::Ongoing {
            // 終了・中止済みの戦闘は、参加師団が既に消滅済みでも正常(過去の記録)。
            continue;
        }

        for &division_id in battle
            .attacker_division_ids
            .iter()
            .chain(battle.defender_division_ids.iter())
        {
            let dpath = format!("{path} participant {division_id:?}");
            match save.military.divisions.get(&division_id) {
                None => push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    dpath,
                    format!("references unknown {division_id:?}"),
                ),
                Some(division) => {
                    if division.combat_id != Some(**battle_id) {
                        push(
                            issues,
                            SaveValidationCode::ReverseMapInconsistent,
                            dpath.clone(),
                            format!(
                                "division combat_id {:?} does not point back to {battle_id:?}",
                                division.combat_id
                            ),
                        );
                    }
                    if let Some(prior) = ongoing_participants.insert(division_id, **battle_id)
                        && prior != **battle_id
                    {
                        push(
                            issues,
                            SaveValidationCode::DuplicateMembership,
                            dpath,
                            format!(
                                "{division_id:?} participates in both {prior:?} and {:?}",
                                battle_id
                            ),
                        );
                    }
                }
            }
        }
    }

    let mut divisions: Vec<(&DivisionId, &crate::military::data::Division)> =
        save.military.divisions.iter().collect();
    divisions.sort_by_key(|(k, _)| k.0);
    for (division_id, division) in divisions {
        if let Some(combat_id) = division.combat_id {
            match save.battles.battles.get(&combat_id) {
                Some(battle) if battle.status == BattleStatus::Ongoing => {
                    let participates = battle.attacker_division_ids.contains(division_id)
                        || battle.defender_division_ids.contains(division_id);
                    if !participates {
                        push(
                            issues,
                            SaveValidationCode::ReverseMapInconsistent,
                            format!("military.divisions[{division_id:?}].combat_id"),
                            format!("{combat_id:?} does not list {division_id:?} as a participant"),
                        );
                    }
                }
                Some(_) => {}
                None => {}
            }
        }
    }
}

// ─── Army ────────────────────────────────────────────────────────────────────

fn validate_armies(save: &SaveGameV1, idx: &RefIndex, issues: &mut Vec<SaveValidationIssue>) {
    let mut armies: Vec<(&ArmyId, &crate::military::army::Army)> =
        save.armies.armies.iter().collect();
    armies.sort_by_key(|(k, _)| k.0);

    let mut division_membership: HashMap<DivisionId, ArmyId> = HashMap::new();

    for (army_id, army) in &armies {
        let path = format!("armies.armies[{army_id:?}]");
        if !idx.countries.contains(&army.owner) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.owner"),
                format!("references unknown {:?}", army.owner),
            );
        }
        if army.member_division_ids.is_empty() {
            push(
                issues,
                SaveValidationCode::EmptyCollection,
                path.clone(),
                "an Army must not be persisted with zero member divisions",
            );
        }

        for (mi, &division_id) in army.member_division_ids.iter().enumerate() {
            let mpath = format!("{path}.member_division_ids[{mi}]");
            match save.military.divisions.get(&division_id) {
                None => push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    mpath,
                    format!("references unknown {division_id:?}"),
                ),
                Some(division) => {
                    if division.owner != army.owner {
                        push(
                            issues,
                            SaveValidationCode::OwnershipMismatch,
                            mpath.clone(),
                            format!(
                                "division owner {:?} does not match army owner {:?}",
                                division.owner, army.owner
                            ),
                        );
                    }
                    if let Some(prior) = division_membership.insert(division_id, **army_id)
                        && prior != **army_id
                    {
                        push(
                            issues,
                            SaveValidationCode::DuplicateMembership,
                            mpath,
                            format!(
                                "{division_id:?} belongs to both {prior:?} and {:?}",
                                army_id
                            ),
                        );
                    }
                }
            }
        }
    }

    let mut division_army_map: Vec<(&DivisionId, &ArmyId)> =
        save.armies.division_army_map.iter().collect();
    division_army_map.sort_by_key(|(k, _)| k.0);
    for (division_id, army_id) in division_army_map {
        let path = format!("armies.division_army_map[{division_id:?}]");
        if !idx.divisions.contains(division_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                path.clone(),
                format!("references unknown {division_id:?}"),
            );
        }
        match save.armies.armies.get(army_id) {
            None => push(
                issues,
                SaveValidationCode::DanglingReference,
                path,
                format!("references unknown {army_id:?}"),
            ),
            Some(army) => {
                if !army.member_division_ids.contains(division_id) {
                    push(
                        issues,
                        SaveValidationCode::ReverseMapInconsistent,
                        path,
                        format!("{army_id:?}.member_division_ids does not contain {division_id:?}"),
                    );
                }
            }
        }
    }

    for (army_id, army) in &armies {
        for &division_id in &army.member_division_ids {
            if save.armies.division_army_map.get(&division_id) != Some(*army_id) {
                push(
                    issues,
                    SaveValidationCode::ReverseMapInconsistent,
                    format!("armies.armies[{army_id:?}].member_division_ids"),
                    format!(
                        "{division_id:?} is a member but division_army_map does not map it back to {army_id:?}"
                    ),
                );
            }
        }
    }

    let mut next_army_number: Vec<(&CountryId, &u32)> =
        save.armies.next_army_number.iter().collect();
    next_army_number.sort_by_key(|(k, _)| k.0);
    for (owner, &next_number) in next_army_number {
        let path = format!("armies.next_army_number[{owner:?}]");
        if !idx.countries.contains(owner) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                path.clone(),
                format!("references unknown {owner:?}"),
            );
        }
        let max_used = save
            .armies
            .armies
            .values()
            .filter(|a| a.owner == *owner)
            .filter_map(|a| parse_army_number(&a.name))
            .max();
        if let Some(max_used) = max_used
            && next_number <= max_used
        {
            push(
                issues,
                SaveValidationCode::NextIdCollision,
                path,
                format!(
                    "next_army_number {next_number} must be greater than the highest existing 'Army N' number {max_used} for {owner:?}"
                ),
            );
        }
    }
}

/// `create_army`が生成する"Army {N}"形式の名前からNを取り出す(命名衝突検証専用)。
/// このcrateには他にArmy.nameを書き換える経路が存在しない(P21-SAVE-002C調査で確認済み)。
fn parse_army_number(name: &str) -> Option<u32> {
    name.strip_prefix("Army ")?.trim().parse().ok()
}

// ─── AI ──────────────────────────────────────────────────────────────────────

fn validate_ai(save: &SaveGameV1, idx: &RefIndex, issues: &mut Vec<SaveValidationIssue>) {
    let mut country_ai: Vec<(&CountryId, &crate::save::dto::SavedCountryAiState)> =
        save.country_ai.ai_states.iter().collect();
    country_ai.sort_by_key(|(k, _)| k.0);
    for (country_id, state) in country_ai {
        if !idx.countries.contains(country_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("country_ai.ai_states[{country_id:?}]"),
                format!("references unknown {country_id:?}"),
            );
        }
        let _ = state;
    }

    let mut military_ai: Vec<(&CountryId, &crate::save::dto::SavedMilitaryAiState)> =
        save.military_ai.ai_states.iter().collect();
    military_ai.sort_by_key(|(k, _)| k.0);
    for (country_id, state) in military_ai {
        if !idx.countries.contains(country_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("military_ai.ai_states[{country_id:?}]"),
                format!("references unknown {country_id:?}"),
            );
        }
        let _ = state;
    }
}

// ─── Frontline ───────────────────────────────────────────────────────────────

fn validate_frontlines(save: &SaveGameV1, idx: &RefIndex, issues: &mut Vec<SaveValidationIssue>) {
    let mut frontlines: Vec<(&FrontlineId, &crate::war::frontline::Frontline)> =
        save.frontlines.frontlines.iter().collect();
    frontlines.sort_by_key(|(k, _)| k.0);

    for (fl_id, fl) in &frontlines {
        let path = format!("frontlines.frontlines[{fl_id:?}]");
        let war = save.wars.wars.get(&fl.war_id);
        if war.is_none() {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.war_id"),
                format!("references unknown {:?}", fl.war_id),
            );
        }
        if fl.attacker_country_id == fl.defender_country_id {
            push(
                issues,
                SaveValidationCode::SetOverlap,
                path.clone(),
                format!(
                    "attacker_country_id and defender_country_id must differ, both are {:?}",
                    fl.attacker_country_id
                ),
            );
        }
        if !idx.countries.contains(&fl.attacker_country_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.attacker_country_id"),
                format!("references unknown {:?}", fl.attacker_country_id),
            );
        }
        if !idx.countries.contains(&fl.defender_country_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.defender_country_id"),
                format!("references unknown {:?}", fl.defender_country_id),
            );
        }
        if let Some(war) = war {
            if !war.attackers.contains(&fl.attacker_country_id) {
                push(
                    issues,
                    SaveValidationCode::ParticipantMismatch,
                    format!("{path}.attacker_country_id"),
                    format!(
                        "{:?} is not among {:?}'s attackers",
                        fl.attacker_country_id, fl.war_id
                    ),
                );
            }
            if !war.defenders.contains(&fl.defender_country_id) {
                push(
                    issues,
                    SaveValidationCode::ParticipantMismatch,
                    format!("{path}.defender_country_id"),
                    format!(
                        "{:?} is not among {:?}'s defenders",
                        fl.defender_country_id, fl.war_id
                    ),
                );
            }
        }

        for (ri, sid) in fl
            .attacker_front_regions
            .iter()
            .chain(fl.defender_front_regions.iter())
            .enumerate()
        {
            if !idx.states.contains(sid) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{path}.front_regions[{ri}]"),
                    format!("references unknown {sid:?}"),
                );
            }
        }
        for (pi, (a, b)) in fl.border_region_pairs.iter().enumerate() {
            if !idx.states.contains(a) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{path}.border_region_pairs[{pi}].0"),
                    format!("references unknown {a:?}"),
                );
            }
            if !idx.states.contains(b) {
                push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    format!("{path}.border_region_pairs[{pi}].1"),
                    format!("references unknown {b:?}"),
                );
            }
        }
    }

    let mut plan_membership: HashMap<DivisionId, FrontlineId> = HashMap::new();
    let mut army_plan_membership: HashMap<ArmyId, FrontlineId> = HashMap::new();
    let mut plans: Vec<(
        &(FrontlineId, CountryId),
        &crate::war::frontline::FrontlinePlan,
    )> = save.frontlines.plans.iter().collect();
    plans.sort_by_key(|(k, _)| (k.0.0, k.1.0));

    for (&(fl_key, country_key), plan) in plans.iter().copied() {
        let path = format!("frontlines.plans[({fl_key:?}, {country_key:?})]");
        if !idx.frontlines.contains(&fl_key) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.frontline_id"),
                format!("references unknown {fl_key:?}"),
            );
        }
        if !idx.countries.contains(&country_key) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.commanding_country_id"),
                format!("references unknown {country_key:?}"),
            );
        }
        if let Some(fl) = save.frontlines.frontlines.get(&fl_key)
            && country_key != fl.attacker_country_id
            && country_key != fl.defender_country_id
        {
            push(
                issues,
                SaveValidationCode::ParticipantMismatch,
                path.clone(),
                format!("commanding_country_id {country_key:?} is not a participant of {fl_key:?}"),
            );
        }
        if let Some(objective) = plan.objective_region_id
            && !idx.states.contains(&objective)
        {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                format!("{path}.objective_region_id"),
                format!("references unknown {objective:?}"),
            );
        }

        // P21-007: 攻勢線(計画データのみ)の検証。`Some(空Vec)`は無効(未設定は常に
        // None)、各地点は実在・陸地・「このFrontlineの戦争における敵国支配」、
        // 複数地点なら隣接グラフ上で連結していることを要求する。
        // `war::frontline::FrontlineRegistry::set_offensive_line`のランタイム検証と
        // 同じ規則を、セーブのDTO形状(`save.states: Vec<StateData>`)に対して直接
        // 適用する(このファイルの既存の隣接双方向性チェックと同じ方針で、
        // ランタイムの`StateRegistry`構築には依存しない)。
        if let Some(line_ids) = &plan.offensive_line_region_ids {
            let opath = format!("{path}.offensive_line_region_ids");
            if line_ids.is_empty() {
                push(
                    issues,
                    SaveValidationCode::EmptyCollection,
                    opath.clone(),
                    "offensive_line_region_ids must not be Some(empty); use None for 'unset'",
                );
            }

            let enemy_id = save.frontlines.frontlines.get(&fl_key).map(|fl| {
                if country_key == fl.attacker_country_id {
                    fl.defender_country_id
                } else {
                    fl.attacker_country_id
                }
            });

            let mut seen_line_ids: HashSet<StateId> = HashSet::new();
            for (li, &sid) in line_ids.iter().enumerate() {
                let lpath = format!("{opath}[{li}]");
                if !seen_line_ids.insert(sid) {
                    push(
                        issues,
                        SaveValidationCode::DuplicateId,
                        lpath,
                        format!("{sid:?} is listed more than once"),
                    );
                    continue;
                }
                match save.states.iter().find(|s| s.id == sid) {
                    None => push(
                        issues,
                        SaveValidationCode::DanglingReference,
                        lpath,
                        format!("references unknown {sid:?}"),
                    ),
                    Some(state) => {
                        if state.is_sea {
                            push(
                                issues,
                                SaveValidationCode::NonLandRegion,
                                lpath.clone(),
                                format!("{sid:?} is a sea state"),
                            );
                        }
                        // P21-008: 設定時(`FrontlineRegistry::set_offensive_line`)は
                        // 敵支配Stateのみを要求するが、ここ(ロード時)はそれより緩い条件を
                        // 使う。攻勢を実行した結果、攻勢線の一部または全部が自国支配
                        // (確保済み)に変化しているのが正常な状態であるため、「敵支配または
                        // commanding country自身の支配」のいずれかを正当とし、それ以外の
                        // 第三国支配だけを不正とする(存在しない/海/重複/非連結は
                        // このロード時検証で引き続き無条件に拒否する)。
                        if let Some(enemy) = enemy_id
                            && state.controller() != enemy
                            && state.controller() != country_key
                        {
                            push(
                                issues,
                                SaveValidationCode::OwnershipMismatch,
                                lpath,
                                format!(
                                    "{sid:?} is controlled by {:?}, expected this frontline's enemy {enemy:?} or the commanding country {country_key:?} (captured)",
                                    state.controller()
                                ),
                            );
                        }
                    }
                }
            }

            if line_ids.len() > 1 {
                let region_set: HashSet<StateId> = line_ids.iter().copied().collect();
                let mut visited: HashSet<StateId> = HashSet::new();
                let mut stack = vec![line_ids[0]];
                visited.insert(line_ids[0]);
                while let Some(current) = stack.pop() {
                    if let Some(state) = save.states.iter().find(|s| s.id == current) {
                        for &neighbor in &state.neighbors {
                            if region_set.contains(&neighbor) && visited.insert(neighbor) {
                                stack.push(neighbor);
                            }
                        }
                    }
                }
                if visited.len() != region_set.len() {
                    push(
                        issues,
                        SaveValidationCode::DisconnectedRegionSet,
                        opath,
                        "offensive_line_region_ids must form a single connected component via state adjacency",
                    );
                }
            }
        }

        for (di, &division_id) in plan.assigned_division_ids.iter().enumerate() {
            let dpath = format!("{path}.assigned_division_ids[{di}]");
            match save.military.divisions.get(&division_id) {
                None => push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    dpath,
                    format!("references unknown {division_id:?}"),
                ),
                Some(division) => {
                    if division.owner != country_key {
                        push(
                            issues,
                            SaveValidationCode::OwnershipMismatch,
                            dpath.clone(),
                            format!(
                                "division owner {:?} does not match plan's commanding country {country_key:?}",
                                division.owner
                            ),
                        );
                    }
                    if let Some(prior) = plan_membership.insert(division_id, fl_key)
                        && prior != fl_key
                    {
                        push(
                            issues,
                            SaveValidationCode::DuplicateMembership,
                            dpath,
                            format!("{division_id:?} is assigned to both {prior:?} and {fl_key:?}"),
                        );
                    }
                }
            }
        }

        // P21-005: assigned_army_ids(Army↔Frontline割当)の検証。assigned_division_ids
        // と同じ方針(dangling参照・所有者不一致・複数Frontlineへの二重割当を拒否)。
        for (ai, &army_id) in plan.assigned_army_ids.iter().enumerate() {
            let apath = format!("{path}.assigned_army_ids[{ai}]");
            match save.armies.armies.get(&army_id) {
                None => push(
                    issues,
                    SaveValidationCode::DanglingReference,
                    apath,
                    format!("references unknown {army_id:?}"),
                ),
                Some(army) => {
                    if army.owner != country_key {
                        push(
                            issues,
                            SaveValidationCode::OwnershipMismatch,
                            apath.clone(),
                            format!(
                                "army owner {:?} does not match plan's commanding country {country_key:?}",
                                army.owner
                            ),
                        );
                    }
                    if let Some(prior) = army_plan_membership.insert(army_id, fl_key)
                        && prior != fl_key
                    {
                        push(
                            issues,
                            SaveValidationCode::DuplicateMembership,
                            apath,
                            format!("{army_id:?} is assigned to both {prior:?} and {fl_key:?}"),
                        );
                    }
                }
            }
        }
    }

    let mut division_frontline_map: Vec<(&DivisionId, &FrontlineId)> =
        save.frontlines.division_frontline_map.iter().collect();
    division_frontline_map.sort_by_key(|(k, _)| k.0);
    for (division_id, fl_id) in division_frontline_map {
        let path = format!("frontlines.division_frontline_map[{division_id:?}]");
        if !idx.divisions.contains(division_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                path.clone(),
                format!("references unknown {division_id:?}"),
            );
        }
        if !idx.frontlines.contains(fl_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                path.clone(),
                format!("references unknown {fl_id:?}"),
            );
        }
        let listed_in_a_plan = plans.iter().any(|((plan_fl, _), plan)| {
            plan_fl == fl_id && plan.assigned_division_ids.contains(division_id)
        });
        if !listed_in_a_plan {
            push(
                issues,
                SaveValidationCode::ReverseMapInconsistent,
                path,
                format!(
                    "{division_id:?} -> {fl_id:?} has no matching FrontlinePlan.assigned_division_ids entry"
                ),
            );
        }
    }

    for (&(fl_key, _), plan) in plans.iter().copied() {
        for &division_id in &plan.assigned_division_ids {
            if save.frontlines.division_frontline_map.get(&division_id) != Some(&fl_key) {
                push(
                    issues,
                    SaveValidationCode::ReverseMapInconsistent,
                    "frontlines.plans assigned_division_ids",
                    format!(
                        "{division_id:?} is assigned in a plan for {fl_key:?} but division_frontline_map does not map it back"
                    ),
                );
            }
        }
    }

    // P21-005: army_frontline_map(ArmyId -> FrontlineId)の検証。division_frontline_map
    // と同じ双方向一貫性チェック(dangling参照・FrontlinePlan.assigned_army_idsとの
    // 相互整合性)。
    let mut army_frontline_map: Vec<(&ArmyId, &FrontlineId)> =
        save.frontlines.army_frontline_map.iter().collect();
    army_frontline_map.sort_by_key(|(k, _)| k.0);
    for (army_id, fl_id) in army_frontline_map {
        let path = format!("frontlines.army_frontline_map[{army_id:?}]");
        if !save.armies.armies.contains_key(army_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                path.clone(),
                format!("references unknown {army_id:?}"),
            );
        }
        if !idx.frontlines.contains(fl_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                path.clone(),
                format!("references unknown {fl_id:?}"),
            );
        }
        let listed_in_a_plan = plans.iter().any(|((plan_fl, _), plan)| {
            plan_fl == fl_id && plan.assigned_army_ids.contains(army_id)
        });
        if !listed_in_a_plan {
            push(
                issues,
                SaveValidationCode::ReverseMapInconsistent,
                path,
                format!(
                    "{army_id:?} -> {fl_id:?} has no matching FrontlinePlan.assigned_army_ids entry"
                ),
            );
        }
    }

    for (&(fl_key, _), plan) in plans.iter().copied() {
        for &army_id in &plan.assigned_army_ids {
            if save.frontlines.army_frontline_map.get(&army_id) != Some(&fl_key) {
                push(
                    issues,
                    SaveValidationCode::ReverseMapInconsistent,
                    "frontlines.plans assigned_army_ids",
                    format!(
                        "{army_id:?} is assigned in a plan for {fl_key:?} but army_frontline_map does not map it back"
                    ),
                );
            }
        }
    }

    let mut generated: Vec<&DivisionId> = save
        .frontlines
        .frontline_generated_movements
        .iter()
        .collect();
    generated.sort_by_key(|d| d.0);
    for division_id in generated {
        if !idx.divisions.contains(division_id) {
            push(
                issues,
                SaveValidationCode::DanglingReference,
                "frontlines.frontline_generated_movements",
                format!("references unknown {division_id:?}"),
            );
        }
    }
}

// ─── P21-SAVE-002D: 適用前の静的マスター互換性確認 ────────────────────────────

/// `ValidatedSaveGameV1`の検証時に使用した静的マスターと、適用先Worldの現在の静的マスターが
/// 一致するかどうかだけを確認する(002Cの参照整合性検証全体をやり直すわけではない)。
///
/// `SaveValidationContext`を適用先Worldから新たに構築し直して渡すことで、
/// 「検証時に使用したコンテキスト」と「適用先の現在のコンテキスト」が異なる可能性
/// (例: 起動後に`assets/data/*.ron`が差し替わった、複数バージョンのビルドが混在する等)
/// を無視しない。002Cの`validate_countries`/`validate_states`/`validate_divisions`の
/// ロジックを複製せず、静的マスター参照の存在確認(`HashMap::contains_key`)だけを
/// このラウンド用に独立した小さな関数として持つ(既存関数を流用すると、静的マスター以外の
/// 参照整合性まで再チェックしてしまい、`ApplyLoadError`のカテゴリ分けが曖昧になるため)。
pub(crate) fn check_static_master_compatibility(
    save: &SaveGameV1,
    context: &SaveValidationContext,
) -> Result<(), SaveValidationErrors> {
    let mut issues = Vec::new();

    // `WorldStage::PreIndustrial`はゲーム開始時点の既定段階であり、他の段階と異なり
    // `assets/data/world_stages.ron`に定義エントリを持たない(§307付近の検証と同じ理由)。
    if save.world_civilization.current_stage != WorldStage::PreIndustrial
        && !context
            .world_stage_definitions
            .contains_key(&save.world_civilization.current_stage)
    {
        push(
            &mut issues,
            SaveValidationCode::DanglingReference,
            "world_civilization.current_stage",
            format!(
                "{:?} is not defined in the current world's static WorldStage definitions",
                save.world_civilization.current_stage
            ),
        );
    }

    for (i, country) in save.countries.iter().enumerate() {
        let base = format!("countries[{i}]");
        for (qi, item) in country.construction_queue.iter().enumerate() {
            if !context
                .building_definitions
                .contains_key(&item.building_type)
            {
                push(
                    &mut issues,
                    SaveValidationCode::DanglingReference,
                    format!("{base}.construction_queue[{qi}].building_type"),
                    format!(
                        "{:?} is not defined in the current world's BuildingRegistry",
                        item.building_type
                    ),
                );
            }
        }
        for (qi, item) in country.recruitment_queue.iter().enumerate() {
            if !context.division_definitions.contains_key(&item.division_id) {
                push(
                    &mut issues,
                    SaveValidationCode::DanglingReference,
                    format!("{base}.recruitment_queue[{qi}].division_id"),
                    format!(
                        "{:?} is not defined in the current world's MilitaryRegistry.definitions",
                        item.division_id
                    ),
                );
            }
        }
        let mut tech_ids: Vec<&String> = country
            .research_state
            .completed_technologies
            .iter()
            .collect();
        tech_ids.sort();
        for tech_id in tech_ids {
            if !context.technology_definitions.contains_key(tech_id) {
                push(
                    &mut issues,
                    SaveValidationCode::DanglingReference,
                    format!("{base}.research_state.completed_technologies"),
                    format!(
                        "technology id '{tech_id}' is not defined in the current world's TechnologyRegistry"
                    ),
                );
            }
        }
    }

    for (i, state) in save.states.iter().enumerate() {
        let mut buildings: Vec<&BuildingType> = state.buildings.keys().collect();
        buildings.sort_by_key(|bt| format!("{bt:?}"));
        for b_type in buildings {
            if !context.building_definitions.contains_key(b_type) {
                push(
                    &mut issues,
                    SaveValidationCode::DanglingReference,
                    format!("states[{i}].buildings[{b_type:?}]"),
                    format!("{b_type:?} is not defined in the current world's BuildingRegistry"),
                );
            }
        }
    }

    let mut divisions: Vec<(&DivisionId, &crate::military::data::Division)> =
        save.military.divisions.iter().collect();
    divisions.sort_by_key(|(k, _)| k.0);
    for (division_id, division) in divisions {
        if !context.division_definitions.contains_key(&division.def_id) {
            push(
                &mut issues,
                SaveValidationCode::DanglingReference,
                format!("military.divisions[{division_id:?}].def_id"),
                format!(
                    "{:?} is not defined in the current world's MilitaryRegistry.definitions",
                    division.def_id
                ),
            );
        }
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(SaveValidationErrors { issues })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::{CountryData, EconomicSystem, GovernmentType};
    use crate::diplomacy::claims::{ClaimSource, TerritorialClaim};
    use crate::diplomacy::crisis::WarGoal;
    use crate::diplomacy::crisis::{CrisisPhase, DiplomaticCrisis};
    use crate::diplomacy::data::{
        ActiveDiplomaticActivity, ActiveTreaty, DiplomaticActivityType, DiplomaticRelation,
        TreatyType,
    };
    use crate::military::army::Army;
    use crate::military::battle::{Battle, BattleStatus};
    use crate::military::data::{Division, DivisionSize, DivisionStatus, DivisionType};
    use crate::save::dto::{
        SAVE_FORMAT_VERSION_V1, SaveGameV1, SavedArmyRegistry, SavedBattleRegistry,
        SavedClaimRegistry, SavedCountryAiRegistry, SavedCountryAiState, SavedCrisisRegistry,
        SavedDiplomacyRegistry, SavedFrontlineRegistry, SavedGameDate, SavedMilitaryAiRegistry,
        SavedMilitaryAiState, SavedMilitaryRegistry, SavedWarJustificationRegistry,
        SavedWarRegistry, SavedWorldCivilizationState,
    };
    use crate::state::data::{StateData, StateRegistry};
    use crate::war::data::{War, WarStatus};
    use crate::war::frontline::{Frontline, FrontlinePlan, FrontlineStance};
    use crate::war::justification::WarJustification;
    use crate::war::military_ai::MilitaryAiDecisionReason;
    use std::collections::HashMap;

    /// 検証コンテキストが借用する静的マスターデータの所有側(fixture専用)。
    struct StaticData {
        buildings: HashMap<BuildingType, BuildingDefinition>,
        technologies: HashMap<String, TechnologyDefinition>,
        divisions: HashMap<DivisionDefinitionId, DivisionDefinition>,
        world_stages: HashMap<WorldStage, WorldStageDefinition>,
    }

    impl StaticData {
        fn context(&self) -> SaveValidationContext<'_> {
            SaveValidationContext {
                building_definitions: &self.buildings,
                technology_definitions: &self.technologies,
                division_definitions: &self.divisions,
                world_stage_definitions: &self.world_stages,
            }
        }
    }

    fn building_definition(building_type: BuildingType, max_level: u32) -> BuildingDefinition {
        BuildingDefinition {
            building_type,
            name: format!("{building_type:?}"),
            construction_cost: 100.0,
            required_progress: 10.0,
            required_workforce: 10.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: HashMap::new(),
            maintenance_cost: 1.0,
            max_level,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        }
    }

    fn division_definition(id: DivisionDefinitionId) -> DivisionDefinition {
        DivisionDefinition {
            id,
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
        }
    }

    fn static_data() -> StaticData {
        let mut buildings = HashMap::new();
        buildings.insert(
            BuildingType::Farm,
            building_definition(BuildingType::Farm, 5),
        );

        let mut technologies = HashMap::new();
        technologies.insert(
            "tech_a".to_string(),
            TechnologyDefinition {
                id: "tech_a".to_string(),
                name: "Tech A".to_string(),
                description: String::new(),
                field: TechnologyField::Science,
                cost: 100.0,
                prerequisites: Vec::new(),
                minimum_world_stage: WorldStage::PreIndustrial,
                effects: Vec::new(),
                is_world_milestone: false,
                milestone_target_stage: None,
                ui_order: 0,
            },
        );

        let mut divisions = HashMap::new();
        divisions.insert(
            DivisionDefinitionId(0),
            division_definition(DivisionDefinitionId(0)),
        );

        let mut world_stages = HashMap::new();
        world_stages.insert(
            WorldStage::PreIndustrial,
            WorldStageDefinition {
                stage: WorldStage::PreIndustrial,
                display_name: "Pre-Industrial".to_string(),
                description: String::new(),
                required_previous_stage: None,
                milestone_technologies: Vec::new(),
                required_country_count: 1,
            },
        );

        StaticData {
            buildings,
            technologies,
            divisions,
            world_stages,
        }
    }

    fn make_division(id: usize, owner: CountryId, current_state: StateId) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state,
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 1_000,
            max_manpower: 1_000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    /// 全カテゴリの検証が「正常」と判定する代表的なベースライン`SaveGameV1`。
    /// 各テストはこれを`.clone()`し、1箇所だけ壊してから検証する。
    fn baseline_save() -> SaveGameV1 {
        let country0 = CountryData {
            id: CountryId(0),
            capital_state_id: StateId(0),
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        };
        let country1 = CountryData {
            id: CountryId(1),
            capital_state_id: StateId(1),
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        };

        let s0 = StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            neighbors: vec![StateId(1)],
            ..StateData::default()
        };
        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(0), StateId(2)],
            ..StateData::default()
        };
        let s2 = StateData {
            id: StateId(2),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(1)],
            ..StateData::default()
        };
        let states = StateRegistry::build(vec![s0, s1, s2]).states;

        let division = make_division(0, CountryId(0), StateId(0));
        let mut divisions = HashMap::new();
        divisions.insert(division.id, division);

        let army = Army {
            id: ArmyId(0),
            owner: CountryId(0),
            name: "Army 1".to_string(),
            member_division_ids: vec![DivisionId(0)],
        };
        let mut armies = HashMap::new();
        armies.insert(army.id, army);
        let mut division_army_map = HashMap::new();
        division_army_map.insert(DivisionId(0), ArmyId(0));
        let mut next_army_number = HashMap::new();
        next_army_number.insert(CountryId(0), 2u32);

        let war = War {
            id: WarId(0),
            name: "Test War".to_string(),
            attackers: [CountryId(0)].into_iter().collect(),
            defenders: [CountryId(1)].into_iter().collect(),
            war_goals: vec![WarGoal {
                attacker: CountryId(0),
                defender: CountryId(1),
                goal_type: crate::diplomacy::crisis::WarGoalType::ConquerState,
                target_states: vec![StateId(1)],
                base_peace_cost: 20.0,
                international_concern: 10.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: std::collections::HashSet::new(),
            status: WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: std::collections::HashSet::new(),
        };
        let mut wars = HashMap::new();
        wars.insert(war.id, war);

        let mut relations = HashMap::new();
        let pair = DiplomaticPairKey(CountryId(0), CountryId(1));
        let mut cooldowns = HashMap::new();
        cooldowns.insert(CountryId(1), 5u32);
        relations.insert(
            pair,
            DiplomaticRelation {
                opinion: 10.0,
                tension: 0.0,
                trust: 50.0,
                threat: 0.0,
                at_war: true,
                has_military_access: false,
                truce_until: None,
                treaties: vec![ActiveTreaty {
                    treaty_type: TreatyType::NonAggressionPact,
                    countries: (CountryId(0), CountryId(1)),
                    signed_date: "1800/01/01".to_string(),
                    is_active: false,
                }],
                cooldowns,
                active_activity: Some(ActiveDiplomaticActivity {
                    activity_type: DiplomaticActivityType::ImproveRelations,
                    initiator: CountryId(0),
                    target: CountryId(1),
                    days_remaining: 3,
                    daily_opinion_change: 1.0,
                }),
                last_updated_date: "1800/01/01".to_string(),
            },
        );

        let mut country_ai_states = HashMap::new();
        country_ai_states.insert(
            CountryId(1),
            SavedCountryAiState {
                country_id: CountryId(1),
                mode: crate::country::country_ai::CountryAiMode::AtWar,
                decision_reason: crate::country::country_ai::CountryAiDecisionReason::WarInProgress,
                last_daily_evaluation_day: 0,
                last_weekly_evaluation_day: 0,
                last_monthly_evaluation_day: 0,
                cooldown_until_day: 0,
            },
        );
        let mut military_ai_states = HashMap::new();
        military_ai_states.insert(
            CountryId(1),
            SavedMilitaryAiState {
                country_id: CountryId(1),
                last_evaluated_day: 0,
                last_decision_reason: MilitaryAiDecisionReason::NoActiveWar,
                estimated_own_power: 0,
                estimated_enemy_power: 0,
            },
        );

        SaveGameV1 {
            version: SAVE_FORMAT_VERSION_V1,
            date: SavedGameDate {
                year: 1800,
                month: 1,
                day: 1,
                accumulator: 0.0,
            },
            game_speed: 1,
            player_country: Some(CountryId(0)),
            world_civilization: SavedWorldCivilizationState {
                current_stage: WorldStage::PreIndustrial,
                milestone_countries: HashMap::new(),
                last_advanced_date: "1800/01/01".to_string(),
            },
            countries: vec![country0, country1],
            states,
            diplomacy: SavedDiplomacyRegistry { relations },
            war_justifications: SavedWarJustificationRegistry::default(),
            wars: SavedWarRegistry { wars, next_id: 1 },
            claims: SavedClaimRegistry::default(),
            crises: SavedCrisisRegistry::default(),
            country_ai: SavedCountryAiRegistry {
                ai_states: country_ai_states,
            },
            military_ai: SavedMilitaryAiRegistry {
                ai_states: military_ai_states,
            },
            military: SavedMilitaryRegistry {
                divisions,
                next_division_id: 1,
            },
            battles: SavedBattleRegistry::default(),
            armies: SavedArmyRegistry {
                armies,
                division_army_map,
                next_id: 1,
                next_army_number,
            },
            frontlines: SavedFrontlineRegistry::default(),
        }
    }

    fn issues_for(mutate: impl FnOnce(&mut SaveGameV1)) -> Vec<SaveValidationIssue> {
        let statics = static_data();
        let mut save = baseline_save();
        mutate(&mut save);
        match validate_save_game_v1(save, &statics.context()) {
            Ok(_) => Vec::new(),
            Err(errors) => errors.issues,
        }
    }

    fn has_code(issues: &[SaveValidationIssue], code: SaveValidationCode) -> bool {
        issues.iter().any(|i| i.code == code)
    }

    // ─── ベースライン ────────────────────────────────────────────────────────

    #[test]
    fn baseline_save_passes_validation() {
        let issues = issues_for(|_| {});
        assert!(issues.is_empty(), "baseline must be valid, got {issues:?}");
    }

    #[test]
    fn accepts_empty_claims_and_crises() {
        let issues = issues_for(|save| {
            assert!(save.claims.claims.is_empty());
            assert!(save.crises.crises.is_empty());
        });
        assert!(issues.is_empty());
    }

    #[test]
    fn accepts_correct_next_id_for_empty_registries() {
        let issues = issues_for(|save| {
            assert_eq!(save.claims.next_id, 0);
            assert_eq!(save.crises.next_id, 0);
            assert_eq!(save.battles.next_id, 0);
        });
        assert!(issues.is_empty());
    }

    #[test]
    fn accepts_valid_in_progress_movement_division() {
        let issues = issues_for(|save| {
            let division = save.military.divisions.get_mut(&DivisionId(0)).unwrap();
            division.status = DivisionStatus::Moving;
            division.destination = Some(StateId(1));
            division.current_path = vec![StateId(1)];
            division.movement_progress = 0.42;
        });
        assert!(
            issues.is_empty(),
            "in-progress movement must be valid, got {issues:?}"
        );
    }

    #[test]
    fn accepts_valid_ongoing_battle_with_matching_combat_id() {
        let issues = issues_for(|save| {
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .status = DivisionStatus::Fighting;
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .combat_id = Some(BattleId(0));
            save.battles.battles.insert(
                BattleId(0),
                Battle {
                    id: BattleId(0),
                    war_id: WarId(0),
                    state_id: StateId(0),
                    attacker_country: CountryId(1),
                    defender_country: CountryId(0),
                    attacker_division_ids: Vec::new(),
                    defender_division_ids: vec![DivisionId(0)],
                    attacker_origins: HashMap::new(),
                    start_date: "1800/01/02".to_string(),
                    elapsed_days: 0,
                    status: BattleStatus::Ongoing,
                },
            );
            save.battles.next_id = 1;
        });
        assert!(
            issues.is_empty(),
            "valid ongoing battle must be accepted, got {issues:?}"
        );
    }

    // ─── 基本状態 ────────────────────────────────────────────────────────────

    #[test]
    fn duplicate_country_id_is_rejected() {
        let issues = issues_for(|save| {
            let mut c = save.countries[1].clone();
            c.id = CountryId(0);
            save.countries.push(c);
        });
        assert!(has_code(&issues, SaveValidationCode::DuplicateId));
    }

    #[test]
    fn duplicate_state_id_is_rejected() {
        let issues = issues_for(|save| {
            let mut s = save.states[1].clone();
            s.id = StateId(0);
            save.states.push(s);
        });
        assert!(has_code(&issues, SaveValidationCode::DuplicateId));
    }

    #[test]
    fn missing_player_country_is_rejected() {
        let issues = issues_for(|save| save.player_country = None);
        assert!(has_code(&issues, SaveValidationCode::MissingValue));
    }

    #[test]
    fn unknown_player_country_is_rejected() {
        let issues = issues_for(|save| save.player_country = Some(CountryId(999)));
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_capital_state_id_is_rejected() {
        let issues = issues_for(|save| save.countries[0].capital_state_id = StateId(999));
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn capital_owned_by_another_country_is_rejected() {
        let issues = issues_for(|save| save.countries[0].capital_state_id = StateId(1));
        assert!(has_code(&issues, SaveValidationCode::OwnershipMismatch));
    }

    #[test]
    fn unknown_state_owner_is_rejected() {
        let issues = issues_for(|save| save.states[2].owner_country_id = CountryId(999));
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_state_controller_is_rejected() {
        let issues = issues_for(|save| save.states[2].controller_country = Some(CountryId(999)));
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_state_original_owner_is_rejected() {
        let issues = issues_for(|save| save.states[2].original_owner = Some(CountryId(999)));
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_neighbor_reference_is_rejected() {
        let issues = issues_for(|save| save.states[2].neighbors.push(StateId(999)));
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn asymmetric_neighbor_is_rejected() {
        let issues = issues_for(|save| save.states[0].neighbors.push(StateId(2)));
        assert!(has_code(&issues, SaveValidationCode::AsymmetricAdjacency));
    }

    #[test]
    fn self_adjacent_state_is_rejected() {
        let issues = issues_for(|save| save.states[0].neighbors.push(StateId(0)));
        assert!(has_code(&issues, SaveValidationCode::SelfAdjacency));
    }

    #[test]
    fn duplicate_neighbor_entry_is_rejected() {
        let issues = issues_for(|save| {
            let dup = save.states[0].neighbors[0];
            save.states[0].neighbors.push(dup);
        });
        assert!(has_code(&issues, SaveValidationCode::DuplicateAdjacency));
    }

    #[test]
    fn invalid_game_date_month_is_rejected() {
        let issues = issues_for(|save| save.date.month = 13);
        assert!(has_code(&issues, SaveValidationCode::InvalidRange));
    }

    #[test]
    fn invalid_game_date_day_is_rejected() {
        let issues = issues_for(|save| save.date.day = 32);
        assert!(has_code(&issues, SaveValidationCode::InvalidRange));
    }

    #[test]
    fn invalid_game_date_accumulator_is_rejected() {
        let issues = issues_for(|save| save.date.accumulator = 1.5);
        assert!(has_code(&issues, SaveValidationCode::InvalidRange));
    }

    #[test]
    fn non_finite_game_date_accumulator_is_rejected() {
        let issues = issues_for(|save| save.date.accumulator = f64::NAN);
        assert!(has_code(&issues, SaveValidationCode::InvalidRange));
    }

    #[test]
    fn game_speed_zero_is_rejected() {
        let issues = issues_for(|save| save.game_speed = 0);
        assert!(has_code(&issues, SaveValidationCode::InvalidRange));
    }

    #[test]
    fn game_speed_above_four_is_rejected() {
        let issues = issues_for(|save| save.game_speed = 5);
        assert!(has_code(&issues, SaveValidationCode::InvalidRange));
    }

    #[test]
    fn unknown_world_stage_is_rejected() {
        let mut save = baseline_save();
        save.world_civilization.current_stage = WorldStage::ElectricalAge;
        let statics = static_data(); // world_stagesにはPreIndustrialしか登録していない
        let result = validate_save_game_v1(save, &statics.context());
        let issues = result.err().map(|e| e.issues).unwrap_or_default();
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_milestone_country_is_rejected() {
        let issues = issues_for(|save| {
            let mut set = std::collections::HashSet::new();
            set.insert(999usize);
            save.world_civilization
                .milestone_countries
                .insert(WorldStage::PreIndustrial, set);
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_building_type_in_construction_queue_is_rejected() {
        let issues = issues_for(|save| {
            save.countries[0].construction_queue.push(
                crate::building::construction::ConstructionQueueItem {
                    state_id: StateId(0),
                    building_type: BuildingType::Mine,
                    target_level: 1,
                    progress: 0.0,
                    required_progress: 10.0,
                    paid_cost: 0.0,
                    status: crate::building::construction::ConstructionStatus::InQueue,
                },
            );
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn building_level_exceeding_max_is_rejected() {
        let issues = issues_for(|save| {
            save.states[0].buildings.insert(BuildingType::Farm, 99);
        });
        assert!(has_code(&issues, SaveValidationCode::InvalidRange));
    }

    /// P21-009要求テスト項目23: 資源在庫のNaNはapply前に拒否される。
    #[test]
    fn nan_stockpile_amount_is_rejected() {
        let issues = issues_for(|save| {
            save.countries[0].stockpile.amounts.insert(
                crate::economy::resources::ResourceType::RawMagicCrystal,
                f64::NAN,
            );
        });
        assert!(has_code(&issues, SaveValidationCode::NonFiniteValue));
    }

    /// P21-009要求テスト項目23: 資源在庫のInfinityはapply前に拒否される。
    #[test]
    fn infinite_stockpile_amount_is_rejected() {
        let issues = issues_for(|save| {
            save.countries[0].stockpile.amounts.insert(
                crate::economy::resources::ResourceType::MagicCrystal,
                f64::INFINITY,
            );
        });
        assert!(has_code(&issues, SaveValidationCode::NonFiniteValue));
    }

    /// P21-009要求テスト項目23: 資源在庫の負数はapply前に拒否される。
    #[test]
    fn negative_stockpile_amount_is_rejected() {
        let issues = issues_for(|save| {
            save.countries[0].stockpile.amounts.insert(
                crate::economy::resources::ResourceType::RawMagicCrystal,
                -1.0,
            );
        });
        assert!(has_code(&issues, SaveValidationCode::InvalidRange));
    }

    /// P21-009要求テスト項目23: 正常な非負・有限の在庫値は拒否されない。
    #[test]
    fn valid_finite_non_negative_stockpile_amount_is_accepted() {
        let issues = issues_for(|save| {
            save.countries[0].stockpile.amounts.insert(
                crate::economy::resources::ResourceType::RawMagicCrystal,
                42.0,
            );
        });
        assert!(
            issues.is_empty(),
            "valid stockpile must be accepted, got {issues:?}"
        );
    }

    /// 新規建物種別(CrystalMine/CrystalRefinery)が静的マスターに存在しない場合、
    /// 既存のBuildingType dangling参照チェックへ自動的に乗ることを確認する。
    #[test]
    fn unknown_crystal_mine_building_reference_is_rejected() {
        let issues = issues_for(|save| {
            save.states[0]
                .buildings
                .insert(BuildingType::CrystalMine, 1);
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_recruitment_division_definition_is_rejected() {
        let issues = issues_for(|save| {
            save.countries[0].recruitment_queue.push(
                crate::military::recruitment::RecruitmentQueueItem {
                    division_id: DivisionDefinitionId(999),
                    target_state: StateId(0),
                    days_remaining: 5,
                    total_days: 10,
                },
            );
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_completed_technology_is_rejected() {
        let issues = issues_for(|save| {
            save.countries[0]
                .research_state
                .completed_technologies
                .insert("nonexistent_tech".to_string());
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_division_definition_is_rejected() {
        let issues = issues_for(|save| {
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .def_id = DivisionDefinitionId(999);
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    // ─── 外交・戦争 ──────────────────────────────────────────────────────────

    #[test]
    fn unknown_diplomatic_pair_country_is_rejected() {
        let issues = issues_for(|save| {
            let relation = save
                .diplomacy
                .relations
                .remove(&DiplomaticPairKey(CountryId(0), CountryId(1)))
                .unwrap();
            save.diplomacy
                .relations
                .insert(DiplomaticPairKey(CountryId(0), CountryId(999)), relation);
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unnormalized_diplomatic_pair_key_is_rejected() {
        let issues = issues_for(|save| {
            let relation = save
                .diplomacy
                .relations
                .remove(&DiplomaticPairKey(CountryId(0), CountryId(1)))
                .unwrap();
            save.diplomacy
                .relations
                .insert(DiplomaticPairKey(CountryId(1), CountryId(0)), relation);
        });
        assert!(has_code(&issues, SaveValidationCode::InvalidRange));
    }

    #[test]
    fn treaty_participant_mismatch_is_rejected() {
        let issues = issues_for(|save| {
            let relation = save
                .diplomacy
                .relations
                .get_mut(&DiplomaticPairKey(CountryId(0), CountryId(1)))
                .unwrap();
            relation.treaties[0].countries = (CountryId(0), CountryId(0));
        });
        assert!(has_code(&issues, SaveValidationCode::ParticipantMismatch));
    }

    #[test]
    fn active_activity_participant_mismatch_is_rejected() {
        let issues = issues_for(|save| {
            let relation = save
                .diplomacy
                .relations
                .get_mut(&DiplomaticPairKey(CountryId(0), CountryId(1)))
                .unwrap();
            relation.active_activity.as_mut().unwrap().target = CountryId(0);
        });
        assert!(has_code(&issues, SaveValidationCode::ParticipantMismatch));
    }

    #[test]
    fn unknown_war_participant_is_rejected() {
        let issues = issues_for(|save| {
            save.wars.wars.get_mut(&WarId(0)).unwrap().attackers =
                [CountryId(999)].into_iter().collect();
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn empty_attackers_is_rejected() {
        let issues = issues_for(|save| {
            save.wars.wars.get_mut(&WarId(0)).unwrap().attackers.clear();
        });
        assert!(has_code(&issues, SaveValidationCode::EmptyCollection));
    }

    #[test]
    fn empty_defenders_is_rejected() {
        let issues = issues_for(|save| {
            save.wars.wars.get_mut(&WarId(0)).unwrap().defenders.clear();
        });
        assert!(has_code(&issues, SaveValidationCode::EmptyCollection));
    }

    #[test]
    fn overlapping_attackers_and_defenders_is_rejected() {
        let issues = issues_for(|save| {
            save.wars
                .wars
                .get_mut(&WarId(0))
                .unwrap()
                .defenders
                .insert(CountryId(0));
        });
        assert!(has_code(&issues, SaveValidationCode::SetOverlap));
    }

    #[test]
    fn winner_not_a_participant_is_rejected() {
        let issues = issues_for(|save| {
            save.wars.wars.get_mut(&WarId(0)).unwrap().winner = Some(CountryId(999));
        });
        assert!(has_code(&issues, SaveValidationCode::ParticipantMismatch));
    }

    #[test]
    fn processed_battle_ids_may_reference_removed_battles() {
        // BattleRegistry::cleanup_finished_battlesが完了戦闘を削除する仕様のため、
        // 存在しないBattleIdをprocessed_battle_idsが指していても正常。
        let issues = issues_for(|save| {
            save.wars
                .wars
                .get_mut(&WarId(0))
                .unwrap()
                .processed_battle_ids
                .insert(BattleId(999));
        });
        assert!(
            issues.is_empty(),
            "stale processed_battle_ids must be accepted, got {issues:?}"
        );
    }

    #[test]
    fn unknown_war_justification_participant_is_rejected() {
        let issues = issues_for(|save| {
            save.war_justifications.justifications.insert(
                0,
                WarJustification {
                    id: 0,
                    initiator: CountryId(999),
                    target: CountryId(1),
                    target_state: StateId(1),
                    start_date: "1800/01/01".to_string(),
                    required_days: 30,
                    days_passed: 0,
                    is_ready: false,
                },
            );
            save.war_justifications.next_id = 1;
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_claim_reference_is_rejected() {
        let issues = issues_for(|save| {
            save.claims.claims.insert(
                ClaimId(0),
                TerritorialClaim {
                    id: ClaimId(0),
                    claimant_country: CountryId(999),
                    target_state: StateId(0),
                    strength: 10.0,
                    created_date: "1800/01/01".to_string(),
                    is_permanent: false,
                    source: ClaimSource::BorderDispute,
                },
            );
            save.claims.next_id = 1;
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_crisis_reference_is_rejected() {
        let issues = issues_for(|save| {
            save.crises.crises.insert(
                DiplomaticCrisisId(0),
                DiplomaticCrisis {
                    id: DiplomaticCrisisId(0),
                    initiator: CountryId(0),
                    target: CountryId(999),
                    war_goals: Vec::new(),
                    start_date: "1800/01/01".to_string(),
                    current_phase: CrisisPhase::Negotiating,
                    escalation: 0.0,
                    initiator_support: 0.0,
                    target_resistance: 0.0,
                    days_in_phase: 0,
                    deadline_date: None,
                    international_concern: 0.0,
                    third_party_reactions: HashMap::new(),
                },
            );
            save.crises.next_id = 1;
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    /// P21-010要求テスト項目26: initiator/targetが同一のCrisisは拒否される
    /// (`CrisisRegistry::can_start_crisis`が同じ不変条件をランタイム側でも検証している)。
    #[test]
    fn crisis_with_identical_initiator_and_target_is_rejected() {
        let issues = issues_for(|save| {
            save.crises.crises.insert(
                DiplomaticCrisisId(0),
                DiplomaticCrisis {
                    id: DiplomaticCrisisId(0),
                    initiator: CountryId(0),
                    target: CountryId(0),
                    war_goals: Vec::new(),
                    start_date: "1800/01/01".to_string(),
                    current_phase: CrisisPhase::Preparing,
                    escalation: 0.0,
                    initiator_support: 0.0,
                    target_resistance: 0.0,
                    days_in_phase: 0,
                    deadline_date: None,
                    international_concern: 0.0,
                    third_party_reactions: HashMap::new(),
                },
            );
            save.crises.next_id = 1;
        });
        assert!(has_code(&issues, SaveValidationCode::SetOverlap));
    }

    // ─── Registryのキー・次回ID ───────────────────────────────────────────────

    #[test]
    fn map_key_element_id_mismatch_is_rejected() {
        let issues = issues_for(|save| {
            let war = save.wars.wars.remove(&WarId(0)).unwrap();
            save.wars.wars.insert(WarId(1), war);
            save.wars.next_id = 2;
        });
        assert!(has_code(&issues, SaveValidationCode::MapKeyMismatch));
    }

    #[test]
    fn next_id_collision_is_rejected() {
        let issues = issues_for(|save| save.wars.next_id = 0);
        assert!(has_code(&issues, SaveValidationCode::NextIdCollision));
    }

    // ─── 師団・戦闘 ──────────────────────────────────────────────────────────

    #[test]
    fn unknown_division_owner_is_rejected() {
        let issues = issues_for(|save| {
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .owner = CountryId(999);
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_division_current_state_is_rejected() {
        let issues = issues_for(|save| {
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .current_state = StateId(999);
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_division_movement_path_entry_is_rejected() {
        let issues = issues_for(|save| {
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .current_path = vec![StateId(999)];
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn unknown_combat_id_is_rejected() {
        let issues = issues_for(|save| {
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .combat_id = Some(BattleId(999));
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn combat_id_not_pointing_back_from_battle_is_rejected() {
        let issues = issues_for(|save| {
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .status = DivisionStatus::Fighting;
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .combat_id = Some(BattleId(0));
            save.battles.battles.insert(
                BattleId(0),
                Battle {
                    id: BattleId(0),
                    war_id: WarId(0),
                    state_id: StateId(0),
                    attacker_country: CountryId(1),
                    defender_country: CountryId(0),
                    attacker_division_ids: Vec::new(),
                    defender_division_ids: Vec::new(), // Division(0)が参加者に含まれない
                    attacker_origins: HashMap::new(),
                    start_date: "1800/01/02".to_string(),
                    elapsed_days: 0,
                    status: BattleStatus::Ongoing,
                },
            );
            save.battles.next_id = 1;
        });
        assert!(has_code(
            &issues,
            SaveValidationCode::ReverseMapInconsistent
        ));
    }

    #[test]
    fn battle_participant_not_reflecting_division_combat_id_is_rejected() {
        let issues = issues_for(|save| {
            // Division(0)のcombat_idはNoneのまま、Battleの参加者リストにだけ載せる。
            save.battles.battles.insert(
                BattleId(0),
                Battle {
                    id: BattleId(0),
                    war_id: WarId(0),
                    state_id: StateId(0),
                    attacker_country: CountryId(0),
                    defender_country: CountryId(1),
                    attacker_division_ids: vec![DivisionId(0)],
                    defender_division_ids: Vec::new(),
                    attacker_origins: HashMap::new(),
                    start_date: "1800/01/02".to_string(),
                    elapsed_days: 0,
                    status: BattleStatus::Ongoing,
                },
            );
            save.battles.next_id = 1;
        });
        assert!(has_code(
            &issues,
            SaveValidationCode::ReverseMapInconsistent
        ));
    }

    #[test]
    fn duplicate_battle_participation_is_rejected() {
        let issues = issues_for(|save| {
            let other_division = make_division(1, CountryId(0), StateId(0));
            save.military
                .divisions
                .insert(other_division.id, other_division);
            save.military.next_division_id = 2;
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .combat_id = Some(BattleId(0));
            save.military
                .divisions
                .get_mut(&DivisionId(1))
                .unwrap()
                .combat_id = Some(BattleId(1));

            save.battles.battles.insert(
                BattleId(0),
                Battle {
                    id: BattleId(0),
                    war_id: WarId(0),
                    state_id: StateId(0),
                    attacker_country: CountryId(0),
                    defender_country: CountryId(1),
                    attacker_division_ids: vec![DivisionId(0), DivisionId(1)],
                    defender_division_ids: Vec::new(),
                    attacker_origins: HashMap::new(),
                    start_date: "1800/01/02".to_string(),
                    elapsed_days: 0,
                    status: BattleStatus::Ongoing,
                },
            );
            save.battles.battles.insert(
                BattleId(1),
                Battle {
                    id: BattleId(1),
                    war_id: WarId(0),
                    state_id: StateId(2),
                    attacker_country: CountryId(0),
                    defender_country: CountryId(1),
                    attacker_division_ids: vec![DivisionId(0)],
                    defender_division_ids: Vec::new(),
                    attacker_origins: HashMap::new(),
                    start_date: "1800/01/02".to_string(),
                    elapsed_days: 0,
                    status: BattleStatus::Ongoing,
                },
            );
            save.battles.next_id = 2;
        });
        assert!(has_code(&issues, SaveValidationCode::DuplicateMembership));
    }

    // ─── Army ────────────────────────────────────────────────────────────────

    #[test]
    fn unknown_army_owner_is_rejected() {
        let issues = issues_for(|save| {
            save.armies.armies.get_mut(&ArmyId(0)).unwrap().owner = CountryId(999);
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn empty_army_is_rejected() {
        let issues = issues_for(|save| {
            save.armies
                .armies
                .get_mut(&ArmyId(0))
                .unwrap()
                .member_division_ids
                .clear();
        });
        assert!(has_code(&issues, SaveValidationCode::EmptyCollection));
    }

    #[test]
    fn army_with_foreign_division_is_rejected() {
        let issues = issues_for(|save| {
            save.armies.armies.get_mut(&ArmyId(0)).unwrap().owner = CountryId(1);
        });
        assert!(has_code(&issues, SaveValidationCode::OwnershipMismatch));
    }

    #[test]
    fn division_double_army_membership_is_rejected() {
        let issues = issues_for(|save| {
            let second_army = Army {
                id: ArmyId(1),
                owner: CountryId(0),
                name: "Army 2".to_string(),
                member_division_ids: vec![DivisionId(0)],
            };
            save.armies.armies.insert(ArmyId(1), second_army);
            save.armies.next_id = 2;
        });
        assert!(has_code(&issues, SaveValidationCode::DuplicateMembership));
    }

    #[test]
    fn division_army_map_missing_reverse_entry_is_rejected() {
        let issues = issues_for(|save| {
            save.armies.division_army_map.remove(&DivisionId(0));
        });
        assert!(has_code(
            &issues,
            SaveValidationCode::ReverseMapInconsistent
        ));
    }

    #[test]
    fn division_army_map_pointing_to_unknown_division_is_rejected() {
        let issues = issues_for(|save| {
            save.armies
                .division_army_map
                .insert(DivisionId(999), ArmyId(0));
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn division_army_map_pointing_to_unknown_army_is_rejected() {
        let issues = issues_for(|save| {
            save.armies
                .division_army_map
                .insert(DivisionId(0), ArmyId(999));
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn army_naming_counter_collision_is_rejected() {
        let issues = issues_for(|save| {
            save.armies.next_army_number.insert(CountryId(0), 1);
        });
        assert!(has_code(&issues, SaveValidationCode::NextIdCollision));
    }

    // ─── AI ──────────────────────────────────────────────────────────────────

    #[test]
    fn country_ai_map_key_mismatch_is_rejected() {
        let issues = issues_for(|save| {
            let state = save.country_ai.ai_states.remove(&CountryId(1)).unwrap();
            save.country_ai.ai_states.insert(CountryId(0), state);
        });
        assert!(has_code(&issues, SaveValidationCode::MapKeyMismatch));
    }

    #[test]
    fn military_ai_map_key_mismatch_is_rejected() {
        let issues = issues_for(|save| {
            let state = save.military_ai.ai_states.remove(&CountryId(1)).unwrap();
            save.military_ai.ai_states.insert(CountryId(0), state);
        });
        assert!(has_code(&issues, SaveValidationCode::MapKeyMismatch));
    }

    #[test]
    fn ai_state_referencing_unknown_country_is_rejected() {
        let issues = issues_for(|save| {
            let mut state = save.country_ai.ai_states.remove(&CountryId(1)).unwrap();
            state.country_id = CountryId(999);
            save.country_ai.ai_states.insert(CountryId(999), state);
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    // ─── Frontline ───────────────────────────────────────────────────────────

    fn insert_valid_frontline(save: &mut SaveGameV1) {
        save.frontlines.frontlines.insert(
            FrontlineId(0),
            Frontline {
                frontline_id: FrontlineId(0),
                war_id: WarId(0),
                attacker_country_id: CountryId(0),
                defender_country_id: CountryId(1),
                attacker_front_regions: vec![StateId(0)],
                defender_front_regions: vec![StateId(1)],
                border_region_pairs: vec![(StateId(0), StateId(1))],
            },
        );
        save.frontlines.plans.insert(
            (FrontlineId(0), CountryId(0)),
            FrontlinePlan {
                frontline_id: FrontlineId(0),
                commanding_country_id: CountryId(0),
                stance: FrontlineStance::Defend,
                objective_region_id: Some(StateId(1)),
                assigned_division_ids: vec![DivisionId(0)],
                // ArmyId(0)はbaseline_save()自体がCountryId(0)所有として用意する既存Army。
                assigned_army_ids: vec![ArmyId(0)],
                offensive_line_region_ids: None,
            },
        );
        save.frontlines
            .division_frontline_map
            .insert(DivisionId(0), FrontlineId(0));
        save.frontlines
            .army_frontline_map
            .insert(ArmyId(0), FrontlineId(0));
        save.frontlines.next_frontline_id = 1;
    }

    #[test]
    fn valid_frontline_setup_is_accepted() {
        let issues = issues_for(insert_valid_frontline);
        assert!(
            issues.is_empty(),
            "valid frontline setup must be accepted, got {issues:?}"
        );
    }

    #[test]
    fn frontline_plan_wrong_owner_division_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .owner = CountryId(1);
        });
        assert!(has_code(&issues, SaveValidationCode::OwnershipMismatch));
    }

    #[test]
    fn frontline_plan_non_participant_commander_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            // Frontline(0)の参加国はCountryId(0)/CountryId(1)だけなので、
            // 「既知の国だがこの前線には参加していない」を作るため第3国を用意する。
            save.countries.push(CountryData {
                id: CountryId(2),
                capital_state_id: StateId(3),
                government_type: GovernmentType::Monarchy,
                economic_system: EconomicSystem::FreeMarket,
                ..CountryData::default()
            });
            save.states.push(StateData {
                id: StateId(3),
                owner_country_id: CountryId(2),
                ..StateData::default()
            });
            let plan = save
                .frontlines
                .plans
                .remove(&(FrontlineId(0), CountryId(0)))
                .unwrap();
            save.frontlines.plans.insert(
                (FrontlineId(0), CountryId(2)),
                FrontlinePlan {
                    commanding_country_id: CountryId(2),
                    assigned_division_ids: Vec::new(),
                    assigned_army_ids: Vec::new(),
                    ..plan
                },
            );
        });
        assert!(has_code(&issues, SaveValidationCode::ParticipantMismatch));
    }

    #[test]
    fn division_frontline_map_reverse_mismatch_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.frontlines
                .plans
                .get_mut(&(FrontlineId(0), CountryId(0)))
                .unwrap()
                .assigned_division_ids
                .clear();
        });
        assert!(has_code(
            &issues,
            SaveValidationCode::ReverseMapInconsistent
        ));
    }

    #[test]
    fn frontline_generated_movement_referencing_unknown_division_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.frontlines
                .frontline_generated_movements
                .insert(DivisionId(999));
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    // ─── P21-005: Army↔Frontline割当 ────────────────────────────────────────

    #[test]
    fn army_frontline_map_dangling_army_id_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.frontlines
                .army_frontline_map
                .insert(ArmyId(999), FrontlineId(0));
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn army_frontline_map_dangling_frontline_id_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.frontlines
                .army_frontline_map
                .insert(ArmyId(0), FrontlineId(999));
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn army_frontline_map_reverse_mismatch_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.frontlines
                .plans
                .get_mut(&(FrontlineId(0), CountryId(0)))
                .unwrap()
                .assigned_army_ids
                .clear();
        });
        assert!(has_code(
            &issues,
            SaveValidationCode::ReverseMapInconsistent
        ));
    }

    /// P21-005要求テスト項目48: assigned_army_idsが存在しないArmyIdを参照する場合、拒否される。
    #[test]
    fn frontline_plan_assigned_army_unknown_id_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.frontlines
                .plans
                .get_mut(&(FrontlineId(0), CountryId(0)))
                .unwrap()
                .assigned_army_ids
                .push(ArmyId(999));
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    /// P21-005要求テスト項目49: Armyの所有者とPlanのcommanding_country_idが一致しない場合、拒否される。
    #[test]
    fn frontline_plan_assigned_army_wrong_owner_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.armies.armies.get_mut(&ArmyId(0)).unwrap().owner = CountryId(1);
        });
        assert!(has_code(&issues, SaveValidationCode::OwnershipMismatch));
    }

    /// P21-005要求テスト項目50: 同じArmyが複数のFrontlineへ割り当てられている(assigned_army_ids
    /// 側で二重登録されている)データは拒否される。
    #[test]
    fn army_assigned_to_two_frontlines_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            let second_frontline = crate::war::frontline::Frontline {
                frontline_id: FrontlineId(1),
                war_id: WarId(0),
                attacker_country_id: CountryId(0),
                defender_country_id: CountryId(1),
                attacker_front_regions: vec![StateId(0)],
                defender_front_regions: vec![StateId(1)],
                border_region_pairs: vec![(StateId(0), StateId(1))],
            };
            save.frontlines
                .frontlines
                .insert(FrontlineId(1), second_frontline);
            save.frontlines.plans.insert(
                (FrontlineId(1), CountryId(0)),
                FrontlinePlan {
                    frontline_id: FrontlineId(1),
                    commanding_country_id: CountryId(0),
                    stance: FrontlineStance::Defend,
                    objective_region_id: None,
                    assigned_division_ids: Vec::new(),
                    // ArmyId(0)はFrontlineId(0)の方にも既に割り当てられている(二重登録)。
                    assigned_army_ids: vec![ArmyId(0)],
                    offensive_line_region_ids: None,
                },
            );
            save.frontlines.next_frontline_id = 2;
        });
        assert!(has_code(&issues, SaveValidationCode::DuplicateMembership));
    }

    // ─── 攻勢線(P21-007) ────────────────────────────────────────────────────

    /// `insert_valid_frontline`が作るFrontline(0)は攻撃側CountryId(0)/防御側
    /// CountryId(1)。CountryId(0)の計画から見た敵支配陸地はStateId(1)/StateId(2)
    /// (いずれもbaseline_saveでCountryId(1)所有、StateId(1)-StateId(2)間に隣接あり)。
    fn set_offensive_line_on_plan(save: &mut SaveGameV1, line: Option<Vec<StateId>>) {
        save.frontlines
            .plans
            .get_mut(&(FrontlineId(0), CountryId(0)))
            .unwrap()
            .offensive_line_region_ids = line;
    }

    #[test]
    fn offensive_line_single_valid_region_is_accepted() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            set_offensive_line_on_plan(save, Some(vec![StateId(1)]));
        });
        assert!(
            issues.is_empty(),
            "single valid enemy-controlled land region must be accepted, got {issues:?}"
        );
    }

    #[test]
    fn offensive_line_connected_multi_region_is_accepted() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            set_offensive_line_on_plan(save, Some(vec![StateId(1), StateId(2)]));
        });
        assert!(
            issues.is_empty(),
            "connected multi-region offensive line must be accepted, got {issues:?}"
        );
    }

    #[test]
    fn offensive_line_some_empty_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            set_offensive_line_on_plan(save, Some(Vec::new()));
        });
        assert!(has_code(&issues, SaveValidationCode::EmptyCollection));
    }

    #[test]
    fn offensive_line_duplicate_region_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            set_offensive_line_on_plan(save, Some(vec![StateId(1), StateId(1)]));
        });
        assert!(has_code(&issues, SaveValidationCode::DuplicateId));
    }

    #[test]
    fn offensive_line_unknown_region_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            set_offensive_line_on_plan(save, Some(vec![StateId(999)]));
        });
        assert!(has_code(&issues, SaveValidationCode::DanglingReference));
    }

    #[test]
    fn offensive_line_sea_region_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.states.push(StateData {
                id: StateId(4),
                owner_country_id: CountryId(1),
                is_sea: true,
                ..StateData::default()
            });
            set_offensive_line_on_plan(save, Some(vec![StateId(4)]));
        });
        assert!(has_code(&issues, SaveValidationCode::NonLandRegion));
    }

    /// P21-008要求テスト項目21: 攻勢を実行した結果、攻勢線の全Stateが自国支配(占領)へ
    /// 変化したセーブは、引き続き正当な攻勢線として受理されなければならない
    /// (P21-007時点の「自国領は常に不正」という主張を、この仕様変更に合わせて置き換える。
    /// `controller_country`だけを変更し`owner_country_id`は敵国のまま維持する — これは
    /// このゲームの占領モデル[`StateData::controller()`が`controller_country.
    /// unwrap_or(owner_country_id)`]に合わせた、法的所有権と実効支配を区別する表現)。
    #[test]
    fn offensive_line_fully_captured_is_accepted() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            set_offensive_line_on_plan(save, Some(vec![StateId(1)]));
            let state = save.states.iter_mut().find(|s| s.id == StateId(1)).unwrap();
            state.controller_country = Some(CountryId(0));
        });
        assert!(
            issues.is_empty(),
            "a fully-captured (own-controlled) offensive line must be accepted, got {issues:?}"
        );
    }

    /// P21-008要求テスト項目20: 攻勢線の一部だけが自国支配へ変化した(残りは引き続き敵支配の
    /// ままの)セーブも受理されなければならない。State1(確保済み)とState2(未確保、
    /// 引き続き敵支配)は`insert_valid_frontline`のbaseline上で隣接しているため、
    /// 連結性検証も同時に満たす。
    #[test]
    fn offensive_line_partially_captured_is_accepted() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            set_offensive_line_on_plan(save, Some(vec![StateId(1), StateId(2)]));
            let state = save.states.iter_mut().find(|s| s.id == StateId(1)).unwrap();
            state.controller_country = Some(CountryId(0));
            // StateId(2)はbaseline_save()のままCountryId(1)(敵国)支配を維持する。
        });
        assert!(
            issues.is_empty(),
            "a partially-captured offensive line must be accepted, got {issues:?}"
        );
    }

    #[test]
    fn offensive_line_third_party_territory_is_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            save.countries.push(CountryData {
                id: CountryId(2),
                capital_state_id: StateId(4),
                government_type: GovernmentType::Monarchy,
                economic_system: EconomicSystem::FreeMarket,
                ..CountryData::default()
            });
            save.states.push(StateData {
                id: StateId(4),
                owner_country_id: CountryId(2),
                ..StateData::default()
            });
            set_offensive_line_on_plan(save, Some(vec![StateId(4)]));
        });
        assert!(has_code(&issues, SaveValidationCode::OwnershipMismatch));
    }

    #[test]
    fn offensive_line_disconnected_regions_are_rejected() {
        let issues = issues_for(|save| {
            insert_valid_frontline(save);
            // StateId(4): CountryId(1)所有の陸地だが、StateId(1)ともStateId(2)とも
            // 隣接しない(孤立した敵地域)。
            save.states.push(StateData {
                id: StateId(4),
                owner_country_id: CountryId(1),
                neighbors: Vec::new(),
                ..StateData::default()
            });
            set_offensive_line_on_plan(save, Some(vec![StateId(1), StateId(4)]));
        });
        assert!(has_code(&issues, SaveValidationCode::DisconnectedRegionSet));
    }

    // ─── 安全性 ──────────────────────────────────────────────────────────────

    #[test]
    fn multiple_issues_are_collected_from_a_single_bad_save() {
        let issues = issues_for(|save| {
            save.player_country = None;
            save.game_speed = 0;
            save.countries[0].capital_state_id = StateId(999);
        });
        assert!(
            issues.len() >= 3,
            "expected multiple collected issues, got {issues:?}"
        );
    }

    #[test]
    fn issue_order_is_stable_across_repeated_runs() {
        let statics = static_data();
        let mut save = baseline_save();
        save.player_country = None;
        save.game_speed = 0;
        save.countries[0].capital_state_id = StateId(999);

        let first = validate_save_game_v1(save.clone(), &statics.context())
            .err()
            .unwrap()
            .issues;
        let second = validate_save_game_v1(save, &statics.context())
            .err()
            .unwrap()
            .issues;
        assert_eq!(
            first, second,
            "repeated validation of the same data must report issues in the same order"
        );
    }

    #[test]
    fn validation_does_not_mutate_the_context_static_data() {
        let statics = static_data();
        let building_count_before = statics.buildings.len();
        let tech_count_before = statics.technologies.len();
        let save = baseline_save();
        let _ = validate_save_game_v1(save, &statics.context());
        assert_eq!(statics.buildings.len(), building_count_before);
        assert_eq!(statics.technologies.len(), tech_count_before);
    }

    #[test]
    fn validation_never_panics_on_heavily_malformed_data() {
        let issues = issues_for(|save| {
            save.player_country = Some(CountryId(999));
            save.countries[0].capital_state_id = StateId(999);
            save.states[0].neighbors.push(StateId(999));
            save.wars.wars.get_mut(&WarId(0)).unwrap().attackers.clear();
            save.wars.wars.get_mut(&WarId(0)).unwrap().defenders.clear();
            save.armies
                .armies
                .get_mut(&ArmyId(0))
                .unwrap()
                .member_division_ids
                .clear();
            save.military
                .divisions
                .get_mut(&DivisionId(0))
                .unwrap()
                .def_id = DivisionDefinitionId(999);
            save.frontlines
                .frontline_generated_movements
                .insert(DivisionId(999));
        });
        assert!(
            !issues.is_empty(),
            "expected the pervasively broken save to be rejected"
        );
    }

    /// RONの`HashMap`に同一キーが複数回記述された場合の実際のDeserialize挙動を確認する。
    /// この結果次第で「重複キー検出」を検証項目として主張してよいかが決まるため、
    /// 実際の挙動を確認せず「検出済み」と報告しないよう、ここで直接検証する。
    #[test]
    fn duplicate_hashmap_keys_in_ron_are_silently_overwritten_by_the_last_value() {
        let ron_str = "{0: 1, 0: 2}";
        let parsed: std::collections::HashMap<u32, u32> =
            ron::from_str(ron_str).expect("duplicate-keyed RON map must still parse");
        assert_eq!(
            parsed.len(),
            1,
            "duplicate keys collapse into a single entry (serde/ron does not reject duplicates)"
        );
        assert_eq!(
            parsed.get(&0),
            Some(&2),
            "the later value silently wins; the earlier one is lost before validate.rs ever sees it"
        );
        // この制約により、SaveGameV1のHashMapフィールドに対する「重複キーの検出」は
        // 構造的に不可能である(Deserialize時点で既に1件へ収束しているため)。
        // このラウンドではV1のMapをVecへ変更する等のスキーマ変更は行わない
        // (要求仕様: 「この問題のためだけにV1のMapをVecへ変更しない」)。
    }
}
