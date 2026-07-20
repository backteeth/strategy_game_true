/// デバッグモジュール
/// 開発中のデバッグ情報を表示する（将来的に拡張予定）
use bevy::prelude::*;

/// デバッグ情報プラグイン（現在は最小実装）
pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, _app: &mut App) {
        // 将来: FPS表示、州境界線のデバッグ描画など
    }
}
