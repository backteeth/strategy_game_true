//! P20-009: 最小限のUIローカライゼーション基盤。
//!
//! 翻訳文字列は `assets/localization/{ja-JP,en-US}.ron` に一元管理し、Rustコード側は
//! 安定した翻訳キーとテンプレート引数(`{name}`形式)のみを扱う。ゲームシミュレーション・
//! AI判断・セーブ対象状態には翻訳済み文字列を混入させず、表示直前(Text構築時)にのみ解決する。
//!
//! - 既定言語: 日本語 (ja-JP)
//! - フォールバック: 英語 (en-US)。フォールバックにもキーが無い場合は開発時に識別可能な
//!   `⟦MISSING:key⟧` マーカーを返す(空文字列や原文で黙って隠さない)。
//! - 言語切り替え: `CurrentLocale` リソースを変更するだけで、`LocalizedText` を持つ全Textが
//!   自動的に再翻訳される(画面の作り直し不要)。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::collections::HashMap;

/// 対応言語。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    JaJp,
    EnUs,
}

impl Locale {
    pub const ALL: [Locale; 2] = [Locale::JaJp, Locale::EnUs];

    pub fn code(self) -> &'static str {
        match self {
            Locale::JaJp => "ja-JP",
            Locale::EnUs => "en-US",
        }
    }

    /// 言語切り替えボタンを押した際の遷移先。
    pub fn next(self) -> Locale {
        match self {
            Locale::JaJp => Locale::EnUs,
            Locale::EnUs => Locale::JaJp,
        }
    }

    /// 切り替えボタンに表示する「切り替え先言語の自称」(翻訳キーを介さない固有名詞)。
    pub fn own_name(self) -> &'static str {
        match self {
            Locale::JaJp => "日本語",
            Locale::EnUs => "English",
        }
    }
}

/// 起動時の既定言語。
pub const DEFAULT_LOCALE: Locale = Locale::JaJp;
/// 未定義キーのフォールバック先言語。
pub const FALLBACK_LOCALE: Locale = Locale::EnUs;
/// キーがどちらの言語にも無い場合に返す、開発時に識別可能な欠落マーカーの接頭辞。
pub const MISSING_KEY_MARKER_PREFIX: &str = "⟦MISSING:";

const JA_JP_RON: &str = include_str!("../assets/localization/ja-JP.ron");
const EN_US_RON: &str = include_str!("../assets/localization/en-US.ron");

/// 翻訳カタログのロード/検証エラー。
#[derive(Debug)]
pub struct CatalogError {
    pub message: String,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// 全言語の翻訳テーブルを保持するリソース。
#[derive(Resource)]
pub struct TranslationCatalog {
    tables: HashMap<Locale, HashMap<String, String>>,
}

impl TranslationCatalog {
    /// 埋め込み済みRONリソースをパースし、重複キー・空値を検証してロードする。
    pub fn load() -> Result<Self, CatalogError> {
        let mut tables = HashMap::new();
        tables.insert(Locale::JaJp, Self::parse(Locale::JaJp, JA_JP_RON)?);
        tables.insert(Locale::EnUs, Self::parse(Locale::EnUs, EN_US_RON)?);
        Ok(Self { tables })
    }

    /// テスト等で任意のRON文字列からカタログの一部を構築するための低レベルヘルパー。
    pub fn parse_entries(raw: &str) -> Result<Vec<(String, String)>, CatalogError> {
        ron::from_str::<Vec<(String, String)>>(raw).map_err(|e| CatalogError {
            message: format!("failed to parse translation RON: {e}"),
        })
    }

    fn parse(locale: Locale, raw: &str) -> Result<HashMap<String, String>, CatalogError> {
        let entries = Self::parse_entries(raw)?;

        let mut seen = std::collections::HashSet::new();
        let mut map = HashMap::with_capacity(entries.len());
        for (key, value) in entries {
            if !seen.insert(key.clone()) {
                return Err(CatalogError {
                    message: format!("{}: duplicate translation key '{key}'", locale.code()),
                });
            }
            if value.trim().is_empty() {
                return Err(CatalogError {
                    message: format!("{}: translation for key '{key}' is empty", locale.code()),
                });
            }
            map.insert(key, value);
        }
        Ok(map)
    }

