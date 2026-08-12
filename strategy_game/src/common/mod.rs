/// 共通の型定義モジュール
/// 型安全なID型とその他共通ユーティリティを提供する
use serde::{Deserialize, Serialize};

/// 国家を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CountryId(pub usize);

/// 州を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StateId(pub usize);

/// 軍隊を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArmyId(pub usize);

/// 師団・部隊定義を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DivisionId(pub usize);

/// 戦争を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WarId(pub usize);

/// 外交危機を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DiplomaticCrisisId(pub usize);

/// 領土請求を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClaimId(pub usize);

/// 条約を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TreatyId(pub usize);

/// 陸上戦闘を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BattleId(pub usize);

/// 前線を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrontlineId(pub usize);

/// 編成(複数師団の永続的な集合、いわゆる「軍」)を一意に識別するID型。
/// 既存の`ArmyId`(1師団)と紛らわしいため、複数師団の集合は「ArmyGroup」と呼ぶ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArmyGroupId(pub usize);
