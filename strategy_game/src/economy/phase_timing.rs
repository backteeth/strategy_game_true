//! P32-026: 保存・復元の工程別計時(任意。通常は無効)。
//!
//! 計時は**測定の入れ物**で、経済状態・保存データ・判定には一切入らない(この型を保存形式へ渡す経路は無い)。
//! 無効(`Timing::off()`、既定)のときは時計を読まず、工程の処理をそのまま呼ぶだけ。経済計算へ時計や実行時間を入力しない。
//!
//! 工程は**互いに入れ子にならない**(親子なし)。共有の保存・復元(`map_economy_file::save_timed` / `load_timed`)が
//! 直列に実行する工程を、それぞれ1回の区間として計るので、工程の時間を足しても二重計上にならない。
//! 全体の時間は別に(関数の入口から出口まで)計り、工程で覆っていない時間は [`Timing::uncovered`] で明示する。

use std::time::{Duration, Instant};

/// 工程ごとの累計(同じ名前の工程が複数回呼ばれたら、呼出回数と時間を合算する)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub name: &'static str,
    pub calls: u32,
    pub total: Duration,
}

/// 結果の区別。計時の対象が成功したか、判定で拒否されたか、エラーで終わったか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Outcome {
    /// まだ終わっていない(または計時が無効)。
    #[default]
    Unset,
    Success,
    /// 互換性の判定で拒否された(復元しない)。
    Rejected,
    /// 読み込み・検証・書込などのエラー。
    Error,
}

#[derive(Debug, Clone, Default)]
pub struct Timing {
    enabled: bool,
    started: Option<Instant>,
    total: Duration,
    spans: Vec<Span>,
    outcome: Outcome,
}

impl Timing {
    /// 無効(何も計らない)。
    pub fn off() -> Self {
        Self::default()
    }

    /// 有効。
    pub fn on() -> Self {
        Timing { enabled: true, ..Self::default() }
    }

    pub fn is_on(&self) -> bool {
        self.enabled
    }

    /// 工程 `name` を1回の区間として計って `f` を実行する。無効のときは `f` を呼ぶだけ。
    pub fn run<T>(&mut self, name: &'static str, f: impl FnOnce() -> T) -> T {
        if !self.enabled {
            return f();
        }
        let t = Instant::now();
        let r = f();
        let el = t.elapsed();
        match self.spans.iter_mut().find(|s| s.name == name) {
            Some(s) => {
                s.calls += 1;
                s.total += el;
            }
            None => self.spans.push(Span { name, calls: 1, total: el }),
        }
        r
    }

    /// 全体の計時の開始(関数の入口)。
    pub(crate) fn begin_total(&mut self) {
        if self.enabled {
            self.started = Some(Instant::now());
        }
    }

    /// 全体の計時の終了(関数の出口)と結果の記録。
    pub(crate) fn end_total(&mut self, outcome: Outcome) {
        if self.enabled {
            if let Some(t) = self.started.take() {
                self.total += t.elapsed();
            }
            self.outcome = outcome;
        }
    }

    pub fn total(&self) -> Duration {
        self.total
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// 工程(出現順)。
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    pub fn span(&self, name: &str) -> Option<&Span> {
        self.spans.iter().find(|s| s.name == name)
    }

    /// 工程の合計(親子がないので、そのまま足してよい)。
    pub fn covered(&self) -> Duration {
        self.spans.iter().map(|s| s.total).sum()
    }

    /// 工程で覆っていない時間(全体 − 工程の合計。関数の分岐・ループ・計時自体の費用などを含む)。
    pub fn uncovered(&self) -> Duration {
        self.total.saturating_sub(self.covered())
    }
}
