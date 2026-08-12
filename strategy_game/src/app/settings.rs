/// ゲーム設定を保持するResourceモジュール
use bevy::prelude::*;

/// カメラ移動・ズームに関する設定
#[derive(Resource, Debug)]
pub struct CameraSettings {
    /// キーボード移動速度（ピクセル/秒）
    pub move_speed: f32,
    /// ズーム速度（1スクロール当たりの倍率変化）
    pub zoom_speed: f32,
    /// 最小ズームスケール（拡大上限: 値が小さいほど拡大）
    pub min_scale: f32,
    /// 最大ズームスケール（縮小上限）
    pub max_scale: f32,
    /// ドラッグ移動感度
    pub drag_sensitivity: f32,
    /// カメラが移動できるX方向の最大距離（ワールド座標）
    pub map_bound_x: f32,
    /// カメラが移動できるY方向の最大距離（ワールド座標）
    pub map_bound_y: f32,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            move_speed: 400.0,
            zoom_speed: 0.1,
            min_scale: 0.3,
            max_scale: 5.0,
            drag_sensitivity: 1.0,
            map_bound_x: 1600.0,
            map_bound_y: 1200.0,
        }
    }
}
