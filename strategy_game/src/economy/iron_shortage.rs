//! P29-009: 建設 AI が Iron の供給不足を認識するための、**フロー基準**の履歴。
//!
//! 従来の建設 AI は Iron 備蓄の瞬間値(閾値 30)だけを見ていた。月次会計では入力の消費後に
//! Mine の産出が加わるため、深刻な不足でも備蓄は月産程度に戻り、不足と認識されなかった
//! (P29-008-DIAG-001)。本モジュールは、月次生産が記録する **MilitaryFactory の Iron 未充足の要求量**
//! (`CountryProductionOutcome::iron_unmet_military`)を、その月の Mine レベル合計とともに保持する。
//!
//! ## 定義
//! - 建設上の需要: MilitaryFactory が、労働力・物流・Iron 以外の入力で到達できる稼働率の上限まで
//!   動くために足りなかった Iron(Iron/月)。供給不足で消費も減った状態を「均衡」と誤認しない。
//!   Factory の未充足は需要に含めない(過剰に増設された Factory の最大需要を満たすために Mine を
//!   建て続けない。Factory の増設抑止は別タスク)。Iron 以外の制約で動けない分は、上限の定義に
//!   より未充足に含まれない。
//! - 継続的な不足: 判断月を最後とする連続 [`IRON_SHORTAGE_WINDOW`] か月すべてで未充足が正であり、
//!   かつその間 Mine レベル合計が現在と同じ(= 直近の Mine の完成・喪失の影響が実績に反映済み)。
//! - 未集計: 履歴が窓幅に満たない・最後の月が今月でない・月が連続しない・集計失敗の月を含む・
//!   窓内で Mine レベル合計が変わった。この場合は「不足なし」でも「不足」でもなく、建設 AI は
//!   従来の判断(備蓄閾値 30)に委ねる。0 や無限大として扱わない。
//!
//! ## 保存
//! `CountryAiRegistry` の追加フィールド(Save V1 の `#[serde(default)]`)。旧 Save では空で、
//! 次の [`IRON_SHORTAGE_WINDOW`] か月の実績が揃うまで未集計(従来判断)。過去の月は推測しない。
//! New Game では消去し、Load では Save の値で置き換える。

use serde::{Deserialize, Serialize};

/// 継続的な不足とみなす連続月数(P29-005 の生産履歴の窓幅と同じ 4。Mine の建設期間が
/// おおむね 2〜3 か月で、完成後の効果が実績に反映されるまでの期間もこれに合わせる)。
pub const IRON_SHORTAGE_WINDOW: usize = 4;

/// Iron 未充足量の固定小数点の倍率(Iron/月 × 1000 を整数で保持する)。
const MILLI: f64 = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronShortageSample {
    pub year: i32,
    pub month: u8,
    /// MilitaryFactory の Iron 未充足の要求量 × 1000。集計失敗は`None`(0 と区別)。
    pub unmet_milli: Option<u64>,
    /// この月次生産の時点の、国家が所有する州の Mine レベル合計。
    pub mine_levels: u32,
}

impl IronShortageSample {
    fn ordinal(&self) -> i64 {
        month_ordinal(self.year, self.month)
    }
}

fn month_ordinal(year: i32, month: u8) -> i64 {
    year as i64 * 12 + (month as i64 - 1)
}

