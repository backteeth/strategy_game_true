/// セーブファイル機構モジュール。
///
/// - `dto`: セーブ形式のDTO定義(Bevy非依存、P21-SAVE-002A/A1)。
/// - `export`: ランタイムResourceからDTOへの純粋な変換(Bevy非依存、P21-SAVE-002B)。
/// - `write`: RONシリアライズと安全なファイル書き込み(Bevy非依存、P21-SAVE-002B)。
/// - `runtime`: Bevy ECSとの接続(SystemParam・Message・PostUpdateシステム・Plugin、
///   P21-SAVE-002B)。Bevy固有の型はこのファイルだけに閉じ込め、他の5ファイルは
///   `bevy`を一切importしない。
/// - `read`: ファイル読み取り・RON解析・version検証・構造化エラーへの変換
///   (Bevy非依存、P21-SAVE-002C)。
/// - `validate`: DTO内のID・参照・不変条件検証、静的マスター参照の検証、
///   `ValidatedSaveGameV1`の生成(Bevy非依存、P21-SAVE-002C)。
/// - `apply`: 検証済み`ValidatedSaveGameV1`のBevy Worldへの原子的適用
///   (Prepare→Commitの二段階、P21-SAVE-002D)。`runtime.rs`と同じくBevy固有の型
///   (`World`/`Resource`)を扱う。
///
/// P21-SAVE-002Eで、読込→検証→適用を接続する`handle_load_requests`・`LoadGamePlugin`・
/// Save/Load同時要求の調停(`runtime.rs`)、ゲーム内UI(`ui::top_bar`のセーブ/ロードボタン、
/// `ui::load_confirm`のロード確認ダイアログ)、`main.rs`への`SaveGamePlugin`/
/// `LoadGamePlugin`登録を追加した。`GameState`追加・複数スロット・
/// セーブファイル一覧・オートセーブ・非同期I/Oはいずれも実装していない(P21-SAVE-002F以降)。
///
/// P21-SAVE-002F1で、`apply.rs`のランタイム互換性検査(セーブのCountry/StateId集合と
/// 適用先Worldの現在の集合の完全一致比較、002Dから既に存在)の失敗診断を強化した
/// (`ApplyLoadError::RuntimeCompatibility`へ`RuntimeCompatibilityIssue`を追加し、
/// Country/Stateどちらの不一致か、欠落/余剰の実際のID一覧を決定的な順序で保持する)。
/// これはマイグレーション機能ではなく、Worldを変更する前に安全に拒否するための
/// ランタイム互換性検証である。V1 DTO/RONスキーマ/`export.rs`/`write.rs`/`read.rs`は
/// このラウンドで変更していない。
///
/// P21-SAVE-003で、`handle_load_requests`を`GameState::CountrySelection`でも実行可能にし
/// (`in_playing_or_country_selection`)、起動直後の「続きから」ボタン(`ui::country_selection`)
/// から既存の単一スロットを直接ロードできるようにした。`GameState`は追加していない
/// (`Loading`状態は不要と判断)。New Game専用の正規データ生成(`app::loader::spawn_debug_divisions`)
/// をLoaded Game進入時にスキップするため、遷移制御専用の一時Resource
/// `app::game_state::PlayingEntryMode`を追加した(セーブ対象ではない)。
pub mod apply;
pub mod dto;
pub mod export;
pub mod read;
pub mod runtime;
pub mod validate;
pub mod write;

pub use apply::{
    ApplyLoadError, ApplyLoadOutcome, RuntimeCompatibilityIssue, apply_validated_save,
};
pub use dto::{SAVE_FORMAT_VERSION_V1, SaveGameV1};
pub use export::{SaveGameSources, build_save_game_v1};
pub use read::{LoadSaveError, read_and_validate_save_file};
pub use runtime::{
    LastLoadOutcome, LastSaveOutcome, LoadExecutionCount, LoadGamePlugin, LoadOperationError,
    LoadOutcome, LoadRequestMessage, SaveExecutionCount, SaveFileConfig, SaveGamePlugin,
    SaveGameResourceParams, SaveRequestMessage, handle_load_requests, handle_save_requests,
};
pub use validate::{
    SaveValidationCode, SaveValidationContext, SaveValidationErrors, SaveValidationIssue,
    ValidatedSaveGameV1, validate_save_game_v1,
};
pub use write::{SaveError, SaveOutcome, SavePathConfig, write_save_file};
