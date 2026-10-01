//! P28-009: 選択Divisionの戦力・装備・組織率・補給・戦闘準備率の表示用ビュー。
//!
//! 値は新しい式で再計算せず、既存の正規データ・正規関数だけを使う。
//! - 補給充足率: `MilitaryRegistry::division_supply_fulfillment`(日次補給判定が最後に
//!   そのDivisionへ配分した`0.0..=1.0`の値。国家の備蓄日数ではない)。
//! - 戦力・装備・補給の充足率と戦闘準備率:
//!   `military::combat_calc::calculate_combat_readiness_with_supply`(戦闘解決が使うものと
//!   同一関数)。戦闘準備率は`min(戦力比, 装備比, 補給)`であり、組織率・地形などの
//!   補正は含まない(最終戦闘力ではない)。
//! - 組織率: `Division.organization / Division.max_organization`の比率表示のみ。
//!
//! 表示専用で、レジストリ・Saveには一切書き込まない。

use crate::common::{DivisionId, StateId};
use crate::military::combat_calc::{
    COMBAT_READINESS_SCALE, CombatReadiness, CombatReadinessError,
    calculate_combat_readiness_with_supply,
};
use crate::military::data::{Division, DivisionStatus};

/// 文言取得関数(キーのみ)。
pub type Tr<'a> = &'a dyn Fn(&'static str) -> String;
/// 文言取得関数(引数展開付き)。
pub type Trf<'a> = &'a dyn Fn(&'static str, Vec<(&'static str, String)>) -> String;

/// 直近の日次補給判定が、そのDivisionへ配分した値の読み取り結果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SupplyReading {
    /// 日次補給判定で配分済みの値(`0.0..=1.0`)。
    Allocated(f64),
    /// まだ日次補給判定を受けていない(新規Division・旧Saveのロード直後など)。
    /// 戦闘計算では`MilitaryRegistry::supply_fulfillment_of`により`1.0`として扱われる。
    NotYetAllocated,
    /// 非有限または`0.0..=1.0`の範囲外(正規の生成経路では到達しない)。
    Invalid(f64),
}

impl SupplyReading {
    /// `MilitaryRegistry::division_supply_fulfillment.get(&id).copied()`から読み取る。
    pub fn from_entry(entry: Option<f64>) -> Self {
        match entry {
            None => SupplyReading::NotYetAllocated,
            Some(v) if v.is_finite() && (0.0..=1.0).contains(&v) => SupplyReading::Allocated(v),
            Some(v) => SupplyReading::Invalid(v),
        }
    }
}

/// 戦闘準備率(`min(戦力比, 装備比, 補給)`)を制約している項目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitingFactor {
    Manpower,
    Equipment,
    Supply,
}

/// 1個のDivisionの表示用ビュー(純粋なデータ。Bevy非依存)。
#[derive(Debug, Clone, PartialEq)]
pub struct DivisionStatusView {
    pub id: DivisionId,
    pub current_state: StateId,
    pub status: DivisionStatus,
    pub manpower: u64,
    pub max_manpower: u64,
    pub equipment: f64,
    pub max_equipment: f64,
    pub organization: f32,
    pub max_organization: f32,
    pub supply: SupplyReading,
    /// 戦闘解決と同一の正規関数の結果。データ異常時は`Err`。
    pub readiness: Result<CombatReadiness, CombatReadinessError>,
}

impl DivisionStatusView {
    /// `supply_entry`: `division_supply_fulfillment`のエントリ(未計算なら`None`)。
    /// `supply_used_by_combat`: 戦闘計算が使う値(`MilitaryRegistry::supply_fulfillment_of`)。
    pub fn from_division(
        division: &Division,
        supply_entry: Option<f64>,
        supply_used_by_combat: f64,
    ) -> Self {
        Self {
            id: division.id,
            current_state: division.current_state,
            status: division.status,
            manpower: division.manpower,
            max_manpower: division.max_manpower,
            equipment: division.equipment,
            max_equipment: division.max_equipment,
            organization: division.organization,
            max_organization: division.max_organization,
            supply: SupplyReading::from_entry(supply_entry),
            readiness: calculate_combat_readiness_with_supply(
                division.manpower,
                division.max_manpower,
                division.equipment,
                division.max_equipment,
                supply_used_by_combat,
            ),
        }
    }

