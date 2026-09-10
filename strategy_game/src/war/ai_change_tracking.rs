//! P26-008-PRE-002: AI再評価のイベント駆動dirty化。
//!
//! `mark_ai_registries_dirty_daily`(旧実装)は`DayChangedMessage`のたびに
//! `CountryAiRegistry`/`MilitaryAiRegistry`の全国家を無条件で`mark_all_dirty()`していた。
//! これにより、週次(研究AI・戦争準備AI)・月次(建設AI)・3日おき(戦線計画AI)という
//! 設計意図のcadenceが、`is_weekly_due`/`is_monthly_due`/`is_evaluation_due`の
//! `|| dirty`分岐によって実質的に常に「今日評価してよい」へ上書きされ、毎日評価される
//! 状態になっていた(`verification_logs/phase-26/p26-008-pre-001/`で実測確認済み)。
//!
//! 本モジュールは、`StateRegistry`(州のcontroller/owner)と`WarRegistry`(戦争の
//! status/参加国)という、AI判断が実際に参照する外部状態のうち「定期評価を待たず
//! 即時再評価すべき」種類の変化(spec §7・§8: 戦争開始・終了・州支配変更)だけを
//! 日次で比較し、**実際に関係する国家だけ**を`mark_country_dirty`する。
//!
//! `AiChangeSnapshot`はSaveへは一切保存しない(P26-006の首都連結性と同じ「再計算専用」
//! 方針)。New Game開始時・Load直後のいずれも`AiChangeSnapshot::default()`
//! (未初期化=空)から始まり、`diff_affected_countries`は未初期化な比較元に対しては
//! 空集合を返す(「まだ一度も観測していない」ことと「何もかも変化した」ことを区別する
//! ブートストラップ規則、§13)。これにより、New Game 1日目やLoad直後の1日目に
//! 州28件・戦争0件全てが「変化した」と誤認されて無関係な全国家がdirty化される
//! ことを避ける――実際の即時再評価の要否は、この後段では`CountryAiState.dirty`/
//! `MilitaryAiState.dirty`自体のSave往復(旧Saveのみdefault=true)が担う。

use crate::common::{CountryId, StateId, WarId};
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

/// 1州分の、AI判断にとって意味のある支配状態(実効支配者と法的所有者)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateControlSnapshot {
    controller: CountryId,
    owner: CountryId,
}

/// 1戦争分の、AI判断にとって意味のある状態(status・参加国集合)。
#[derive(Debug, Clone, PartialEq, Eq)]
struct WarParticipationSnapshot {
    status: WarStatus,
    /// `CountryId`昇順・重複なし(攻撃側・防御側を区別しない――区別してもdirty化する
    /// 対象集合は変わらないため、参加国かどうかだけを見れば十分)。
    participants: Vec<CountryId>,
}

/// 日次のAI再評価トリガー判定に使う、州支配・戦争参加状態のスナップショット
/// (Saveへは保存しない、`capture`で毎回再計算する)。
///
/// `CountryId`/`StateId`/`WarId`は`Ord`を実装しない(このコードベース全体の既存規約
/// ――決定的な順序が必要な箇所は`.sort_by_key(|id| id.0)`で都度明示する)ため、
/// 格納には`HashMap`/`HashSet`を使い、外部へ返す`Vec<CountryId>`だけを
/// `diff_affected_countries`の最後で明示的にソート・重複排除する。
#[derive(Resource, Debug, Clone, Default)]
pub struct AiChangeSnapshot {
    state_control: HashMap<StateId, StateControlSnapshot>,
    war_participation: HashMap<WarId, WarParticipationSnapshot>,
}

impl AiChangeSnapshot {
    /// 現在の`StateRegistry`/`WarRegistry`からスナップショットを構築する
    /// (読み取り専用、いずれのRegistryも変更しない)。
    pub fn capture(state_registry: &StateRegistry, war_registry: &WarRegistry) -> Self {
        let state_control = state_registry
            .states
            .iter()
            .map(|s| {
                (
                    s.id,
                    StateControlSnapshot {
                        controller: s.controller(),
                        owner: s.owner_country_id,
                    },
                )
            })
            .collect();

        let war_participation = war_registry
            .wars
            .values()
            .map(|w| {
                let mut participants: Vec<CountryId> =
                    w.attackers.iter().chain(w.defenders.iter()).copied().collect();
                participants.sort_by_key(|c| c.0);
                participants.dedup();
                (
                    w.id,
                    WarParticipationSnapshot {
                        status: w.status,
                        participants,
                    },
                )
            })
            .collect();

        Self { state_control, war_participation }
    }

