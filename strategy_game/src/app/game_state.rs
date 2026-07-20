/// ゲーム状態の定義
/// States トレイトで管理し、将来のメニュー追加に対応できる構造にする
use bevy::prelude::*;

/// ゲーム全体の状態遷移
/// 起動時: MainMenu → DataLoaderPlugin が CountrySelection へ遷移
/// 国家選択後: CountrySelection → Playing
#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    /// 初期状態（ローダーが即座に CountrySelection へ遷移する）
    #[default]
    MainMenu,
    /// 国家選択画面
    CountrySelection,
    /// ゲームプレイ中
    Playing,
}