    /// 装備を必要としないDivision(必要量0)。
    pub fn requires_no_equipment(&self) -> bool {
        self.max_equipment == 0.0
    }

    /// 戦闘準備率を制約している項目。準備率が100%(制約なし)または算出不能なら空。
    /// 同率の場合は該当する項目をすべて返す。
    pub fn limiting_factors(&self) -> Vec<LimitingFactor> {
        let Ok(r) = &self.readiness else {
            return Vec::new();
        };
        let min = r.combat_readiness_units;
        if min >= COMBAT_READINESS_SCALE {
            return Vec::new();
        }
        let mut out = Vec::new();
        if r.manpower_readiness_units == min {
            out.push(LimitingFactor::Manpower);
        }
        if r.equipment_readiness_units == min {
            out.push(LimitingFactor::Equipment);
        }
        if r.supply_readiness_units == min {
            out.push(LimitingFactor::Supply);
        }
        out
    }

    fn manpower_percent(&self) -> String {
        match &self.readiness {
            Ok(r) => percent_from_fraction(units_to_fraction(r.manpower_readiness_units)),
            Err(_) => {
                if self.max_manpower == 0 {
                    percent_from_fraction(0.0)
                } else {
                    percent_from_fraction(self.manpower as f64 / self.max_manpower as f64)
                }
            }
        }
    }

    fn equipment_percent(&self) -> Option<String> {
        self.readiness
            .as_ref()
            .ok()
            .map(|r| percent_from_fraction(units_to_fraction(r.equipment_readiness_units)))
    }

    fn organization_percent(&self) -> Option<String> {
        let max = self.max_organization as f64;
        let cur = self.organization as f64;
        if !max.is_finite() || !cur.is_finite() || max <= 0.0 {
            return None;
        }
        Some(percent_from_fraction(cur / max))
    }

    fn supply_percent(&self) -> Option<String> {
        match self.supply {
            SupplyReading::Allocated(v) => Some(percent_from_fraction(v)),
            _ => None,
        }
    }

    fn limiting_text(&self, tr: Tr) -> String {
        let factors = self.limiting_factors();
        if factors.is_empty() {
            return tr("military_panel.limit_none");
        }
        let sep = tr("military_panel.limit_separator");
        factors
            .iter()
            .map(|f| match f {
                LimitingFactor::Manpower => tr("military_panel.limit_manpower"),
                LimitingFactor::Equipment => tr("military_panel.limit_equipment"),
                LimitingFactor::Supply => tr("military_panel.limit_supply"),
            })
            .collect::<Vec<_>>()
            .join(&sep)
    }

