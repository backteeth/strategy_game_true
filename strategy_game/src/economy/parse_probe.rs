//! P32-027: ヘッダー解析の比較測定のための**測定専用**の部品(製品の解析・保存・復元には使われない)。
//!
//! - 保存文章の要素数・入れ子の統計(`text_stats`。正常な文章を数えるだけで、文字列置換や切断はしない)
//! - ヘッダー型の複製による解析と、全文の読み飛ばし(`IgnoredAny`)の解析(製品の `Header` と同じ読み方の比較用)
//! - `kind`・版を保ったまま、payload の形と量だけを変えた**妥当な RON** の fixture 生成
//! - プロセスのメモリ値の取得(Windows の PowerShell 経由。取れなければ未測定)
//!
//! 製品の `decode` は変更していない。製品側のヘッダー解析・本体解析の時間は `map_economy_file::decode_timed` の工程で計る。

use serde::Deserialize;
use serde::de::IgnoredAny;

/// 製品の `map_economy_file::Header` と同じ項目・同じ読み方(`implicit_some`)の複製。比較用。
#[derive(Deserialize)]
struct HeaderReplica {
    kind: Option<String>,
    format_version: Option<u32>,
    #[allow(dead_code)]
    version: Option<u32>,
}

/// 製品のヘッダー解析と同じ呼び出し(`Options::default().with_default_extension(IMPLICIT_SOME).from_str`)。
pub fn parse_header_replica(text: &str) -> Result<(Option<String>, Option<u32>), String> {
    let h: HeaderReplica = ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
        .from_str(text)
        .map_err(|e| e.to_string())?;
    Ok((h.kind, h.format_version))
}

/// 全文を `IgnoredAny` として読み飛ばす(ヘッダー型を使わず、読み飛ばしだけの時間を見る)。
pub fn parse_ignored_any(text: &str) -> Result<(), String> {
    let _: IgnoredAny = ron::from_str(text).map_err(|e| e.to_string())?;
    Ok(())
}

/// 文章の統計(正常なRON文章を走査して数える)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextStats {
    pub bytes: usize,
    pub lines: usize,
    /// `(` の数(構造体・タプルの個数)。
    pub parens: usize,
    /// `[` の数(列の個数)。
    pub brackets: usize,
    pub max_depth: usize,
    /// `:` の数(名前つきフィールドの個数。文字列の中は数えない)。
    pub fields: usize,
}

pub fn text_stats(text: &str) -> TextStats {
    let mut s = TextStats { bytes: text.len(), lines: text.lines().count(), ..Default::default() };
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for b in text.bytes() {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'(' => {
                s.parens += 1;
                depth += 1;
                s.max_depth = s.max_depth.max(depth);
            }
            b'[' => {
                s.brackets += 1;
                depth += 1;
                s.max_depth = s.max_depth.max(depth);
            }
            b')' | b']' => depth = depth.saturating_sub(1),
            b':' => s.fields += 1,
            _ => {}
        }
    }
    s
}

/// payload の形。`kind` と版は固定で、`snapshot` の中身だけを変える(どれも妥当な RON)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `[1, 2, 3, ...]`(平らな整数列)
    FlatInts,
    /// `[(a: 1, b: 2.5), ...]`(名前つき構造体の列)
    StructList,
    /// `[(1, 2.5), ...]`(タプルの列)
    TupleList,
    /// `[[1, 2, ...], [..], ...]`(短い列の列)
    ListOfLists,
    /// `[(v: (v: (v: ... 1 ...))), ...]`(深さ `DEEP_LEVELS` の入れ子の列)
    DeepNest,
    /// 長い1つの文字列
    LongString,
    /// `[Farm, Mine, Railway, ...]`(名前つきの列挙子の列)
    NamedVariants,
    /// `[1, "..", 2, "..", ...]`(平らな整数列に、文字列 `".."` を挟んだもの。数の個数は FlatInts と同程度で、文字列の中に `..` がある)
    IntsWithDotDotStrings,
}

