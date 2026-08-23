//! P22-002: 観戦中の世界統一検知と終戦通知の統合テスト。
//!
//! プレイヤー国家(CountryId(0))と対戦国(CountryId(1))のみのActive国家2か国世界で、
//! 「プレイヤー国家が消滅し、対戦国だけがActiveになる」という講和・州移転の結果
//! (`country::elimination`/`state::ownership`の出力そのもの)を`CountryRegistry`へ
//! 直接反映させ、以下の一連の表示順序を実際のBevy Systemスケジューリング・実際の
//! ボタン押下(`Interaction::Pressed`)で駆動して固定する:
//!
//! 1. プレイヤー国家と対戦国がActive
//! 2. プレイヤー国家が消滅(対戦国だけがActiveになる)
//! 3. 敗北オーバーレイが先に表示される(世界統一オーバーレイはまだ出ない)
//! 4. 敗北通知を閉じる(実ボタン押下)
//! 5. 世界統一オーバーレイが表示される(勝者国名・成立日を含む)
//! 6. 世界統一通知を閉じる(実ボタン押下)
//! 7. 再度表示されない(複数フレーム経過後も)
//!
//! 消滅そのものを引き起こす講和・州移転Domain APIの正しさ自体は
//! `tests/p21_019_crisis_war_peace_resolution_e2e_test.rs`・
//! `tests/p21_020_ownership_transfer_e2e_test.rs`・
//! `tests/p21_021_country_elimination_e2e_test.rs`で既に実データE2Eとして検証済み
//! のため、本テストは「消滅がどう起きたか」ではなく「消滅後のオーバーレイ表示順序」
//! に焦点を絞り、`CountryRegistry`の状態遷移という共通の出力点から駆動する。

use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::GameDate;
use strategy_game::common::CountryId;
use strategy_game::country::{
    CountryData, CountryRegistry, CountryStatus, PlayerCountry, PlayerEliminationMode,
    WorldConclusionMode, detect_player_elimination, detect_world_conclusion,
};
use strategy_game::localization::TranslationCorePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::ui::player_elimination::{
    ContinueSpectatingButton, PlayerEliminationOverlayRoot, PlayerEliminationPlugin,
};
use strategy_game::ui::world_conclusion::{
    ContinueSpectatingAfterConclusionButton, WorldConclusionOverlayRoot, WorldConclusionPlugin,
};

/// `country::CountryPlugin`全体(AI・国力再評価等、War/Military/Diplomacy Registryを
/// 要求する重量級System群を含む)を丸ごと組み込むと、この統合テストの主眼(消滅検知
/// →敗北オーバーレイ→世界統一検知→観戦者向けオーバーレイという表示順序)とは
/// 無関係な大量のセットアップが必要になる。本テストが実際に必要とするのは
/// `detect_player_elimination`/`detect_world_conclusion`という2つの検知System
/// そのものであり、これらは`country::mod`から直接`pub fn`として公開されている
/// ため、ここではその2つだけを実際のSystem実行順序(`.after()`)付きで直接登録する
/// (`country::elimination`/`state::ownership`が生成する消滅結果自体の正しさは
/// 冒頭のdocコメントに挙げた既存E2Eで検証済み)。UI側の2プラグイン
/// (`PlayerEliminationPlugin`/`WorldConclusionPlugin`)は本テストの直接の対象なので
/// そのまま実プラグインを使う。
fn setup_app(player: CountryId, rival: CountryId) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::state::app::StatesPlugin)
        .add_plugins(TranslationCorePlugin)
        .add_plugins(PlayerEliminationPlugin)
        .add_plugins(WorldConclusionPlugin);

    app.insert_resource(CountryRegistry {
        countries: vec![
            CountryData {
                id: player,
                name: "Arcadia".to_string(),
                ..CountryData::default()
            },
            CountryData {
                id: rival,
                name: "Passguard".to_string(),
                ..CountryData::default()
            },
        ],
    });
    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_resource(StateRegistry::default());
    app.insert_resource(PlayerEliminationMode::default());
    app.insert_resource(WorldConclusionMode::default());
    app.insert_resource(GameDate::new(1850, 6, 15));
    app.add_systems(
        Update,
        (
            detect_player_elimination,
            detect_world_conclusion.after(detect_player_elimination),
        )
            .run_if(in_state(GameState::Playing)),
    );
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn overlay_display(app: &mut App, root: fn(&mut World) -> Display) -> Display {
    root(app.world_mut())
}

