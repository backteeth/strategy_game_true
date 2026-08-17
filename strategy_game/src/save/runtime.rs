/// P21-SAVE-002B: Bevy ECSとの接続層。
///
/// DTO定義(`dto.rs`)・純粋変換(`export.rs`)・ファイルI/O(`write.rs`)にはBevyへの依存を
/// 持ち込まない設計を維持するため、Bevy固有の型(`Resource`/`SystemParam`/`Message`/
/// `Plugin`/スケジュール接続)はこのファイルに閉じ込める。
///
/// P21-SAVE-002E: 読込→検証→適用を接続する`handle_load_requests`・`LoadGamePlugin`・
/// Save/Loadの同時要求調停をこのファイルへ追加した。
///
/// P21-SAVE-003: `handle_load_requests`を`GameState::CountrySelection`でも実行可能にし
/// (`in_playing_or_country_selection`)、起動直後の「続きから」導線(`ui::country_selection`の
/// 続きからボタン)に対応した。CountrySelection中の成功時だけ`PlayingEntryMode`と
/// `NextState<GameState>`を制御する。複数スロット・オートセーブ・非同期I/Oは
/// 引き続き実装しない。
use crate::app::game_state::{GameState, PlayingEntryMode};
use crate::app::time::{GameDate, GameSpeed};
use crate::building::data::BuildingRegistry;
use crate::country::country_ai::CountryAiRegistry;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::localization::{CurrentLocale, Loc, TranslationCatalog, translate};
use crate::map::camera::CameraResetRequestMessage;
use crate::military::army::ArmyRegistry;
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::research::data::TechnologyRegistry;
use crate::research::world_stage::WorldCivilizationState;
use crate::save::apply::{ApplyLoadError, ApplyLoadOutcome, apply_validated_save};
use crate::save::export::{SaveGameSources, build_save_game_v1};
use crate::save::read::{LoadSaveError, read_and_validate_save_file};
use crate::save::validate::SaveValidationContext;
use crate::save::write::{SaveOutcome, SavePathConfig, write_save_file};
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use crate::war::data::WarRegistry;
use crate::war::frontline::FrontlineRegistry;
use crate::war::justification::WarJustificationRegistry;
use crate::war::military_ai::MilitaryAiRegistry;
use bevy::ecs::system::{SystemParam, SystemState};
use bevy::prelude::*;
use std::path::PathBuf;

/// セーブ要求。`GameState::Playing`中に一度きり発行される想定の一回性Message。
/// UIはまだこれを発行する箇所を持たない(テストまたは将来のUIタスクが発行する)。
#[derive(Message, Debug, Clone, Copy)]
pub struct SaveRequestMessage;

/// 直近のセーブ結果を保持する非永続リソース(将来のUIタスクが読む想定。それ自体は
/// `SaveGameV1`へシリアライズされる対象ではない)。
#[derive(Resource, Debug, Clone, Default)]
pub struct LastSaveOutcome(pub Option<SaveOutcome>);

/// 実際にファイルへ書き込んだ回数(診断・テスト用)。同一フレーム内の複数
/// `SaveRequestMessage`が1回のセーブへ正しく集約されていることを検証するために使う。
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct SaveExecutionCount(pub u32);

/// セーブ実行時に使用する保存先設定。既定値はプロジェクト相対`saves/savegame_v1.ron`
/// (`SavePathConfig::default()`)。テストでは`SaveFileConfig { path: .. }`を明示的に
/// 上書きして、一意な一時ディレクトリへ差し替える。
#[derive(Resource, Debug, Clone, Default)]
pub struct SaveFileConfig {
    pub path: SavePathConfig,
}

/// セーブに必要な17個のResourceを1つのSystemParamへ束ねる
/// (`Res<T>`を個別の引数として並べるとBevyのSystem引数上限に達するため。
/// `src/localization.rs`の`Loc<'w>`と同じ理由・同じパターン)。
/// 全フィールドが`Res`(`ResMut`ではない)であり、セーブ処理がこれらのResourceを
/// 変更できないことが型システムレベルで保証される。
#[derive(SystemParam)]
pub struct SaveGameResourceParams<'w> {
    pub game_date: Res<'w, GameDate>,
    pub game_speed: Res<'w, GameSpeed>,
    pub player_country: Res<'w, PlayerCountry>,
    pub world_civilization: Res<'w, WorldCivilizationState>,
    pub country_registry: Res<'w, CountryRegistry>,
    pub state_registry: Res<'w, StateRegistry>,
    pub diplomacy_registry: Res<'w, DiplomacyRegistry>,
    pub war_justification_registry: Res<'w, WarJustificationRegistry>,
    pub war_registry: Res<'w, WarRegistry>,
    pub claim_registry: Res<'w, ClaimRegistry>,
    pub crisis_registry: Res<'w, CrisisRegistry>,
    pub country_ai_registry: Res<'w, CountryAiRegistry>,
    pub military_ai_registry: Res<'w, MilitaryAiRegistry>,
    pub military_registry: Res<'w, MilitaryRegistry>,
    pub battle_registry: Res<'w, BattleRegistry>,
    pub army_registry: Res<'w, ArmyRegistry>,
    pub frontline_registry: Res<'w, FrontlineRegistry>,
}

impl SaveGameResourceParams<'_> {
    /// Bevyの`Res<T>`束を、Bevy非依存の`SaveGameSources`(単純な`&Xxx`の束)へ変換する。
    fn as_sources(&self) -> SaveGameSources<'_> {
        SaveGameSources {
            game_date: &self.game_date,
            game_speed: &self.game_speed,
            player_country: &self.player_country,
            world_civilization: &self.world_civilization,
            country_registry: &self.country_registry,
            state_registry: &self.state_registry,
            diplomacy_registry: &self.diplomacy_registry,
            war_justification_registry: &self.war_justification_registry,
            war_registry: &self.war_registry,
            claim_registry: &self.claim_registry,
            crisis_registry: &self.crisis_registry,
            country_ai_registry: &self.country_ai_registry,
            military_ai_registry: &self.military_ai_registry,
            military_registry: &self.military_registry,
            battle_registry: &self.battle_registry,
            army_registry: &self.army_registry,
            frontline_registry: &self.frontline_registry,
        }
    }
}

/// セーブ結果の記録・通知に必要な4個のResource/SystemParamを1つへ束ねる
/// (`SaveGameResourceParams`と同じ理由: `Res`/`ResMut`個別引数を並べるとBevyの
/// System引数上限[既定7個]に達するため)。
#[derive(SystemParam)]
pub struct SaveOutcomeReporter<'w> {
    pub last_outcome: ResMut<'w, LastSaveOutcome>,
    pub execution_count: ResMut<'w, SaveExecutionCount>,
    pub notif_writer: MessageWriter<'w, GameNotification>,
    pub loc: Loc<'w>,
}

