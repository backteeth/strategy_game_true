//! P20-009: UI表示文字列のハードコード残存検査。
//!
//! 実行方法:
//!   cargo test --test p20_009_hardcoded_string_scan_test -- --nocapture
//!
//! `src/ui/*.rs` および 通知メッセージを構築する非UIモジュール
//! (economy/research/diplomacy::update/politics/debug の各mod.rs)を対象に、
//! `Text::new("<literal>")` (Text本文へ直接埋め込まれた文字列リテラル)や
//! `message: "<literal>"` (GameNotificationへ直接埋め込まれた文字列リテラル)を走査し、
//! 翻訳キー経由(t()/tf()/localized_text())を通さないハードコードされた
//! 表示文字列が残っていないことを検証する。
//!
//! 誤検出を避けるための除外は、ファイル・完全一致する部分文字列・理由を明記した
//! 固定リストでのみ許可する(ディレクトリ全体の除外や、検査を無効化する広範な許可は行わない)。

use std::path::PathBuf;

/// (相対パス, 検出される正確なリテラル片, 除外理由)
struct Exemption {
    file: &'static str,
    literal: &'static str,
    reason: &'static str,
}

const EXEMPTIONS: &[Exemption] = &[
    Exemption {
        file: "src/ui/diplomacy_panel.rs",
        literal: "\"\"",
        reason: "DiplomacyHeaderTextの初期値。空文字列であり翻訳対象の文言を含まず、\
                  同フレーム後のupdate_diplomacy_panel_uiで即座にLocalizedText経由へ差し替わる。",
    },
    Exemption {
        file: "src/ui/top_bar.rs",
        literal: "\"\"",
        reason: "TopBarPlayerInfoTextの初期値。空文字列であり翻訳対象の文言を含まず、\
                  同フレーム後のupdate_top_bar_player_infoで即座にLocalizedText経由へ差し替わる。",
    },
    Exemption {
        file: "src/ui/top_bar.rs",
        literal: "\"1800/01/01\"",
        reason: "TopBarDateTextの初期placeholder値(YYYY/MM/DD形式)。Startup直後に\
                  update_top_bar_dateで即座に上書きされる。日付は言語非依存の数値フォーマットで、\
                  実際の表示はformat_date_for_localeを経由する。",
    },
    Exemption {
        file: "src/ui/top_bar.rs",
        literal: "\"||\"",
        reason: "一時停止ボタンの記号ラベル(言語に依存しないポーズ記号)。",
    },
    Exemption {
        file: "src/ui/top_bar.rs",
        literal: ">{}",
        reason: "速度切替ボタンの記号+数値ラベル(format!(\">{}\", i))。言語に依存しない記号。",
    },
    Exemption {
        file: "src/ui/politics_panel.rs",
        literal: "\"\"",
        reason: "PoliticsHeaderTextの初期値。空文字列であり翻訳対象の文言を含まず、\
                  同フレーム後のupdate_politics_panel_uiで即座にLocalizedText経由へ差し替わる。",
    },
    Exemption {
        file: "src/ui/research_panel.rs",
        literal: "\"\"",
        reason: "ResearchHeaderTextの初期値。空文字列であり翻訳対象の文言を含まず、\
                  同フレーム後のupdate_research_panel_uiで即座にLocalizedText経由へ差し替わる。",
    },
    Exemption {
        file: "src/ui/economy_panel.rs",
        literal: "\"-1%\"",
        reason: "税率マイナス調整ボタンの記号ラベル(言語に依存しない数値+%記号)。",
    },
    Exemption {
        file: "src/ui/economy_panel.rs",
        literal: "\"+1%\"",
        reason: "税率プラス調整ボタンの記号ラベル(言語に依存しない数値+%記号)。",
    },
    Exemption {
        file: "src/ui/country_selection.rs",
        literal: "\"\"",
        reason: "ContinueStatusTextの初期値。空文字列であり翻訳対象の文言を含まず、\
                  併せて付与されるLocalizedText::default()(空key)により\
                  update_continue_status_textが実際のロード失敗を検出するまで未初期化を保つ。\
                  失敗時はLocalizedText経由へ即座に差し替わる。",
    },
];