fn defeat_overlay_display(world: &mut World) -> Display {
    let mut q = world.query::<(&PlayerEliminationOverlayRoot, &Node)>();
    q.single(world).unwrap().1.display
}

fn conclusion_overlay_display(world: &mut World) -> Display {
    let mut q = world.query::<(&WorldConclusionOverlayRoot, &Node)>();
    q.single(world).unwrap().1.display
}

fn press_defeat_overlay_continue_button(app: &mut App) {
    app.world_mut().spawn((
        ContinueSpectatingButton,
        Interaction::Pressed,
        BackgroundColor(Color::WHITE),
    ));
    app.update();
}

fn press_world_conclusion_continue_button(app: &mut App) {
    app.world_mut().spawn((
        ContinueSpectatingAfterConclusionButton,
        Interaction::Pressed,
        BackgroundColor(Color::WHITE),
    ));
    app.update();
}

#[test]
fn defeat_overlay_shows_before_world_conclusion_overlay_and_neither_reappears_after_dismissal() {
    let player = CountryId(0);
    let rival = CountryId(1);
    let mut app = setup_app(player, rival);

    // 1. プレイヤー国家と対戦国がActive: まだどちらのオーバーレイも出ない。
    assert_eq!(
        overlay_display(&mut app, defeat_overlay_display),
        Display::None
    );
    assert_eq!(
        overlay_display(&mut app, conclusion_overlay_display),
        Display::None
    );
    assert_eq!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::NotConcluded
    );

    // 2. 講和・州移転によってプレイヤー国家が消滅(対戦国だけがActiveになる)。
    //    `country::elimination::apply_country_elimination`が実際に生成する結果
    //    (`CountryStatus::Eliminated`)を直接反映する(§冒頭のdocコメント参照)。
    {
        let mut registry = app.world_mut().resource_mut::<CountryRegistry>();
        let country = registry.get_mut(player).unwrap();
        country.status = CountryStatus::Eliminated {
            eliminated_date: "1850/06/15".to_string(),
            annexed_by: rival,
        };
    }
    app.update();

    // 3. 敗北オーバーレイが先に表示される。世界統一のDomain状態は既にPendingに
    //    進んでいるはずだが、UI上はまだ出ない。
    assert_eq!(
        *app.world().resource::<PlayerEliminationMode>(),
        PlayerEliminationMode::DefeatOverlayPending
    );
    assert!(matches!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::OverlayPending { .. }
    ));
    assert_eq!(
        overlay_display(&mut app, defeat_overlay_display),
        Display::Flex,
        "the defeat overlay must show first"
    );
    assert_eq!(
        overlay_display(&mut app, conclusion_overlay_display),
        Display::None,
        "the world-conclusion overlay must not show before the defeat overlay is dismissed, \
         even though its domain state is already pending"
    );

    // 4. 敗北通知を閉じる(実ボタン押下)。
    press_defeat_overlay_continue_button(&mut app);
    assert_eq!(
        *app.world().resource::<PlayerEliminationMode>(),
        PlayerEliminationMode::Spectating
    );

    // 5. 世界統一オーバーレイが表示される(勝者国名・成立日を含む)。
    assert_eq!(
        overlay_display(&mut app, defeat_overlay_display),
        Display::None,
        "the defeat overlay must close once dismissed"
    );
    assert_eq!(
        overlay_display(&mut app, conclusion_overlay_display),
        Display::Flex,
        "the world-conclusion overlay must show once the defeat overlay has been dismissed"
    );

    // 世界統一成立日は、敗北オーバーレイを閉じた日ではなく、実際にActive国家が
    // 1か国になった日(この統合テストでは同日: "1850/06/15")のまま。
    assert_eq!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::OverlayPending {
            winner_country_id: rival,
            conclusion_date: "1850/06/15".to_string(),
        }
    );

    // 6. 世界統一通知を閉じる(実ボタン押下)。
    press_world_conclusion_continue_button(&mut app);
    assert_eq!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::Acknowledged
    );
    assert_eq!(
        overlay_display(&mut app, conclusion_overlay_display),
        Display::None,
        "the world-conclusion overlay must close once dismissed"
    );

    // 7. 再度表示されない(複数フレーム経過後も)。
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        overlay_display(&mut app, defeat_overlay_display),
        Display::None
    );
    assert_eq!(
        overlay_display(&mut app, conclusion_overlay_display),
        Display::None,
        "the world-conclusion overlay must never reappear after being acknowledged"
    );
    assert_eq!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::Acknowledged
    );
}
