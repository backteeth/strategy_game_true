//! P32-016: 新ステート経路(`MapEconomySimulation`)の実行時状態の**RONファイル**への保存・読込と、
//! 原子的な置換(Bevy非依存)。ファイル入出力は、シミュレーション状態の取得・復元
//! (`economy::map_state::snapshot`)から分離している: このモジュールは `MapEconomySnapshot` を
//! 封筒に入れて読み書きするだけで、状態の意味の検証は常に `MapEconomySimulation::restore` に任せる。
//!
//! ## 形式
//! - 保存先: `<base>/map_economy/<名前>.mapeco.ron`(`<base>` は通常 `saves`)。**名前は英数字・`_`・`-`だけ**で、
//!   パスの区切り・`..`・絶対パス・拡張子を含められない(保存先の外へは書けない)。
//! - 封筒: `(kind: "map_economy_state", format_version: 1, snapshot: (...))`。旧V1(`saves/savegame_v1.ron`、
//!   `kind` を持たず `version` を持つ)とは**別形式**で、V1を新形式として読まない・新形式をV1として読まない。
//! - 非対応の版・別の `kind` は拒否し、現行版として推測で読まない。必須項目の欠落は拒否し、既定値で補完しない
//!   (明示された空の一覧・明示された `None` は許す)。未知のフィールドも拒否する。
//!
//! ## 保護
//! - 既存の保存先ファイルを上書きするのは、**自形式として完全に解析できる場合だけ**。旧V1・別の `kind`・
//!   非対応の版・壊れたファイル・シンボリックリンク・通常ファイルでないものは、自動で上書きしない。
//! - 保存先ディレクトリは `<base>/map_economy` であること(シンボリックリンク/ジャンクションで
//!   別の場所を指していれば拒否)。`saves/savegame_v1.ron`(旧V1のスロット)は保存先になり得ない。
//!
//! ## 原子的な置換と保証範囲
//! 同じディレクトリの一時ファイル(このプロセスが `create_new` で作ったものだけ)へ全内容を書き、
//! `flush`・`sync_all` の後に `std::fs::rename` で最終ファイルへ置換する(先に削除してから置く方法は使わない)。
//! 失敗した場合、最終ファイルには触れず、自分が作った一時ファイルだけを削除する。
//! **保証するのは「この一連の操作の失敗では既存ファイルが壊れない」ことまで**。電源断・OSクラッシュを含む
//! 永続性(ディレクトリエントリの同期など)は保証しておらず、確認もしていない。
//! 確認と置換の間に別のプロセスがファイルを書き換える競合(TOCTOU)も防いでいない(単一利用者のローカルな
//! 保存を想定)。
//!
//! ## 静的データとの照合
//! ファイル内の `context` は復元先の静的入力から求めた値と照合するだけで、採用しない。これは内部の同一性確認
//! (FNV-1a 64bit。改ざん検知や真正性の保証ではない)で、外部署名による照合(P32-017)ではない。

use crate::building::data::BuildingRegistry;
use crate::economy::map_state::snapshot::{
    MapEconomySimulation, MapEconomySnapshot, RestoreError, SnapshotDate,
};
use crate::world::WorldModel;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const MAP_ECONOMY_FILE_KIND: &str = "map_economy_state";
pub const MAP_ECONOMY_FILE_FORMAT_VERSION: u32 = 1;
pub const MAP_ECONOMY_DIR_NAME: &str = "map_economy";
pub const MAP_ECONOMY_EXTENSION: &str = ".mapeco.ron";
/// 読み込みを許すファイルの上限(破損・無関係な巨大ファイルでメモリを使い切らないため)。
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_NAME_LEN: usize = 64;

// ---------------------------------------------------------------- 封筒

#[derive(Serialize)]
struct FileRef<'a> {
    kind: &'a str,
    format_version: u32,
    snapshot: &'a MapEconomySnapshot,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileOwned {
    kind: String,
    format_version: u32,
    snapshot: MapEconomySnapshot,
}

/// 種別判定用の最小ヘッダー(他のフィールドは読み飛ばす。欠落は `None`)。
#[derive(Deserialize)]
struct Header {
    kind: Option<String>,
    format_version: Option<u32>,
    /// 旧V1のセーブが持つフィールド(`kind` を持たない)。
    version: Option<u32>,
}