    /// 単一選択用の詳細行(戦力・装備・組織率・補給・戦闘準備率の順)。
    pub fn detail_lines(&self, tr: Tr, trf: Trf) -> Vec<String> {
        let mut lines = Vec::new();

        lines.push(trf(
            "military_panel.strength_ratio",
            vec![
                ("current", self.manpower.to_string()),
                ("max", self.max_manpower.to_string()),
                ("percent", self.manpower_percent()),
            ],
        ));

        if self.requires_no_equipment() {
            lines.push(tr("military_panel.equipment_not_required"));
        } else if let Some(percent) = self.equipment_percent() {
            lines.push(trf(
                "military_panel.equipment",
                vec![
                    ("current", format_quantity(self.equipment)),
                    ("max", format_quantity(self.max_equipment)),
                    ("percent", percent),
                ],
            ));
        } else {
            lines.push(trf(
                "military_panel.equipment_unavailable",
                vec![
                    ("current", format_quantity(self.equipment)),
                    ("max", format_quantity(self.max_equipment)),
                ],
            ));
        }

        let org_current = format!("{:.0}", self.organization);
        let org_max = format!("{:.0}", self.max_organization);
        match self.organization_percent() {
            Some(percent) => lines.push(trf(
                "military_panel.organization_ratio",
                vec![
                    ("current", org_current),
                    ("max", org_max),
                    ("percent", percent),
                ],
            )),
            None => lines.push(trf(
                "military_panel.organization_unavailable",
                vec![("current", org_current), ("max", org_max)],
            )),
        }

        lines.push(match self.supply {
            SupplyReading::Allocated(v) => trf(
                "military_panel.supply_allocated",
                vec![("percent", percent_from_fraction(v))],
            ),
            SupplyReading::NotYetAllocated => tr("military_panel.supply_not_computed"),
            SupplyReading::Invalid(v) => trf(
                "military_panel.supply_invalid",
                vec![("value", format!("{v}"))],
            ),
        });

        lines.push(match &self.readiness {
            Ok(r) => trf(
                "military_panel.combat_readiness",
                vec![
                    (
                        "percent",
                        percent_from_fraction(units_to_fraction(r.combat_readiness_units)),
                    ),
                    ("limiting", self.limiting_text(tr)),
                ],
            ),
            Err(e) => trf(
                "military_panel.combat_readiness_unavailable",
                vec![("reason", format!("{e:?}"))],
            ),
        });

        lines
    }

    /// 複数選択用の1行表示(Divisionごとに必ず個別の値を出す。平均はしない)。
    pub fn compact_line(&self, tr: Tr, trf: Trf) -> String {
        let manpower = format!(
            "{}/{} ({})",
            self.manpower,
            self.max_manpower,
            self.manpower_percent()
        );
        let equipment = if self.requires_no_equipment() {
            tr("military_panel.compact_not_required")
        } else {
            match self.equipment_percent() {
                Some(p) => format!(
                    "{}/{} ({})",
                    format_quantity(self.equipment),
                    format_quantity(self.max_equipment),
                    p
                ),
                None => tr("military_panel.compact_unavailable"),
            }
        };
        let organization = self
            .organization_percent()
            .unwrap_or_else(|| tr("military_panel.compact_unavailable"));
        let supply = match self.supply {
            SupplyReading::Allocated(_) => self
                .supply_percent()
                .unwrap_or_else(|| tr("military_panel.compact_unavailable")),
            SupplyReading::NotYetAllocated => tr("military_panel.compact_supply_not_computed"),
            SupplyReading::Invalid(_) => tr("military_panel.compact_supply_invalid"),
        };
        let readiness = match &self.readiness {
            Ok(r) => {
                let pct = percent_from_fraction(units_to_fraction(r.combat_readiness_units));
                if self.limiting_factors().is_empty() {
                    pct
                } else {
                    format!("{pct} [{}]", self.limiting_text(tr))
                }
            }
            Err(_) => tr("military_panel.compact_unavailable"),
        };
        trf(
            "military_panel.multi_detail_line",
            vec![
                ("manpower", manpower),
                ("equipment", equipment),
                ("organization", organization),
                ("supply", supply),
                ("readiness", readiness),
            ],
        )
    }
}

fn units_to_fraction(units: u64) -> f64 {
    units as f64 / COMBAT_READINESS_SCALE as f64
}

/// 割合(`0.0`〜`1.0`が正常範囲)を、見た目で誤解しない百分率文字列にする。
/// - ちょうど0は`0%`、満額は`100%`。
/// - それ以外は小数第1位へ**切り捨て**、`0.0%`や`100.0%`には丸めない(`0.1%`〜`99.9%`)。
/// - 1.0を超える異常値は丸めずそのまま小数第1位で示す。非有限は`-`。
pub fn percent_from_fraction(fraction: f64) -> String {
    if !fraction.is_finite() {
        return "-".to_string();
    }
    if fraction <= 0.0 {
        return "0%".to_string();
    }
    if fraction == 1.0 {
        return "100%".to_string();
    }
    if fraction > 1.0 {
        return format!("{:.1}%", fraction * 100.0);
    }
    let tenths = ((fraction * 1000.0).floor() as i64).clamp(1, 999);
    format!("{}.{}%", tenths / 10, tenths % 10)
}

