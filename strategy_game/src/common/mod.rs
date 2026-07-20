/// 共通の型定義モジュール
/// 型安全なID型とその他共通ユーティリティを提供する
use serde::{Deserialize, Serialize};

/// 国家を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CountryId(pub usize);

/// 州を一意に識別するID型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StateId(pub usize);
