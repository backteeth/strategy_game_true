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

/// P21-SAVE-003: `GameState::Playing`へどちらの経路から進入したかを表す、遷移制御専用の
/// 一時Resource。`SaveGameV1`へシリアライズされる対象ではない(ゲームの正規保存対象では
/// ない)。`CountrySelection`には「New Game開始」(`ui::country_selection::handle_start_button`)
/// と「続きからロード」(`save::runtime::handle_load_requests`のロード成功時)の2つの
/// `Playing`進入経路があり、`OnEnter(GameState::Playing)`側の`app::loader::spawn_debug_divisions`
/// が、New Game用の正規データ生成(デバッグ用初期師団の配置)をLoaded Game側では
/// 実行しないために、進入経路をこのResourceで判定する。
///
/// 既定値は`NewGame`(New Gameが従来からの唯一の経路だったため、後方互換の安全側)。
/// 各`Playing`進入経路の直前に明示的に値を設定し、`OnEnter(GameState::Playing)`の処理完了後
/// (`reset_playing_entry_mode`、`spawn_debug_divisions`の後に`.chain()`で実行)に必ず
/// `NewGame`へ決定的に初期化し直す。
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayingEntryMode {
    #[default]
    NewGame,
    LoadedGame,
}

/// `OnEnter(GameState::Playing)`の処理完了後に`PlayingEntryMode`を既定値へ戻す。
/// `spawn_debug_divisions`など、このResourceを読むOnEnterシステムの後に`.chain()`で
/// 実行される必要がある(呼び出し側の登録順に依存する)。
pub fn reset_playing_entry_mode(mut mode: ResMut<PlayingEntryMode>) {
    *mode = PlayingEntryMode::NewGame;
}