/// `PostUpdate`で実行されるセーブシステム。
///
/// `PostUpdate`はそのフレームの`Update`スケジュール全体(`DailySimulationSet`の全チェーン
/// および全UI/入力システム)が完了した後にのみ実行されるため、移動・戦闘・Army清掃・
/// 講和処理後の確定状態を保存できる(`DailySimulationSet`の途中状態を保存しない)。
///
/// 同一フレーム内に複数の`SaveRequestMessage`が来ても、`.read()`で全て読み切ってから
/// 高々1回だけ`write_save_file`を呼ぶ(1回へ集約)。`GameState::Playing`以外では、
/// システム自体を実行しない(`.run_if(in_state(GameState::Playing))`で登録するため)。
/// `GamePaused`はこの関数のどこにも読み書きされない。
///
/// P21-SAVE-002E: 同一フレームに`LoadRequestMessage`も来ている場合、Loadを優先し、
/// このフレームのSaveは実行しない(`SaveRequestMessage`は読み切って消費するだけで、
/// 次フレームへは持ち越さない)。既存スロットを、ロードしたかった意図に反して現在状態で
/// 上書きしてしまう事故を防ぐため。成功・失敗いずれの場合も通知を1回だけ発行する。
pub fn handle_save_requests(
    mut requests: MessageReader<SaveRequestMessage>,
    mut pending_loads: MessageReader<LoadRequestMessage>,
    sources: SaveGameResourceParams,
    path_config: Res<SaveFileConfig>,
    mut reporter: SaveOutcomeReporter,
) {
    let request_count = requests.read().count();
    if request_count == 0 {
        return;
    }

    if pending_loads.read().count() > 0 {
        // LoadRequestMessage自体はhandle_load_requests側が別途消費するため、ここでは
        // 「存在を確認するためだけの読み取り」であり、二重消費の問題は起きない
        // (MessageReaderはSystemごとに独立したカーソルを持つ)。
        return;
    }

    let save = build_save_game_v1(&sources.as_sources());
    let outcome = write_save_file(&save, &path_config.path);
    reporter.execution_count.0 += 1;

    let key = match &outcome {
        SaveOutcome::Success { .. } => "notif.save_success",
        SaveOutcome::Failure { .. } => "notif.save_failed",
    };
    reporter.notif_writer.write(GameNotification {
        message: reporter.loc.t(key),
    });

    reporter.last_outcome.0 = Some(outcome);
}

/// セーブ機構のプラグイン。`SaveRequestMessage`/`LoadRequestMessage`の登録、非永続
/// リソースの初期化、`PostUpdate`へのセーブシステム登録を行う。
///
/// `LoadRequestMessage`はこのプラグインが登録する(`handle_save_requests`がSave/Load
/// 同時要求の調停のために読み取る必要があるため)。`LoadGamePlugin`側では再登録しない
/// (Bevyの`add_message`は同一Message型を複数箇所から登録しても安全だが
/// [`state::StatePlugin`/`map::selection::SelectionPlugin`が`StateSelectionChanged`を
/// 同様に二重登録している既存の前例で確認済み]、不要な二重登録は避ける)。
pub struct SaveGamePlugin;

impl Plugin for SaveGamePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SaveRequestMessage>()
            .add_message::<LoadRequestMessage>()
            .init_resource::<SaveFileConfig>()
            .init_resource::<LastSaveOutcome>()
            .init_resource::<SaveExecutionCount>()
            .add_systems(
                PostUpdate,
                handle_save_requests.run_if(in_state(GameState::Playing)),
            );
    }
}

/// ロード要求。`GameState::Playing`中に一度きり発行される想定の一回性Message。
#[derive(Message, Debug, Clone, Copy)]
pub struct LoadRequestMessage;

/// ロードが「読込・検証」段階と「適用」段階のどちらで失敗したかを区別する構造化エラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOperationError {
    /// ファイル読込・RON解析・version検証・参照整合性検証(P21-SAVE-002C)の失敗。
    ReadOrValidate(LoadSaveError),
    /// 検証済みDTOのResourceへの適用(P21-SAVE-002D)の失敗。
    Apply(ApplyLoadError),
}

/// ロード結果。将来のUI/通知タスクがそのまま利用できる構造。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    Success {
        path: PathBuf,
    },
    Failure {
        path: PathBuf,
        error: LoadOperationError,
    },
}

/// 直近のロード結果を保持する非永続リソース(`SaveGameV1`へシリアライズされる対象ではない)。
#[derive(Resource, Debug, Clone, Default)]
pub struct LastLoadOutcome(pub Option<LoadOutcome>);

/// 実際に読込→検証→適用を実行した回数(診断・テスト用)。同一フレーム内の複数
/// `LoadRequestMessage`が1回の実行へ正しく集約されていることを検証するために使う。
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct LoadExecutionCount(pub u32);

/// ロード失敗の分類から、通知に使う翻訳キーと引数を決定する。Validation失敗は
/// 全件を長文表示せず、件数だけを通知に載せる(詳細は診断ログ側で確認する設計)。
///
/// P21-SAVE-003: `crate`内可視にしたのは、`ui::country_selection`がCountrySelection画面上の
/// インライン失敗表示(`GameNotification`がPlaying専用UIでしか描画されないため、
/// CountrySelectionでは別途この分類を再利用する必要がある)で同じキー/引数解決を
/// 再利用するため。ロード失敗の分類ロジックを複製しない。
pub(crate) fn load_failure_notification(
    error: &LoadOperationError,
) -> (&'static str, Vec<(&'static str, String)>) {
    match error {
        LoadOperationError::ReadOrValidate(LoadSaveError::FileNotFound(_)) => {
            ("notif.load_failed_file_not_found", Vec::new())
        }
        LoadOperationError::ReadOrValidate(LoadSaveError::Read(_)) => {
            ("notif.load_failed_read", Vec::new())
        }
        LoadOperationError::ReadOrValidate(LoadSaveError::Deserialize(_)) => {
            ("notif.load_failed_deserialize", Vec::new())
        }
        LoadOperationError::ReadOrValidate(LoadSaveError::UnsupportedVersion { .. }) => {
            ("notif.load_failed_unsupported_version", Vec::new())
        }
        LoadOperationError::ReadOrValidate(LoadSaveError::Validation(errors)) => (
            "notif.load_failed_validation",
            vec![("count", errors.issues.len().to_string())],
        ),
        LoadOperationError::Apply(_) => ("notif.load_failed_apply", Vec::new()),
    }
}

/// `handle_load_requests`を`GameState::Playing`と`GameState::CountrySelection`の両方で
/// 実行可能にするrun condition(P21-SAVE-003: 起動直後の「続きから」導線)。
/// `GameState::MainMenu`(起動直後、静的マスターRON読込中)では実行しない。
pub fn in_playing_or_country_selection(state: Res<State<GameState>>) -> bool {
    matches!(state.get(), GameState::Playing | GameState::CountrySelection)
}