/// このファイルが何であると識別されたか(自形式でない場合)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedKind {
    /// `kind` を持たず `version` を持つ(旧V1セーブの形)。
    LegacyV1 { version: u32 },
    /// 別の `kind`。
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileError {
    /// 名前が許可された形式でない(英数字・`_`・`-` のみ、1〜64文字、予約名でない)。
    InvalidName(String),
    /// 保存先が `<base>/map_economy` の外、または旧V1のスロット。
    LocationRefused(String),
    /// 保存先(ディレクトリまたはファイル)がシンボリックリンク等。
    Symlink(PathBuf),
    NotFound(PathBuf),
    Io {
        op: &'static str,
        path: PathBuf,
        message: String,
    },
    TooLarge { bytes: u64 },
    /// 自形式ではない(旧V1・別の `kind`)。
    NotThisKind(DetectedKind),
    UnsupportedVersion { found: u32 },
    /// 構文エラー・途中切れ・必須項目の欠落・未知のフィールド・型の不一致。
    Malformed(String),
    Serialize(String),
    /// 既存の保存先ファイルを自形式として識別・検証できないため、上書きしない。
    OverwriteRefused { path: PathBuf, reason: String },
    /// ファイルは自形式として正しいが、復元先の状態へ復元できない(現在の状態は変更されていない)。
    Restore(RestoreError),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::InvalidName(s) => write!(f, "invalid save name: {s}"),
            FileError::LocationRefused(s) => write!(f, "save location refused: {s}"),
            FileError::Symlink(p) => write!(f, "{} is a symbolic link / reparse point (refused)", p.display()),
            FileError::NotFound(p) => write!(f, "{} does not exist", p.display()),
            FileError::Io { op, path, message } => write!(f, "{op} failed for {}: {message}", path.display()),
            FileError::TooLarge { bytes } => write!(f, "file is too large ({bytes} bytes)"),
            FileError::NotThisKind(DetectedKind::LegacyV1 { version }) => write!(
                f,
                "this is a legacy V1 save (version {version}), a different format from the map economy file; it is not loaded or overwritten"
            ),
            FileError::NotThisKind(DetectedKind::Other(k)) => {
                write!(f, "kind \"{k}\" is not \"{MAP_ECONOMY_FILE_KIND}\"")
            }
            FileError::UnsupportedVersion { found } => write!(
                f,
                "unsupported format_version {found} (supported: {MAP_ECONOMY_FILE_FORMAT_VERSION})"
            ),
            FileError::Malformed(s) => write!(f, "malformed map economy file: {s}"),
            FileError::Serialize(s) => write!(f, "serialization failed: {s}"),
            FileError::OverwriteRefused { path, reason } => write!(
                f,
                "{} already exists and cannot be identified as a map economy file ({reason}); it was not overwritten",
                path.display()
            ),
            FileError::Restore(e) => write!(f, "restore refused: {e}"),
        }
    }
}

/// 封筒へ入れて直列化する(ファイルは触らない)。
pub fn encode(snapshot: &MapEconomySnapshot) -> Result<String, FileError> {
    let file = FileRef {
        kind: MAP_ECONOMY_FILE_KIND,
        format_version: MAP_ECONOMY_FILE_FORMAT_VERSION,
        snapshot,
    };
    ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
        .map_err(|e| FileError::Serialize(e.to_string()))
}

/// テキストを封筒として解析する(ファイルは触らない)。`kind`・版を先に判定し、本体は厳密に解析する。
pub fn decode(text: &str) -> Result<MapEconomySnapshot, FileError> {
    // ヘッダーだけは、値をそのまま(`kind: "..."`)読めるよう `implicit_some` で解析する(本体の `Option` は明示のまま)。
    let header: Header = ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
        .from_str(text)
        .map_err(|e| FileError::Malformed(e.to_string()))?;
    match header.kind {
        Some(k) if k == MAP_ECONOMY_FILE_KIND => {}
        Some(k) => return Err(FileError::NotThisKind(DetectedKind::Other(k))),
        None => {
            return Err(match header.version {
                Some(version) => FileError::NotThisKind(DetectedKind::LegacyV1 { version }),
                None => FileError::Malformed("missing `kind`".to_string()),
            });
        }
    }
    match header.format_version {
        Some(MAP_ECONOMY_FILE_FORMAT_VERSION) => {}
        Some(found) => return Err(FileError::UnsupportedVersion { found }),
        None => return Err(FileError::Malformed("missing `format_version`".to_string())),
    }
    let file: FileOwned = ron::from_str(text).map_err(|e| FileError::Malformed(e.to_string()))?;
    Ok(file.snapshot)
}

