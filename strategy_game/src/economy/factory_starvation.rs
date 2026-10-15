//! P29-010: 入力(Iron)不足で稼働できない Factory の増設抑止(案 B2)のための履歴。
//!
//! ## 判定単位
//! **国家**(国家の Factory 全 entry)。Iron は国家の共通備蓄で、月次生産の入力充足率は州ごとでは
//! なく国家単位で決まる(`economy::production` の Step 2・D4)。したがって、同じ入力不足を受ける
//! Factory の増設は、建設先の州を変えても同じ制約を受ける。州単位の判定は、建設先を変えるだけで
//! 抑止を迂回できてしまう。一方、**全 entry が**条件を満たす場合だけ抑止する(1 つでも正常に
//! 稼働できている/Iron 以外の制約で低稼働の entry があれば、その国は抑止しない)ため、特定州の
//! 低稼働を理由に、正常に稼働できる設備まで一律に除外しない。
//!
//! ## 月次の観測(`CountryProductionOutcome`)
//! - `factory_entries`: level > 0 の Factory entry(州)の数。
//! - `factory_iron_starved_entries`: Iron 不足が**実際の稼働制約**(労働力・物流・破産・Iron 以外の
//!   入力で決まる上限より、実際の稼働率が厳密に低い)で、かつ稼働率が 0.1 **未満**の entry の数。
//!   Iron 以外で低稼働の entry は数えない。Iron と他の制約が同率(同時に律速)の場合も数えない。
//!   稼働率 0 でも、Iron が制約なら数える(実績としての 0)。
//!
//! 月が「飢餓」⇔ `factory_entries > 0` かつ全 entry が飢餓。
//!
//! ## 設備署名(再観測)
//! 各月に、Factory entry の (州, レベル) の集合から決定論的な署名を記録する。完成・増設・喪失・
//! 獲得で署名が変わると、古い実績は別の設備状態のものなので、**署名が現在と一致する直近 4 か月が
//! 揃うまで**(= 4 か月の再観測)抑止しない。
//!
//! ## 抑止の条件(`assess`)
//! 判断月を最後とする連続 [`FACTORY_STARVATION_WINDOW`] か月がすべて「飢餓」で、署名が現在と一致。
//! 履歴不足・欠測・署名の不一致・Factory が 1 つもない(初回建設)は**抑止しない**(従来の判断)。
//! 永続的な禁止フラグはない: 入力不足が解消した月(飢餓でない月)が実績に入れば、直ちに復帰する。
//!
//! ## 保存
//! `CountryAiRegistry` の追加フィールド(`#[serde(default)]`)。旧 Save では空で、4 か月の実績が
//! 揃うまで抑止しない。過去の月は推測しない。New Game で消去、Load で Save の値に置換。

use serde::{Deserialize, Serialize};

/// 連続して「飢餓」でなければならない月数(B1 の窓幅・P29-005 の窓幅と同じ 4)。
pub const FACTORY_STARVATION_WINDOW: usize = 4;

/// この稼働率「未満」を低稼働とみなす(0.1 ちょうどは含めない)。
pub const FACTORY_LOW_OPERATION_THRESHOLD: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactoryStarvationSample {
    pub year: i32,
    pub month: u8,
    /// この月の Factory 設備署名((州, レベル)の集合から決定論的に導出)。
    pub signature: u64,
    /// 全 entry が Iron 不足で稼働率 0.1 未満(Factory が 1 つ以上ある月だけ true になりうる)。
    pub starved: bool,
}

impl FactoryStarvationSample {
    fn ordinal(&self) -> i64 {
        month_ordinal(self.year, self.month)
    }
}

fn month_ordinal(year: i32, month: u8) -> i64 {
    year as i64 * 12 + (month as i64 - 1)
}

