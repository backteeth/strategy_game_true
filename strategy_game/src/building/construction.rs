use crate::building::data::BuildingType;
use crate::common::StateId;
use serde::{Deserialize, Serialize};

pub const REFUND_RATIO: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstructionStatus {
    InQueue,
    InProgress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstructionQueueItem {
    pub state_id: StateId,
    pub building_type: BuildingType,
    pub target_level: u32,
    pub progress: f64,
    pub required_progress: f64,
    pub paid_cost: f64,
    pub status: ConstructionStatus,
}

/// 日次の建設進捗に使う、注文の共通の動作(旧`ConstructionQueueItem`と、新ステート用の注文の
/// 両方が同じ進捗・完成判定を通る)。
pub trait ConstructionOrder {
    /// 1日分の進捗を進め(状態を`InProgress`にし、進捗を+1)、完成条件を満たしたら`true`。
    fn advance_one_day(&mut self) -> bool;
}

impl ConstructionOrder for ConstructionQueueItem {
    fn advance_one_day(&mut self) -> bool {
        self.status = ConstructionStatus::InProgress;
        self.progress += 1.0;
        self.progress >= self.required_progress
    }
}

/// 完成した注文の反映結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionOutcome {
    /// 施設を反映した(または反映済みとして扱う)。注文をキューから外す。
    Applied,
    /// 完成の前提が満たされず反映できない。**施設も費用も変更せず**、注文をキューへ残す
    /// (次の日に再評価する。返金や取消しのルールは追加しない)。
    Blocked,
}

/// 建設キューを1日進める共通処理。全注文を1日進め、完成した注文ごとに`on_complete`を
/// **ちょうど1回**呼ぶ。`Applied`の注文だけをキューから外す。戻り値は外した件数。
/// (旧`handle_daily_construction`の進め方と同一: 先に全注文を進め、完成した注文を昇順に反映し、
/// 外すのは後ろから。)
pub fn advance_construction_queue<I: ConstructionOrder>(
    queue: &mut Vec<I>,
    mut on_complete: impl FnMut(&I) -> CompletionOutcome,
) -> usize {
    let mut remove = Vec::new();
    for (idx, item) in queue.iter_mut().enumerate() {
        if item.advance_one_day() && on_complete(item) == CompletionOutcome::Applied {
            remove.push(idx);
        }
    }
    let n = remove.len();
    for idx in remove.into_iter().rev() {
        queue.remove(idx);
    }
    n
}

// ---------------------------------------------------------------- 手動建設注文の共通規則(P32-013)

/// 手動の建設注文を拒否する理由。旧`ui::state_panel`の受付と、新ステート(`MapStateId`)の受付が
/// **同じ判定**を通る。判定の順序は旧UIの実際の順序(定義 → クリスタル鉱床 → クリスタル専用州 → 上限 →
/// 重複 → 資金)のまま。注文主体と対象の対応(所有・経済計算上の対応)は、呼び出し側が先に確認する。
#[derive(Debug, Clone, PartialEq)]
pub enum ManualBuildRejection {
    /// 建物定義が無い。
    UndefinedBuilding,
    /// クリスタル採掘施設を、発見済みのクリスタル鉱床が無い地域へ。
    NoCrystalDeposit,
    /// 通常のMineを、クリスタル専用地域(通常の採掘対象の鉱床が無い)へ。
    CrystalOnlyState,
    /// 建設上限に達している。
    MaxLevel { level: u32, max: u32 },
    /// 同じ地域の同じ施設が既に建設キューにある。
    AlreadyQueued,
    /// 国庫が建設費に満たない。
    InsufficientFunds { cost: f64, treasury: f64 },
}

/// 手動建設注文の受付判定(旧UIの検証の本体。UIはここを呼ぶ)。通れば建物定義を返す。
/// 支払い・キュー登録はしない(呼び出し側が、検証が全部通ってから1回だけ行う)。
pub fn check_manual_build_order<'a>(
    def: Option<&'a crate::building::data::BuildingDefinition>,
    building: BuildingType,
    deposits: &[crate::economy::resources::StateResourceDeposit],
    current_level: u32,
    already_queued: bool,
    treasury: f64,
) -> Result<&'a crate::building::data::BuildingDefinition, ManualBuildRejection> {
    use crate::economy::resources::{ResourceType, has_discovered_deposit, is_crystal_only_state};
    let def = def.ok_or(ManualBuildRejection::UndefinedBuilding)?;
    if building.requires_magic_crystal_deposit()
        && !has_discovered_deposit(deposits, ResourceType::MagicCrystal)
    {
        return Err(ManualBuildRejection::NoCrystalDeposit);
    }
    // P21-009-FIX-001: クリスタル専用地域には通常Mineの対象鉱床が無いため新規建設を許可しない。
    if building == BuildingType::Mine && is_crystal_only_state(deposits) {
        return Err(ManualBuildRejection::CrystalOnlyState);
    }
    if current_level >= def.max_level {
        return Err(ManualBuildRejection::MaxLevel {
            level: current_level,
            max: def.max_level,
        });
    }
    if already_queued {
        return Err(ManualBuildRejection::AlreadyQueued);
    }
    if treasury < def.construction_cost {
        return Err(ManualBuildRejection::InsufficientFunds {
            cost: def.construction_cost,
            treasury,
        });
    }
    Ok(def)
}

/// キャンセル時の返金額(旧UIの規則): 支払済み費用 × `REFUND_RATIO`。進捗・状態(待機中/建設中)・
/// 端数に依存せず、丸めもしない(表示だけが整数に丸められる)。
pub fn cancel_refund(paid_cost: f64) -> f64 {
    paid_cost * REFUND_RATIO
}