/// 建設 AI へ渡す不足の評価。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IronShortageAssessment {
    /// 未集計(履歴不足・古い・欠測・Mine レベルの変化直後)。従来の判断に委ねる。
    Unavailable,
    /// 集計済みで、継続的な不足はない(一時的な不足を含む)。
    NoSustainedShortage,
    /// 継続的な不足。窓内の月平均の未充足量(Iron/月)。
    Sustained { average_unmet: f64 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronShortageHistory {
    samples: Vec<IronShortageSample>,
}

impl IronShortageHistory {
    /// 月次生産の実績を追加する。同じ月・過去の月・不正な月は無視して`false`。
    pub fn record(&mut self, year: i32, month: u8, unmet: Option<f64>, mine_levels: u32) -> bool {
        if !(1..=12).contains(&month) {
            return false;
        }
        let unmet_milli = unmet
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| (v * MILLI).round() as u64);
        let sample = IronShortageSample {
            year,
            month,
            unmet_milli,
            mine_levels,
        };
        if let Some(last) = self.samples.last()
            && sample.ordinal() <= last.ordinal()
        {
            return false;
        }
        self.samples.push(sample);
        if self.samples.len() > IRON_SHORTAGE_WINDOW {
            let excess = self.samples.len() - IRON_SHORTAGE_WINDOW;
            self.samples.drain(..excess);
        }
        true
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn samples(&self) -> &[IronShortageSample] {
        &self.samples
    }

    /// `now`(判断月)・`mine_levels`(現在の Mine レベル合計)に対する評価。
    pub fn assess(&self, now: (i32, u8), mine_levels: u32) -> IronShortageAssessment {
        let window = IRON_SHORTAGE_WINDOW;
        if self.samples.len() < window {
            return IronShortageAssessment::Unavailable;
        }
        let tail = &self.samples[self.samples.len() - window..];
        let last_ok = tail
            .last()
            .is_some_and(|s| s.ordinal() == month_ordinal(now.0, now.1));
        let consecutive = tail.windows(2).all(|w| w[1].ordinal() - w[0].ordinal() == 1);
        let same_mines = tail.iter().all(|s| s.mine_levels == mine_levels);
        if !last_ok || !consecutive || !same_mines {
            return IronShortageAssessment::Unavailable;
        }
        let mut sum = 0.0;
        for s in tail {
            match s.unmet_milli {
                None => return IronShortageAssessment::Unavailable,
                Some(0) => return IronShortageAssessment::NoSustainedShortage,
                Some(v) => sum += v as f64 / MILLI,
            }
        }
        IronShortageAssessment::Sustained {
            average_unmet: sum / window as f64,
        }
    }

    /// Load 時の防御: 暦順でない・不正な月を含む履歴は、推測で補わず空にする。
    pub fn sanitized(mut self) -> Self {
        let valid = self.samples.iter().all(|s| (1..=12).contains(&s.month))
            && self
                .samples
                .windows(2)
                .all(|w| w[0].ordinal() < w[1].ordinal());
        if !valid {
            self.samples.clear();
        } else if self.samples.len() > IRON_SHORTAGE_WINDOW {
            let excess = self.samples.len() - IRON_SHORTAGE_WINDOW;
            self.samples.drain(..excess);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(vals: &[Option<f64>], mines: u32) -> IronShortageHistory {
        let mut h = IronShortageHistory::default();
        for (i, v) in vals.iter().enumerate() {
            h.record(1800 + (i / 12) as i32, (i % 12) as u8 + 1, *v, mines);
        }
        h
    }

    #[test]
    fn sustained_shortage_needs_a_full_contiguous_window_with_positive_unmet() {
        let h = filled(&[Some(40.0), Some(50.0), Some(60.0)], 3);
        assert_eq!(h.assess((1800, 3), 3), IronShortageAssessment::Unavailable, "3 か月では未集計");
        let h = filled(&[Some(40.0), Some(50.0), Some(60.0), Some(50.0)], 3);
        assert_eq!(
            h.assess((1800, 4), 3),
            IronShortageAssessment::Sustained { average_unmet: 50.0 }
        );
        assert_eq!(h.assess((1800, 6), 3), IronShortageAssessment::Unavailable, "古い履歴");
    }

    #[test]
    fn a_temporary_shortage_is_not_sustained_and_zero_is_distinct_from_unavailable() {
        let h = filled(&[Some(0.0), Some(50.0), Some(60.0), Some(50.0)], 3);
        assert_eq!(h.assess((1800, 4), 3), IronShortageAssessment::NoSustainedShortage);
        let h = filled(&[Some(50.0), None, Some(60.0), Some(50.0)], 3);
        assert_eq!(h.assess((1800, 4), 3), IronShortageAssessment::Unavailable, "集計失敗は 0 ではない");
    }

    #[test]
    fn a_change_in_mine_levels_makes_the_window_unavailable_until_it_refills() {
        let mut h = filled(&[Some(40.0), Some(40.0), Some(40.0), Some(40.0)], 3);
        // Mine が 1 レベル増えた(完成)。古い 4 件はもう「現在の設備」の実績ではない。
        assert_eq!(h.assess((1800, 4), 4), IronShortageAssessment::Unavailable);
        for m in 5..=8u8 {
            h.record(1800, m, Some(10.0), 4);
        }
        assert_eq!(
            h.assess((1800, 8), 4),
            IronShortageAssessment::Sustained { average_unmet: 10.0 }
        );
    }

    #[test]
    fn duplicates_year_wrap_and_invalid_loads() {
        let mut h = IronShortageHistory::default();
        assert!(h.record(1800, 11, Some(1.0), 1));
        assert!(!h.record(1800, 11, Some(9.0), 1), "同じ月は二重に追加しない");
        assert!(h.record(1800, 12, Some(1.0), 1));
        assert!(h.record(1801, 1, Some(1.0), 1));
        assert!(h.record(1801, 2, Some(1.0), 1));
        assert!(h.assess((1801, 2), 1) != IronShortageAssessment::Unavailable);
        assert!(!h.record(1801, 13, Some(1.0), 1));
        for i in 3..=8 {
            h.record(1801, i, Some(1.0), 1);
        }
        assert_eq!(h.samples().len(), IRON_SHORTAGE_WINDOW);
        let bad = IronShortageHistory {
            samples: vec![
                IronShortageSample { year: 1800, month: 5, unmet_milli: Some(1), mine_levels: 1 },
                IronShortageSample { year: 1800, month: 4, unmet_milli: Some(1), mine_levels: 1 },
            ],
        };
        assert!(bad.sanitized().is_empty());
    }
}
