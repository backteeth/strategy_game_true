use crate::app::game_state::GameState;
use bevy::prelude::*;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum DailySimulationSet {
    TimeUpdate,
    Economy,
    Research,
    Diplomacy,
    CountryAi,
    WarPreparation,
    MilitaryAi,
    FrontlineOrders,
    MilitaryAction,
    WarResolution,
    UiUpdate,
}

// ─── 定数 ──────────────────────────────────────────────────────────────────

/// 各月の日数
const DAYS_IN_MONTH: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// 速度ごとのゲーム内秒数/実時間秒
const SPEED_DAYS_PER_REAL_SECOND: [f64; 4] = [1.0, 3.0, 7.0, 30.0];

// ─── メッセージ ──────────────────────────────────────────────────────────────

/// 日付が進んだことを通知するメッセージ（日次更新）
#[derive(Message, Debug, Clone, Copy)]
pub struct DayChangedMessage {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

/// 月が切り替わったことを通知するメッセージ（月次更新）
#[derive(Message, Debug, Clone, Copy)]
pub struct MonthChangedMessage {
    pub year: i32,
    pub month: u8,
}

// ─── データ型 ───────────────────────────────────────────────────────────────

/// ゲーム内日付
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GameDate {
    pub year: i32,
    /// 月（1〜12）
    pub month: u8,
    /// 日（1〜28/30/31）
    pub day: u8,
    /// 経過日数の小数点以下アキュムレーター
    accumulator: f64,
}

impl Default for GameDate {
    fn default() -> Self {
        Self {
            year: 1800,
            month: 1,
            day: 1,
            accumulator: 0.0,
        }
    }
}

impl GameDate {
    /// 新しい GameDate を作成
    pub fn new(year: i32, month: u8, day: u8) -> Self {
        Self {
            year,
            month,
            day,
            accumulator: 0.0,
        }
    }

    /// 経過日数アキュムレーターに値を加算する（テスト・調整用）
    pub fn add_accumulator(&mut self, amount: f64) {
        self.accumulator += amount;
    }

    /// 日付を文字列として取得する ("YYYY/MM/DD")
    pub fn display(&self) -> String {
        format!("{:04}/{:02}/{:02}", self.year, self.month, self.day)
    }

    /// 指定日数後の GameDate を計算して取得する
    pub fn add_days(&self, days: u32) -> GameDate {
        let mut y = self.year;
        let mut m = self.month;
        let mut d = self.day as u32 + days;

        loop {
            let max_d = DAYS_IN_MONTH[(m as usize) - 1] as u32;
            if d <= max_d {
                break;
            }
            d -= max_d;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }

        GameDate {
            year: y,
            month: m,
            day: d as u8,
            accumulator: 0.0,
        }
    }

    /// "YYYY/MM/DD" または "YYYY-MM-DD" から GameDate を生成する
    pub fn from_string(s: &str) -> Option<GameDate> {
        let parts: Vec<&str> = if s.contains('/') {
            s.split('/').collect()
        } else {
            s.split('-').collect()
        };
        if parts.len() != 3 {
            return None;
        }
        let year = parts[0].trim().parse::<i32>().ok()?;
        let month = parts[1].trim().parse::<u8>().ok()?;
        let day = parts[2].trim().parse::<u8>().ok()?;
        Some(GameDate::new(year, month, day))
    }

    /// 2つの日付間の概算経過日数を取得する (簡易計算)
    pub fn days_since(&self, start: &GameDate) -> i32 {
        let y_diff = self.year - start.year;
        let m_diff = self.month as i32 - start.month as i32;
        let d_diff = self.day as i32 - start.day as i32;
        y_diff * 365 + m_diff * 30 + d_diff
    }

    /// self が other と同日かそれ以降か判定する
    pub fn is_at_least(&self, other: &GameDate) -> bool {
        if self.year != other.year {
            return self.year > other.year;
        }
        if self.month != other.month {
            return self.month > other.month;
        }
        self.day >= other.day
    }
}

/// 速度設定（1〜4）
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameSpeed(pub u8);

impl Default for GameSpeed {
    fn default() -> Self {
        Self(1)
    }
}

impl GameSpeed {
    /// 1秒間に進む日数
    pub fn days_per_real_second(self) -> f64 {
        SPEED_DAYS_PER_REAL_SECOND[(self.0 as usize).saturating_sub(1).min(3)]
    }
}

/// 一時停止状態
#[derive(Resource, Debug, Default)]
pub struct GamePaused(pub bool);

// ─── プラグイン ──────────────────────────────────────────────────────────────

pub struct GameTimePlugin;

impl Plugin for GameTimePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GameDate::default())
            .insert_resource(GameSpeed::default())
            .insert_resource(GamePaused(true))
            .add_message::<DayChangedMessage>()
            .add_message::<MonthChangedMessage>();

        // SystemSet の実行順序を保証
        app.configure_sets(
            Update,
            (
                DailySimulationSet::TimeUpdate,
                DailySimulationSet::Economy,
                DailySimulationSet::Research,
                DailySimulationSet::Diplomacy,
                DailySimulationSet::CountryAi,
                DailySimulationSet::WarPreparation,
                DailySimulationSet::MilitaryAi,
                DailySimulationSet::FrontlineOrders,
                DailySimulationSet::MilitaryAction,
                DailySimulationSet::WarResolution,
                DailySimulationSet::UiUpdate,
            )
                .chain()
                .run_if(in_state(GameState::Playing)),
        );

        app.add_systems(
            Update,
            (toggle_pause_key, advance_game_date)
                .chain()
                .in_set(DailySimulationSet::TimeUpdate),
        );
    }
}

/// スペースキーで一時停止を切り替える
fn toggle_pause_key(keys: Res<ButtonInput<KeyCode>>, mut paused: ResMut<GamePaused>) {
    if keys.just_pressed(KeyCode::Space) {
        paused.0 = !paused.0;
    }
}

/// ゲーム内日付を進める（日次・月次の切り替わり時にMessageを発行）
fn advance_game_date(
    time: Res<Time>,
    speed: Res<GameSpeed>,
    paused: Res<GamePaused>,
    mut date: ResMut<GameDate>,
    mut day_events: MessageWriter<DayChangedMessage>,
    mut month_events: MessageWriter<MonthChangedMessage>,
) {
    if paused.0 {
        return;
    }

    let delta = time.delta_secs_f64() * speed.days_per_real_second();
    date.accumulator += delta;

    if date.accumulator >= 1.0 {
        let days = date.accumulator.floor() as u32;
        date.accumulator -= days as f64;

        for _ in 0..days {
            let old_month = date.month;
            date.day += 1;
            let max_day = DAYS_IN_MONTH[(date.month as usize) - 1];
            if date.day > max_day {
                date.day = 1;
                date.month += 1;
                if date.month > 12 {
                    date.month = 1;
                    date.year += 1;
                }
            }

            // 日次メッセージ発行
            day_events.write(DayChangedMessage {
                year: date.year,
                month: date.month,
                day: date.day,
            });

            // 月次メッセージ発行（月が変わった1回目の日）
            if date.month != old_month {
                month_events.write(MonthChangedMessage {
                    year: date.year,
                    month: date.month,
                });
            }
        }
    }
}
