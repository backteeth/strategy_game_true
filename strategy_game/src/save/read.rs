/// P21-SAVE-002C: セーブファイルの読み取り・RON解析・version検証・構造化エラーへの変換。
///
/// このファイルは`bevy`を一切importしない(`dto.rs`/`export.rs`/`write.rs`/`validate.rs`と
/// 同じ方針)。読み取り中は次を一切行わない: ファイルの変更、`.tmp`の削除、staleファイルの
/// 修復、現在のResourceへの書き込み、`GamePaused`の変更、UI状態の変更、panic。
use crate::save::dto::{SAVE_FORMAT_VERSION_V1, SaveGameV1};
use crate::save::validate::{
    SaveValidationContext, SaveValidationErrors, ValidatedSaveGameV1, validate_save_game_v1,
};
use crate::save::write::SavePathConfig;
use serde::Deserialize;
use std::fs;
use std::io::ErrorKind;

/// `SaveGameV1`全体をDeserializeするより前に`version`だけを読み取るための最小ヘッダー型。
///
/// `#[serde(deny_unknown_fields)]`を付けていないため、`version`以外のフィールドを持つ
/// 完全な`SaveGameV1`のRONに対しても、`version`さえ存在すれば正常にDeserializeできる
/// (serdeの派生`Deserialize`実装は、既知のフィールド名に一致しないキーを既定で読み飛ばす。
/// `tests::version_header_parses_from_a_full_save_game_v1_document`で実際に確認する)。
/// これにより、未来バージョンのセーブを「壊れたV1」として`Deserialize`エラーに埋もれさせず、
/// `UnsupportedVersion`として区別できる。
#[derive(Debug, Deserialize)]
struct SaveVersionHeader {
    version: u32,
}

/// 読込・検証全体で発生し得る構造化エラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadSaveError {
    /// セーブファイルが存在しない。
    FileNotFound(String),
    /// ファイルは存在するが読み取りに失敗した(権限・I/Oエラー等)。
    Read(String),
    /// RONとして解析できない、またはversion確認後の`SaveGameV1`としてDeserializeできない。
    Deserialize(String),
    /// `version`は読み取れたが、このビルドが対応するV1ではない。
    UnsupportedVersion { found: u32 },
    /// 参照整合性・静的マスター参照・不変条件の検証に失敗した。
    Validation(SaveValidationErrors),
}

