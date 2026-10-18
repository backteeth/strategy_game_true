//! 検証で見つかった問題。`code`で種類を判別でき、`message`に対象IDや座標と修正の手がかりを含む。

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// 情報(属性だけの変更など。対応不要)。
    Info,
    /// 診断(制作途中なら許容。未決のルールに関わるものもここ)。
    Warning,
    /// エラー(その検証モードでは失敗)。
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueCode {
    // --- 構造(ファイル同士だけで判定できる) ---
    DuplicateProvinceId,
    DuplicateProvinceCell,
    ProvinceIdNotBelowNext,
    RetiredIdNotBelowNext,
    RetiredIdInUse,
    DuplicateStateId,
    EmptyState,
    UnknownProvinceInState,
    ProvinceInMultipleStates,
    RepresentativeNotMember,
    // --- 地理データとの照合 ---
    FingerprintAlgorithmMismatch,
    GridChanged,
    LandSetChanged,
    CoordinatesChanged,
    AttributesChanged,
    RiversChanged,
    ProvinceCellMissing,
    ProvinceCellIsWater,
    LandCellWithoutProvince,
    UnassignedProvince,
    PartialCoverage,
    StateDisconnected,
    UnknownState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldIssue {
    pub severity: Severity,
    pub code: IssueCode,
    pub message: String,
}

impl WorldIssue {
    pub fn new(severity: Severity, code: IssueCode, message: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
        }
    }
    pub fn error(code: IssueCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }
}

impl fmt::Display for WorldIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}/{:?}] {}", self.severity, self.code, self.message)
    }
}