/// `PostUpdate`で実行されるロードシステム(排他的System、`&mut World`を直接受け取る)。
///
/// 手順を厳守する: (1) `LoadRequestMessage`を集約(0件なら即return) (2) `SaveFileConfig`
/// から単一スロットパスを取得 (3) 現在の静的マスターから`SaveValidationContext`を構築し、
/// その共有借用を確実に終了してから (4) `read_and_validate_save_file` (5) 成功した場合
/// だけ`apply_validated_save` (6) 成功時、`GameState::Playing`中の実行ならカメラ初期化要求
/// (`CameraResetRequestMessage`)を発行し、`GameState::CountrySelection`中の実行(起動直後の
/// 「続きから」)なら代わりに`PlayingEntryMode::LoadedGame`を設定して`Playing`へ遷移する
/// (7) `LastLoadOutcome`/`LoadExecutionCount`を記録 (8) 成功・失敗通知を1回だけ発行。
///
/// ファイル読込・検証(手順4)が失敗した場合は`apply_validated_save`を呼ばない。適用
/// (手順5)が失敗した場合も、P21-SAVE-002Dの設計(Prepare失敗時はWorld無変更)により
/// 現在の正規状態がそのまま維持される。002C/002Dの公開APIだけを経由し、それらを
/// 迂回する独自の読込・適用経路は持たない。
///
/// P21-SAVE-003: `GameState::CountrySelection`中は`GameCamera`がまだ一度もWASD/ドラッグ等で
/// 動かされていない`setup_camera`の既定Transformのままである(`CameraPlugin`は
/// `Startup`で無条件にカメラをspawnする)ため、`CameraResetRequestMessage`を送らなくても
/// カメラは既定位置・ズームで`Playing`へ入る。あえて送らないことで、まだ`GameCamera`が
/// 存在しない将来の起動シーケンス変更に対しても不要な警告を出さない設計にしている。
///
/// `reader_state`は`&mut SystemState<P>`(`ExclusiveSystemParam`としてBevyが公式サポート
/// する形、`Local<T>`と同様にシステム専用の永続ストレージへ自動的に保持される)を必ず
/// 引数として受け取り、この関数の中で新しい`SystemState::new(world)`を作ってはならない。
/// `MessageReader`はカーソル位置を自身の状態として持つため、フレームごとに新しい
/// `SystemState`を作るとカーソルがリセットされ、Bevyのdouble-buffer([約2フレーム保持])
/// にまだ残っている同一メッセージを次のフレーム以降でも「未読」として再度読んでしまい、
/// 1回のLoadボタン押下で`apply_validated_save`が複数フレームにわたって複数回実行されて
/// しまう(実際にP21-SAVE-002EのHeadless描画テストで複数フレームにわたり本システムを
/// 実行させたことで顕在化した実バグ)。`MessageWriter`側(`CameraResetRequestMessage`/
/// `GameNotification`)はカーソルを持たないため、こちらは毎回`SystemState::new`しても
/// 二重発行の問題は起きない。
pub fn handle_load_requests(
    world: &mut World,
    reader_state: &mut SystemState<MessageReader<LoadRequestMessage>>,
) {
    let request_count = reader_state
        .get_mut(world)
        .expect("MessageReader<LoadRequestMessage> must be valid")
        .read()
        .count();
    if request_count == 0 {
        return;
    }

    let requested_from_country_selection =
        *world.resource::<State<GameState>>().get() == GameState::CountrySelection;

    let path_config = world.resource::<SaveFileConfig>().path.clone();

    let read_result = {
        let building_registry = world.resource::<BuildingRegistry>();
        let technology_registry = world.resource::<TechnologyRegistry>();
        let military_registry = world.resource::<MilitaryRegistry>();
        let world_civilization = world.resource::<WorldCivilizationState>();
        let context = SaveValidationContext {
            building_definitions: &building_registry.definitions,
            technology_definitions: &technology_registry.definitions,
            division_definitions: &military_registry.definitions,
            world_stage_definitions: &world_civilization.stage_definitions,
        };
        // ここで静的Resourceへの共有借用(context)が確実に終了する(このブロックの
        // 戻り値はcontextを一切参照しない所有型`Result<..>`のため)。
        read_and_validate_save_file(&path_config, &context)
    };

    let outcome = match read_result {
        Ok(validated) => match apply_validated_save(world, validated) {
            ApplyLoadOutcome::Success => {
                if requested_from_country_selection {
                    // 起動直後の「続きから」: カメラはStartupの既定Transformのままなので
                    // CameraResetRequestMessageは送らない。代わりにPlayingEntryModeを
                    // LoadedGameへ設定してからPlayingへ遷移し、OnEnter(Playing)の
                    // spawn_debug_divisionsがこのロードへデバッグDivisionを追加しないように
                    // する。
                    *world.resource_mut::<PlayingEntryMode>() = PlayingEntryMode::LoadedGame;
                    world
                        .resource_mut::<NextState<GameState>>()
                        .set(GameState::Playing);
                } else {
                    let mut cam_state: SystemState<MessageWriter<CameraResetRequestMessage>> =
                        SystemState::new(world);
                    cam_state
                        .get_mut(world)
                        .expect("MessageWriter<CameraResetRequestMessage> must be valid")
                        .write(CameraResetRequestMessage);
                    cam_state.apply(world);
                }
                LoadOutcome::Success {
                    path: path_config.final_path.clone(),
                }
            }
            ApplyLoadOutcome::Failure(apply_err) => LoadOutcome::Failure {
                path: path_config.final_path.clone(),
                error: LoadOperationError::Apply(apply_err),
            },
        },
        Err(load_err) => LoadOutcome::Failure {
            path: path_config.final_path.clone(),
            error: LoadOperationError::ReadOrValidate(load_err),
        },
    };

    let (notif_key, notif_args): (&'static str, Vec<(&'static str, String)>) = match &outcome {
        LoadOutcome::Success { .. } => ("notif.load_success", Vec::new()),
        LoadOutcome::Failure { error, .. } => load_failure_notification(error),
    };
    let message = {
        let catalog = world.resource::<TranslationCatalog>();
        let locale = world.resource::<CurrentLocale>();
        translate(catalog, locale.0, notif_key, &notif_args)
    };
    {
        let mut state: SystemState<MessageWriter<GameNotification>> = SystemState::new(world);
        state
            .get_mut(world)
            .expect("MessageWriter<GameNotification> must be valid")
            .write(GameNotification { message });
        state.apply(world);
    }

    world.resource_mut::<LastLoadOutcome>().0 = Some(outcome);
    world.resource_mut::<LoadExecutionCount>().0 += 1;
}

/// ロード機構のプラグイン。非永続リソースの初期化、`PostUpdate`へのロードシステム登録を
/// 行う。`LoadRequestMessage`自体は`SaveGamePlugin`が登録するため、`LoadGamePlugin`は
/// `SaveGamePlugin`と併せて登録する想定(main.rsで両方を登録する)。
pub struct LoadGamePlugin;

