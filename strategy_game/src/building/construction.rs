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