    /// keyに対応する生テンプレート文字列(未展開)を取得する。存在しなければNone。
    pub fn raw(&self, locale: Locale, key: &str) -> Option<&str> {
        self.tables
            .get(&locale)
            .and_then(|t| t.get(key))
            .map(|s| s.as_str())
    }

    pub fn contains(&self, locale: Locale, key: &str) -> bool {
        self.tables
            .get(&locale)
            .is_some_and(|t| t.contains_key(key))
    }

    pub fn keys(&self, locale: Locale) -> impl Iterator<Item = &str> {
        self.tables
            .get(&locale)
            .into_iter()
            .flat_map(|t| t.keys().map(|k| k.as_str()))
    }

    pub fn entry_count(&self, locale: Locale) -> usize {
        self.tables.get(&locale).map(|t| t.len()).unwrap_or(0)
    }
}

/// `{name}` 形式のプレースホルダーをargsで置換する。テンプレート内に存在しないargは無視される。
fn substitute(template: &str, args: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (name, value) in args {
        let token = format!("{{{name}}}");
        out = out.replace(&token, value);
    }
    out
}

/// テンプレート文字列に含まれる `{name}` プレースホルダー名の集合を抽出する。
pub fn extract_placeholders(template: &str) -> std::collections::BTreeSet<String> {
    let mut placeholders = std::collections::BTreeSet::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after_open = &rest[start + 1..];
        if let Some(end) = after_open.find('}') {
            let name = &after_open[..end];
            // 空 `{}` やネストした `{{` はプレースホルダーとして扱わない。
            if !name.is_empty() && !name.contains('{') && !name.contains(char::is_whitespace) {
                placeholders.insert(name.to_string());
            }
            rest = &after_open[end + 1..];
        } else {
            break;
        }
    }
    placeholders
}

/// keyを`locale`で解決する。無ければ`FALLBACK_LOCALE`(en-US)へフォールバックする。
/// どちらにも無い場合は開発時に識別可能な欠落マーカーを返す(空文字列や原文への黙ったすり替えはしない)。
pub fn translate(
    catalog: &TranslationCatalog,
    locale: Locale,
    key: &str,
    args: &[(&str, String)],
) -> String {
    if let Some(template) = catalog.raw(locale, key) {
        return substitute(template, args);
    }
    if locale != FALLBACK_LOCALE
        && let Some(template) = catalog.raw(FALLBACK_LOCALE, key)
    {
        warn!(
            "[P20-009] translation key '{key}' missing for locale {}; falling back to {}",
            locale.code(),
            FALLBACK_LOCALE.code()
        );
        return substitute(template, args);
    }
    error!(
        "[P20-009] translation key '{key}' missing for locale {} and fallback {}",
        locale.code(),
        FALLBACK_LOCALE.code()
    );
    format!("{MISSING_KEY_MARKER_PREFIX}{key}⟧")
}

/// 現在の言語設定。変更すると`retranslate_on_locale_change`が全`LocalizedText`を再翻訳する。
#[derive(Resource, Clone, Copy, PartialEq, Eq)]
pub struct CurrentLocale(pub Locale);

impl Default for CurrentLocale {
    fn default() -> Self {
        Self(DEFAULT_LOCALE)
    }
}

/// あるTextEntityが「どの翻訳キーを・どの引数で」表示しているかを保持するマーカー。
/// 言語切り替え時、このコンポーネントを持つ全EntityのTextが自動的に再翻訳される。
#[derive(Component, Clone, Default)]
pub struct LocalizedText {
    pub key: &'static str,
    pub args: Vec<(&'static str, String)>,
}

impl LocalizedText {
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            args: Vec::new(),
        }
    }

    pub fn with_args(key: &'static str, args: Vec<(&'static str, String)>) -> Self {
        Self { key, args }
    }

    /// `key`が空(=`LocalizedText::default()`のまま、対応するUI更新Systemが
    /// まだ一度も実行されていない初期状態)の場合はNoneを返す。空キーを
    /// `translate()`へ渡すと両言語に存在せず欠落マーカーへ化けてしまうため、
    /// 呼び出し側(`retranslate_on_locale_change`)はこの場合Textを書き換えない。
    pub fn render(&self, catalog: &TranslationCatalog, locale: Locale) -> Option<String> {
        if self.key.is_empty() {
            return None;
        }
        Some(translate(catalog, locale, self.key, &self.args))
    }
}

