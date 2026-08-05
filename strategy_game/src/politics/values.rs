use serde::{Deserialize, Serialize};

/// 3軸の国家価値観 (-100.0 〜 100.0)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountryValues {
    /// 科学至上 (-100.0) ↔ 魔法至上 (100.0)
    pub science_magic: f32,
    /// 個人主義 (-100.0) ↔ 国家主義 (100.0)
    pub individual_state: f32,
    /// 世俗主義 (-100.0) ↔ 宗教重視 (100.0)
    pub secular_religious: f32,
}

impl Default for CountryValues {
    fn default() -> Self {
        Self {
            science_magic: 0.0,
            individual_state: 0.0,
            secular_religious: 0.0,
        }
    }
}

impl CountryValues {
    pub fn clamp_all(&mut self) {
        self.science_magic = self.science_magic.clamp(-100.0, 100.0);
        self.individual_state = self.individual_state.clamp(-100.0, 100.0);
        self.secular_religious = self.secular_religious.clamp(-100.0, 100.0);
    }
}

/// 改革対象軸
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueAxis {
    ScienceMagic,
    IndividualState,
    SecularReligious,
}

impl ValueAxis {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            ValueAxis::ScienceMagic => "value_axis.science_magic",
            ValueAxis::IndividualState => "value_axis.individual_state",
            ValueAxis::SecularReligious => "value_axis.secular_religious",
        }
    }
}