/// セーブファイルを読み取り、version検証・全参照整合性検証まで行う。
///
/// 処理順: (1) `config.final_path`を読み取る (2) versionだけを先に確認する
/// (3) `SaveGameV1`としてDeserializeする (4) 全参照整合性を検証する
/// (5) 全て成功した場合だけ`ValidatedSaveGameV1`を返す。
///
/// 途中でファイルの変更・`.tmp`の削除・stale修復・現在のResourceへの書き込みは行わない
/// (この関数は`&SavePathConfig`/`&SaveValidationContext`という読み取り専用の入力しか
/// 取らない、という関数シグネチャ自体が構造的な保証になる)。
pub fn read_and_validate_save_file(
    config: &SavePathConfig,
    context: &SaveValidationContext,
) -> Result<ValidatedSaveGameV1, LoadSaveError> {
    let path = &config.final_path;

    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(LoadSaveError::FileNotFound(format!(
                "save file not found: {}",
                path.display()
            )));
        }
        Err(e) => {
            return Err(LoadSaveError::Read(format!(
                "failed to read {}: {e}",
                path.display()
            )));
        }
    };

    if let Ok(header) = ron::from_str::<SaveVersionHeader>(&contents)
        && header.version != SAVE_FORMAT_VERSION_V1
    {
        return Err(LoadSaveError::UnsupportedVersion {
            found: header.version,
        });
    }
    // ヘッダーとしてversionが読み取れなかった場合(version欠落、または構文自体が壊れている)は、
    // ここでは判断せず後続の完全なDeserializeへ委ねる。versionが欠落しているだけの場合も
    // `SaveGameV1::version`には`#[serde(default)]`が付いていないため、後続のDeserializeが
    // 同じ理由で失敗し、`LoadSaveError::Deserialize`として扱われる(意図的な仕様。
    // versionフィールド欠落を「壊れたV1」の一種として扱い、未来バージョンの検出
    // [`UnsupportedVersion`]とは区別しない)。

    let save: SaveGameV1 = ron::from_str(&contents)
        .map_err(|e| LoadSaveError::Deserialize(format!("failed to parse save file: {e}")))?;

    validate_save_game_v1(save, context).map_err(LoadSaveError::Validation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::data::{BuildingDefinition, BuildingType};
    use crate::common::{CountryId, DivisionDefinitionId};
    use crate::country::{CountryData, EconomicSystem, GovernmentType};
    use crate::military::data::DivisionDefinition;
    use crate::research::data::WorldStage;
    use crate::research::world_stage::WorldStageDefinition;
    use crate::save::dto::{
        SavedArmyRegistry, SavedBattleRegistry, SavedClaimRegistry, SavedCountryAiRegistry,
        SavedCrisisRegistry, SavedDiplomacyRegistry, SavedFrontlineRegistry, SavedGameDate,
        SavedMilitaryAiRegistry, SavedMilitaryRegistry, SavedWarJustificationRegistry,
        SavedWarRegistry, SavedWorldCivilizationState,
    };
    use crate::save::write::write_save_file;
    use crate::state::data::{StateData, StateRegistry};
    use std::collections::HashMap;
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
            "strategy_game_p21_save_002c_read_{label}_{}_{nanos}_{n}",
            std::process::id()
        ))
    }

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

    /// 静的マスターデータ一式(検証コンテキスト用)と、それに整合する正常な`SaveGameV1`を
    /// 一緒に構築する(read.rs独自のfixture。validate.rsのfixtureとは意図的に分離し、
    /// 各ファイルのテストがそれぞれ自己完結するようにする)。
    struct ValidFixture {
        save: SaveGameV1,
        building_definitions: HashMap<BuildingType, BuildingDefinition>,
        technology_definitions: HashMap<String, crate::research::data::TechnologyDefinition>,
        division_definitions: HashMap<DivisionDefinitionId, DivisionDefinition>,
        world_stage_definitions: HashMap<WorldStage, WorldStageDefinition>,
    }

    impl ValidFixture {
        fn context(&self) -> SaveValidationContext<'_> {
            SaveValidationContext {
                building_definitions: &self.building_definitions,
                technology_definitions: &self.technology_definitions,
                division_definitions: &self.division_definitions,
                world_stage_definitions: &self.world_stage_definitions,
            }
        }
    }

    fn build_valid_fixture() -> ValidFixture {
        let country = CountryData {
            id: CountryId(0),
            capital_state_id: crate::common::StateId(0),
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        };

        let state = StateData {
            id: crate::common::StateId(0),
            owner_country_id: CountryId(0),
            ..StateData::default()
        };
        let states = StateRegistry::build(vec![state]).states;

        let world_stage_definitions: HashMap<WorldStage, WorldStageDefinition> = [(
            WorldStage::PreIndustrial,
            WorldStageDefinition {
                stage: WorldStage::PreIndustrial,
                display_name: "Pre-Industrial".to_string(),
                description: String::new(),
                required_previous_stage: None,
                milestone_technologies: Vec::new(),
                required_country_count: 1,
            },
        )]
        .into_iter()
        .collect();

        let save = SaveGameV1 {
            version: SAVE_FORMAT_VERSION_V1,
            date: SavedGameDate {
                year: 1800,
                month: 1,
                day: 1,
                accumulator: 0.0,
            },
            game_speed: 1,
            player_country: Some(CountryId(0)),
            world_civilization: SavedWorldCivilizationState {
                current_stage: WorldStage::PreIndustrial,
                milestone_countries: HashMap::new(),
                last_advanced_date: "1800/01/01".to_string(),
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
        };

        ValidFixture {
            save,
            building_definitions: HashMap::new(),
            technology_definitions: HashMap::new(),
            division_definitions: HashMap::new(),
            world_stage_definitions,
        }
    }

    #[test]
    fn reads_a_normal_save_produced_by_write_save_file() {
        let temp_dir = TempTestDir::new("normal_roundtrip");
        let config = temp_dir.config();
        let fixture = build_valid_fixture();

        let outcome = write_save_file(&fixture.save, &config);
        assert!(matches!(
            outcome,
            crate::save::write::SaveOutcome::Success { .. }
        ));

        let result = read_and_validate_save_file(&config, &fixture.context());
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[test]
    fn valid_file_yields_a_validated_save_game_v1_with_the_same_data() {
        let temp_dir = TempTestDir::new("validated_value");
        let config = temp_dir.config();
        let fixture = build_valid_fixture();
        write_save_file(&fixture.save, &config);

        let validated = read_and_validate_save_file(&config, &fixture.context()).unwrap();
        assert_eq!(validated.save().version, SAVE_FORMAT_VERSION_V1);
        assert_eq!(validated.save().countries.len(), 1);
        assert_eq!(validated.save().player_country, Some(CountryId(0)));
    }

    #[test]
    fn missing_file_is_rejected_as_file_not_found() {
        let temp_dir = TempTestDir::new("missing_file");
        let config = temp_dir.config();
        let fixture = build_valid_fixture();

        let result = read_and_validate_save_file(&config, &fixture.context());
        assert!(matches!(result, Err(LoadSaveError::FileNotFound(_))));
    }

    #[test]
    fn unreadable_path_is_rejected_as_read_error() {
        let temp_dir = TempTestDir::new("unreadable");
        let config = temp_dir.config();
        // ファイルの代わりにディレクトリを同じパスへ作ることで、存在はするが
        // 通常ファイルとして読み取れない状態を再現する(write.rsの失敗注入と同じ手法)。
        fs::create_dir_all(&config.final_path).unwrap();
        let fixture = build_valid_fixture();

        let result = read_and_validate_save_file(&config, &fixture.context());
        assert!(
            matches!(result, Err(LoadSaveError::Read(_))),
            "got {result:?}"
        );
    }

    #[test]
    fn malformed_ron_is_rejected_as_deserialize_error() {
        let temp_dir = TempTestDir::new("malformed_ron");
        let config = temp_dir.config();
        fs::create_dir_all(config.final_path.parent().unwrap()).unwrap();
        fs::write(&config.final_path, b"not valid ron at all {{{").unwrap();
        let fixture = build_valid_fixture();

        let result = read_and_validate_save_file(&config, &fixture.context());
        assert!(
            matches!(result, Err(LoadSaveError::Deserialize(_))),
            "got {result:?}"
        );
    }

    #[test]
    fn missing_version_field_is_rejected_as_deserialize_error() {
        let temp_dir = TempTestDir::new("missing_version");
        let config = temp_dir.config();
        let fixture = build_valid_fixture();
        let ron_str = ron::to_string(&fixture.save).unwrap();
        let without_version = ron_str.replacen("version:1,", "", 1);
        assert_ne!(
            without_version, ron_str,
            "test setup must actually remove version"
        );
        fs::create_dir_all(config.final_path.parent().unwrap()).unwrap();
        fs::write(&config.final_path, without_version).unwrap();

        let result = read_and_validate_save_file(&config, &fixture.context());
        assert!(
            matches!(result, Err(LoadSaveError::Deserialize(_))),
            "got {result:?}"
        );
    }

    #[test]
    fn version_zero_is_rejected_as_unsupported_version() {
        let temp_dir = TempTestDir::new("version_zero");
        let config = temp_dir.config();
        let fixture = build_valid_fixture();
        let ron_str = ron::to_string(&fixture.save).unwrap();
        let tampered = ron_str.replacen("version:1,", "version:0,", 1);
        assert_ne!(tampered, ron_str);
        fs::create_dir_all(config.final_path.parent().unwrap()).unwrap();
        fs::write(&config.final_path, tampered).unwrap();

        let result = read_and_validate_save_file(&config, &fixture.context());
        assert_eq!(
            result.err(),
            Some(LoadSaveError::UnsupportedVersion { found: 0 })
        );
    }

    #[test]
    fn version_two_is_rejected_as_unsupported_version() {
        let temp_dir = TempTestDir::new("version_two");
        let config = temp_dir.config();
        let fixture = build_valid_fixture();
        let ron_str = ron::to_string(&fixture.save).unwrap();
        let tampered = ron_str.replacen("version:1,", "version:2,", 1);
        assert_ne!(tampered, ron_str);
        fs::create_dir_all(config.final_path.parent().unwrap()).unwrap();
        fs::write(&config.final_path, tampered).unwrap();

        let result = read_and_validate_save_file(&config, &fixture.context());
        assert_eq!(
            result.err(),
            Some(LoadSaveError::UnsupportedVersion { found: 2 })
        );
    }

    /// `SaveVersionHeader`が、`version`以外の全フィールドを持つ完全な`SaveGameV1`の
    /// RONからも、`version`だけを問題なく読み取れることを直接確認する
    /// (完全なDeserializeより前に軽量ヘッダーで区別する設計そのものの検証)。
    #[test]
    fn version_header_parses_from_a_full_save_game_v1_document() {
        let fixture = build_valid_fixture();
        let ron_str = ron::to_string(&fixture.save).unwrap();
        let header: SaveVersionHeader = ron::from_str(&ron_str)
            .expect("a minimal {version} header must parse from a full SaveGameV1 document");
        assert_eq!(header.version, SAVE_FORMAT_VERSION_V1);
    }

    #[test]
    fn reading_does_not_modify_file_contents_or_mtime() {
        let temp_dir = TempTestDir::new("no_mutation");
        let config = temp_dir.config();
        let fixture = build_valid_fixture();
        write_save_file(&fixture.save, &config);

        let before_contents = fs::read(&config.final_path).unwrap();
        let before_mtime = fs::metadata(&config.final_path)
            .unwrap()
            .modified()
            .unwrap();

        let result = read_and_validate_save_file(&config, &fixture.context());
        assert!(result.is_ok());

        let after_contents = fs::read(&config.final_path).unwrap();
        let after_mtime = fs::metadata(&config.final_path)
            .unwrap()
            .modified()
            .unwrap();

        assert_eq!(
            before_contents, after_contents,
            "reading must not modify file contents"
        );
        assert_eq!(
            before_mtime, after_mtime,
            "reading must not modify file mtime"
        );
        assert!(
            !config.temp_path().exists(),
            "reading must never touch .tmp"
        );
    }

    #[test]
    fn reading_an_invalid_reference_save_reports_validation_error_without_touching_state() {
        let temp_dir = TempTestDir::new("invalid_reference");
        let config = temp_dir.config();
        let mut fixture = build_valid_fixture();
        // player_countryが存在しないCountryIdを指すよう破壊する。
        fixture.save.player_country = Some(CountryId(999));
        write_save_file(&fixture.save, &config);

        let before_contents = fs::read(&config.final_path).unwrap();
        let result = read_and_validate_save_file(&config, &fixture.context());
        let after_contents = fs::read(&config.final_path).unwrap();

        assert!(
            matches!(result, Err(LoadSaveError::Validation(_))),
            "got {result:?}"
        );
        assert_eq!(before_contents, after_contents);
    }
}
