//! P32-016: 新ステート経路(`MapEconomySimulation`)の実行時状態の**RONファイル**への保存・読込と、
//! 原子的な置換(Bevy非依存)。ファイル入出力は、シミュレーション状態の取得・復元
//! (`economy::map_state::snapshot`)から分離している: このモジュールは `MapEconomySnapshot` を
//! 封筒に入れて読み書きするだけで、状態の意味の検証は常に `MapEconomySimulation::restore` に任せる。
//!
//! ## 形式
//! - 保存先: `<base>/map_economy/<名前>.mapeco.ron`(`<base>` は通常 `saves`)。**名前は英数字・`_`・`-`だけ**で、
//!   パスの区切り・`..`・絶対パス・拡張子を含められない(保存先の外へは書けない)。
//! - 封筒: `(kind: "map_economy_state", format_version: 2, identity: (...), snapshot: (...))`。形式1(P32-016)は
//!   項目別の識別情報を持たず検証できないため非対応(読み込まず、変換せず、上書きもしない)。旧V1(`saves/savegame_v1.ron`、
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
//! ## 静的データとの照合(P32-017)
//! `identity` は静的入力の項目別ダイジェスト(BLAKE3-256。内容識別用で、電子署名・改ざん者の認証・真正性の保証ではない)。
//! 復元時は、復元先の静的入力(実際に読み込んで使っているデータ)から作り直し、判定表
//! (`map_state::static_identity::RULES`)で項目別に比べる: 一致 / 警告つきで許可 / 拒否。ファイル内の値は
//! 復元先の値として採用しない。判定を通った後の P32-015 の内部コンテキスト照合(FNV-1a 64bit)は、拒否項目だけを
//! 対象にした最後の砦で、警告項目を含まない(警告で許可した差分を再び拒否しない・無効化もしない)。

