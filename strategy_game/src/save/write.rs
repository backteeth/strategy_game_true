/// P21-SAVE-002B: SaveGameV1を人間が調査可能なRONへシリアライズし、単一スロットへ
/// 安全に書き込む。ファイルI/O層のみを扱う(Bevyへの依存なし)。
///
/// 書き込み手順(指示された順序を厳守):
/// 1. 保存先の親ディレクトリを`create_dir_all`
/// 2. `SaveGameV1`をRON文字列へ完全にシリアライズ
/// 3. 最終ファイルと同じディレクトリの一時ファイルを開く
/// 4. RON全体を一時ファイルへ`write_all`
/// 5. `flush`
/// 6. 一時ファイルへ`sync_all`
/// 7. 一時ファイルから最終ファイルへ`rename`(置換)
/// 8. 成功時、一時ファイルは`rename`によって既に移動済みで残らない
///
/// 失敗時は既存の最終ファイルへ一切触れない(削除も上書きもしない)。
/// `remove_file(final) → rename(tmp, final)`のような「先に消してから置く」経路は使わない。
use crate::save::dto::SaveGameV1;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// 保存先の設定。既定値はプロジェクト相対`saves/savegame_v1.ron`。
/// テストでは一意な一時ディレクトリへ差し替える(`SavePathConfig { final_path: .. }`を
/// 直接構築するだけでよい)。OS標準ユーザーデータディレクトリへの将来移行は、この構造体の
/// `final_path`をどう決定するかだけの変更で完結する(呼び出し側や`SaveGameV1`には影響しない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePathConfig {
    pub final_path: PathBuf,
}

impl Default for SavePathConfig {
    fn default() -> Self {
        Self {
            final_path: PathBuf::from("saves/savegame_v1.ron"),
        }
    }
}

impl SavePathConfig {
    /// 最終ファイルと同一ディレクトリの一時ファイルパス(`<final>.tmp`)。
    pub fn temp_path(&self) -> PathBuf {
        let mut os_string = self.final_path.clone().into_os_string();
        os_string.push(".tmp");
        PathBuf::from(os_string)
    }
}

/// セーブ失敗の種別。技術的詳細(`String`)は診断用であり、ゲーム状態の永続DTO
/// (`SaveGameV1`)には一切含まれない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    Serialize(String),
    CreateDirectory(String),
    CreateTempFile(String),
    Write(String),
    FlushOrSync(String),
    Rename(String),
}

/// セーブ結果。将来のUIタスク(P21-SAVE-002E想定)がそのまま表示に使える構造。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Success { path: PathBuf },
    Failure { path: PathBuf, error: SaveError },
}

/// `SaveGameV1`を安全に単一スロットへ書き込む。
///
/// パニックしない: 全ての失敗は`SaveOutcome::Failure`として返る。
/// 既存の最終ファイルを保護する: 一時ファイルの書き込み・同期が完了するまで最終ファイルへは
/// 一切触れず、`rename`が失敗した場合も最終ファイルは変更されない。
///
/// 本番経路: 置換処理に`std::fs::rename`をそのまま渡すだけの薄いラッパー
/// (`write_save_file_with_replace`本体を参照)。
pub fn write_save_file(save: &SaveGameV1, config: &SavePathConfig) -> SaveOutcome {
    write_save_file_with_replace(save, config, |from, to| fs::rename(from, to))
}

