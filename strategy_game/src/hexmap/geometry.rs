//! P32-001: ヘックス座標系・ワールド座標変換・逆変換(セル選択)。
//!
//! 座標系は3つ:
//! - 軸座標 `(q, r)`: pointy-top。JSONのまま。
//! - 画像座標 `(x, y)`: 制作ツールの座標。Y下向き。`origin`/`size`はこの座標系。
//! - ワールド座標: Bevy。Y上向き。**画像→ワールドのY反転は`image_to_world`/`world_to_image`の
//!   2関数だけ**で行う(拡大率は1:1、表示原点も画像原点のまま)。
//!
//! 変換式(制作ツールの標準式。ツールのソースは未入手のため、実JSONの全セルが
//! 画像矩形に収まること＝`image_overflow_cells`で整合を確認する):
//!   x = origin.x + √3 · size · (q + r/2)
//!   y = origin.y + 1.5 · size · r

use super::data::{HexGrid, HexMap};

const SQRT3: f64 = 1.732_050_807_568_877_2;

/// 画像座標(Y下向き)→ワールド座標(Y上向き)。
pub fn image_to_world(x: f64, y: f64) -> (f64, f64) {
    (x, -y)
}

/// ワールド座標→画像座標。`image_to_world`の逆。
pub fn world_to_image(x: f64, y: f64) -> (f64, f64) {
    (x, -y)
}

impl HexGrid {
    /// セル中心(画像座標)。
    pub fn center_image(&self, q: i32, r: i32) -> (f64, f64) {
        let (q, r) = (q as f64, r as f64);
        (
            self.origin_x + SQRT3 * self.size * (q + r / 2.0),
            self.origin_y + 1.5 * self.size * r,
        )
    }

    /// セル中心(ワールド座標)。
    pub fn center_world(&self, q: i32, r: i32) -> (f64, f64) {
        let (x, y) = self.center_image(q, r);
        image_to_world(x, y)
    }

    /// 頂点`i`(0..6、画像座標)。pointy-topなので頂点は上下に尖り、角度 `60°·i − 30°`
    /// (画像座標のY下向きで時計回り)に並ぶ。
    pub fn corner_image(&self, q: i32, r: i32, i: usize) -> (f64, f64) {
        let (cx, cy) = self.center_image(q, r);
        let a = (60.0 * i as f64 - 30.0).to_radians();
        (cx + self.size * a.cos(), cy + self.size * a.sin())
    }

    /// 頂点`i`(ワールド座標)。
    pub fn corner_world(&self, q: i32, r: i32, i: usize) -> (f64, f64) {
        let (x, y) = self.corner_image(q, r, i);
        image_to_world(x, y)
    }

    /// 画像座標の点を含むセルの軸座標(セルの存在とは無関係)。
    ///
    /// 丸めはキューブ座標の標準手順(各成分を四捨五入し、丸め誤差が最大の成分を
    /// 残り2成分から再計算)。辺・頂点ちょうどの点は、この規則で決定的に1セルへ割り当てる。
    pub fn axial_from_image(&self, x: f64, y: f64) -> (i32, i32) {
        let px = x - self.origin_x;
        let py = y - self.origin_y;
        let qf = (SQRT3 / 3.0 * px - py / 3.0) / self.size;
        let rf = (2.0 / 3.0 * py) / self.size;
        cube_round(qf, rf)
    }

    pub fn axial_from_world(&self, x: f64, y: f64) -> (i32, i32) {
        let (ix, iy) = world_to_image(x, y);
        self.axial_from_image(ix, iy)
    }
}

fn cube_round(qf: f64, rf: f64) -> (i32, i32) {
    let sf = -qf - rf;
    let (mut q, mut r, s) = (qf.round(), rf.round(), sf.round());
    let (dq, dr, ds) = ((q - qf).abs(), (r - rf).abs(), (s - sf).abs());
    if dq > dr && dq > ds {
        q = -r - s;
    } else if dr > ds {
        r = -q - s;
    }
    (q as i32, r as i32)
}

/// ワールド座標の点にあるセルの配列インデックス。非有限入力・欠損セル・地図外は`None`。
/// 全セル走査はせず、逆変換→密な検索表の引きだけで求める。
pub fn pick_cell_world(map: &HexMap, x: f64, y: f64) -> Option<u32> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let (q, r) = map.grid.axial_from_world(x, y);
    map.cell_index(q, r)
}

/// 全セルの頂点を含む外接矩形(ワールド座標)`(min_x, min_y, max_x, max_y)`。
/// 初回表示・全体表示の基準。セルが無ければ`None`。
pub fn world_bounds(map: &HexMap) -> Option<(f64, f64, f64, f64)> {
    let mut b: Option<(f64, f64, f64, f64)> = None;
    for c in map.cells() {
        for i in [0usize, 1, 2, 3, 4, 5] {
            let (x, y) = map.grid.corner_world(c.q, c.r, i);
            b = Some(match b {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    b
}

/// 頂点が画像矩形`[0,w]×[0,h]`をはみ出すセルの数(診断用。セルは削らない)。
pub fn image_overflow_cells(map: &HexMap) -> usize {
    let (w, h) = (map.grid.image_width as f64, map.grid.image_height as f64);
    map.cells()
        .iter()
        .filter(|c| {
            (0..6).any(|i| {
                let (x, y) = map.grid.corner_image(c.q, c.r, i);
                x < -1e-6 || y < -1e-6 || x > w + 1e-6 || y > h + 1e-6
            })
        })
        .count()
}
