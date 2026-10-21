//! P29-005: AI 募兵の能力推定に使う、軍用補給物資の**実際の月次産出**の履歴。
//!
//! P29-004 の能力推定は「直近の記録済み稼働率 × 現在設備」の 1 点推定で、約 4 か月周期で稼働率が
//! 変動する国(P29-004-REV-001: Elfin の 0.43〜0.94)では、高稼働月の値で追加募兵が許可され、
//! 平均では維持できない師団数になった。本モジュールは、月次生産が**実際に備蓄へ反映した量**を
//! 対象年月とともに保持し、直近 N か月の平均(持続的な生産の基礎)を提供する。
//!
//! ## 役割分担
//! - [`crate::economy::supply_balance::MilitarySupplyProductionLedger`]: UI 表示専用・直近 1 か月・
//!   非永続(ロードで空に戻り「未集計」と表示する)。
//! - 本履歴: AI 判断用・直近 N か月・Save に保存(旧 Save では空)。ロードで消去しない。
//!
//! ## 契約
//! - 過去の稼働率に現在の建物レベルを掛けて過去産出を再構成しない(実際の産出だけを記録する)。
//! - 生産 0 の月は有効な実績(`Some(0)`)。集計失敗の月は `None`(0 と区別)で、窓に含まれると
//!   平均は算出されない。
//! - 同じ対象月・過去の月は二重に追加しない。年越しも暦順(年×12+月)で比較する。
//! - 保持するのは窓幅 [`SUPPLY_HISTORY_WINDOW`] か月分まで。

use serde::{Deserialize, Serialize};

/// 持続的な生産の基礎に使う直近の月数(P29-005 で 4・6・12 を比較の上、4 を採用。根拠は報告書)。
pub const SUPPLY_HISTORY_WINDOW: usize = 4;

/// 1 か月分の実績。`units`が`None`なら、その月は集計できなかった(生産 0 ではない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthlyProductionSample {
    pub year: i32,
    pub month: u8,
    pub units: Option<u64>,
}

impl MonthlyProductionSample {
    fn ordinal(&self) -> i64 {
        month_ordinal(self.year, self.month)
    }
}

fn month_ordinal(year: i32, month: u8) -> i64 {
    year as i64 * 12 + (month as i64 - 1)
}

/// 国家 1 つ分の履歴(古い順)。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyProductionHistory {
    samples: Vec<MonthlyProductionSample>,
}

impl SupplyProductionHistory {
    /// 月次生産の実績を追加する。最後の対象月以前(同月の重複・逆順)は無視して`false`を返す。
    /// 窓幅を超える古い実績は捨てる。
    pub fn record(&mut self, year: i32, month: u8, units: Option<u64>) -> bool {
        if !(1..=12).contains(&month) {
            return false;
        }
        let sample = MonthlyProductionSample { year, month, units };
        if let Some(last) = self.samples.last()
            && sample.ordinal() <= last.ordinal()
        {
            return false;
        }
        self.samples.push(sample);
        if self.samples.len() > SUPPLY_HISTORY_WINDOW {
            let excess = self.samples.len() - SUPPLY_HISTORY_WINDOW;
            self.samples.drain(..excess);
        }
        true
    }

