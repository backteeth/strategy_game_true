//! 地理データ(`HexMap`)の識別情報。ゲーム側ファイルへ「同期時点の地理」を記録し、
//! 制作ツールの再出力後に何が変わったかを区別して検出するために使う。
//!
//! 5 種類を別々に計算する(いずれも**入力配列の順序・JSONの整形に依存しない**):
//! - `grid`: orientation / size / origin / 画像寸法 / extent。
//! - `coordinates`: 全セル座標の集合(水域を含む)。
//! - `land_set`: 陸セル(水域でないセル)の座標集合。**座標集合が同じでも陸↔水の変更で変わる**。
//! - `attributes`: 各セルの地形・気候・植生(座標順)。
//! - `rivers`: 正規化済み河川の辺。
//!
//! 正規化: セルは`(r, q)`昇順に整列してからバイト列化する。ハッシュは自前の FNV-1a 64bit
//! (標準の`DefaultHasher`はバージョン間で値が変わりうるため使わない)。数値は固定長
//! リトルエンディアン、文字列は長さ前置。各種別に別々のタグを前置して衝突を避ける。
//! `HashMap`の列挙順には一切依存しない。これらは改ざん検知ではなく、変更検出用の識別情報。

use crate::hexmap::data::HexMap;
use serde::{Deserialize, Serialize};

/// ハッシュ手順の版。手順を変えたら増やし、保存値との比較は不一致扱いにする。
pub const FINGERPRINT_ALGORITHM: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeographyFingerprint {
    pub algorithm: u32,
    pub grid: String,
    pub coordinates: String,
    pub land_set: String,
    pub attributes: String,
    pub rivers: String,
}

/// 保存された識別情報と現在の地理データの比較結果(種別ごとの変化の有無)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GeographyChange {
    pub algorithm_mismatch: bool,
    pub grid: bool,
    pub coordinates: bool,
    pub land_set: bool,
    pub attributes: bool,
    pub rivers: bool,
}

impl GeographyChange {
    pub fn any(&self) -> bool {
        self.algorithm_mismatch
            || self.grid
            || self.coordinates
            || self.land_set
            || self.attributes
            || self.rivers
    }
}

impl GeographyFingerprint {
    pub fn compare(&self, current: &GeographyFingerprint) -> GeographyChange {
        if self.algorithm != current.algorithm {
            return GeographyChange {
                algorithm_mismatch: true,
                ..Default::default()
            };
        }
        GeographyChange {
            algorithm_mismatch: false,
            grid: self.grid != current.grid,
            coordinates: self.coordinates != current.coordinates,
            land_set: self.land_set != current.land_set,
            attributes: self.attributes != current.attributes,
            rivers: self.rivers != current.rivers,
        }
    }
}

struct Fnv(u64);

impl Fnv {
    fn new(tag: &str) -> Self {
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        h.str(tag);
        h
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }
    fn f64(&mut self, v: f64) {
        // -0.0 と 0.0 を同一視する。
        let v = if v == 0.0 { 0.0 } else { v };
        self.bytes(&v.to_bits().to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.bytes(s.as_bytes());
    }
    fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}

/// `(r, q)`昇順に整列したセルのインデックス(`HexMap`内の並び順に依存しない)。
fn sorted_cell_indices(map: &HexMap) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..map.cells().len()).collect();
    idx.sort_by_key(|&i| {
        let c = &map.cells()[i];
        (c.r, c.q)
    });
    idx
}

pub fn compute_fingerprint(map: &HexMap) -> GeographyFingerprint {
    let order = sorted_cell_indices(map);

    let mut grid = Fnv::new("p32-004/grid");
    let g = &map.grid;
    grid.str(crate::hexmap::data::SUPPORTED_ORIENTATION);
    grid.f64(g.size);
    grid.f64(g.origin_x);
    grid.f64(g.origin_y);
    grid.u32(g.image_width);
    grid.u32(g.image_height);
    grid.i32(g.extent.min_q);
    grid.i32(g.extent.max_q);
    grid.i32(g.extent.min_r);
    grid.i32(g.extent.max_r);

    let mut coords = Fnv::new("p32-004/coordinates");
    let mut land = Fnv::new("p32-004/land_set");
    let mut attrs = Fnv::new("p32-004/attributes");
    for &i in &order {
        let c = &map.cells()[i];
        coords.i32(c.q);
        coords.i32(c.r);
        if !c.terrain.is_water() {
            land.i32(c.q);
            land.i32(c.r);
        }
        attrs.i32(c.q);
        attrs.i32(c.r);
        attrs.str(c.terrain.id());
        attrs.str(c.climate.id());
        attrs.str(c.vegetation.id());
    }

    let mut rivers = Fnv::new("p32-004/rivers");
    // `river_edges`は正規化済み(両側表現を統合)で、(q,r,direction)昇順。
    for e in map.river_edges() {
        rivers.i32(e.q);
        rivers.i32(e.r);
        rivers.u8(e.direction);
    }

    GeographyFingerprint {
        algorithm: FINGERPRINT_ALGORITHM,
        grid: grid.finish(),
        coordinates: coords.finish(),
        land_set: land.finish(),
        attributes: attrs.finish(),
        rivers: rivers.finish(),
    }
}