    /// まだ一度も`capture`結果を保持していない(New Game直後・Load直後)ことを表す。
    /// `StateRegistry`は常に28州の固定データを持ち、途中で空になることはないため、
    /// `state_control`が空であることをブートストラップの安全な判定条件として使える。
    fn is_uninitialized(&self) -> bool {
        self.state_control.is_empty()
    }

    /// `self`(前日までのスナップショット)と`current`(今日の実測)を比較し、
    /// 州支配(controller/owner)または戦争(status/参加国)が変化した国家の集合を
    /// `CountryId`昇順・重複なしで返す。
    ///
    /// `self`が未初期化(§ドキュメント参照)の場合は、比較対象が存在しないため
    /// 常に空集合を返す――「何も変化していない」のではなく「まだ観測していない」
    /// ことを表し、無関係な全国家への誤ったdirty化を防ぐ。
    pub fn diff_affected_countries(&self, current: &Self) -> Vec<CountryId> {
        let mut affected: HashSet<CountryId> = HashSet::new();
        if self.is_uninitialized() {
            return Vec::new();
        }

        let mut all_state_ids: HashSet<StateId> = self.state_control.keys().copied().collect();
        all_state_ids.extend(current.state_control.keys().copied());
        for state_id in all_state_ids {
            let before = self.state_control.get(&state_id);
            let after = current.state_control.get(&state_id);
            if before == after {
                continue;
            }
            if let Some(b) = before {
                affected.insert(b.controller);
                affected.insert(b.owner);
            }
            if let Some(a) = after {
                affected.insert(a.controller);
                affected.insert(a.owner);
            }
        }

        let mut all_war_ids: HashSet<WarId> = self.war_participation.keys().copied().collect();
        all_war_ids.extend(current.war_participation.keys().copied());
        for war_id in all_war_ids {
            let before = self.war_participation.get(&war_id);
            let after = current.war_participation.get(&war_id);
            if before == after {
                continue;
            }
            if let Some(b) = before {
                affected.extend(b.participants.iter().copied());
            }
            if let Some(a) = after {
                affected.extend(a.participants.iter().copied());
            }
        }

        let mut result: Vec<CountryId> = affected.into_iter().collect();
        result.sort_by_key(|c| c.0);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::data::StateData;
    use crate::war::data::War;
    use std::collections::HashSet as StdHashSet;

    fn state(id: usize, owner: usize, controller: Option<usize>) -> StateData {
        StateData {
            id: StateId(id),
            owner_country_id: CountryId(owner),
            controller_country: controller.map(CountryId),
            ..StateData::default()
        }
    }

    fn war(id: usize, attacker: usize, defender: usize, status: WarStatus) -> War {
        War {
            id: WarId(id),
            name: "Test War".to_string(),
            attackers: [CountryId(attacker)].into_iter().collect(),
            defenders: [CountryId(defender)].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
            source_crisis_id: None,
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: StdHashSet::new(),
            status,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: StdHashSet::new(),
        }
    }

    // ── ブートストラップ ────────────────────────────────────────────────

    #[test]
    fn uninitialized_snapshot_reports_no_affected_countries_even_against_a_populated_world() {
        let states = StateRegistry::build(vec![state(0, 1, None), state(1, 2, None)]);
        let mut wars = WarRegistry::default();
        wars.add_war(war(0, 1, 2, WarStatus::Active));

        let empty = AiChangeSnapshot::default();
        let current = AiChangeSnapshot::capture(&states, &wars);

        assert_eq!(empty.diff_affected_countries(&current), Vec::<CountryId>::new());
    }

    #[test]
    fn identical_captures_report_no_affected_countries() {
        let states = StateRegistry::build(vec![state(0, 1, None), state(1, 2, Some(1))]);
        let mut wars = WarRegistry::default();
        wars.add_war(war(0, 1, 2, WarStatus::Active));

        let before = AiChangeSnapshot::capture(&states, &wars);
        let after = AiChangeSnapshot::capture(&states, &wars);

        assert_eq!(before.diff_affected_countries(&after), Vec::<CountryId>::new());
    }

    // ── 州支配変化 ──────────────────────────────────────────────────────

    #[test]
    fn controller_change_dirties_old_and_new_controller_and_owner() {
        let mut states = StateRegistry::build(vec![state(0, 1, None)]);
        let wars = WarRegistry::default();
        let before = AiChangeSnapshot::capture(&states, &wars);

        states.get_mut(StateId(0)).unwrap().controller_country = Some(CountryId(2));
        let after = AiChangeSnapshot::capture(&states, &wars);

        assert_eq!(
            before.diff_affected_countries(&after),
            vec![CountryId(1), CountryId(2)],
            "owner(1)と新controller(2)の両方がdirty化されるはず"
        );
    }

    #[test]
    fn recapturing_the_same_controller_is_a_no_op() {
        let mut states = StateRegistry::build(vec![state(0, 1, Some(2))]);
        let wars = WarRegistry::default();
        let before = AiChangeSnapshot::capture(&states, &wars);

        // 同じCountryIdへ再設定(意味的変化なし)。
        states.get_mut(StateId(0)).unwrap().controller_country = Some(CountryId(2));
        let after = AiChangeSnapshot::capture(&states, &wars);

        assert_eq!(before.diff_affected_countries(&after), Vec::<CountryId>::new());
    }

    #[test]
    fn owner_change_dirties_old_and_new_owner() {
        let mut states = StateRegistry::build(vec![state(0, 1, None)]);
        let wars = WarRegistry::default();
        let before = AiChangeSnapshot::capture(&states, &wars);

        states.get_mut(StateId(0)).unwrap().owner_country_id = CountryId(3);
        let after = AiChangeSnapshot::capture(&states, &wars);

        assert_eq!(before.diff_affected_countries(&after), vec![CountryId(1), CountryId(3)]);
    }

    #[test]
    fn unrelated_third_country_is_not_dirtied_by_a_controller_change_elsewhere() {
        let mut states =
            StateRegistry::build(vec![state(0, 1, None), state(1, 9, None) /* 無関係 */]);
        let wars = WarRegistry::default();
        let before = AiChangeSnapshot::capture(&states, &wars);

        states.get_mut(StateId(0)).unwrap().controller_country = Some(CountryId(2));
        let after = AiChangeSnapshot::capture(&states, &wars);

        let affected = before.diff_affected_countries(&after);
        assert!(!affected.contains(&CountryId(9)), "無関係な第三国はdirty化されないはず");
    }

    // ── 戦争開始・終了・参加国変化 ────────────────────────────────────────

    #[test]
    fn war_starting_dirties_all_participants() {
        let states = StateRegistry::build(vec![state(0, 1, None), state(1, 2, None)]);
        let mut wars = WarRegistry::default();
        let before = AiChangeSnapshot::capture(&states, &wars);

        wars.add_war(war(0, 1, 2, WarStatus::Active));
        let after = AiChangeSnapshot::capture(&states, &wars);

        assert_eq!(before.diff_affected_countries(&after), vec![CountryId(1), CountryId(2)]);
    }

    #[test]
    fn war_status_change_dirties_all_participants() {
        let states = StateRegistry::build(vec![state(0, 1, None), state(1, 2, None)]);
        let mut wars = WarRegistry::default();
        wars.add_war(war(0, 1, 2, WarStatus::Active));
        let before = AiChangeSnapshot::capture(&states, &wars);

        wars.wars.get_mut(&WarId(0)).unwrap().status = WarStatus::AttackerVictory;
        let after = AiChangeSnapshot::capture(&states, &wars);

        assert_eq!(before.diff_affected_countries(&after), vec![CountryId(1), CountryId(2)]);
    }

    #[test]
    fn reporting_the_same_active_war_again_is_a_no_op() {
        let states = StateRegistry::build(vec![state(0, 1, None), state(1, 2, None)]);
        let mut wars = WarRegistry::default();
        wars.add_war(war(0, 1, 2, WarStatus::Active));
        let before = AiChangeSnapshot::capture(&states, &wars);
        let after = AiChangeSnapshot::capture(&states, &wars);

        assert_eq!(before.diff_affected_countries(&after), Vec::<CountryId>::new());
    }

    #[test]
    fn unrelated_third_country_is_not_dirtied_by_a_war_between_two_others() {
        let states = StateRegistry::build(vec![
            state(0, 1, None),
            state(1, 2, None),
            state(2, 9, None), // 無関係
        ]);
        let mut wars = WarRegistry::default();
        let before = AiChangeSnapshot::capture(&states, &wars);

        wars.add_war(war(0, 1, 2, WarStatus::Active));
        let after = AiChangeSnapshot::capture(&states, &wars);

        let affected = before.diff_affected_countries(&after);
        assert!(!affected.contains(&CountryId(9)));
    }

    // ── 複数変化・順序非依存 ────────────────────────────────────────────

    #[test]
    fn multiple_simultaneous_changes_produce_a_deduplicated_sorted_union() {
        let mut states =
            StateRegistry::build(vec![state(0, 1, None), state(1, 2, None), state(2, 3, None)]);
        let mut wars = WarRegistry::default();
        wars.add_war(war(0, 4, 5, WarStatus::Active));
        let before = AiChangeSnapshot::capture(&states, &wars);

        // 州0はcountry(6)が占領、州1はowner変更、戦争0は終結。
        states.get_mut(StateId(0)).unwrap().controller_country = Some(CountryId(6));
        states.get_mut(StateId(1)).unwrap().owner_country_id = CountryId(7);
        wars.wars.get_mut(&WarId(0)).unwrap().status = WarStatus::WhitePeace;
        let after = AiChangeSnapshot::capture(&states, &wars);

        assert_eq!(
            before.diff_affected_countries(&after),
            vec![
                CountryId(1),
                CountryId(2),
                CountryId(4),
                CountryId(5),
                CountryId(6),
                CountryId(7),
            ]
        );
    }

    #[test]
    fn affected_country_set_does_not_depend_on_hashmap_insertion_order() {
        let mut states_a =
            StateRegistry::build(vec![state(0, 1, None), state(1, 2, None), state(2, 3, None)]);
        // `add_war`はIDを挿入順で自動採番するため、挿入順そのものを入れ替える
        // このテストでは使わず、`WarRegistry.wars`(`pub`)へ直接挿入して意図した
        // `WarId`を固定する(`war::update`のテストと同じ既存パターン)。
        let mut wars_a = WarRegistry::default();
        wars_a.wars.insert(WarId(0), war(0, 4, 5, WarStatus::Active));
        wars_a.wars.insert(WarId(1), war(1, 6, 7, WarStatus::Active));
        let before_a = AiChangeSnapshot::capture(&states_a, &wars_a);
        states_a.get_mut(StateId(0)).unwrap().controller_country = Some(CountryId(8));
        wars_a.wars.get_mut(&WarId(1)).unwrap().status = WarStatus::WhitePeace;
        let after_a = AiChangeSnapshot::capture(&states_a, &wars_a);

        // 逆順で州・戦争を構築しても、集合としては同じ変化を表す。
        let mut states_b =
            StateRegistry::build(vec![state(2, 3, None), state(1, 2, None), state(0, 1, None)]);
        let mut wars_b = WarRegistry::default();
        wars_b.wars.insert(WarId(1), war(1, 6, 7, WarStatus::Active));
        wars_b.wars.insert(WarId(0), war(0, 4, 5, WarStatus::Active));
        let before_b = AiChangeSnapshot::capture(&states_b, &wars_b);
        states_b.get_mut(StateId(0)).unwrap().controller_country = Some(CountryId(8));
        wars_b.wars.get_mut(&WarId(1)).unwrap().status = WarStatus::WhitePeace;
        let after_b = AiChangeSnapshot::capture(&states_b, &wars_b);

        assert_eq!(
            before_a.diff_affected_countries(&after_a),
            before_b.diff_affected_countries(&after_b),
        );
    }
}
