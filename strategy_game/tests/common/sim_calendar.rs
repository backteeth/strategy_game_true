//! P28-002-PRE-002: 長期E2Eハーネス共通の簡略暦を、実在する年・月へ写像する。
//!
//! 各ハーネスは「1か月=30日(day 1〜30)+月境界tick」の簡略暦で、通算の月番号
//! (`month_index`、1始まり、1 = 開始年の1月)を増やしながら日次・月次メッセージを
//! 発行する。以前はこの通算番号をそのまま`month`として発行していたため、13か月目
//! 以降に13月・14月…という実在しない月が生じ、講和処理の日付計算
//! (`GameDate::add_days`)が範囲外アクセスでpanicし得た。
//!
//! ここでは通算番号を`(年, 1〜12月)`へ変換するだけで、1か月の日数(30日)・月次tickの
//! 位置・総日数は変えない。状態を持たない純粋関数なので、テスト間で共有される
//! 隠れた状態はない。

/// 通算の月番号`month_index`(1始まり)を、`base_year`起点の`(年, 月)`へ変換する。
/// 12月の次は翌年1月になる(例: `base_year=1800`で12 → (1800, 12)、13 → (1801, 1))。
pub fn sim_calendar_date(base_year: i32, month_index: u8) -> (i32, u8) {
    assert!(month_index >= 1, "month_index is 1-based (got 0)");
    let zero_based = i32::from(month_index - 1);
    (base_year + zero_based / 12, (zero_based % 12) as u8 + 1)
}
