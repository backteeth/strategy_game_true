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