/// Factory entry の (州 ID, レベル) の集合から署名を作る(順序に依存しない、FNV-1a)。
pub fn factory_signature(entries: &[(usize, u32)]) -> u64 {
    let mut sorted: Vec<(usize, u32)> = entries.to_vec();
    sorted.sort_unstable();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (id, level) in sorted {
        for b in (id as u64).to_le_bytes().iter().chain(level.to_le_bytes().iter()) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactoryStarvationHistory {
    samples: Vec<FactoryStarvationSample>,
}

impl FactoryStarvationHistory {
    /// 月次生産の実績を追加する。同じ月・過去の月・不正な月は無視して`false`。
    pub fn record(&mut self, year: i32, month: u8, signature: u64, starved: bool) -> bool {
        if !(1..=12).contains(&month) {
            return false;
        }
        let sample = FactoryStarvationSample {
            year,
            month,
            signature,
            starved,
        };
        if let Some(last) = self.samples.last()
            && sample.ordinal() <= last.ordinal()
        {
            return false;
        }
        self.samples.push(sample);
        if self.samples.len() > FACTORY_STARVATION_WINDOW {
            let excess = self.samples.len() - FACTORY_STARVATION_WINDOW;
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

    pub fn samples(&self) -> &[FactoryStarvationSample] {
        &self.samples
    }

    /// Factory の増設を抑止するか。`now`: 判断月、`signature`: 現在の Factory 設備署名、
    /// `factory_entries`: 現在の Factory entry の数(0 なら初回建設で、抑止しない)。
    pub fn blocks_expansion(&self, now: (i32, u8), signature: u64, factory_entries: usize) -> bool {
        if factory_entries == 0 {
            return false;
        }
        let window = FACTORY_STARVATION_WINDOW;
        if self.samples.len() < window {
            return false;
        }
        let tail = &self.samples[self.samples.len() - window..];
        let last_ok = tail
            .last()
            .is_some_and(|s| s.ordinal() == month_ordinal(now.0, now.1));
        let consecutive = tail.windows(2).all(|w| w[1].ordinal() - w[0].ordinal() == 1);
        let same_fleet = tail.iter().all(|s| s.signature == signature);
        last_ok && consecutive && same_fleet && tail.iter().all(|s| s.starved)
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
        } else if self.samples.len() > FACTORY_STARVATION_WINDOW {
            let excess = self.samples.len() - FACTORY_STARVATION_WINDOW;
            self.samples.drain(..excess);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(starved: &[bool], sig: u64) -> FactoryStarvationHistory {
        let mut h = FactoryStarvationHistory::default();
        for (i, s) in starved.iter().enumerate() {
            h.record(1800 + (i / 12) as i32, (i % 12) as u8 + 1, sig, *s);
        }
        h
    }

    #[test]
    fn four_consecutive_starved_months_block_but_three_do_not() {
        let sig = factory_signature(&[(1, 5), (2, 3)]);
        assert!(!filled(&[true, true, true], sig).blocks_expansion((1800, 3), sig, 2));
        assert!(filled(&[true; 4], sig).blocks_expansion((1800, 4), sig, 2));
        assert!(!filled(&[true; 4], sig).blocks_expansion((1800, 6), sig, 2), "古い履歴");
    }

    #[test]
    fn one_non_starved_month_breaks_the_condition_and_recovery_is_immediate() {
        let sig = 7;
        assert!(!filled(&[true, true, false, true], sig).blocks_expansion((1800, 4), sig, 1));
        let mut h = filled(&[true; 4], sig);
        assert!(h.blocks_expansion((1800, 4), sig, 1));
        h.record(1800, 5, sig, false); // 入力不足が解消した月
        assert!(!h.blocks_expansion((1800, 5), sig, 1), "永続的な禁止はなく直ちに復帰する");
    }

    #[test]
    fn a_changed_fleet_requires_a_fresh_observation_and_no_factory_never_blocks() {
        let old = factory_signature(&[(1, 5)]);
        let new = factory_signature(&[(1, 6)]);
        let mut h = filled(&[true; 4], old);
        assert!(!h.blocks_expansion((1800, 4), new, 1), "増設後の署名は一致しない");
        for m in 5..=8u8 {
            h.record(1800, m, new, true);
        }
        assert!(h.blocks_expansion((1800, 8), new, 1), "4 か月の再観測後に有効");
        assert!(!h.blocks_expansion((1800, 8), new, 0), "Factory がない(初回建設)は抑止しない");
    }

    #[test]
    fn signature_is_order_independent_and_discriminating() {
        assert_eq!(factory_signature(&[(1, 2), (3, 4)]), factory_signature(&[(3, 4), (1, 2)]));
        assert_ne!(factory_signature(&[(1, 2)]), factory_signature(&[(1, 3)]));
        assert_ne!(factory_signature(&[(1, 2)]), factory_signature(&[(2, 2)]));
    }

    #[test]
    fn duplicates_year_wrap_retention_and_invalid_loads() {
        let mut h = FactoryStarvationHistory::default();
        assert!(h.record(1800, 12, 1, true));
        assert!(!h.record(1800, 12, 1, false), "同じ月は二重に追加しない");
        assert!(h.record(1801, 1, 1, true));
        assert!(!h.record(1801, 13, 1, true));
        for m in 2..=8 {
            h.record(1801, m, 1, true);
        }
        assert_eq!(h.samples().len(), FACTORY_STARVATION_WINDOW);
        let bad = FactoryStarvationHistory {
            samples: vec![
                FactoryStarvationSample { year: 1800, month: 5, signature: 0, starved: true },
                FactoryStarvationSample { year: 1800, month: 4, signature: 0, starved: true },
            ],
        };
        assert!(bad.sanitized().is_empty());
    }
}