// ---------------------------------------------------------------- 保存先

/// 保存先の制限。`<base>/map_economy/<名前>.mapeco.ron` だけを扱う。
#[derive(Debug, Clone)]
pub struct SaveLocation {
    base: PathBuf,
}

fn check_name(name: &str) -> Result<(), FileError> {
    let bad = |why: &str| Err(FileError::InvalidName(format!("{name:?}: {why}")));
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return bad("length must be 1..=64");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return bad("only ASCII letters, digits, '_' and '-' are allowed (no path separators, dots, drive letters or extensions)");
    }
    // Windowsの予約デバイス名(拡張子が付いても同じデバイスを指す)。
    let upper = name.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || ["COM", "LPT"].iter().any(|p| {
            upper.strip_prefix(p).is_some_and(|d| d.len() == 1 && d.chars().all(|c| c.is_ascii_digit()))
        });
    if reserved {
        return bad("reserved device name");
    }
    Ok(())
}

fn io_err(op: &'static str, path: &Path, e: &io::Error) -> FileError {
    FileError::Io { op, path: path.to_path_buf(), message: e.to_string() }
}

impl SaveLocation {
    /// `base` は通常 `saves`。実際のファイルは `<base>/map_economy/` 配下だけ。
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn dir(&self) -> PathBuf {
        self.base.join(MAP_ECONOMY_DIR_NAME)
    }

    /// 名前から保存先のパスを作る(名前の検証のみ。ファイルシステムは触らない)。
    pub fn file_path(&self, name: &str) -> Result<PathBuf, FileError> {
        check_name(name)?;
        let path = self.dir().join(format!("{name}{MAP_ECONOMY_EXTENSION}"));
        refuse_legacy_slot(&path)?;
        Ok(path)
    }

    /// 保存先ディレクトリを検証して返す。リンクで別の場所を指していれば拒否する(実体のパスで確認)。
    fn verified_dir(&self, create: bool) -> Result<PathBuf, FileError> {
        let dir = self.dir();
        if create {
            fs::create_dir_all(&dir).map_err(|e| io_err("create directory", &dir, &e))?;
        }
        let meta = match fs::symlink_metadata(&dir) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(FileError::NotFound(dir)),
            Err(e) => return Err(io_err("inspect directory", &dir, &e)),
        };
        if meta.file_type().is_symlink() {
            return Err(FileError::Symlink(dir));
        }
        if !meta.is_dir() {
            return Err(FileError::LocationRefused(format!("{} is not a directory", dir.display())));
        }
        let canon_dir = fs::canonicalize(&dir).map_err(|e| io_err("resolve directory", &dir, &e))?;
        let canon_base =
            fs::canonicalize(&self.base).map_err(|e| io_err("resolve base directory", &self.base, &e))?;
        let name_ok = canon_dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(MAP_ECONOMY_DIR_NAME));
        if canon_dir.parent() != Some(canon_base.as_path()) || !name_ok {
            return Err(FileError::LocationRefused(format!(
                "{} resolves to {}, outside {}/{MAP_ECONOMY_DIR_NAME}",
                dir.display(),
                canon_dir.display(),
                canon_base.display()
            )));
        }
        Ok(dir)
    }

    /// 名前と保存先を検証し、ファイルのパスを返す。既存のファイルがシンボリックリンク等なら拒否する。
    fn target(&self, name: &str, create_dir: bool) -> Result<PathBuf, FileError> {
        let path = self.file_path(name)?;
        let dir = self.verified_dir(create_dir)?;
        let path = dir.join(path.file_name().expect("file name"));
        match fs::symlink_metadata(&path) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    return Err(FileError::Symlink(path));
                }
                if !m.is_file() {
                    return Err(FileError::LocationRefused(format!("{} is not a regular file", path.display())));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err("inspect file", &path, &e)),
        }
        Ok(path)
    }
}