/// 装備量などの数量表示。整数なら小数なし、そうでなければ小数第1位。
pub fn format_quantity(value: f64) -> String {
    if !value.is_finite() {
        return "?".to_string();
    }
    if (value - value.round()).abs() < 1e-9 {
        format!("{:.0}", value)
    } else {
        format!("{:.1}", value)
    }
}

/// このモジュールが使う翻訳キー(JA/EN両方に存在することをテストで検証する)。
pub const DIVISION_STATUS_VIEW_KEYS: &[&str] = &[
    "military_panel.strength_ratio",
    "military_panel.equipment",
    "military_panel.equipment_not_required",
    "military_panel.equipment_unavailable",
    "military_panel.organization_ratio",
    "military_panel.organization_unavailable",
    "military_panel.supply_allocated",
    "military_panel.supply_not_computed",
    "military_panel.supply_invalid",
    "military_panel.combat_readiness",
    "military_panel.combat_readiness_unavailable",
    "military_panel.limit_manpower",
    "military_panel.limit_equipment",
    "military_panel.limit_supply",
    "military_panel.limit_none",
    "military_panel.limit_separator",
    "military_panel.multi_detail_line",
    "military_panel.compact_not_required",
    "military_panel.compact_supply_not_computed",
    "military_panel.compact_supply_invalid",
    "military_panel.compact_unavailable",
    "military_panel.selected_missing",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::DivisionDefinitionId;
    use crate::localization::{Locale, TranslationCatalog, extract_placeholders};
    use crate::military::data::{DivisionSize, DivisionType};

    fn division(
        id: usize,
        manpower: u64,
        max_manpower: u64,
        equipment: f64,
        max_equipment: f64,
        organization: f32,
        max_organization: f32,
    ) -> Division {
        Division {
            id: DivisionId(id),
            owner: crate::common::CountryId(0),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(3),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower,
            equipment,
            max_equipment,
            organization,
            max_organization,
            morale: 50.0,
            max_morale: 50.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn view(d: &Division, supply_entry: Option<f64>) -> DivisionStatusView {
        DivisionStatusView::from_division(d, supply_entry, supply_entry.unwrap_or(1.0))
    }

    /// キーと引数をそのまま文字列化するテスト用の翻訳関数。
    fn tr(key: &'static str) -> String {
        key.to_string()
    }
    fn trf(key: &'static str, args: Vec<(&'static str, String)>) -> String {
        let a: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{key}[{}]", a.join(";"))
    }

    fn lines(v: &DivisionStatusView) -> Vec<String> {
        v.detail_lines(&tr, &trf)
    }

    #[test]
    fn percent_text_never_rounds_a_nonzero_value_to_zero_or_a_partial_value_to_full() {
        assert_eq!(percent_from_fraction(0.0), "0%");
        assert_eq!(percent_from_fraction(1.0), "100%");
        assert_eq!(percent_from_fraction(0.5), "50.0%");
        assert_eq!(percent_from_fraction(0.000_001), "0.1%");
        assert_eq!(percent_from_fraction(0.999_999), "99.9%");
        assert_eq!(percent_from_fraction(0.9995), "99.9%");
        assert_eq!(percent_from_fraction(f64::NAN), "-");
        assert_eq!(percent_from_fraction(-0.5), "0%");
        assert_eq!(
            percent_from_fraction(1.25),
            "125.0%",
            "異常な超過は隠さない"
        );
    }

    #[test]
    fn zero_supply_is_shown_as_zero_and_is_the_limiting_factor() {
        let d = division(1, 10_000, 10_000, 100.0, 100.0, 50.0, 50.0);
        let v = view(&d, Some(0.0));
        assert_eq!(v.supply, SupplyReading::Allocated(0.0));
        let text = lines(&v);
        assert!(
            text.iter()
                .any(|l| l.contains("supply_allocated[percent=0%]")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|l| l.contains("combat_readiness[percent=0%")),
            "{text:?}"
        );
        assert_eq!(v.limiting_factors(), vec![LimitingFactor::Supply]);
    }

    #[test]
    fn partial_supply_is_the_limiting_factor_and_is_not_rounded_to_full() {
        let d = division(1, 10_000, 10_000, 100.0, 100.0, 50.0, 50.0);
        let v = view(&d, Some(0.6));
        let text = lines(&v);
        assert!(
            text.iter()
                .any(|l| l.contains("supply_allocated[percent=60.0%]")),
            "{text:?}"
        );
        assert_eq!(v.limiting_factors(), vec![LimitingFactor::Supply]);
        let almost = view(&d, Some(0.999_9));
        assert!(
            lines(&almost).iter().any(|l| l.contains("percent=99.9%")),
            "ほぼ満額でも100%と表示しない"
        );
    }

    #[test]
    fn full_supply_and_full_equipment_have_no_limiting_factor() {
        let d = division(1, 10_000, 10_000, 100.0, 100.0, 50.0, 50.0);
        let v = view(&d, Some(1.0));
        assert!(v.limiting_factors().is_empty());
        let text = lines(&v);
        assert!(
            text.iter()
                .any(|l| l
                    .contains("combat_readiness[percent=100%;limiting=military_panel.limit_none]")),
            "{text:?}"
        );
    }

    #[test]
    fn equipment_shortage_is_reported_and_limits_readiness() {
        let d = division(1, 10_000, 10_000, 25.0, 100.0, 50.0, 50.0);
        let v = view(&d, Some(1.0));
        let text = lines(&v);
        assert!(
            text.iter()
                .any(|l| l.contains("military_panel.equipment[current=25;max=100;percent=25.0%]")),
            "{text:?}"
        );
        assert_eq!(v.limiting_factors(), vec![LimitingFactor::Equipment]);
    }

    #[test]
    fn tied_constraints_are_all_reported() {
        // 戦力50%・装備50%・補給100% → 戦力と装備が同率で制約。
        let d = division(1, 5_000, 10_000, 50.0, 100.0, 50.0, 50.0);
        let v = view(&d, Some(1.0));
        assert_eq!(
            v.limiting_factors(),
            vec![LimitingFactor::Manpower, LimitingFactor::Equipment]
        );
    }

    #[test]
    fn organization_zero_is_zero_percent_not_a_readiness_constraint() {
        let d = division(1, 10_000, 10_000, 100.0, 100.0, 0.0, 50.0);
        let v = view(&d, Some(1.0));
        let text = lines(&v);
        assert!(
            text.iter()
                .any(|l| l.contains("organization_ratio[current=0;max=50;percent=0%]")),
            "{text:?}"
        );
        // 組織率は戦闘準備率(戦力・装備・補給の最小値)に含まれない。
        assert!(v.limiting_factors().is_empty());
        assert_eq!(
            v.readiness.unwrap().combat_readiness_units,
            COMBAT_READINESS_SCALE
        );
    }

    #[test]
    fn division_requiring_no_equipment_is_not_shown_as_short_or_divided_by_zero() {
        let d = division(1, 10_000, 10_000, 0.0, 0.0, 50.0, 50.0);
        let v = view(&d, Some(1.0));
        assert!(v.requires_no_equipment());
        let text = lines(&v);
        assert!(
            text.iter()
                .any(|l| l == "military_panel.equipment_not_required"),
            "{text:?}"
        );
        assert!(!text.iter().any(|l| l.contains("equipment_unavailable")));
        assert!(v.limiting_factors().is_empty(), "必要装備0は制約にならない");
        let compact = v.compact_line(&tr, &trf);
        assert!(
            compact.contains("equipment=military_panel.compact_not_required"),
            "{compact}"
        );
    }

    #[test]
    fn not_yet_computed_supply_is_distinguished_from_zero_and_matches_combat_default() {
        let d = division(1, 10_000, 10_000, 100.0, 100.0, 50.0, 50.0);
        // エントリなし: 表示は「未計算」、戦闘計算は1.0扱い(`supply_fulfillment_of`)。
        let v = DivisionStatusView::from_division(&d, None, 1.0);
        assert_eq!(v.supply, SupplyReading::NotYetAllocated);
        let text = lines(&v);
        assert!(
            text.iter()
                .any(|l| l == "military_panel.supply_not_computed"),
            "{text:?}"
        );
        assert!(
            !text
                .iter()
                .any(|l| l.contains("percent=0%") && l.contains("supply"))
        );
        assert!(v.limiting_factors().is_empty());
    }

    #[test]
    fn invalid_supply_value_is_flagged_not_shown_as_a_percentage() {
        let d = division(1, 10_000, 10_000, 100.0, 100.0, 50.0, 50.0);
        for bad in [f64::NAN, f64::INFINITY, -0.1, 1.5] {
            let v = DivisionStatusView::from_division(&d, Some(bad), bad);
            assert!(matches!(v.supply, SupplyReading::Invalid(_)), "{bad}");
            let text = lines(&v);
            assert!(
                text.iter()
                    .any(|l| l.starts_with("military_panel.supply_invalid[")),
                "{text:?}"
            );
            assert!(
                text.iter()
                    .any(|l| l.starts_with("military_panel.combat_readiness_unavailable[")),
                "正規関数が拒否する値は準備率を出さない: {text:?}"
            );
        }
    }

    #[test]
    fn invalid_equipment_state_is_reported_as_unavailable() {
        // equipment > max_equipment は正規関数が拒否する(黙ってclampしない)。
        let d = division(1, 10_000, 10_000, 150.0, 100.0, 50.0, 50.0);
        let v = view(&d, Some(1.0));
        assert!(v.readiness.is_err());
        let text = lines(&v);
        assert!(
            text.iter()
                .any(|l| l.starts_with("military_panel.equipment_unavailable[")),
            "{text:?}"
        );
    }

    #[test]
    fn multi_selection_keeps_each_divisions_own_values() {
        let a = division(7, 10_000, 10_000, 100.0, 100.0, 50.0, 50.0);
        let b = division(9, 10_000, 10_000, 100.0, 100.0, 50.0, 50.0);
        let va = view(&a, Some(0.0));
        let vb = view(&b, Some(1.0));
        let la = va.compact_line(&tr, &trf);
        let lb = vb.compact_line(&tr, &trf);
        assert!(la.contains("supply=0%"), "{la}");
        assert!(
            la.contains("readiness=0% [military_panel.limit_supply]"),
            "{la}"
        );
        assert!(lb.contains("supply=100%"), "{lb}");
        assert!(lb.contains("readiness=100%"), "{lb}");
        assert_ne!(la, lb);
        assert_eq!(va.id, DivisionId(7));
        assert_eq!(vb.id, DivisionId(9));
    }

    #[test]
    fn manpower_percent_uses_the_canonical_readiness_units() {
        let d = division(1, 7_600, 10_000, 100.0, 100.0, 50.0, 50.0);
        let v = view(&d, Some(1.0));
        assert!(
            lines(&v)[0].contains("current=7600;max=10000;percent=76.0%"),
            "{:?}",
            lines(&v)
        );
        // max_manpower == 0 でも0除算しない(正規関数は0%扱い)。
        let z = view(&division(2, 0, 0, 100.0, 100.0, 50.0, 50.0), Some(1.0));
        assert!(lines(&z)[0].contains("percent=0%"));
    }

    #[test]
    fn all_view_keys_exist_in_both_locales_with_matching_placeholders() {
        let catalog = TranslationCatalog::load().expect("embedded catalogs must parse");
        for key in DIVISION_STATUS_VIEW_KEYS {
            for locale in [Locale::JaJp, Locale::EnUs] {
                assert!(catalog.contains(locale, key), "{key} missing in {locale:?}");
            }
            let ja = extract_placeholders(catalog.raw(Locale::JaJp, key).unwrap());
            let en = extract_placeholders(catalog.raw(Locale::EnUs, key).unwrap());
            assert_eq!(ja, en, "placeholder mismatch for {key}");
        }
    }
}
