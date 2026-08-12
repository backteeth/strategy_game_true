/// 共通の型定義モジュール
/// 型安全なID型とその他共通ユーティリティを提供する
use serde::{Deserialize, Serialize};

/// 国家を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CountryId(pub usize);

/// 州を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StateId(pub usize);

/// 個別師団インスタンスを一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DivisionId(pub usize);

/// 師団・部隊定義(テンプレート)を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DivisionDefinitionId(pub usize);

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

/// 軍(複数師団の永続的な集合)を一意に識別するID型。
/// 個別師団は`DivisionId`。複数の軍を束ねる上位階層(軍集団)を将来追加する場合は
/// 「ArmyGroup」の名称をそちらのために予約し、この型とは別名にすること
/// (P21-004R: この階層名は現時点で唯一のもの)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArmyId(pub usize);