/// 旧V1のスロット(`savegame_v1.ron`)や、`.mapeco.ron` でないパスは保存先にしない。
fn refuse_legacy_slot(path: &Path) -> Result<(), FileError> {
    let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if file.eq_ignore_ascii_case("savegame_v1.ron") || !file.ends_with(MAP_ECONOMY_EXTENSION) {
        return Err(FileError::LocationRefused(format!(
            "{} is not a map economy save file (the legacy V1 slot saves/savegame_v1.ron is never written)",
            path.display()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------- 書き込みの部品(失敗を再現可能に注入できる)

/// 一時ファイルへの書き込み操作。
pub trait TempFile {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
    /// OSへの同期(`sync_all`)。
    fn sync(&mut self) -> io::Result<()>;
}

/// 保存のファイル操作。本番は [`RealIo`]。テストは失敗を注入する実装を渡す。
pub trait SaveIo {
    /// 一時ファイルを `create_new`(既存なら失敗)で作る。作ったのは自分だけ。
    fn create_temp(&self, path: &Path) -> io::Result<Box<dyn TempFile>>;
    /// 一時ファイルを最終ファイルへ置換する。
    fn replace(&self, temp: &Path, final_path: &Path) -> io::Result<()>;
    /// 自分が作った一時ファイルを削除する。
    fn remove_temp(&self, path: &Path) -> io::Result<()>;
}

pub struct RealIo;

struct RealTemp(fs::File);

impl TempFile for RealTemp {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        io::Write::write_all(&mut self.0, bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.0)
    }
    fn sync(&mut self) -> io::Result<()> {
        self.0.sync_all()
    }
}

impl SaveIo for RealIo {
    fn create_temp(&self, path: &Path) -> io::Result<Box<dyn TempFile>> {
        let f = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        Ok(Box::new(RealTemp(f)))
    }
    /// `std::fs::rename`。既存の最終ファイルがあれば置き換える(Windowsでは `MoveFileEx` の
    /// 置換、Unixでは `rename(2)`)。先に最終ファイルを削除することはしない。
    fn replace(&self, temp: &Path, final_path: &Path) -> io::Result<()> {
        fs::rename(temp, final_path)
    }
    fn remove_temp(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_path(final_path: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = final_path.file_name().expect("file name").to_os_string();
    name.push(format!(".{}-{nanos}-{n}.tmp", std::process::id()));
    final_path.with_file_name(name)
}

// ---------------------------------------------------------------- 保存・読込

#[derive(Debug, Clone, PartialEq)]
pub struct SaveReceipt {
    pub path: PathBuf,
    pub bytes: usize,
    /// 保存した日付(最後に処理を完了した日)。
    pub date: SnapshotDate,
    /// 既存の自形式のファイルを置き換えたか。
    pub replaced_existing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadReceipt {
    pub path: PathBuf,
    /// 復元した日付(最後に処理を完了した日。再開後の最初の処理日はその翌日)。
    pub date: SnapshotDate,
}

fn read_limited(path: &Path) -> Result<String, FileError> {
    let meta = fs::metadata(path).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            FileError::NotFound(path.to_path_buf())
        } else {
            io_err("inspect file", path, &e)
        }
    })?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(FileError::TooLarge { bytes: meta.len() });
    }
    let bytes = fs::read(path).map_err(|e| io_err("read", path, &e))?;
    String::from_utf8(bytes).map_err(|_| FileError::Malformed("not valid UTF-8".to_string()))
}

/// シミュレーションの現在の状態を保存する。`sim` は変更しない。失敗時、既存のファイルは変わらず、
/// 成功を返さない。日と日の間(`MapEconomySimulation` の取得契約)でのみ呼べる。
pub fn save(
    sim: &MapEconomySimulation,
    world: &WorldModel,
    buildings: &BuildingRegistry,
    location: &SaveLocation,
    name: &str,
) -> Result<SaveReceipt, FileError> {
    save_with_io(sim, world, buildings, location, name, &RealIo)
}

/// [`save`] のファイル操作を差し替えられる版(失敗の再現テスト用)。
pub fn save_with_io(
    sim: &MapEconomySimulation,
    world: &WorldModel,
    buildings: &BuildingRegistry,
    location: &SaveLocation,
    name: &str,
    io: &dyn SaveIo,
) -> Result<SaveReceipt, FileError> {
    // 1. 取得 2. 直列化(ここまでファイルは触らない)
    let snapshot = sim.snapshot(world, buildings);
    let text = encode(&snapshot)?;
    // 検証: 直列化した結果が、そのまま同じスナップショットとして読めること(書いたものが読めない保存を作らない)。
    if decode(&text)? != snapshot {
        return Err(FileError::Serialize("the encoded text does not decode to the same snapshot".to_string()));
    }

    // 3. 保存先の検証と、既存ファイルの識別
    let path = location.target(name, true)?;
    let replaced_existing = match fs::symlink_metadata(&path) {
        Ok(_) => {
            let existing = read_limited(&path).map_err(|e| FileError::OverwriteRefused {
                path: path.clone(),
                reason: e.to_string(),
            })?;
            decode(&existing).map_err(|e| FileError::OverwriteRefused {
                path: path.clone(),
                reason: e.to_string(),
            })?;
            true
        }
        Err(_) => false,
    };

    // 4. 一時ファイル(同じディレクトリ。自分が create_new で作ったものだけ)
    let mut created = None;
    for _ in 0..8 {
        let tmp = temp_path(&path);
        match io.create_temp(&tmp) {
            Ok(f) => {
                created = Some((tmp, f));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err("create temporary file", &tmp, &e)),
        }
    }
    let Some((tmp, mut file)) = created else {
        return Err(FileError::Io {
            op: "create temporary file",
            path: path.clone(),
            message: "could not find an unused temporary file name".to_string(),
        });
    };

    // 5. 全内容を書き、flush・同期。失敗したら自分の一時ファイルだけを消して終わる(最終ファイルには触れない)。
    let written = (|| -> io::Result<()> {
        file.write_all(text.as_bytes())?;
        file.flush()?;
        file.sync()
    })();
    drop(file);
    if let Err(e) = written {
        return Err(cleanup_after(io, &tmp, io_err("write temporary file", &tmp, &e)));
    }

    // 6. 成功した場合だけ最終保存先へ置換
    if let Err(e) = io.replace(&tmp, &path) {
        return Err(cleanup_after(io, &tmp, io_err("replace the save file", &path, &e)));
    }
    Ok(SaveReceipt { path, bytes: text.len(), date: snapshot.date, replaced_existing })
}

/// 失敗後、自分が作った一時ファイルだけを削除する。削除に失敗した場合は、その旨を元のエラーへ添える。
fn cleanup_after(io: &dyn SaveIo, tmp: &Path, err: FileError) -> FileError {
    match io.remove_temp(tmp) {
        Ok(()) => err,
        Err(e) => match err {
            FileError::Io { op, path, message } => FileError::Io {
                op,
                path,
                message: format!("{message} (additionally, the temporary file {} could not be removed: {e})", tmp.display()),
            },
            other => other,
        },
    }
}

/// ファイルを読み、封筒を検証して、スナップショットを返す(復元はしない)。
pub fn read_snapshot(location: &SaveLocation, name: &str) -> Result<MapEconomySnapshot, FileError> {
    let path = location.target(name, false)?;
    decode(&read_limited(&path)?)
}

/// ファイルから復元する: 読み込み → 封筒の検証 → DTOと静的コンテキストの検証 → 候補の再構築 →
/// 成功時だけ `sim` を置き換える。どこで失敗しても `sim` は変更されない。ファイル内の識別情報は
/// 採用せず、`sim`(復元先)の静的入力から求めた値と照合する。
pub fn load(
    sim: &mut MapEconomySimulation,
    world: &WorldModel,
    buildings: &BuildingRegistry,
    location: &SaveLocation,
    name: &str,
) -> Result<LoadReceipt, FileError> {
    let path = location.target(name, false)?;
    let snapshot = decode(&read_limited(&path)?)?;
    sim.restore(&snapshot, world, buildings).map_err(FileError::Restore)?;
    Ok(LoadReceipt { path, date: snapshot.date })
}
