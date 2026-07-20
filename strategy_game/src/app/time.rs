use crate::app::game_state::GameState;
/// ゲーム内時間管理モジュール
/// GameDate、一時停止、速度変更を管理する
/// 描画フレームとゲーム内時間処理を分離するため、アキュムレーター方式を使用する
use bevy::prelude::*;

// ─── 定数 ──────────────────────────────────────────────────────────────────

/// 各月の日数（うるう年は現段階では考慮しない、TODO: Phase 3でうるう年対応）
const DAYS_IN_MONTH: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// 速度ごとのゲーム内秒数/実時間秒（1秒あたりに何ゲーム日進むか）
/// Speed1=1日/秒, Speed2=3日/秒, Speed3=7日/秒, Speed4=30日/秒
const SPEED_DAYS_PER_REAL_SECOND: [f64; 4] = [1.0, 3.0, 7.0, 30.0];

// ─── データ型 ───────────────────────────────────────────────────────────────

/// ゲーム内日付
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GameDate {
    pub year: i32,
    /// 月（1〜12）
    pub month: u8,
    /// 日（1〜28/30/31）
    pub day: u8,
    /// 経過日数の小数点以下アキュムレーター（描画と分離するため）
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
    /// 日付を文字列として取得する
    pub fn display(&self) -> String {
        format!("{:04}/{:02}/{:02}", self.year, self.month, self.day)
    }

    /// n 日進める
    fn advance_days(&mut self, days: u32) {
        for _ in 0..days {
            self.day += 1;
            let max_day = DAYS_IN_MONTH[(self.month as usize) - 1];
            if self.day > max_day {
                self.day = 1;
                self.month += 1;
                if self.month > 12 {
                    self.month = 1;
                    self.year += 1;
                }
            }
        }
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
    /// 1日進めるのに必要な実時間（秒）
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
            .insert_resource(GamePaused(true)) // 開始時は停止
            .add_systems(
                Update,
                (advance_game_date, toggle_pause_key).run_if(in_state(GameState::Playing)),
            );
    }
}

/// スペースキーで一時停止を切り替える
fn toggle_pause_key(keys: Res<ButtonInput<KeyCode>>, mut paused: ResMut<GamePaused>) {
    if keys.just_pressed(KeyCode::Space) {
        paused.0 = !paused.0;
    }
}

/// ゲーム内日付を進める（描画フレームから独立したアキュムレーター方式）
fn advance_game_date(
    time: Res<Time>,
    speed: Res<GameSpeed>,
    paused: Res<GamePaused>,
    mut date: ResMut<GameDate>,
) {
    if paused.0 {
        return;
    }

    let delta = time.delta_secs_f64() * speed.days_per_real_second();
    date.accumulator += delta;

    // アキュムレーターが 1.0 以上になった分だけ日付を進める
    if date.accumulator >= 1.0 {
        let days = date.accumulator.floor() as u32;
        date.accumulator -= days as f64;
        date.advance_days(days);
    }
}