/// `locale`+`catalog`をまとめたSystemParam。
/// Bevyの`SystemParam`はタプル実装が最大16個までのため、多数のResourceを読む
/// パネル更新Systemでパラメータ数を1つ節約するために使う。
#[derive(SystemParam)]
pub struct Loc<'w> {
    pub locale: Res<'w, CurrentLocale>,
    pub catalog: Res<'w, TranslationCatalog>,
}

impl Loc<'_> {
    pub fn t(&self, key: &'static str) -> String {
        translate(&self.catalog, self.locale.0, key, &[])
    }

    pub fn tf(&self, key: &'static str, args: Vec<(&'static str, String)>) -> String {
        translate(&self.catalog, self.locale.0, key, &args)
    }
}

/// `t!`相当のショートハンド: 引数無しキーを現在の言語で解決する。
pub fn t(catalog: &TranslationCatalog, locale: Locale, key: &'static str) -> String {
    translate(catalog, locale, key, &[])
}

/// テンプレート引数付きで翻訳を解決する。
pub fn tf(
    catalog: &TranslationCatalog,
    locale: Locale,
    key: &'static str,
    args: Vec<(&'static str, String)>,
) -> String {
    translate(catalog, locale, key, &args)
}

/// `Text`へ翻訳結果を書き込み、再翻訳用の`LocalizedText`マーカーを返す。
/// パネルのsetup(Startup)・update(Update)の両方で使う共通ヘルパー。
pub fn localized_text(
    catalog: &TranslationCatalog,
    locale: Locale,
    key: &'static str,
    args: Vec<(&'static str, String)>,
) -> (Text, LocalizedText) {
    let rendered = translate(catalog, locale, key, &args);
    (Text::new(rendered), LocalizedText::with_args(key, args))
}

/// 言語切り替えボタンのマーカー。
#[derive(Component)]
pub struct LanguageToggleButton;

/// 言語切り替えボタンのラベルTextマーカー(切り替え先言語の自称を表示する)。
#[derive(Component)]
pub struct LanguageToggleButtonText;

fn handle_language_toggle_button(
    mut locale: ResMut<CurrentLocale>,
    q_btn: Query<&Interaction, (Changed<Interaction>, With<LanguageToggleButton>)>,
) {
    for interaction in q_btn.iter() {
        if *interaction == Interaction::Pressed {
            locale.0 = locale.0.next();
        }
    }
}

fn update_language_toggle_button_label(
    locale: Res<CurrentLocale>,
    mut text_q: Query<&mut Text, With<LanguageToggleButtonText>>,
) {
    if !locale.is_changed() {
        return;
    }
    let label = locale.0.next().own_name();
    for mut text in text_q.iter_mut() {
        if text.0 != label {
            text.0 = label.to_string();
        }
    }
}

/// `CurrentLocale`が変化したら、`LocalizedText`を持つ全EntityのTextを再翻訳する。
/// パネルの開閉状態に関係なく全件処理するため、閉じたパネルも再度開いたときには
/// 既に新しい言語で表示される。
fn retranslate_on_locale_change(
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut query: Query<(&mut Text, &LocalizedText)>,
) {
    if !locale.is_changed() {
        return;
    }
    for (mut text, marker) in query.iter_mut() {
        let Some(rendered) = marker.render(&catalog, locale.0) else {
            continue;
        };
        if text.0 != rendered {
            text.0 = rendered;
        }
    }
}

/// OSウィンドウタイトルを現在の言語へ追従させる。
fn retranslate_window_title(
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
) {
    if !locale.is_changed() {
        return;
    }
    let title = t(&catalog, locale.0, "app.window_title");
    for mut window in windows.iter_mut() {
        if window.title != title {
            window.title = title.clone();
        }
    }
}

/// 再配布可能でCJKグリフを含む埋め込みフォント(Noto Sans JP, SIL OFL 1.1)。
/// ライセンス文書は `assets/fonts/NotoSansJP-OFL.txt` に同梱。
const NOTO_SANS_JP_DATA: &[u8] = include_bytes!("../assets/fonts/NotoSansJP-Variable.ttf");

/// bevyの`default_font`機能が`AssetId::<Font>::default()`へ登録する埋め込みFiraMono-subset
/// (Latin専用、日本語グリフ非対応)を、日本語グリフを含むNoto Sans JPで上書きする。
/// `TextFont::default()`は`FontSource::Handle(Handle::default())`を指すため、既存の
/// 全`TextFont { ..default() }`呼び出し箇所を一切変更せずにフォントだけを差し替えられる。
/// OSインストール済みフォントには一切依存しない(system_font_discovery機能は未使用)。
fn install_japanese_capable_font(mut fonts: ResMut<Assets<Font>>) {
    let font = Font::from_bytes(NOTO_SANS_JP_DATA.to_vec());
    fonts
        .insert(AssetId::<Font>::default(), font)
        .unwrap_or_else(|e| {
            panic!("[P20-009] failed to install Noto Sans JP as the default font: {e:?}")
        });
}

/// `CurrentLocale`+`TranslationCatalog`リソースのみを提供する最小プラグイン。
///
/// フォント差し替えやWindow/Text操作を一切行わないため、`Assets<Font>`や`Window`が
/// 存在しない最小構成のBevy `App`(例: `MinimalPlugins`を使う統合テスト)へ安全に
/// 追加できる。ゲームロジック側プラグイン(Economy/Research/Politics/Diplomacy/Debug)は
/// 通知メッセージの翻訳に`CurrentLocale`/`TranslationCatalog`を必要とするため、
/// それぞれの`build()`内でこのプラグインを追加する(冪等: 2回目以降は即return)。
pub struct TranslationCorePlugin;

impl Plugin for TranslationCorePlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<TranslationCatalog>() {
            return;
        }

        let catalog = TranslationCatalog::load()
            .unwrap_or_else(|e| panic!("[P20-009] failed to load translation catalog: {e}"));

        app.insert_resource(catalog)
            .insert_resource(CurrentLocale::default());
    }

    /// 複数のゲームロジックプラグイン(Economy/Research/Politics/Diplomacy/Debug)が
    /// それぞれ`build()`内で追加するため、重複追加をBevyのデフォルト挙動(panic)
    /// ではなく`build()`冒頭の`contains_resource`チェックによる無害なno-opにする。
    fn is_unique(&self) -> bool {
        false
    }
}

