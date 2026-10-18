//! 型付きID。異なる種類のIDを取り違えるとコンパイルエラーになる(`From`/`Deref`は用意しない)。
//! 旧`common::StateId(usize)`とは別型で、相互変換も提供しない(新旧は同じ意味ではない)。

use serde::{Deserialize, Serialize};
use std::fmt;

/// プロヴィンスの安定ID。ゲーム側のプロヴィンス表に保存された値で、座標・配列位置・
/// 列挙順からは決まらない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProvinceId(pub u32);

/// 新しいステート(プロヴィンスの集合)のID。旧`StateId`とは無関係。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MapStateId(pub u32);

/// ヘックスの軸座標。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HexCoord {
    pub q: i32,
    pub r: i32,
}

impl HexCoord {
    pub fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }
}

impl fmt::Display for ProvinceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Province#{}", self.0)
    }
}
impl fmt::Display for MapStateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MapState#{}", self.0)
    }
}
impl fmt::Display for HexCoord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(q={}, r={})", self.q, self.r)
    }
}