impl Plugin for LoadGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SaveFileConfig>()
            .init_resource::<LastLoadOutcome>()
            .init_resource::<LoadExecutionCount>()
            .add_systems(
                PostUpdate,
                // P21-SAVE-003: 起動直後の「続きから」導線のため、GameState::Playingに加えて
                // GameState::CountrySelectionでも実行可能にする(in_playing_or_country_selection)。
                handle_load_requests.run_if(in_playing_or_country_selection),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::CountryId;
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::ecs::system::SystemState;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "strategy_game_p21_save_002b_runtime_{label}_{}_{nanos}_{n}",
            std::process::id()
        ))
    }

    struct TempTestDir(PathBuf);

    impl TempTestDir {
        fn new(label: &str) -> Self {
            Self(unique_temp_dir(label))
        }

        fn save_file_config(&self) -> SaveFileConfig {
            SaveFileConfig {
                path: SavePathConfig {
                    final_path: self.0.join("savegame_v1.ron"),
                },
            }
        }
    }

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// テスト専用の最小App: セーブ/ロードに必要なResourceを直接挿入するだけで、
    /// CountryPlugin等の実際のゲームプラグインには依存しない(それらのDailySimulation
    /// システムが同フレームで一緒に走って結果を汚さないようにするため、意図的に最小限)。
    ///
    /// P21-SAVE-002E: `handle_save_requests`が通知発行に`Loc`(`TranslationCatalog`/
    /// `CurrentLocale`)と`MessageWriter<GameNotification>`を必要とするようになったため、
    /// これらもここで用意する(`TranslationCorePlugin`は複数箇所からの追加を許容する設計
    /// [`is_unique() == false`]なので安全に追加できる)。`BuildingRegistry`/
    /// `TechnologyRegistry`はロード側(`handle_load_requests`)が`SaveValidationContext`
    /// 構築に必要とする。
    fn setup_minimal_save_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
            .add_plugins(bevy::state::app::StatesPlugin)
            .add_plugins(crate::localization::TranslationCorePlugin);

        app.insert_resource(GameDate::default())
            .insert_resource(GameSpeed::default())
            .insert_resource(PlayerCountry(Some(CountryId(1))))
            .insert_resource(WorldCivilizationState::default())
            .insert_resource(CountryRegistry::default())
            .insert_resource(StateRegistry::default())
            .insert_resource(DiplomacyRegistry::default())
            .insert_resource(WarJustificationRegistry::default())
            .insert_resource(WarRegistry::default())
            .insert_resource(ClaimRegistry::default())
            .insert_resource(CrisisRegistry::default())
            .insert_resource(CountryAiRegistry::default())
            .insert_resource(MilitaryAiRegistry::default())
            .insert_resource(MilitaryRegistry::default())
            .insert_resource(BattleRegistry::default())
            .insert_resource(ArmyRegistry::default())
            .insert_resource(FrontlineRegistry::default())
            .insert_resource(BuildingRegistry::default())
            .insert_resource(TechnologyRegistry::default())
            .insert_resource(crate::app::time::GamePaused(true))
            .add_message::<GameNotification>();

        app.add_plugins(SaveGamePlugin);
        app
    }

    fn send_save_request(app: &mut App) {
        let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .expect("MessageWriter<SaveRequestMessage> system param must be valid")
            .write(SaveRequestMessage);
        state.apply(app.world_mut());
    }

    #[test]
    fn no_save_request_creates_no_file() {
        let temp_dir = TempTestDir::new("no_request");
        let mut app = setup_minimal_save_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);

        app.update();

        assert_eq!(app.world().resource::<SaveExecutionCount>().0, 0);
        assert!(app.world().resource::<LastSaveOutcome>().0.is_none());
    }

    #[test]
    fn save_request_outside_playing_state_does_not_save() {
        let temp_dir = TempTestDir::new("not_playing");
        let mut app = setup_minimal_save_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::CountrySelection);

        send_save_request(&mut app);
        app.update();

        assert_eq!(
            app.world().resource::<SaveExecutionCount>().0,
            0,
            "system must not even run (run_if gate) outside GameState::Playing"
        );
        assert!(!temp_dir.0.join("savegame_v1.ron").exists());
    }

    #[test]
    fn save_request_while_playing_saves_via_post_update() {
        let temp_dir = TempTestDir::new("playing_saves");
        let mut app = setup_minimal_save_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);

        send_save_request(&mut app);
        app.update();

        assert_eq!(app.world().resource::<SaveExecutionCount>().0, 1);
        match &app.world().resource::<LastSaveOutcome>().0 {
            Some(SaveOutcome::Success { .. }) => {}
            other => panic!("expected Some(Success), got {other:?}"),
        }
        assert!(temp_dir.0.join("savegame_v1.ron").exists());
    }

    #[test]
    fn multiple_save_requests_in_one_frame_collapse_to_a_single_save() {
        let temp_dir = TempTestDir::new("collapse");
        let mut app = setup_minimal_save_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);

        send_save_request(&mut app);
        send_save_request(&mut app);
        send_save_request(&mut app);
        app.update();

        assert_eq!(
            app.world().resource::<SaveExecutionCount>().0,
            1,
            "3 requests in the same frame must collapse into exactly 1 save execution"
        );
    }

    #[test]
    fn resource_mutated_in_update_is_reflected_in_the_same_frame_save() {
        fn mutate_game_speed(mut speed: ResMut<GameSpeed>) {
            speed.0 = 3;
        }

        let temp_dir = TempTestDir::new("same_frame_mutation");
        let mut app = setup_minimal_save_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);
        app.add_systems(Update, mutate_game_speed);

        send_save_request(&mut app);
        app.update();

        let contents = std::fs::read_to_string(temp_dir.0.join("savegame_v1.ron")).unwrap();
        let restored: crate::save::dto::SaveGameV1 = ron::from_str(&contents).unwrap();
        assert_eq!(
            restored.game_speed, 3,
            "PostUpdate save must observe the value written by this frame's Update system, not the pre-frame value"
        );
    }

    #[test]
    fn save_does_not_change_game_paused() {
        let temp_dir = TempTestDir::new("paused_unchanged");
        let mut app = setup_minimal_save_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);

        let before = app.world().resource::<crate::app::time::GamePaused>().0;
        send_save_request(&mut app);
        app.update();
        let after = app.world().resource::<crate::app::time::GamePaused>().0;

        assert_eq!(before, after, "GamePaused must be unchanged by a save");
    }

    // ─── P21-SAVE-002E: ロードランタイム・Save/Load調停 ────────────────────────

    use crate::common::StateId;
    use crate::country::{CountryData, EconomicSystem, GovernmentType};
    use crate::research::data::WorldStage;
    use crate::research::world_stage::WorldStageDefinition;
    use crate::save::apply::ApplyLoadError;
    use crate::save::dto::{
        SAVE_FORMAT_VERSION_V1, SaveGameV1, SavedArmyRegistry, SavedBattleRegistry,
        SavedClaimRegistry, SavedCountryAiRegistry, SavedCrisisRegistry, SavedDiplomacyRegistry,
        SavedFrontlineRegistry, SavedGameDate, SavedMilitaryAiRegistry, SavedMilitaryRegistry,
        SavedWarJustificationRegistry, SavedWarRegistry, SavedWorldCivilizationState,
    };
    use crate::state::data::StateData;
    use std::collections::HashMap;

    /// `setup_minimal_save_app`に、ロード成功を実際に検証できるだけの最小限の
    /// 地図(1か国・1州)と静的世界段階定義を追加し、`LoadGamePlugin`も登録する。
    fn setup_minimal_load_app() -> App {
        let mut app = setup_minimal_save_app();

        let country = CountryData {
            id: CountryId(0),
            capital_state_id: StateId(0),
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        };
        app.insert_resource(CountryRegistry {
            countries: vec![country],
        });

        let state = StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            ..StateData::default()
        };
        app.insert_resource(StateRegistry::build(vec![state]));
        app.insert_resource(PlayerCountry(Some(CountryId(0))));

        let mut stage_definitions = HashMap::new();
        stage_definitions.insert(
            WorldStage::PreIndustrial,
            WorldStageDefinition {
                stage: WorldStage::PreIndustrial,
                display_name: "Pre-Industrial".to_string(),
                description: String::new(),
                required_previous_stage: None,
                milestone_technologies: Vec::new(),
                required_country_count: 1,
            },
        );
        app.insert_resource(WorldCivilizationState {
            current_stage: WorldStage::PreIndustrial,
            stage_definitions,
            milestone_countries: HashMap::new(),
            last_advanced_date: "1800/01/01".to_string(),
        });

        // P21-SAVE-002Dの`prepare_load`が必須Resourceとして存在確認する一時状態
        // (選択・パネル・カメラドラッグ・通知履歴・旧AI戦争キュー)。実際の`UiPlugin`/
        // `DivisionSelectionPlugin`等は意図的に追加しない(それらのUpdateシステムが
        // 同フレームで一緒に走って結果を汚さないようにするため)。
        app.insert_resource(crate::map::division_selection::SelectedDivision::default())
            .insert_resource(crate::map::division_selection::DragSelectState::default())
            .insert_resource(crate::military::army::SelectedArmy::default())
            .insert_resource(crate::state::SelectedState::default())
            .insert_resource(crate::ui::ActivePanel::default())
            .insert_resource(crate::ui::diplomacy_panel::DiplomacyPanelState::default())
            .insert_resource(crate::ui::military_panel::MilitaryPanelState::default())
            .insert_resource(crate::ui::peace_panel::PeacePanelState::default())
            .insert_resource(crate::ui::politics_panel::PoliticsPanelState::default())
            .insert_resource(crate::ui::research_panel::ResearchPanelState::default())
            .insert_resource(crate::map::camera::CameraDragState::default())
            .insert_resource(crate::ui::notification::NotificationHistory::default())
            .insert_resource(crate::country::country_ai::PendingAiWarDeclarations::default())
            .add_message::<crate::map::camera::CameraResetRequestMessage>();

        app.add_plugins(LoadGamePlugin);
        app
    }

    /// `setup_minimal_load_app`の地図(CountryId(0)/StateId(0))とだけ一致する、
    /// 代表的な正常セーブ。
    fn representative_valid_save() -> SaveGameV1 {
        let country = CountryData {
            id: CountryId(0),
            capital_state_id: StateId(0),
            treasury: 7500.0,
            government_type: GovernmentType::Republic,
            economic_system: EconomicSystem::Mercantilism,
            ..CountryData::default()
        };
        let state = StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            ..StateData::default()
        };
        let states = StateRegistry::build(vec![state]).states;

        SaveGameV1 {
            version: SAVE_FORMAT_VERSION_V1,
            date: SavedGameDate {
                year: 1802,
                month: 3,
                day: 10,
                accumulator: 0.1,
            },
            game_speed: 2,
            player_country: Some(CountryId(0)),
            world_civilization: SavedWorldCivilizationState {
                current_stage: WorldStage::PreIndustrial,
                milestone_countries: HashMap::new(),
                last_advanced_date: "1802/01/01".to_string(),
            },
            countries: vec![country],
            states,
            diplomacy: SavedDiplomacyRegistry::default(),
            war_justifications: SavedWarJustificationRegistry::default(),
            wars: SavedWarRegistry::default(),
            claims: SavedClaimRegistry::default(),
            crises: SavedCrisisRegistry::default(),
            country_ai: SavedCountryAiRegistry::default(),
            military_ai: SavedMilitaryAiRegistry::default(),
            military: SavedMilitaryRegistry::default(),
            battles: SavedBattleRegistry::default(),
            armies: SavedArmyRegistry::default(),
            frontlines: SavedFrontlineRegistry::default(),
        }
    }

    fn write_file(path: &std::path::Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn send_load_request(app: &mut App) {
        let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .expect("MessageWriter<LoadRequestMessage> system param must be valid")
            .write(LoadRequestMessage);
        state.apply(app.world_mut());
    }

    #[test]
    fn no_load_request_does_not_read() {
        let temp_dir = TempTestDir::new("no_load_request");
        let mut app = setup_minimal_load_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);

        app.update();

        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 0);
        assert!(app.world().resource::<LastLoadOutcome>().0.is_none());
    }

    #[test]
    fn load_request_outside_playing_or_country_selection_state_does_not_read() {
        // P21-SAVE-003: handle_load_requestsはGameState::Playingに加えて
        // GameState::CountrySelectionでも実行されるようになったため(起動直後の
        // 「続きから」導線)、この回帰テストの対象はMainMenu(起動直後、静的マスターRON
        // 読込中でありロードを許可すべきではない唯一の残りの状態)へ更新する。
        let temp_dir = TempTestDir::new("load_not_playing_or_country_selection");
        let mut app = setup_minimal_load_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::MainMenu);

        send_load_request(&mut app);
        app.update();

        assert_eq!(
            app.world().resource::<LoadExecutionCount>().0,
            0,
            "system must not even run (run_if gate) outside GameState::Playing/CountrySelection"
        );
    }

    #[test]
    fn load_request_while_playing_reads_validates_and_applies() {
        let temp_dir = TempTestDir::new("load_success");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_load_request(&mut app);
        app.update();

        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 1);
        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Success { .. }) => {}
            other => panic!("expected Some(Success), got {other:?}"),
        }
        assert_eq!(app.world().resource::<GameDate>().year, 1802);
        assert_eq!(
            app.world().resource::<CountryRegistry>().countries[0].treasury,
            7500.0
        );
    }

    #[test]
    fn multiple_load_requests_in_one_frame_collapse_to_a_single_execution() {
        let temp_dir = TempTestDir::new("load_collapse");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_load_request(&mut app);
        send_load_request(&mut app);
        send_load_request(&mut app);
        app.update();

        assert_eq!(
            app.world().resource::<LoadExecutionCount>().0,
            1,
            "3 requests in the same frame must collapse into exactly 1 load execution"
        );
    }

    #[test]
    fn load_success_sets_game_paused_true() {
        let temp_dir = TempTestDir::new("load_paused");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        app.insert_resource(crate::app::time::GamePaused(false));
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_load_request(&mut app);
        app.update();

        assert!(app.world().resource::<crate::app::time::GamePaused>().0);
    }

    #[test]
    fn missing_save_file_preserves_current_state() {
        let temp_dir = TempTestDir::new("load_missing_file");
        let mut app = setup_minimal_load_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);
        let before_treasury = app.world().resource::<CountryRegistry>().countries[0].treasury;

        send_load_request(&mut app);
        app.update();

        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Failure {
                error: LoadOperationError::ReadOrValidate(LoadSaveError::FileNotFound(_)),
                ..
            }) => {}
            other => panic!("expected FileNotFound failure, got {other:?}"),
        }
        assert_eq!(
            app.world().resource::<CountryRegistry>().countries[0].treasury,
            before_treasury
        );
    }

    #[test]
    fn malformed_ron_preserves_current_state() {
        let temp_dir = TempTestDir::new("load_malformed");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        write_file(&config.path.final_path, "not valid ron at all {{{");
        let before_treasury = app.world().resource::<CountryRegistry>().countries[0].treasury;

        send_load_request(&mut app);
        app.update();

        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Failure {
                error: LoadOperationError::ReadOrValidate(LoadSaveError::Deserialize(_)),
                ..
            }) => {}
            other => panic!("expected Deserialize failure, got {other:?}"),
        }
        assert_eq!(
            app.world().resource::<CountryRegistry>().countries[0].treasury,
            before_treasury
        );
    }

    #[test]
    fn unsupported_version_preserves_current_state() {
        let temp_dir = TempTestDir::new("load_version");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        let ron_str = ron::to_string(&representative_valid_save()).unwrap();
        let tampered = ron_str.replacen("version:1,", "version:2,", 1);
        assert_ne!(
            tampered, ron_str,
            "test setup must actually tamper the version"
        );
        write_file(&config.path.final_path, &tampered);
        let before_treasury = app.world().resource::<CountryRegistry>().countries[0].treasury;

        send_load_request(&mut app);
        app.update();

        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Failure {
                error:
                    LoadOperationError::ReadOrValidate(LoadSaveError::UnsupportedVersion { found: 2 }),
                ..
            }) => {}
            other => panic!("expected UnsupportedVersion failure, got {other:?}"),
        }
        assert_eq!(
            app.world().resource::<CountryRegistry>().countries[0].treasury,
            before_treasury
        );
    }

    #[test]
    fn validation_failure_preserves_current_state() {
        let temp_dir = TempTestDir::new("load_validation_fail");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        let mut save = representative_valid_save();
        save.player_country = Some(CountryId(999));
        write_file(&config.path.final_path, &ron::to_string(&save).unwrap());
        let before_treasury = app.world().resource::<CountryRegistry>().countries[0].treasury;

        send_load_request(&mut app);
        app.update();

        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Failure {
                error: LoadOperationError::ReadOrValidate(LoadSaveError::Validation(_)),
                ..
            }) => {}
            other => panic!("expected Validation failure, got {other:?}"),
        }
        assert_eq!(
            app.world().resource::<CountryRegistry>().countries[0].treasury,
            before_treasury
        );
    }

    #[test]
    fn apply_failure_preserves_current_state() {
        let temp_dir = TempTestDir::new("load_apply_fail");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);

        // 内部的には整合しているが、適用先の現在の地図(StateId(0)のみ)と一致しない
        // State集合を持つセーブ(002Cの検証は通り、002Dの適用段階でRuntimeCompatibility
        // として拒否される想定)。
        let mut save = representative_valid_save();
        save.states.push(StateData {
            id: StateId(1),
            owner_country_id: CountryId(0),
            ..StateData::default()
        });
        write_file(&config.path.final_path, &ron::to_string(&save).unwrap());
        let before_treasury = app.world().resource::<CountryRegistry>().countries[0].treasury;

        send_load_request(&mut app);
        app.update();

        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Failure {
                error: LoadOperationError::Apply(ApplyLoadError::RuntimeCompatibility { .. }),
                ..
            }) => {}
            other => panic!("expected Apply/RuntimeCompatibility failure, got {other:?}"),
        }
        assert_eq!(
            app.world().resource::<CountryRegistry>().countries[0].treasury,
            before_treasury
        );
    }

    // ─── P21-SAVE-002F1: Country/State ID集合の互換性検査(runtime/E2E) ────────
    //
    // `apply.rs`側のID集合検査自体は002Dから既に存在した
    // (`apply_failure_preserves_current_state`が既存の証拠)。ここではその失敗が
    // 実際のPostUpdate実行を通じて正しく1回だけ・正しい種類の失敗として観測され、
    // 通知・カメラ・GamePaused・次フレーム非再実行の全てが守られることを確認する。

    /// `representative_valid_save()`に、現在の最小World(CountryId(0)/StateId(0)のみ)
    /// には無い追加StateId(1)を(既存CountryId(0)所有として)追加した、State集合だけが
    /// 不一致になるセーブを返す。
    fn save_with_extra_state() -> SaveGameV1 {
        let mut save = representative_valid_save();
        save.states.push(StateData {
            id: StateId(1),
            owner_country_id: CountryId(0),
            ..StateData::default()
        });
        save
    }

    // 13. 異なるCountry集合のRONをロードするとApply失敗になる。
    #[test]
    fn different_country_id_set_causes_apply_failure() {
        let temp_dir = TempTestDir::new("country_set_mismatch");
        let mut app = setup_minimal_load_app();
        // Country不一致だけを検査させるため、Worldにも同じ追加StateId(1)を挿入して
        // State集合を一致させる(Worldは検証を通らないため、所有者の整合性は不要)。
        {
            let mut states = app.world().resource::<StateRegistry>().states.clone();
            states.push(StateData {
                id: StateId(1),
                owner_country_id: CountryId(0),
                ..StateData::default()
            });
            app.insert_resource(StateRegistry::build(states));
        }
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);

        let mut save = representative_valid_save();
        save.countries.push(CountryData {
            id: CountryId(1),
            capital_state_id: StateId(1),
            government_type: GovernmentType::Republic,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        });
        save.states.push(StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            ..StateData::default()
        });
        write_file(&config.path.final_path, &ron::to_string(&save).unwrap());
        let before_treasury = app.world().resource::<CountryRegistry>().countries[0].treasury;

        send_load_request(&mut app);
        app.update();

        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Failure {
                error: LoadOperationError::Apply(ApplyLoadError::RuntimeCompatibility { issue, .. }),
                ..
            }) => {
                assert!(
                    matches!(
                        issue,
                        crate::save::apply::RuntimeCompatibilityIssue::CountrySetMismatch { .. }
                    ),
                    "expected CountrySetMismatch, got {issue:?}"
                );
            }
            other => {
                panic!("expected Apply/RuntimeCompatibility(CountrySetMismatch), got {other:?}")
            }
        }
        assert_eq!(
            app.world().resource::<CountryRegistry>().countries[0].treasury,
            before_treasury
        );
    }

    // 14. 異なるState集合のRONをロードするとApply失敗になる
    //     (`apply_failure_preserves_current_state`と同じ土台だが、ここでは
    //     `RuntimeCompatibilityIssue::StateSetMismatch`の中身まで確認する)。
    #[test]
    fn different_state_id_set_causes_apply_failure() {
        let temp_dir = TempTestDir::new("state_set_mismatch");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);

        let save = save_with_extra_state();
        write_file(&config.path.final_path, &ron::to_string(&save).unwrap());
        let before_treasury = app.world().resource::<CountryRegistry>().countries[0].treasury;

        send_load_request(&mut app);
        app.update();

        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Failure {
                error: LoadOperationError::Apply(ApplyLoadError::RuntimeCompatibility { issue, .. }),
                ..
            }) => {
                assert_eq!(
                    *issue,
                    crate::save::apply::RuntimeCompatibilityIssue::StateSetMismatch {
                        missing_from_save: vec![],
                        extra_in_save: vec![StateId(1)],
                    }
                );
            }
            other => panic!("expected Apply/RuntimeCompatibility(StateSetMismatch), got {other:?}"),
        }
        assert_eq!(
            app.world().resource::<CountryRegistry>().countries[0].treasury,
            before_treasury
        );
    }

    // 15. 失敗通知は正確に1件だけ発行される。
    #[test]
    fn runtime_compatibility_failure_emits_exactly_one_notification() {
        let temp_dir = TempTestDir::new("mismatch_notif_once");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        let save = save_with_extra_state();
        write_file(&config.path.final_path, &ron::to_string(&save).unwrap());

        let mut cursor = NotificationCursor::new(&mut app);
        send_load_request(&mut app);
        app.update();
        let notifications = cursor.drain(&mut app);

        assert_eq!(
            notifications.len(),
            1,
            "exactly one notification must be emitted, got {notifications:?}"
        );
        assert!(!notifications[0].is_empty());
    }

    // 16. カメラリセット要求は発行されない(=カメラは変化しない)。
    #[test]
    fn runtime_compatibility_failure_does_not_request_camera_reset() {
        let temp_dir = TempTestDir::new("mismatch_no_camera_reset");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        let save = save_with_extra_state();
        write_file(&config.path.final_path, &ron::to_string(&save).unwrap());

        send_load_request(&mut app);
        app.update();

        let mut state: SystemState<MessageReader<crate::map::camera::CameraResetRequestMessage>> =
            SystemState::new(app.world_mut());
        let count = state
            .get_mut(app.world_mut())
            .expect("reader")
            .read()
            .count();
        assert_eq!(count, 0, "a failed load must never request a camera reset");
    }

    // 17. GamePausedは変化しない。
    #[test]
    fn runtime_compatibility_failure_does_not_change_game_paused() {
        let temp_dir = TempTestDir::new("mismatch_paused_unchanged");
        let mut app = setup_minimal_load_app();
        app.insert_resource(crate::app::time::GamePaused(false));
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        let save = save_with_extra_state();
        write_file(&config.path.final_path, &ron::to_string(&save).unwrap());

        send_load_request(&mut app);
        app.update();

        assert!(
            !app.world().resource::<crate::app::time::GamePaused>().0,
            "a failed load must not change GamePaused"
        );
    }

    // 18. 次フレームに再実行されない(新しいLoadRequestMessageが無い限り、
    //     LoadExecutionCountはそれ以上増えない)。
    #[test]
    fn runtime_compatibility_failure_does_not_reexecute_on_next_frame() {
        let temp_dir = TempTestDir::new("mismatch_no_reexec");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        let save = save_with_extra_state();
        write_file(&config.path.final_path, &ron::to_string(&save).unwrap());

        send_load_request(&mut app);
        app.update();
        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 1);

        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<LoadExecutionCount>().0,
            1,
            "must not re-execute on subsequent frames without a new LoadRequestMessage"
        );
    }

    #[test]
    fn load_failures_never_panic_across_multiple_kinds() {
        let temp_dir = TempTestDir::new("load_never_panics");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);

        send_load_request(&mut app);
        app.update();

        write_file(&config.path.final_path, "{{{not ron");
        send_load_request(&mut app);
        app.update();

        let mut bad_save = representative_valid_save();
        bad_save.player_country = Some(CountryId(999));
        write_file(&config.path.final_path, &ron::to_string(&bad_save).unwrap());
        send_load_request(&mut app);
        app.update();

        // ここまで到達していること自体が、いずれもpanicしなかった証拠。
        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 3);
    }

    // ─── Save/Load同時要求の調停 ────────────────────────────────────────────

    #[test]
    fn save_only_executes_save_once() {
        let temp_dir = TempTestDir::new("arb_save_only");
        let mut app = setup_minimal_load_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);

        send_save_request(&mut app);
        app.update();

        assert_eq!(app.world().resource::<SaveExecutionCount>().0, 1);
        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 0);
    }

    #[test]
    fn load_only_executes_load_once() {
        let temp_dir = TempTestDir::new("arb_load_only");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_load_request(&mut app);
        app.update();

        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 1);
        assert_eq!(app.world().resource::<SaveExecutionCount>().0, 0);
    }

    #[test]
    fn save_and_load_same_frame_executes_load_only() {
        let temp_dir = TempTestDir::new("arb_both");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_save_request(&mut app);
        send_load_request(&mut app);
        app.update();

        assert_eq!(
            app.world().resource::<LoadExecutionCount>().0,
            1,
            "load must execute"
        );
        assert_eq!(
            app.world().resource::<SaveExecutionCount>().0,
            0,
            "save must be skipped when a load is pending the same frame"
        );
    }

    #[test]
    fn failed_simultaneous_load_still_skips_save() {
        let temp_dir = TempTestDir::new("arb_load_fails_skips_save");
        let mut app = setup_minimal_load_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);

        send_save_request(&mut app);
        send_load_request(&mut app);
        app.update();

        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 1);
        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Failure { .. }) => {}
            other => panic!("expected the simultaneous load to fail (no save file), got {other:?}"),
        }
        assert_eq!(
            app.world().resource::<SaveExecutionCount>().0,
            0,
            "save must be skipped even though the simultaneous load failed"
        );
    }

    #[test]
    fn consumed_save_request_does_not_carry_over_to_next_frame() {
        let temp_dir = TempTestDir::new("arb_no_carry_over");
        let mut app = setup_minimal_load_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);

        send_save_request(&mut app);
        send_load_request(&mut app);
        app.update();
        assert_eq!(app.world().resource::<SaveExecutionCount>().0, 0);

        app.update();
        assert_eq!(
            app.world().resource::<SaveExecutionCount>().0,
            0,
            "the frame-1 SaveRequestMessage must not carry over and execute on frame 2"
        );
    }

    #[test]
    fn multiple_saves_and_loads_still_collapse_to_one_load() {
        let temp_dir = TempTestDir::new("arb_multi_both");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_save_request(&mut app);
        send_save_request(&mut app);
        send_load_request(&mut app);
        send_load_request(&mut app);
        send_load_request(&mut app);
        app.update();

        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 1);
        assert_eq!(app.world().resource::<SaveExecutionCount>().0, 0);
    }

    // ─── 通知 ────────────────────────────────────────────────────────────────

    fn read_notifications(app: &mut App) -> Vec<String> {
        let mut state: SystemState<MessageReader<GameNotification>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .expect("reader")
            .read()
            .map(|n| n.message.clone())
            .collect()
    }

    #[test]
    fn save_success_emits_exactly_one_notification() {
        let temp_dir = TempTestDir::new("notif_save_success");
        let mut app = setup_minimal_load_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);

        send_save_request(&mut app);
        app.update();

        let notifications = read_notifications(&mut app);
        assert_eq!(
            notifications.len(),
            1,
            "exactly one save-success notification expected"
        );
    }

    #[test]
    fn save_failure_emits_exactly_one_notification() {
        let temp_dir = TempTestDir::new("notif_save_failure");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        // write.rsの失敗注入と同じ手法: 一時ファイルパスをディレクトリとして先取りし、
        // File::createを確実に失敗させる。
        std::fs::create_dir_all(config.path.temp_path()).unwrap();

        send_save_request(&mut app);
        app.update();

        match &app.world().resource::<LastSaveOutcome>().0 {
            Some(SaveOutcome::Failure { .. }) => {}
            other => panic!("expected a save failure, got {other:?}"),
        }
        let notifications = read_notifications(&mut app);
        assert_eq!(
            notifications.len(),
            1,
            "exactly one save-failure notification expected"
        );
    }

    #[test]
    fn load_success_emits_exactly_one_notification() {
        let temp_dir = TempTestDir::new("notif_load_success");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_load_request(&mut app);
        app.update();

        let notifications = read_notifications(&mut app);
        assert_eq!(
            notifications.len(),
            1,
            "exactly one load-success notification expected"
        );
    }

    /// `read_notifications`は呼び出しのたびに新しい`SystemState`(=新しいカーソル)を
    /// 作るため、複数フレームにまたがって「今回新たに届いた分だけ」を数えるテストでは
    /// 使えない(Bevyのメッセージバッファは直近数フレーム分を保持するため、新しい
    /// カーソルは過去のメッセージも再度読んでしまう)。この構造体は1つの`SystemState`を
    /// 呼び出し間で使い回すことで、`handle_notifications`(実際の消費側システム)と
    /// 同じ「前回読んだ位置から続きだけを読む」挙動を再現する。
    struct NotificationCursor(SystemState<MessageReader<'static, 'static, GameNotification>>);

    impl NotificationCursor {
        fn new(app: &mut App) -> Self {
            Self(SystemState::new(app.world_mut()))
        }

        fn drain(&mut self, app: &mut App) -> Vec<String> {
            self.0
                .get_mut(app.world_mut())
                .expect("reader")
                .read()
                .map(|n| n.message.clone())
                .collect()
        }
    }

    #[test]
    fn load_failure_categories_produce_distinct_notification_text() {
        let temp_dir = TempTestDir::new("notif_load_failure_categories");

        // FileNotFound
        let mut app = setup_minimal_load_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);
        let mut cursor = NotificationCursor::new(&mut app);
        send_load_request(&mut app);
        app.update();
        let file_not_found_notifs = cursor.drain(&mut app);
        assert_eq!(file_not_found_notifs.len(), 1);

        // Deserialize
        let config = temp_dir.save_file_config();
        write_file(&config.path.final_path, "{{{not ron");
        send_load_request(&mut app);
        app.update();
        let deserialize_notifs = cursor.drain(&mut app);
        assert_eq!(deserialize_notifs.len(), 1);

        assert_ne!(
            file_not_found_notifs[0], deserialize_notifs[0],
            "different load failure categories must produce distinguishable notification text"
        );
    }

    #[test]
    fn same_outcome_is_not_renotified_on_the_next_frame() {
        let temp_dir = TempTestDir::new("notif_no_repeat");
        let mut app = setup_minimal_load_app();
        app.insert_resource(temp_dir.save_file_config());
        app.insert_state(GameState::Playing);
        let mut cursor = NotificationCursor::new(&mut app);

        send_save_request(&mut app);
        app.update();
        assert_eq!(cursor.drain(&mut app).len(), 1);

        // 新たな要求を送らない2フレーム目: 通知は再発行されない。
        app.update();
        assert_eq!(
            cursor.drain(&mut app).len(),
            0,
            "an unchanged outcome must not be re-notified on a frame with no new request"
        );
    }

    #[test]
    fn load_success_notification_appears_after_notification_history_is_cleared() {
        // 002Dの`apply::commit_load`はNotificationHistoryを空へ同期的にリセットする
        // (Resource置換の一部)。ロード成功通知自体は`GameNotification` Messageとして
        // 発行されるだけで、`NotificationHistory`へ反映するのは既存の`handle_notifications`
        // (economy::mod、`DailySimulationSet::UiUpdate`)の役目であり、次のUpdateフレームで
        // 消費される。この2つの事実を組み合わせることで、要求された順序
        // (「リセットが先、ロード完了通知の追加が後」)が構造的に保証されていることを示す:
        // (1) このフレームのPostUpdate終了時点でNotificationHistoryは既に空
        // (2) ロード完了を示すGameNotificationは確かにキューされている
        // (両方が同時に真であれば、後で`handle_notifications`がこのMessageを消費した時点で
        // 追加される内容は、必ず「空になった履歴への追加」になる)。
        let temp_dir = TempTestDir::new("notif_load_clears_history_first");
        let mut app = setup_minimal_load_app();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );
        app.world_mut()
            .resource_mut::<crate::ui::notification::NotificationHistory>()
            .add("stale notification from the old game".to_string());

        send_load_request(&mut app);
        app.update();

        assert!(
            app.world()
                .resource::<crate::ui::notification::NotificationHistory>()
                .recent
                .is_empty(),
            "apply::commit_load must have already cleared NotificationHistory by the end of this frame's PostUpdate"
        );
        let notifications = read_notifications(&mut app);
        assert_eq!(
            notifications.len(),
            1,
            "a load-success GameNotification must be queued for the next Update frame's handle_notifications to consume"
        );
    }

    // ─── P21-SAVE-003: CountrySelection起点の「続きから」ロード ────────────────

    /// `setup_minimal_load_app`と同じ最小Worldを、`GameState::CountrySelection`から
    /// 開始する変種。`PlayingEntryMode`を明示的に挿入する(この最小Appは`AppPlugin`を
    /// 経由しないため、`init_resource`されていない)。
    fn setup_minimal_load_app_at_country_selection() -> App {
        let mut app = setup_minimal_load_app();
        app.insert_resource(PlayingEntryMode::NewGame);
        app.insert_state(GameState::CountrySelection);
        app
    }

    #[test]
    fn load_from_country_selection_does_not_read_when_no_request() {
        let temp_dir = TempTestDir::new("cs_no_request");
        let mut app = setup_minimal_load_app_at_country_selection();
        app.insert_resource(temp_dir.save_file_config());

        app.update();

        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 0);
        assert!(app.world().resource::<LastLoadOutcome>().0.is_none());
    }

    #[test]
    fn load_from_country_selection_success_sets_loaded_game_entry_mode_and_transitions_to_playing()
     {
        let temp_dir = TempTestDir::new("cs_success");
        let mut app = setup_minimal_load_app_at_country_selection();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_load_request(&mut app);
        app.update();

        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Success { .. }) => {}
            other => panic!("expected Some(Success), got {other:?}"),
        }
        assert_eq!(app.world().resource::<LoadExecutionCount>().0, 1);
        assert_eq!(
            app.world().resource::<PlayingEntryMode>(),
            &PlayingEntryMode::LoadedGame,
            "a successful CountrySelection-origin load must mark the entry mode as LoadedGame"
        );
        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::CountrySelection,
            "the state transition is only queued this frame; NextState is applied at the start of the following frame"
        );

        app.update();
        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Playing,
            "the queued transition must have applied by the following frame"
        );
        assert_eq!(
            app.world().resource::<LoadExecutionCount>().0,
            1,
            "the transition frame itself must not re-execute the load (no new LoadRequestMessage)"
        );
    }

    #[test]
    fn load_from_country_selection_success_does_not_request_camera_reset() {
        let temp_dir = TempTestDir::new("cs_no_camera_reset");
        let mut app = setup_minimal_load_app_at_country_selection();
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_load_request(&mut app);
        app.update();

        let mut state: SystemState<MessageReader<CameraResetRequestMessage>> =
            SystemState::new(app.world_mut());
        let count = state
            .get_mut(app.world_mut())
            .expect("reader")
            .read()
            .count();
        assert_eq!(
            count, 0,
            "a startup ('continue') load must not request a camera reset; the camera is already at setup_camera's default"
        );
    }

    #[test]
    fn load_from_country_selection_failure_stays_in_country_selection() {
        let temp_dir = TempTestDir::new("cs_failure_stays");
        let mut app = setup_minimal_load_app_at_country_selection();
        app.insert_resource(temp_dir.save_file_config()); // ファイル無し -> FileNotFound

        send_load_request(&mut app);
        app.update();

        match &app.world().resource::<LastLoadOutcome>().0 {
            Some(LoadOutcome::Failure {
                error: LoadOperationError::ReadOrValidate(LoadSaveError::FileNotFound(_)),
                ..
            }) => {}
            other => panic!("expected FileNotFound failure, got {other:?}"),
        }
        assert_eq!(
            app.world().resource::<PlayingEntryMode>(),
            &PlayingEntryMode::NewGame,
            "a failed load must not touch PlayingEntryMode"
        );

        app.update();
        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::CountrySelection,
            "a failed load must never transition to Playing"
        );
        assert_eq!(
            app.world().resource::<LoadExecutionCount>().0,
            1,
            "must not re-execute on the following frame without a new LoadRequestMessage"
        );
    }

    #[test]
    fn load_from_country_selection_failure_does_not_change_player_country_or_preview() {
        let temp_dir = TempTestDir::new("cs_failure_preserves_selection");
        let mut app = setup_minimal_load_app_at_country_selection();
        app.insert_resource(temp_dir.save_file_config()); // FileNotFound
        app.insert_resource(crate::ui::country_selection::PreviewCountry(Some(
            CountryId(0),
        )));
        let before_player_country = app.world().resource::<PlayerCountry>().0;
        let before_preview = app
            .world()
            .resource::<crate::ui::country_selection::PreviewCountry>()
            .0;

        send_load_request(&mut app);
        app.update();

        assert_eq!(
            app.world().resource::<PlayerCountry>().0,
            before_player_country
        );
        assert_eq!(
            app.world()
                .resource::<crate::ui::country_selection::PreviewCountry>()
                .0,
            before_preview
        );
    }

    #[test]
    fn load_while_already_playing_still_requests_camera_reset_and_does_not_touch_entry_mode() {
        // 回帰確認: GameState::Playing中のロード(既存のゲーム内ロード)は
        // P21-SAVE-003以前の挙動から変化していないこと。
        let temp_dir = TempTestDir::new("playing_still_resets_camera");
        let mut app = setup_minimal_load_app();
        app.insert_resource(PlayingEntryMode::NewGame);
        let config = temp_dir.save_file_config();
        app.insert_resource(config.clone());
        app.insert_state(GameState::Playing);
        write_file(
            &config.path.final_path,
            &ron::to_string(&representative_valid_save()).unwrap(),
        );

        send_load_request(&mut app);
        app.update();

        let mut state: SystemState<MessageReader<CameraResetRequestMessage>> =
            SystemState::new(app.world_mut());
        let count = state
            .get_mut(app.world_mut())
            .expect("reader")
            .read()
            .count();
        assert_eq!(
            count, 1,
            "an in-Playing load must still request a camera reset, unchanged from before P21-SAVE-003"
        );
        assert_eq!(
            app.world().resource::<PlayingEntryMode>(),
            &PlayingEntryMode::NewGame,
            "an in-Playing load must never touch PlayingEntryMode (no state transition occurs)"
        );
    }
}