    /// 保存した標本から履歴を再構築する(検証付き)。`record`と同じ不変条件(月 1..=12、暦順に厳密な
    /// 昇順、窓幅以下)を満たさなければ拒否し、無検証の代入で履歴を作らない。
    pub fn from_samples(samples: Vec<MonthlyProductionSample>) -> Result<Self, String> {
        if samples.len() > SUPPLY_HISTORY_WINDOW {
            return Err(format!(
                "supply production history has {} samples (window {SUPPLY_HISTORY_WINDOW})",
                samples.len()
            ));
        }
        if samples.iter().any(|s| !(1..=12).contains(&s.month)) {
            return Err("supply production history has a sample with an invalid month".to_string());
        }
        if samples.windows(2).any(|w| w[0].ordinal() >= w[1].ordinal()) {
            return Err("supply production history samples are not strictly ascending".to_string());
        }
        Ok(Self { samples })
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn samples(&self) -> &[MonthlyProductionSample] {
        &self.samples
    }

    /// `now`(判断時点の年月)を最後とする、連続した [`SUPPLY_HISTORY_WINDOW`] か月の実績の平均
    /// (units/月、端数切捨て)。次のいずれかなら`None`(通常募兵を保留する): 実績が窓幅に
    /// 満たない・最後の月が`now`でない(古い履歴)・月が連続していない(欠測)・窓内に集計失敗の月がある。
    pub fn average_units(&self, now: (i32, u8)) -> Option<u128> {
        let window = SUPPLY_HISTORY_WINDOW;
        if self.samples.len() < window {
            return None;
        }
        let tail = &self.samples[self.samples.len() - window..];
        if tail.last()?.ordinal() != month_ordinal(now.0, now.1) {
            return None;
        }
        if tail.windows(2).any(|w| w[1].ordinal() - w[0].ordinal() != 1) {
            return None;
        }
        let mut sum: u128 = 0;
        for s in tail {
            sum = sum.checked_add(s.units? as u128)?;
        }
        Some(sum / window as u128)
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
        } else if self.samples.len() > SUPPLY_HISTORY_WINDOW {
            let excess = self.samples.len() - SUPPLY_HISTORY_WINDOW;
            self.samples.drain(..excess);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(values: &[u64], start: (i32, u8)) -> SupplyProductionHistory {
        let mut h = SupplyProductionHistory::default();
        let (mut y, mut m) = start;
        for &v in values {
            h.record(y, m, Some(v));
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
        h
    }

    #[test]
    fn average_requires_a_full_contiguous_window_ending_now() {
        let h = filled(&[100, 200, 300], (1800, 1));
        assert_eq!(h.average_units((1800, 3)), None, "3 か月では不足");
        let h = filled(&[100, 200, 300, 400], (1800, 1));
        assert_eq!(h.average_units((1800, 4)), Some(250));
        assert_eq!(h.average_units((1800, 5)), None, "最後の月が now でない");
    }

    #[test]
    fn zero_months_count_and_unavailable_months_block_the_average() {
        let h = filled(&[0, 0, 400, 400], (1800, 1));
        assert_eq!(h.average_units((1800, 4)), Some(200), "0 は有効な実績");
        let mut h = filled(&[100, 100, 100], (1800, 1));
        h.record(1800, 4, None);
        assert_eq!(h.average_units((1800, 4)), None, "集計失敗の月は 0 ではない");
        for m in 5..=8 {
            h.record(1800, m, Some(100));
        }
        assert_eq!(h.average_units((1800, 8)), Some(100), "窓から外れれば回復する");
    }

    #[test]
    fn duplicates_and_out_of_order_months_are_ignored_and_year_wrap_orders_correctly() {
        let mut h = filled(&[10, 20], (1800, 11));
        assert!(!h.record(1800, 12, Some(999)), "同じ対象月は二重に追加しない");
        assert!(!h.record(1800, 11, Some(999)), "過去の月は追加しない");
        assert!(h.record(1801, 1, Some(30)));
        assert!(h.record(1801, 2, Some(40)));
        assert_eq!(h.average_units((1801, 2)), Some(25));
        assert!(!h.record(1801, 13, Some(1)), "不正な月");
    }

    #[test]
    fn gap_in_months_blocks_the_window() {
        let mut h = filled(&[100, 100], (1800, 1));
        h.record(1800, 4, Some(100));
        h.record(1800, 5, Some(100));
        assert_eq!(h.average_units((1800, 5)), None, "3 月が欠測");
    }

    #[test]
    fn retains_only_the_window_and_sanitizes_invalid_loads() {
        let h = filled(&[1, 2, 3, 4, 5, 6], (1800, 1));
        assert_eq!(h.samples().len(), SUPPLY_HISTORY_WINDOW);
        assert_eq!(h.samples()[0].units, Some(3));
        let bad = SupplyProductionHistory {
            samples: vec![
                MonthlyProductionSample {
                    year: 1800,
                    month: 5,
                    units: Some(1),
                },
                MonthlyProductionSample {
                    year: 1800,
                    month: 4,
                    units: Some(1),
                },
            ],
        };
        assert!(bad.sanitized().is_empty());
    }

    /// 窓幅 4・6・12 の応答(周期変動・恒久増産・恒久減産)。報告書 §2 の比較の根拠
    /// (窓幅の比較は純粋な算術で行い、実装の窓幅は [`SUPPLY_HISTORY_WINDOW`] 固定)。
    #[test]
    fn window_comparison_on_cycle_and_permanent_changes() {
        let avg = |series: &[u64], end: usize, w: usize| -> f64 {
            series[end + 1 - w..=end].iter().sum::<u64>() as f64 / w as f64
        };
        // 4 か月周期(平均 200 の周期 [280, 280, 140, 100])
        let cycle: Vec<u64> = [280u64, 280, 140, 100].iter().cycle().take(48).copied().collect();
        for w in [4usize, 6, 12] {
            let worst = (11..48).map(|e| avg(&cycle, e, w) - 200.0).fold(f64::MIN, f64::max);
            // 周期の整数倍の窓(4・12)は周期を完全に平均化し、過大評価しない。
            // 6 か月は周期と整合しないため、周期の位相によって平均を過大評価する月が残る。
            match w {
                4 | 12 => assert!(worst.abs() < 1e-9, "w={w}: {worst}"),
                _ => assert!(worst > 10.0, "w={w}: {worst}"),
            }
        }
        // 恒久増産(100→200)・恒久減産(200→100): 平均が中間(150)を越えるまでの月数は窓幅に比例。
        for w in [4usize, 6, 12] {
            let up: Vec<u64> = (0..24).map(|i| if i < 12 { 100 } else { 200 }).collect();
            let down: Vec<u64> = (0..24).map(|i| if i < 12 { 200 } else { 100 }).collect();
            let lag_up = (12..24).find(|&e| avg(&up, e, w) >= 150.0).unwrap() - 12 + 1;
            let lag_down = (12..24).find(|&e| avg(&down, e, w) <= 150.0).unwrap() - 12 + 1;
            assert_eq!(lag_up, w.div_ceil(2));
            assert_eq!(lag_down, w.div_ceil(2));
        }
    }
}
