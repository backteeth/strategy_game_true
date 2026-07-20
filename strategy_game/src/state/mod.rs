/// state モジュール
/// 州データと選択状態を管理する
/// データ本体は DataLoaderPlugin が RON から読み込んで StateRegistry に注入する
pub mod data;

use crate::common::StateId;
use bevy::prelude::*;
use data::StateRegistry;

/// 現在選択中の州を保持するリソース（None = 未選択）
#[derive(Resource, Default, Debug)]
pub struct SelectedState(pub Option<StateId>);

/// 州の選択状態が変化したことを通知するメッセージ
/// Bevy 0.19: Event → Message
/// フィールドは将来の詳細情報用として保持（現在は SelectedState リソースを参照）
#[allow(dead_code)]
#[derive(Message, Debug)]
pub struct StateSelectionChanged(pub Option<StateId>);

/// 州管理プラグイン
pub struct StatePlugin;

impl Plugin for StatePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(StateRegistry::default())
            .insert_resource(SelectedState::default())
            .add_message::<StateSelectionChanged>();
    }
}