/// 走査対象ファイル(P20-009でユーザー表示文字列を扱う全モジュール)。
const TARGET_FILES: &[&str] = &[
    "src/ui/mod.rs",
    "src/ui/country_selection.rs",
    "src/ui/state_panel.rs",
    "src/ui/economy_panel.rs",
    "src/ui/research_panel.rs",
    "src/ui/politics_panel.rs",
    "src/ui/military_panel.rs",
    "src/ui/diplomacy_panel.rs",
    "src/ui/peace_panel.rs",
    "src/ui/load_confirm.rs",
    "src/ui/top_bar.rs",
    "src/ui/notification.rs",
    "src/economy/mod.rs",
    "src/research/mod.rs",
    "src/diplomacy/update.rs",
    "src/politics/mod.rs",
    "src/debug/mod.rs",
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `prefix(` の直後にある `"..."` リテラルを抽出する(エスケープされた引用符は
/// 本コードベースの対象文字列には出現しないため未対応)。
fn find_literal_calls<'a>(content: &'a str, prefix: &str) -> Vec<(usize, &'a str)> {
    let mut results = Vec::new();
    let needle = format!("{prefix}(\"");
    let mut search_start = 0usize;
    while let Some(rel) = content[search_start..].find(&needle) {
        let start = search_start + rel + needle.len() - 1; // 開き引用符の位置
        let after_quote = &content[start + 1..];
        if let Some(end_rel) = after_quote.find('"') {
            let literal_with_quotes = &content[start..start + 1 + end_rel + 1];
            let line_no = content[..start].matches('\n').count() + 1;
            results.push((line_no, literal_with_quotes));
        }
        search_start = start + 1;
    }
    results
}

fn is_exempted(file: &str, literal_snippet: &str) -> bool {
    EXEMPTIONS
        .iter()
        .any(|e| e.file == file && literal_snippet.contains(e.literal))
}

#[test]
fn no_hardcoded_text_new_literals_outside_allowlist() {
    let mut failures = Vec::new();

    for &rel_path in TARGET_FILES {
        let path = manifest_dir().join(rel_path);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("[P20-009] failed to read {rel_path}: {e}"));

        for (line_no, literal) in find_literal_calls(&content, "Text::new") {
            if !is_exempted(rel_path, literal) {
                failures.push(format!(
                    "{rel_path}:{line_no}: hardcoded Text::new({literal}) not routed through t()/tf()/localized_text()"
                ));
            }
        }

        // format!("...", ...) 経由でTextへ渡されるテンプレートリテラルもチェックする。
        for (line_no, literal) in find_literal_calls(&content, "Text::new(format!") {
            if !is_exempted(rel_path, literal) {
                failures.push(format!(
                    "{rel_path}:{line_no}: hardcoded Text::new(format!({literal}...)) template not routed through translation catalog"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "[P20-009] hardcoded UI text literals found (not in EXEMPTIONS allowlist):\n{}",
        failures.join("\n")
    );
    println!(
        "[P20-009] hardcoded-string residual scan: {} files scanned, 0 unexempted findings, {} allowlisted exemptions",
        TARGET_FILES.len(),
        EXEMPTIONS.len()
    );
}

#[test]
fn no_hardcoded_game_notification_message_literals() {
    let mut failures = Vec::new();

    for &rel_path in TARGET_FILES {
        let path = manifest_dir().join(rel_path);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("[P20-009] failed to read {rel_path}: {e}"));

        for (line_no, literal) in find_literal_calls(&content, "message:") {
            if !is_exempted(rel_path, literal) {
                failures.push(format!(
                    "{rel_path}:{line_no}: hardcoded GameNotification message literal {literal} not routed through t()/tf()"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "[P20-009] hardcoded GameNotification message literals found:\n{}",
        failures.join("\n")
    );
}

/// 除外リスト自体が対象ファイルに実在する記述と対応していることを検証する
/// (陳腐化した除外エントリが黙って検査を無効化し続けることを防ぐ)。
#[test]
fn all_exemptions_correspond_to_actual_occurrences() {
    for exemption in EXEMPTIONS {
        let path = manifest_dir().join(exemption.file);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("[P20-009] failed to read {}: {e}", exemption.file));
        assert!(
            content.contains(exemption.literal),
            "[P20-009] stale exemption: '{}' not found in {} (reason: {})",
            exemption.literal,
            exemption.file,
            exemption.reason
        );
    }
}

/// TARGET_FILESに列挙した全パスが実在することを保証する(リネーム・削除の検知)。
#[test]
fn target_files_exist() {
    for &rel_path in TARGET_FILES {
        let path = manifest_dir().join(rel_path);
        assert!(
            path.is_file(),
            "[P20-009] target file listed in scan does not exist: {rel_path}"
        );
    }
}