/// `write_save_file`の本体。置換処理(手順7)だけを`replace`として外部から差し替え可能にする
/// (P21-SAVE-002B1: `rename`失敗を決定的に注入するテストのため)。
/// 公開APIは増やさない(このファイル内のテストからのみ`use super::*`経由で呼ぶ private 関数)。
///
/// 手順1〜6・8はテストの有無に関わらず本番と全く同じ経路を通る。`replace`が置き換えるのは
/// 手順7(`tmp → final`への置換)だけであり、`SaveGameV1`自体にテスト都合の分岐は一切ない。
fn write_save_file_with_replace<F>(
    save: &SaveGameV1,
    config: &SavePathConfig,
    replace: F,
) -> SaveOutcome
where
    F: FnOnce(&Path, &Path) -> std::io::Result<()>,
{
    let final_path = config.final_path.clone();

    // 1. 親ディレクトリを作成
    if let Some(parent) = final_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent)
    {
        return SaveOutcome::Failure {
            path: final_path,
            error: SaveError::CreateDirectory(e.to_string()),
        };
    }

    // 2. RON文字列へ完全にシリアライズ(人間が調査可能な形式: PrettyConfig使用)
    let ron_string = match ron::ser::to_string_pretty(save, ron::ser::PrettyConfig::default()) {
        Ok(s) => s,
        Err(e) => {
            return SaveOutcome::Failure {
                path: final_path,
                error: SaveError::Serialize(e.to_string()),
            };
        }
    };

    let temp_path = config.temp_path();

    // 3. 一時ファイルを開く(既存のstale .tmpがあっても`File::create`は上書きするため
    //    そのまま安全に再試行できる)
    let mut file = match fs::File::create(&temp_path) {
        Ok(f) => f,
        Err(e) => {
            return SaveOutcome::Failure {
                path: final_path,
                error: SaveError::CreateTempFile(e.to_string()),
            };
        }
    };

    // 4. RON全体を書き込む
    if let Err(e) = file.write_all(ron_string.as_bytes()) {
        let _ = fs::remove_file(&temp_path);
        return SaveOutcome::Failure {
            path: final_path,
            error: SaveError::Write(e.to_string()),
        };
    }

    // 5. flush
    if let Err(e) = file.flush() {
        let _ = fs::remove_file(&temp_path);
        return SaveOutcome::Failure {
            path: final_path,
            error: SaveError::FlushOrSync(e.to_string()),
        };
    }

    // 6. sync_all(OSバッファではなくディスクへの書き込みを保証)
    if let Err(e) = file.sync_all() {
        let _ = fs::remove_file(&temp_path);
        return SaveOutcome::Failure {
            path: final_path,
            error: SaveError::FlushOrSync(e.to_string()),
        };
    }
    drop(file);

    // 7. 一時ファイルから最終ファイルへ置換。
    //    本番経路(`write_save_file`)ではWindows/Unixとも`std::fs::rename`は同一ファイル
    //    システム内であれば既存の最終ファイルを安全に置き換える(事前のremove_fileは
    //    行わない)。ただし「全OS・全ファイルシステムで完全にアトミック」とは断定しない
    //    (temp/finalが異なるファイルシステムに置かれた場合の挙動はOS依存)。
    if let Err(e) = replace(&temp_path, &final_path) {
        let _ = fs::remove_file(&temp_path);
        return SaveOutcome::Failure {
            path: final_path,
            error: SaveError::Rename(e.to_string()),
        };
    }

    // 8. renameによって一時ファイルは最終パスへ移動済み(残っていない)
    SaveOutcome::Success { path: final_path }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::CountryId;
    use crate::country::{CountryData, EconomicSystem, GovernmentType};
    use crate::save::dto::{
        SAVE_FORMAT_VERSION_V1, SavedArmyRegistry, SavedBattleRegistry, SavedClaimRegistry,
        SavedCountryAiRegistry, SavedCrisisRegistry, SavedDiplomacyRegistry,
        SavedFrontlineRegistry, SavedGameDate, SavedMilitaryAiRegistry, SavedMilitaryRegistry,
        SavedWarJustificationRegistry, SavedWarRegistry, SavedWorldCivilizationState,
    };
    use crate::state::data::StateRegistry;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 実リポジトリの`saves/`へ書き込まないよう、テストごとに一意なOS一時ディレクトリを
    /// 発行する(並列実行されるテスト同士の衝突を避けるため、プロセスID+アトミック
    /// カウンタ+タイムスタンプを組み合わせる)。
    fn unique_temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "strategy_game_p21_save_002b_{label}_{}_{nanos}_{n}",
            std::process::id()
        ))
    }

    /// テスト終了時(パニック時含む)に一時ディレクトリを後始末するRAIIガード。
    struct TempTestDir(PathBuf);

    impl TempTestDir {
        fn new(label: &str) -> Self {
            Self(unique_temp_dir(label))
        }

        fn config(&self) -> SavePathConfig {
            SavePathConfig {
                final_path: self.0.join("savegame_v1.ron"),
            }
        }
    }

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn representative_save(game_speed: u8) -> SaveGameV1 {
        SaveGameV1 {
            version: SAVE_FORMAT_VERSION_V1,
            date: SavedGameDate {
                year: 1801,
                month: 6,
                day: 15,
                accumulator: 0.37,
            },
            game_speed,
            player_country: Some(CountryId(1)),
            world_civilization: SavedWorldCivilizationState {
                current_stage: crate::research::data::WorldStage::IndustrialRevolution,
                milestone_countries: HashMap::new(),
                last_advanced_date: "1801/01/01".to_string(),
            },
            countries: vec![CountryData {
                id: CountryId(1),
                name: "Test Country".to_string(),
                government_type: GovernmentType::Monarchy,
                economic_system: EconomicSystem::Mercantilism,
                ..CountryData::default()
            }],
            states: StateRegistry::default().states,
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

    #[test]
    fn creates_missing_temp_directory_automatically() {
        let temp_dir = TempTestDir::new("missing_dir");
        // savesディレクトリ自体をまだ作らず、さらにネストしたサブディレクトリへ保存を試みる。
        let config = SavePathConfig {
            final_path: temp_dir.0.join("nested").join("a").join("savegame_v1.ron"),
        };
        let save = representative_save(1);

        let outcome = write_save_file(&save, &config);
        assert_eq!(
            outcome,
            SaveOutcome::Success {
                path: config.final_path.clone()
            }
        );
        assert!(config.final_path.exists());
    }

    #[test]
    fn saved_file_round_trips_through_ron_from_str() {
        let temp_dir = TempTestDir::new("round_trip");
        let config = temp_dir.config();
        let save = representative_save(3);

        let outcome = write_save_file(&save, &config);
        assert!(matches!(outcome, SaveOutcome::Success { .. }));

        let contents = fs::read_to_string(&config.final_path).unwrap();
        let restored: SaveGameV1 =
            ron::from_str(&contents).expect("written RON must parse back into SaveGameV1");
        assert_eq!(restored.version, SAVE_FORMAT_VERSION_V1);
        assert_eq!(restored.game_speed, 3);
        assert_eq!(restored.countries.len(), 1);
    }

    #[test]
    fn saved_ron_contains_version_field() {
        let temp_dir = TempTestDir::new("has_version");
        let config = temp_dir.config();
        write_save_file(&representative_save(1), &config);

        let contents = fs::read_to_string(&config.final_path).unwrap();
        assert!(contents.contains("version"));
    }

    #[test]
    fn no_tmp_file_remains_after_successful_save() {
        let temp_dir = TempTestDir::new("no_tmp_left");
        let config = temp_dir.config();
        let outcome = write_save_file(&representative_save(1), &config);
        assert!(matches!(outcome, SaveOutcome::Success { .. }));
        assert!(!config.temp_path().exists());
    }

    /// Windows上での重要な検証: 既存の最終ファイルがある状態でも、`rename`によって
    /// 安全に置き換えられる(remove_file(final)を先に呼ぶフォールバックは使っていない)。
    #[test]
    fn second_save_safely_updates_the_same_slot_on_windows() {
        let temp_dir = TempTestDir::new("second_save");
        let config = temp_dir.config();

        let first = write_save_file(&representative_save(1), &config);
        assert!(matches!(first, SaveOutcome::Success { .. }));
        let first_contents = fs::read_to_string(&config.final_path).unwrap();
        assert!(
            first_contents.contains("game_speed:1") || first_contents.contains("game_speed: 1")
        );

        let second = write_save_file(&representative_save(4), &config);
        assert!(
            matches!(second, SaveOutcome::Success { .. }),
            "rename must be able to replace an existing final file on this platform, got {second:?}"
        );

        let second_contents = fs::read_to_string(&config.final_path).unwrap();
        let restored: SaveGameV1 = ron::from_str(&second_contents).unwrap();
        assert_eq!(
            restored.game_speed, 4,
            "second save's content must fully replace the first save's content"
        );
        assert!(!config.temp_path().exists());
    }

    /// 書き込み・置換に失敗した場合、既存の最終ファイルが維持される。
    /// 一時ファイルパスへ事前にディレクトリを作っておくことで、`File::create`(一時ファイル
    /// オープン)を確実に失敗させる(ディレクトリを通常ファイルとして開くことはできないため、
    /// 環境に依存しない再現性のある失敗注入方法)。
    #[test]
    fn write_failure_preserves_existing_final_file_and_does_not_panic() {
        let temp_dir = TempTestDir::new("write_failure");
        let config = temp_dir.config();

        let first = write_save_file(&representative_save(9), &config);
        assert!(matches!(first, SaveOutcome::Success { .. }));
        let original_contents = fs::read_to_string(&config.final_path).unwrap();

        // 一時ファイルパスをディレクトリとして先取りし、次のFile::createを確実に失敗させる。
        fs::create_dir_all(config.temp_path()).unwrap();

        let second = write_save_file(&representative_save(99), &config);
        assert!(
            matches!(
                second,
                SaveOutcome::Failure {
                    error: SaveError::CreateTempFile(_),
                    ..
                }
            ),
            "expected a CreateTempFile failure, got {second:?}"
        );

        let contents_after_failure = fs::read_to_string(&config.final_path).unwrap();
        assert_eq!(
            contents_after_failure, original_contents,
            "existing final file must be untouched after a failed save"
        );

        // 後始末(このディレクトリはTempTestDir::dropが親ごと削除するため必須ではないが、
        // 明示的にクリーンアップしておく)
        let _ = fs::remove_dir_all(config.temp_path());
    }

    /// P21-SAVE-002B1: 置換(rename)処理そのものが失敗した場合に、既存の最終ファイルが
    /// 維持されることを直接実証する。`write_failure_preserves_existing_final_file_and_does_not_panic`
    /// (CreateTempFile失敗を注入)とは異なり、一時ファイルの書き込み・flush・syncまでは
    /// 本番と全く同じ経路で成功させ、置換関数だけを決定的に失敗させる。
    #[test]
    fn rename_failure_preserves_existing_final_file_and_does_not_panic() {
        let temp_dir = TempTestDir::new("rename_failure");
        let config = temp_dir.config();

        // 1. 内容Aを通常のwrite_save_fileで保存する
        let first = write_save_file(&representative_save(11), &config);
        assert!(matches!(first, SaveOutcome::Success { .. }));

        // 2. 最終ファイルが存在し、内容Aを読み戻せることを確認する(意味的な比較、
        //    RONのバイト列比較ではない)
        let original_contents = fs::read_to_string(&config.final_path).unwrap();
        let original_restored: SaveGameV1 = ron::from_str(&original_contents).unwrap();
        assert_eq!(original_restored.game_speed, 11);

        // 3〜5. 内容Bの保存を開始する。一時ファイルの書き込み・flush・syncまでは
        //    write_save_file_with_replaceの本番と同じ経路を通るため成功する。
        //    置換関数だけが意図的にio::Errorを返す(決定的な失敗注入)。
        let second = write_save_file_with_replace(&representative_save(22), &config, |_, _| {
            Err(std::io::Error::other("injected rename failure for testing"))
        });

        // 6〜7. SaveOutcome::Failureが返り、エラー種別がSaveError::Renameである
        assert!(
            matches!(
                second,
                SaveOutcome::Failure {
                    error: SaveError::Rename(_),
                    ..
                }
            ),
            "expected a Rename failure, got {second:?}"
        );

        // 8. 最終ファイルが内容Aのまま維持されている(内容Bへ変化していない)。
        //    意味的な比較(Deserialize後の代表フィールド)で確認する。
        let contents_after_failure = fs::read_to_string(&config.final_path).unwrap();
        let restored_after_failure: SaveGameV1 = ron::from_str(&contents_after_failure).unwrap();
        assert_eq!(
            restored_after_failure.game_speed, 11,
            "final file must still hold content A's data after a rename failure, not B's"
        );

        // 9. .tmpが後始末されている
        assert!(
            !config.temp_path().exists(),
            "temp file must be cleaned up after a rename failure"
        );

        // 10. (このテスト自体がここまで到達して完了していることが、panicしなかった証拠)

        // 11. 続けて通常保存を行えば内容Bへ更新できる(rename失敗後も通常の保存経路が
        //     壊れていないことの確認、回帰なし)
        let third = write_save_file(&representative_save(33), &config);
        assert!(matches!(third, SaveOutcome::Success { .. }));
        let final_contents = fs::read_to_string(&config.final_path).unwrap();
        let final_restored: SaveGameV1 = ron::from_str(&final_contents).unwrap();
        assert_eq!(
            final_restored.game_speed, 33,
            "a normal save after a rename failure must succeed and fully update the slot"
        );
        assert!(!config.temp_path().exists());
    }

    #[test]
    fn save_outcome_records_success_and_failure_structurally() {
        let temp_dir = TempTestDir::new("outcome_shape");
        let config = temp_dir.config();

        let success = write_save_file(&representative_save(1), &config);
        match success {
            SaveOutcome::Success { path } => assert_eq!(path, config.final_path),
            other => panic!("expected Success, got {other:?}"),
        }

        // 親をファイルとして作ることでcreate_dir_all自体を失敗させる
        let blocked_parent = temp_dir.0.join("blocked_parent_as_file");
        fs::write(&blocked_parent, b"not a directory").unwrap();
        let bad_config = SavePathConfig {
            final_path: blocked_parent.join("savegame_v1.ron"),
        };
        let failure = write_save_file(&representative_save(1), &bad_config);
        match failure {
            SaveOutcome::Failure { path, error } => {
                assert_eq!(path, bad_config.final_path);
                assert!(matches!(error, SaveError::CreateDirectory(_)));
            }
            other => panic!("expected Failure, got {other:?}"),
        }
    }

    #[test]
    fn tests_use_only_os_temp_dir_never_the_repository_saves_directory() {
        // このモジュールの全テストは`TempTestDir`(std::env::temp_dir()配下)のみを使い、
        // 一度も`SavePathConfig::default()`(プロジェクト相対`saves/`)を呼んでいない
        // ことをコードレビューで確認できる。念のため、defaultのパス自体がリポジトリ相対の
        // 文字列であることだけを構造的に確認する(defaultを実際に書き込みには使わない)。
        let default_config = SavePathConfig::default();
        assert_eq!(
            default_config.final_path,
            PathBuf::from("saves/savegame_v1.ron")
        );
    }
}
