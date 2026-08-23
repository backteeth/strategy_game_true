//! P21-018フォローアップ: プレイヤーが新しくCrisisの要求(`DemandSent`)の対象に
//! なった瞬間、ゲーム速度に関わらず必ず気づけるよう自動的に一時停止する。
//!
//! 背景: `CRISIS_DEMAND_PERIOD_DAYS`(30日、P21-011)は変更しないが、ゲーム速度
//! 4(30日/秒)では応答猶予がわずか約1秒で過ぎてしまい、プレイヤーが気づく前に
//! 期限切れ→拒否扱い→宣戦布告まで進んでしまうことがユーザー実プレイで確認された
//! (P21-018がAIによる対プレイヤーClaim/Crisisの現実的な発生源になったことで
//! 顕在化した、P21-011由来の既存UXギャップ)。
//!
//! 対象Crisisごとに一度だけ一時停止する(同じCrisisが`DemandSent`のまま
//! 何度もPlaying/Pauseを行き来しても再度停止しない)。プレイヤー自身が開始した
//! Crisis(target≠プレイヤー)では発火しない。

use crate::app::time::GamePaused;
use crate::common::DiplomaticCrisisId;
use crate::country::PlayerCountry;
use crate::diplomacy::crisis::{CrisisPhase, CrisisRegistry};
use bevy::prelude::*;
use std::collections::HashSet;

/// 既に一時停止トリガーの判定対象にした(=プレイヤーへ通知済みとみなす)Crisis ID。
/// Saveへは含めない(このセッション内だけの、UI通知用の一時状態)。
#[derive(Resource, Default, Debug)]
pub struct SeenPlayerTargetedCrisisIds(HashSet<DiplomaticCrisisId>);

/// System: `CrisisRegistry`を走査し、`current_phase == DemandSent`かつ
/// `target == プレイヤー国`のCrisisで、まだ一時停止トリガーの対象にしていない
/// ものが見つかれば`GamePaused`を`true`にする。1件見つかれば十分なので即座に
/// returnし、以降の走査はしない。
pub fn pause_on_new_crisis_targeting_player(
    crisis_registry: Res<CrisisRegistry>,
    player_country: Res<PlayerCountry>,
    mut seen: ResMut<SeenPlayerTargetedCrisisIds>,
    mut paused: ResMut<GamePaused>,
) {
    let Some(player) = player_country.0 else {
        return;
    };

    for crisis in crisis_registry.crises.values() {
        if !seen.0.insert(crisis.id) {
            continue;
        }
        if crisis.target == player && crisis.current_phase == CrisisPhase::DemandSent {
            paused.0 = true;
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::CountryId;
    use crate::diplomacy::crisis::DiplomaticCrisis;
    use std::collections::HashMap;

    fn crisis(id: usize, initiator: usize, target: usize, phase: CrisisPhase) -> DiplomaticCrisis {
        DiplomaticCrisis {
            id: DiplomaticCrisisId(id),
            initiator: CountryId(initiator),
            target: CountryId(target),
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            current_phase: phase,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1800/01/31".to_string()),
            international_concern: 0.0,
            third_party_reactions: HashMap::new(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        }
    }

    fn build_app(player: Option<CountryId>) -> App {
        let mut app = App::new();
        app.insert_resource(CrisisRegistry::default());
        app.insert_resource(PlayerCountry(player));
        app.insert_resource(SeenPlayerTargetedCrisisIds::default());
        app.insert_resource(GamePaused(false));
        app.add_systems(Update, pause_on_new_crisis_targeting_player);
        app
    }

    /// 要求テスト: プレイヤーが対象のDemandSent Crisisが新規に現れたら一時停止する。
    #[test]
    fn pauses_when_player_becomes_the_target_of_a_new_demand() {
        let mut app = build_app(Some(CountryId(0)));
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .add_crisis(crisis(0, 1, 0, CrisisPhase::DemandSent));
        app.update();
        assert!(app.world().resource::<GamePaused>().0);
    }

    /// プレイヤーが開始者(initiator)側のCrisisでは一時停止しない。
    #[test]
    fn does_not_pause_when_player_is_the_initiator() {
        let mut app = build_app(Some(CountryId(0)));
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .add_crisis(crisis(0, 0, 1, CrisisPhase::DemandSent));
        app.update();
        assert!(!app.world().resource::<GamePaused>().0);
    }

    /// プレイヤーが無関係な第三国のCrisisでは一時停止しない。
    #[test]
    fn does_not_pause_for_a_crisis_not_involving_the_player() {
        let mut app = build_app(Some(CountryId(0)));
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .add_crisis(crisis(0, 1, 2, CrisisPhase::DemandSent));
        app.update();
        assert!(!app.world().resource::<GamePaused>().0);
    }

    /// DemandSent以外のフェーズ(例: 既に解決済み)では一時停止しない。
    #[test]
    fn does_not_pause_for_non_demand_sent_phases() {
        let mut app = build_app(Some(CountryId(0)));
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .add_crisis(crisis(0, 1, 0, CrisisPhase::ResolvedPeacefully));
        app.update();
        assert!(!app.world().resource::<GamePaused>().0);
    }

    /// 同じCrisisが複数フレームにわたって存在しても、一時停止トリガーは一度だけ
    /// (プレイヤーが手動でPlayingへ戻した後、勝手に再度Pauseされない)。
    #[test]
    fn only_triggers_once_per_crisis_even_if_player_unpauses_again() {
        let mut app = build_app(Some(CountryId(0)));
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .add_crisis(crisis(0, 1, 0, CrisisPhase::DemandSent));
        app.update();
        assert!(app.world().resource::<GamePaused>().0);

        app.world_mut().resource_mut::<GamePaused>().0 = false;
        app.update();
        app.update();
        assert!(
            !app.world().resource::<GamePaused>().0,
            "must not re-pause once the player has already been alerted to this crisis"
        );
    }

    /// プレイヤー国家未設定(観戦/未選択状態)ではpanicしない。
    #[test]
    fn no_player_country_does_not_panic() {
        let mut app = build_app(None);
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .add_crisis(crisis(0, 1, 2, CrisisPhase::DemandSent));
        app.update();
        assert!(!app.world().resource::<GamePaused>().0);
    }

    /// 複数のCrisisが同一フレームに存在しても、プレイヤー対象のものが1件でもあれば
    /// 一時停止する(走査順に依存しない)。
    #[test]
    fn pauses_even_when_the_player_targeted_crisis_is_not_the_first_one_seen() {
        let mut app = build_app(Some(CountryId(0)));
        {
            let mut registry = app.world_mut().resource_mut::<CrisisRegistry>();
            registry.add_crisis(crisis(0, 2, 3, CrisisPhase::DemandSent));
            registry.add_crisis(crisis(1, 1, 0, CrisisPhase::DemandSent));
        }
        app.update();
        assert!(app.world().resource::<GamePaused>().0);
    }
}