/// P20-009ローカライゼーションのUI表示層プラグイン。`UiPlugin`から自動的に読み込まれる
/// (main.rsやHeadlessテストのプラグインリストを変更する必要はない)。
/// フォント差し替え・言語切替ボタン・Text/Window再翻訳など、`DefaultPlugins`
/// (`Assets<Font>`, `Window`)の存在を前提とするシステムはすべてここに置く。
pub struct LocalizationPlugin;

impl Plugin for LocalizationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TranslationCorePlugin)
            .add_systems(PreStartup, install_japanese_capable_font)
            .add_systems(
                Update,
                (
                    handle_language_toggle_button,
                    update_language_toggle_button_label,
                    retranslate_on_locale_change,
                    retranslate_window_title,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_uses_current_locale_when_key_present() {
        let raw_ja = r#"[("greeting", "こんにちは")]"#;
        let raw_en = r#"[("greeting", "Hello")]"#;
        let mut tables = HashMap::new();
        tables.insert(
            Locale::JaJp,
            TranslationCatalog::parse(Locale::JaJp, raw_ja).unwrap(),
        );
        tables.insert(
            Locale::EnUs,
            TranslationCatalog::parse(Locale::EnUs, raw_en).unwrap(),
        );
        let catalog = TranslationCatalog { tables };

        assert_eq!(t(&catalog, Locale::JaJp, "greeting"), "こんにちは");
        assert_eq!(t(&catalog, Locale::EnUs, "greeting"), "Hello");
    }

    #[test]
    fn translate_falls_back_to_en_us_when_ja_jp_key_missing() {
        let raw_ja = r#"[]"#;
        let raw_en = r#"[("only_in_en", "Hello")]"#;
        let mut tables = HashMap::new();
        tables.insert(
            Locale::JaJp,
            TranslationCatalog::parse(Locale::JaJp, raw_ja).unwrap(),
        );
        tables.insert(
            Locale::EnUs,
            TranslationCatalog::parse(Locale::EnUs, raw_en).unwrap(),
        );
        let catalog = TranslationCatalog { tables };

        assert_eq!(t(&catalog, Locale::JaJp, "only_in_en"), "Hello");
    }

    #[test]
    fn translate_returns_detectable_marker_when_key_missing_everywhere() {
        let mut tables = HashMap::new();
        tables.insert(Locale::JaJp, HashMap::new());
        tables.insert(Locale::EnUs, HashMap::new());
        let catalog = TranslationCatalog { tables };

        let result = t(&catalog, Locale::JaJp, "totally_missing_key");
        assert!(
            result.starts_with(MISSING_KEY_MARKER_PREFIX),
            "missing key must produce a detectable marker, got: {result}"
        );
        assert!(result.contains("totally_missing_key"));
    }

    #[test]
    fn substitute_replaces_named_placeholders() {
        let mut tables = HashMap::new();
        tables.insert(
            Locale::EnUs,
            TranslationCatalog::parse(
                Locale::EnUs,
                r#"[("greet", "Hello, {name}! You have {count} messages.")]"#,
            )
            .unwrap(),
        );
        tables.insert(Locale::JaJp, HashMap::new());
        let catalog = TranslationCatalog { tables };

        let result = tf(
            &catalog,
            Locale::EnUs,
            "greet",
            vec![("name", "Alice".to_string()), ("count", "3".to_string())],
        );
        assert_eq!(result, "Hello, Alice! You have 3 messages.");
    }

    #[test]
    fn extract_placeholders_finds_all_named_tokens() {
        let placeholders = extract_placeholders("{a} and {b} and {a} again");
        assert_eq!(placeholders.len(), 2);
        assert!(placeholders.contains("a"));
        assert!(placeholders.contains("b"));
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let raw = r#"[("dup", "one"), ("dup", "two")]"#;
        let err = TranslationCatalog::parse(Locale::EnUs, raw).unwrap_err();
        assert!(err.message.contains("duplicate"));
    }

    #[test]
    fn empty_value_is_rejected() {
        let raw = r#"[("blank", "")]"#;
        let err = TranslationCatalog::parse(Locale::EnUs, raw).unwrap_err();
        assert!(err.message.contains("empty"));
    }

    #[test]
    fn real_catalog_loads_and_ja_en_key_sets_match() {
        let catalog = TranslationCatalog::load().expect("embedded RON catalogs must parse");
        let ja_keys: std::collections::BTreeSet<&str> = catalog.keys(Locale::JaJp).collect();
        let en_keys: std::collections::BTreeSet<&str> = catalog.keys(Locale::EnUs).collect();
        assert_eq!(
            ja_keys, en_keys,
            "ja-JP and en-US must expose exactly the same key set"
        );
        assert!(catalog.entry_count(Locale::JaJp) > 100);
    }

    #[test]
    fn real_catalog_placeholders_match_between_locales() {
        let catalog = TranslationCatalog::load().expect("embedded RON catalogs must parse");
        for key in catalog.keys(Locale::JaJp) {
            let ja_template = catalog.raw(Locale::JaJp, key).unwrap();
            let en_template = catalog
                .raw(Locale::EnUs, key)
                .unwrap_or_else(|| panic!("key '{key}' missing from en-US"));
            let ja_ph = extract_placeholders(ja_template);
            let en_ph = extract_placeholders(en_template);
            assert_eq!(
                ja_ph, en_ph,
                "placeholder set mismatch for key '{key}': ja={ja_ph:?} en={en_ph:?}"
            );
        }
    }
}
