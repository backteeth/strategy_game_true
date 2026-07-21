use bevy::prelude::*;

/// プレイヤーへのゲーム内通知メッセージ
#[derive(Message, Debug, Clone)]
pub struct GameNotification {
    pub message: String,
}

/// 直近の通知履歴リソース
#[derive(Resource, Default)]
pub struct NotificationHistory {
    pub recent: Vec<String>,
}

impl NotificationHistory {
    pub fn add(&mut self, msg: String) {
        // 同じ警告の重複抑制
        if self.recent.last() == Some(&msg) {
            return;
        }
        if self.recent.len() >= 5 {
            self.recent.remove(0);
        }
        self.recent.push(msg);
    }
}