pub const DEEP_LEVELS: usize = 24;

impl Shape {
    pub const ALL: [Shape; 8] = [
        Shape::FlatInts,
        Shape::StructList,
        Shape::TupleList,
        Shape::ListOfLists,
        Shape::DeepNest,
        Shape::LongString,
        Shape::NamedVariants,
        Shape::IntsWithDotDotStrings,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Shape::FlatInts => "flat_ints",
            Shape::StructList => "struct_list",
            Shape::TupleList => "tuple_list",
            Shape::ListOfLists => "list_of_lists",
            Shape::DeepNest => "deep_nest",
            Shape::LongString => "long_string",
            Shape::NamedVariants => "named_variants",
            Shape::IntsWithDotDotStrings => "ints_with_dotdot_strings",
        }
    }

    pub fn parse(s: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|x| x.name() == s)
    }
}

/// `kind`・版を保った妥当な RON 文章(封筒の形)。`snapshot` の payload は約 `target_bytes` バイト。
pub fn fixture(shape: Shape, target_bytes: usize) -> String {
    let mut p = String::with_capacity(target_bytes + 64);
    match shape {
        Shape::LongString => {
            p.push('"');
            while p.len() < target_bytes {
                p.push_str("abcdefghij");
            }
            p.push('"');
        }
        _ => {
            p.push('[');
            let mut i = 0u64;
            let deep = {
                let mut d = String::from("1");
                for _ in 0..DEEP_LEVELS {
                    d = format!("(v: {d})");
                }
                d
            };
            while p.len() < target_bytes {
                if i > 0 {
                    p.push_str(", ");
                }
                match shape {
                    Shape::FlatInts => p.push_str(&i.to_string()),
                    Shape::StructList => p.push_str(&format!("(a: {i}, b: {}.5)", i % 97)),
                    Shape::TupleList => p.push_str(&format!("({i}, {}.5)", i % 97)),
                    Shape::ListOfLists => p.push_str(&format!("[{i}, {}, {}, {}]", i + 1, i + 2, i + 3)),
                    Shape::DeepNest => p.push_str(&deep),
                    Shape::IntsWithDotDotStrings => p.push_str(&format!("{i}, \"..\"")),
                    Shape::NamedVariants => p.push_str(["Farm", "Mine", "Railway", "Factory"][(i % 4) as usize]),
                    Shape::LongString => unreachable!(),
                }
                i += 1;
            }
            p.push(']');
        }
    }
    format!("(\n    kind: \"map_economy_state\",\n    format_version: 2,\n    identity: (scheme: \"fixture\", items: []),\n    snapshot: {p},\n)\n")
}

/// プロセスのメモリ値(バイト)。意味: `working_set` = 現在ワーキングセット(物理メモリ上にあるページ)、
/// `private_bytes` = プライベートのコミット済みバイト、`peak_working_set` = **プロセス開始からの**ワーキングセットの最大値
/// (試行ごとのピークではない)。前後の観測値をピークと呼ばない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemInfo {
    pub working_set: u64,
    pub private_bytes: u64,
    pub peak_working_set: u64,
}

/// 取得できなければ `None`(未測定)。Windows の PowerShell(`Get-Process`)を子プロセスとして呼ぶ。計時の区間の外で呼ぶこと。
pub fn process_memory() -> Option<MemInfo> {
    if !cfg!(windows) {
        return None;
    }
    let pid = std::process::id();
    let script = format!("$p=Get-Process -Id {pid}; \"$($p.WorkingSet64) $($p.PrivateMemorySize64) $($p.PeakWorkingSet64)\"");
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let mut it = text.split_whitespace().map(|x| x.parse::<u64>());
    Some(MemInfo { working_set: it.next()?.ok()?, private_bytes: it.next()?.ok()?, peak_working_set: it.next()?.ok()? })
}