use crate::economy::map_state::snapshot::{
    MapEconomySimulation, MapEconomySnapshot, RestoreError, SnapshotDate,
};
use crate::economy::map_state::static_identity::{
    IdentityError, SavedIdentity, StaticIdentity, StaticInputs, Verdict, judge,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const MAP_ECONOMY_FILE_KIND: &str = "map_economy_state";
pub const MAP_ECONOMY_FILE_FORMAT_VERSION: u32 = 2;
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
    identity: &'a SavedIdentity,
    snapshot: &'a MapEconomySnapshot,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileOwned {
    kind: String,
    format_version: u32,
    identity: SavedIdentity,
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
    /// 静的入力の識別情報を作れない(非有限値など)。
    Identity(IdentityError),
    /// 静的入力の判定で復元できない(現在の状態は変更されていない)。保存側の識別情報の欠落・非対応の方式も含む。
    Incompatible(Verdict),
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
            FileError::UnsupportedVersion { found: 1 } => write!(
                f,
                "map economy file format_version 1 (P32-016) has no per-item static identity and cannot be verified; it is not loaded, converted or overwritten (supported: {MAP_ECONOMY_FILE_FORMAT_VERSION}). This is unrelated to the legacy game SaveGameV1"
            ),
            FileError::UnsupportedVersion { found } => write!(
                f,
                "unsupported format_version {found} (supported: {MAP_ECONOMY_FILE_FORMAT_VERSION})"
            ),
            FileError::Identity(e) => write!(f, "{e}"),
            FileError::Incompatible(v) => write!(f, "static data is incompatible with this save: {v}"),
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
/// 解析した封筒(識別情報は判定前の保存側の値で、復元先の値としては使わない)。
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedFile {
    pub identity: SavedIdentity,
    pub snapshot: MapEconomySnapshot,
}

pub fn encode(snapshot: &MapEconomySnapshot, identity: &SavedIdentity) -> Result<String, FileError> {
    let file = FileRef {
        kind: MAP_ECONOMY_FILE_KIND,
        format_version: MAP_ECONOMY_FILE_FORMAT_VERSION,
        identity,
        snapshot,
    };
    ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
        .map_err(|e| FileError::Serialize(e.to_string()))
}

/// テキストを封筒として解析する(ファイルは触らない)。`kind`・版を先に判定し、本体は厳密に解析する。
pub fn decode(text: &str) -> Result<DecodedFile, FileError> {
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
    Ok(DecodedFile { identity: file.identity, snapshot: file.snapshot })
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
    inputs: &StaticInputs<'_>,
    location: &SaveLocation,
    name: &str,
) -> Result<SaveReceipt, FileError> {
    save_with_io(sim, inputs, location, name, &RealIo)
}

/// [`save`] のファイル操作を差し替えられる版(失敗の再現テスト用)。
pub fn save_with_io(
    sim: &MapEconomySimulation,
    inputs: &StaticInputs<'_>,
    location: &SaveLocation,
    name: &str,
    io: &dyn SaveIo,
) -> Result<SaveReceipt, FileError> {
    // 1. 取得 2. 直列化(ここまでファイルは触らない)
    let snapshot = sim.snapshot(inputs.world, inputs.buildings);
    let identity = StaticIdentity::compute(sim, inputs).map_err(FileError::Identity)?.to_saved();
    let text = encode(&snapshot, &identity)?;
    // 検証: 直列化した結果が、そのまま同じスナップショットとして読めること(書いたものが読めない保存を作らない)。
    let back = decode(&text)?;
    if back.snapshot != snapshot || back.identity != identity {
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

/// ファイルを読み、封筒を検証して、解析結果を返す(判定も復元もしない)。
pub fn read_file(location: &SaveLocation, name: &str) -> Result<DecodedFile, FileError> {
    let path = location.target(name, false)?;
    decode(&read_limited(&path)?)
}

/// ファイルを読み、封筒を検証して、スナップショットだけを返す(判定も復元もしない)。
pub fn read_snapshot(location: &SaveLocation, name: &str) -> Result<MapEconomySnapshot, FileError> {
    Ok(read_file(location, name)?.snapshot)
}

/// 読み込み → 封筒の検証 → 復元先からの識別情報の生成 → 項目別の判定まで(`inspect`と`load`の共通の前半)。
/// 復元はしない。ファイルも `sim` も変更しない。
struct Examined {
    path: PathBuf,
    bytes: usize,
    file: DecodedFile,
    verdict: Verdict,
}

fn examine(
    sim: &MapEconomySimulation,
    inputs: &StaticInputs<'_>,
    location: &SaveLocation,
    name: &str,
) -> Result<Examined, FileError> {
    let path = location.target(name, false)?;
    let text = read_limited(&path)?;
    let file = decode(&text)?;
    let current = StaticIdentity::compute(sim, inputs).map_err(FileError::Identity)?;
    let verdict = judge(&file.identity, &current);
    Ok(Examined { path, bytes: text.len(), file, verdict })
}

/// 保存ファイルの概要(画面表示用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveSummary {
    pub states: usize,
    pub countries: usize,
    /// 全国家の建設キューの合計件数。
    pub queued_orders: usize,
}

fn summarize(snapshot: &MapEconomySnapshot) -> SaveSummary {
    SaveSummary {
        states: snapshot.states.len(),
        countries: snapshot.countries.len(),
        queued_orders: snapshot.countries.iter().map(|c| c.construction_queue.len()).sum(),
    }
}

/// [`inspect`] の結果。`verdict.is_compatible()` が偽でも、確認そのものは成功(拒否の理由を返す)。
#[derive(Debug, Clone, PartialEq)]
pub struct InspectReport {
    pub path: PathBuf,
    /// 常に [`MAP_ECONOMY_FILE_KIND`](別の `kind` はエラーになる)。
    pub kind: &'static str,
    pub format_version: u32,
    pub bytes: usize,
    /// 保存日付(最後に処理を完了した日)。
    pub date: SnapshotDate,
    pub summary: SaveSummary,
    /// 静的入力の項目別の判定(一致・警告・拒否・識別情報の問題)。
    pub verdict: Verdict,
    /// 判定が通った場合だけ: 候補の検証・再構築(`restore_candidate`)まで行った結果。`Ok` でも**復元はしていない**。
    pub restorable: Option<Result<(), RestoreError>>,
}

/// 保存ファイルを**復元せずに**確認する(画面の事前確認用)。`load` と同じ解析(`decode`: `kind`・版・厳密な本体)と
/// 同じ判定(`judge`)を共有し、緩い別のパーサーは持たない。`sim`・ファイル・保存先を変更しない(ディレクトリも作らない)。
///
/// 戻り値はその**時点**の確認結果で、復元の許可ではない。`load` は常に最初から(ファイルを読み直して)全検証をやり直し、
/// 確認後にファイルや静的入力が変わっていれば、その時点の内容で判定・拒否する。確認と復元の間に別のプロセスが
/// ファイルを書き換える競合(TOCTOU)を防ぐものではない(P32-016 と同じ保証範囲)。
pub fn inspect(
    sim: &MapEconomySimulation,
    inputs: &StaticInputs<'_>,
    location: &SaveLocation,
    name: &str,
) -> Result<InspectReport, FileError> {
    let ex = examine(sim, inputs, location, name)?;
    let restorable = ex.verdict.is_compatible().then(|| {
        sim.restore_candidate(&ex.file.snapshot, inputs.world, inputs.buildings).map(|_| ())
    });
    Ok(InspectReport {
        path: ex.path,
        kind: MAP_ECONOMY_FILE_KIND,
        format_version: MAP_ECONOMY_FILE_FORMAT_VERSION,
        bytes: ex.bytes,
        date: ex.file.snapshot.date,
        summary: summarize(&ex.file.snapshot),
        verdict: ex.verdict,
        restorable,
    })
}

// ---------------------------------------------------------------- 保存の一覧

/// 保存先ディレクトリそのものの状態。
#[derive(Debug, Clone, PartialEq)]
pub enum DirectoryStatus {
    /// まだ存在しない(「保存なし」。作成しない)。
    Missing,
    Readable,
    /// 保存先として使えない(リンク・保存先の外を指す・ディレクトリでない・読み取り不能)。一覧は空または部分的。
    Refused(FileError),
}

/// 一覧の1件の状態。`Valid` は「自形式として完全に解析できた」ことだけを表し、現在の静的入力との互換性は
/// 判定していない(それは [`inspect`])。
#[derive(Debug, Clone, PartialEq)]
pub enum EntryStatus {
    Valid { date: SnapshotDate, bytes: usize },
    /// 壊れている・非対応の形式・別の `kind`・旧V1・リンク・通常ファイルでない・名前が規則に合わない・大きすぎる。
    Unusable(FileError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SaveEntry {
    /// 保存名(`<名前>.mapeco.ron` の `<名前>`)。表示順は注文や国家の識別に使わない。
    pub name: String,
    pub path: PathBuf,
    pub status: EntryStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SaveList {
    pub dir: PathBuf,
    pub directory: DirectoryStatus,
    /// 名前の昇順(安定)。
    pub entries: Vec<SaveEntry>,
}

/// `<base>/map_economy/` の保存を安全に一覧する。
/// - 対象は `*.mapeco.ron` だけ(他の拡張子・一時ファイル・無関係なファイルは含めず、触れない)。
/// - 保存先が無ければ `Missing`(作成しない)。保存先の異常は `directory` に、個々のファイルの異常は各 `entries` に分けて返す。
/// - 読むのは保存先ディレクトリ直下の候補だけ。修復・変換・削除・書き込みはしない。リンクは辿らない。
pub fn list_saves(location: &SaveLocation) -> SaveList {
    let dir = location.dir();
    let mut list = SaveList { dir: dir.clone(), directory: DirectoryStatus::Readable, entries: Vec::new() };
    let verified = match location.verified_dir(false) {
        Ok(d) => d,
        Err(FileError::NotFound(_)) => {
            list.directory = DirectoryStatus::Missing;
            return list;
        }
        Err(e) => {
            list.directory = DirectoryStatus::Refused(e);
            return list;
        }
    };
    let rd = match fs::read_dir(&verified) {
        Ok(rd) => rd,
        Err(e) => {
            list.directory = DirectoryStatus::Refused(io_err("read directory", &verified, &e));
            return list;
        }
    };
    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                list.directory = DirectoryStatus::Refused(io_err("read directory entry", &verified, &e));
                continue;
            }
        };
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else { continue };
        let Some(stem) = file_name.strip_suffix(MAP_ECONOMY_EXTENSION) else { continue };
        let path = verified.join(file_name);
        let status = match entry.file_type() {
            Err(e) => EntryStatus::Unusable(io_err("inspect file", &path, &e)),
            Ok(t) if t.is_symlink() => EntryStatus::Unusable(FileError::Symlink(path.clone())),
            Ok(t) if !t.is_file() => EntryStatus::Unusable(FileError::LocationRefused(format!(
                "{} is not a regular file",
                path.display()
            ))),
            Ok(_) => match check_name(stem) {
                Err(e) => EntryStatus::Unusable(e),
                Ok(()) => match read_limited(&path).and_then(|text| decode(&text).map(|f| (text.len(), f))) {
                    Ok((bytes, f)) => EntryStatus::Valid { date: f.snapshot.date, bytes },
                    Err(e) => EntryStatus::Unusable(e),
                },
            },
        };
        list.entries.push(SaveEntry { name: stem.to_string(), path, status });
    }
    list.entries.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

/// 復元に成功したときの結果。警告で許可した項目は `verdict.warnings` に入る(呼び出し側が利用者へ見せる)。
#[derive(Debug, Clone, PartialEq)]
pub struct LoadReceipt {
    pub path: PathBuf,
    /// 復元した日付(最後に処理を完了した日。再開後の最初の処理日はその翌日)。
    pub date: SnapshotDate,
    /// 静的入力の判定。拒否と構造の問題は空(あれば復元しない)。
    pub verdict: Verdict,
}

/// ファイルから復元する:
/// 1. 封筒と形式を検証(`kind`・版・厳密な本体)
/// 2. 保存側の識別情報を検証(方式・版・項目の欠落/未知/重複/形式)
/// 3. 復元先の静的入力(実際に読み込んで使っているデータ)から識別情報を作る
/// 4. 項目別に判定表で比較(拒否が一つでもあれば復元しない)
/// 5. 候補の状態を検証・再構築(P32-015の内部コンテキスト照合を含む)
/// 6. 成功時だけ `sim` を置き換える
///
/// どこで失敗しても `sim` は変更されない。ファイル内の識別情報・`context` は採用せず、復元先の値と比べるだけ。
/// 警告で許可した差分があっても、人口・国庫・履歴・注文などの動的状態は保存された値のまま復元する
/// (初期化・再計算しない)。1〜4は [`inspect`] と共通の処理で、`inspect` の結果には依存せず毎回やり直す。
pub fn load(
    sim: &mut MapEconomySimulation,
    inputs: &StaticInputs<'_>,
    location: &SaveLocation,
    name: &str,
) -> Result<LoadReceipt, FileError> {
    let ex = examine(sim, inputs, location, name)?;
    if !ex.verdict.is_compatible() {
        return Err(FileError::Incompatible(ex.verdict));
    }
    sim.restore(&ex.file.snapshot, inputs.world, inputs.buildings).map_err(FileError::Restore)?;
    Ok(LoadReceipt { path: ex.path, date: ex.file.snapshot.date, verdict: ex.verdict })
}
