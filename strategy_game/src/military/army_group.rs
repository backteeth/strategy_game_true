/// P21-004: 複数師団の永続的な集合(いわゆる「軍」)を管理するモジュール。
/// 既存の`ArmyId`(1師団)と紛らわしいため、この集合は「ArmyGroup」と呼ぶ
/// (`common::ArmyGroupId`参照)。
///
/// `war::frontline::FrontlineRegistry`と同じ設計方針を踏襲する:
/// - Bevy Entityではなく、Resource内のプレーンなデータとして管理する
///   (このコードベースの全シミュレーション状態と同じ慣習)
/// - 「1師団は同時に1個の編成のみ所属」を、陸軍→編成の逆引きマップで一元的に保証する
/// - 撃破・消滅した師団の参照は`sanitize_references`で日次に整理する
///   (`FrontlineRegistry::sanitize_references`と対になる)
use crate::common::{ArmyGroupId, ArmyId, CountryId};
use crate::military::data::{ArmyStatus, MilitaryRegistry};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 編成(軍)データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmyGroup {
    pub id: ArmyGroupId,
    pub owner: CountryId,
    pub name: String,
    /// 所属師団(ArmyId昇順で安定保持)
    pub member_army_ids: Vec<ArmyId>,
}

/// 全編成を集中管理するリソース
#[derive(Resource, Default, Debug, Clone, Serialize, Deserialize)]
pub struct ArmyGroupRegistry {
    pub groups: HashMap<ArmyGroupId, ArmyGroup>,
    /// ArmyId -> ArmyGroupId のマッピング（1師団は1編成のみ所属）
    pub army_group_map: HashMap<ArmyId, ArmyGroupId>,
    next_id: usize,
    /// 国家ごとの自動採番カウンタ("Army 1", "Army 2", ...)
    next_group_number: HashMap<CountryId, u32>,
}

fn army_is_usable(military_registry: &MilitaryRegistry, army_id: ArmyId, owner: CountryId) -> bool {
    military_registry
        .armies
        .get(&army_id)
        .map(|a| a.owner == owner && a.manpower > 0 && a.status != ArmyStatus::Destroyed)
        .unwrap_or(false)
}

impl ArmyGroupRegistry {
    pub fn next_id(&self) -> usize {
        self.next_id
    }

    /// 陸軍を現在所属している編成(あれば)から取り除く(内部ヘルパー)。
    fn detach_army(&mut self, army_id: ArmyId) {
        if let Some(old_group_id) = self.army_group_map.remove(&army_id)
            && let Some(old_group) = self.groups.get_mut(&old_group_id)
        {
            old_group.member_army_ids.retain(|&id| id != army_id);
        }
    }

    /// 選択中の陸軍から新しい編成を作る。所有者不一致・撃破済み・存在しない陸軍は
    /// 黙って除外する(選択自体が所有者不問のため`map::army_selection`側から
    /// 敵国陸軍が混ざって渡され得る)。有効な陸軍が1件もなければNoneを返す。
    pub fn create_group(
        &mut self,
        owner: CountryId,
        member_army_ids: &[ArmyId],
        military_registry: &MilitaryRegistry,
    ) -> Option<ArmyGroupId> {
        let mut valid_ids: Vec<ArmyId> = member_army_ids
            .iter()
            .copied()
            .filter(|&id| army_is_usable(military_registry, id, owner))
            .collect();

        if valid_ids.is_empty() {
            return None;
        }
        valid_ids.sort_by_key(|id| id.0);

        let group_id = ArmyGroupId(self.next_id);
        self.next_id += 1;

        let number = self.next_group_number.entry(owner).or_insert(1);
        let name = format!("Army {number}");
        *number += 1;

        for &army_id in &valid_ids {
            self.detach_army(army_id);
        }
        for &army_id in &valid_ids {
            self.army_group_map.insert(army_id, group_id);
        }

        self.groups.insert(
            group_id,
            ArmyGroup {
                id: group_id,
                owner,
                name,
                member_army_ids: valid_ids,
            },
        );

        Some(group_id)
    }

    /// 陸軍を既存の編成へ追加する(既に別編成にいた場合はそちらから外れる)。
    pub fn add_army(
        &mut self,
        group_id: ArmyGroupId,
        army_id: ArmyId,
        owner: CountryId,
        military_registry: &MilitaryRegistry,
    ) -> Result<(), &'static str> {
        let group = self.groups.get(&group_id).ok_or("Army group not found")?;
        if group.owner != owner {
            return Err("Army group belongs to a different country");
        }
        if !army_is_usable(military_registry, army_id, owner) {
            return Err("Army not found, foreign-owned, or destroyed");
        }

        self.detach_army(army_id);

        let group = self
            .groups
            .get_mut(&group_id)
            .expect("group existence already checked above");
        if !group.member_army_ids.contains(&army_id) {
            group.member_army_ids.push(army_id);
            group.member_army_ids.sort_by_key(|id| id.0);
        }
        self.army_group_map.insert(army_id, group_id);
        Ok(())
    }

    /// 陸軍を所属編成から除外する(未所属に戻す)。所有者検証込み。
    pub fn remove_army(
        &mut self,
        army_id: ArmyId,
        owner: CountryId,
        military_registry: &MilitaryRegistry,
    ) -> Result<(), &'static str> {
        let army = military_registry
            .armies
            .get(&army_id)
            .ok_or("Army not found")?;
        if army.owner != owner {
            return Err("Army belongs to a different country");
        }
        self.detach_army(army_id);
        Ok(())
    }

    /// 編成を解散する。所属していた陸軍は全員未所属へ戻る。
    pub fn disband(&mut self, group_id: ArmyGroupId, owner: CountryId) -> Result<(), &'static str> {
        let group = self.groups.get(&group_id).ok_or("Army group not found")?;
        if group.owner != owner {
            return Err("Army group belongs to a different country");
        }
        if let Some(group) = self.groups.remove(&group_id) {
            for army_id in group.member_army_ids {
                self.army_group_map.remove(&army_id);
            }
        }
        Ok(())
    }

    /// 陸軍が所属する編成のIDを取得
    pub fn group_for_army(&self, army_id: ArmyId) -> Option<ArmyGroupId> {
        self.army_group_map.get(&army_id).copied()
    }

    /// 選択中陸軍(複数可)のうち、いずれかの編成に所属しているものがあれば、
    /// ArmyId昇順で最初に見つかったものが属する編成を「操作対象の編成」として返す。
    /// UI(追加/除外/軍を選択/解散ボタン)と実行ハンドラの両方がこれを使い、
    /// 「対象編成をどう決めるか」の判断を1箇所に集約する。
    pub fn target_group_for_selection(&self, selected_army_ids: &[ArmyId]) -> Option<ArmyGroupId> {
        let mut ids: Vec<ArmyId> = selected_army_ids.to_vec();
        ids.sort_by_key(|id| id.0);
        ids.iter().find_map(|id| self.group_for_army(*id))
    }

    /// 消滅・撃破済み陸軍の参照を全編成から整理する
    /// (`war::frontline::FrontlineRegistry::sanitize_references`と対になる日次処理)。
    pub fn sanitize_references(&mut self, military_registry: &MilitaryRegistry) {
        for group in self.groups.values_mut() {
            let owner = group.owner;
            group
                .member_army_ids
                .retain(|&army_id| army_is_usable(military_registry, army_id, owner));
        }

        let groups = &self.groups;
        self.army_group_map.retain(|army_id, group_id| {
            groups
                .get(group_id)
                .map(|g| army_is_usable(military_registry, *army_id, g.owner))
                .unwrap_or(false)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::DivisionId;
    use crate::military::data::{ArmyUnit, DivisionSize, DivisionType};

    fn make_army(id: usize, owner: CountryId) -> ArmyUnit {
        ArmyUnit {
            id: ArmyId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: crate::common::StateId(0),
            destination: None,
            current_path: vec![],
            target_state: None,
            manpower: 10_000,
            max_manpower: 10_000,
            equipment: 100.0,
            max_equipment: 100.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 100.0,
            max_morale: 100.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: ArmyStatus::Idle,
            def_id: DivisionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn registry_with_armies(armies: Vec<ArmyUnit>) -> MilitaryRegistry {
        let mut reg = MilitaryRegistry::default();
        for army in armies {
            reg.armies.insert(army.id, army);
        }
        reg
    }

    #[test]
    fn create_group_from_selection_ignores_foreign_and_destroyed_armies() {
        let owner = CountryId(1);
        let mut destroyed = make_army(3, owner);
        destroyed.manpower = 0;
        let military_registry = registry_with_armies(vec![
            make_army(1, owner),
            make_army(2, CountryId(2)), // 他国
            destroyed,
        ]);

        let mut groups = ArmyGroupRegistry::default();
        let group_id = groups
            .create_group(
                owner,
                &[ArmyId(1), ArmyId(2), ArmyId(3)],
                &military_registry,
            )
            .expect("at least one valid army should form a group");

        let group = groups.groups.get(&group_id).unwrap();
        assert_eq!(group.member_army_ids, vec![ArmyId(1)]);
        assert_eq!(group.owner, owner);
        assert_eq!(groups.group_for_army(ArmyId(1)), Some(group_id));
        assert_eq!(groups.group_for_army(ArmyId(2)), None);
    }

    #[test]
    fn create_group_with_no_valid_armies_returns_none() {
        let owner = CountryId(1);
        let military_registry = registry_with_armies(vec![make_army(1, CountryId(2))]);
        let mut groups = ArmyGroupRegistry::default();
        assert!(
            groups
                .create_group(owner, &[ArmyId(1)], &military_registry)
                .is_none()
        );
        assert!(groups.groups.is_empty());
    }

    #[test]
    fn add_army_moves_it_from_previous_group() {
        let owner = CountryId(1);
        let military_registry =
            registry_with_armies(vec![make_army(1, owner), make_army(2, owner)]);
        let mut groups = ArmyGroupRegistry::default();
        let group_a = groups
            .create_group(owner, &[ArmyId(1)], &military_registry)
            .unwrap();
        let group_b = groups
            .create_group(owner, &[ArmyId(2)], &military_registry)
            .unwrap();

        groups
            .add_army(group_b, ArmyId(1), owner, &military_registry)
            .unwrap();

        assert!(!groups.groups[&group_a].member_army_ids.contains(&ArmyId(1)));
        assert_eq!(
            groups.groups[&group_b].member_army_ids,
            vec![ArmyId(1), ArmyId(2)]
        );
        assert_eq!(groups.group_for_army(ArmyId(1)), Some(group_b));
    }

    #[test]
    fn add_army_rejects_foreign_army() {
        let owner = CountryId(1);
        let military_registry =
            registry_with_armies(vec![make_army(1, owner), make_army(2, CountryId(2))]);
        let mut groups = ArmyGroupRegistry::default();
        let group_id = groups
            .create_group(owner, &[ArmyId(1)], &military_registry)
            .unwrap();

        let result = groups.add_army(group_id, ArmyId(2), owner, &military_registry);
        assert!(result.is_err());
        assert_eq!(groups.group_for_army(ArmyId(2)), None);
    }

    #[test]
    fn remove_army_returns_it_to_unassigned() {
        let owner = CountryId(1);
        let military_registry =
            registry_with_armies(vec![make_army(1, owner), make_army(2, owner)]);
        let mut groups = ArmyGroupRegistry::default();
        let group_id = groups
            .create_group(owner, &[ArmyId(1), ArmyId(2)], &military_registry)
            .unwrap();

        groups
            .remove_army(ArmyId(1), owner, &military_registry)
            .unwrap();

        assert_eq!(groups.group_for_army(ArmyId(1)), None);
        assert_eq!(groups.groups[&group_id].member_army_ids, vec![ArmyId(2)]);
    }

    #[test]
    fn remove_army_rejects_non_owner() {
        let owner = CountryId(1);
        let military_registry = registry_with_armies(vec![make_army(1, owner)]);
        let mut groups = ArmyGroupRegistry::default();
        let group_id = groups
            .create_group(owner, &[ArmyId(1)], &military_registry)
            .unwrap();

        let result = groups.remove_army(ArmyId(1), CountryId(2), &military_registry);
        assert!(result.is_err());
        assert_eq!(groups.group_for_army(ArmyId(1)), Some(group_id));
    }

    #[test]
    fn disband_returns_all_members_to_unassigned_and_removes_group() {
        let owner = CountryId(1);
        let military_registry =
            registry_with_armies(vec![make_army(1, owner), make_army(2, owner)]);
        let mut groups = ArmyGroupRegistry::default();
        let group_id = groups
            .create_group(owner, &[ArmyId(1), ArmyId(2)], &military_registry)
            .unwrap();

        groups.disband(group_id, owner).unwrap();

        assert!(!groups.groups.contains_key(&group_id));
        assert_eq!(groups.group_for_army(ArmyId(1)), None);
        assert_eq!(groups.group_for_army(ArmyId(2)), None);
    }

    #[test]
    fn disband_rejects_non_owner_and_keeps_group_intact() {
        let owner = CountryId(1);
        let military_registry = registry_with_armies(vec![make_army(1, owner)]);
        let mut groups = ArmyGroupRegistry::default();
        let group_id = groups
            .create_group(owner, &[ArmyId(1)], &military_registry)
            .unwrap();

        let result = groups.disband(group_id, CountryId(2));
        assert!(result.is_err());
        assert!(groups.groups.contains_key(&group_id));
    }

    #[test]
    fn sanitize_references_removes_destroyed_armies_from_group_and_map() {
        let owner = CountryId(1);
        let mut military_registry =
            registry_with_armies(vec![make_army(1, owner), make_army(2, owner)]);
        let mut groups = ArmyGroupRegistry::default();
        let group_id = groups
            .create_group(owner, &[ArmyId(1), ArmyId(2)], &military_registry)
            .unwrap();

        // 師団1が戦闘で撃破された想定(MilitaryRegistryから削除される)
        military_registry.remove_army(ArmyId(1));

        groups.sanitize_references(&military_registry);

        assert_eq!(groups.groups[&group_id].member_army_ids, vec![ArmyId(2)]);
        assert_eq!(groups.group_for_army(ArmyId(1)), None);
        assert_eq!(groups.group_for_army(ArmyId(2)), Some(group_id));
    }

    #[test]
    fn target_group_for_selection_picks_lowest_army_id_with_a_group() {
        let owner = CountryId(1);
        let military_registry = registry_with_armies(vec![
            make_army(1, owner),
            make_army(2, owner),
            make_army(3, owner),
        ]);
        let mut groups = ArmyGroupRegistry::default();
        let group_id = groups
            .create_group(owner, &[ArmyId(2)], &military_registry)
            .unwrap();

        // 師団1(未所属)・師団2(所属)・師団3(未所属)を選択 → 師団2の編成が対象になる
        assert_eq!(
            groups.target_group_for_selection(&[ArmyId(1), ArmyId(2), ArmyId(3)]),
            Some(group_id)
        );
        // 誰も編成に属していなければNone
        assert_eq!(
            groups.target_group_for_selection(&[ArmyId(1), ArmyId(3)]),
            None
        );
    }

    #[test]
    fn auto_generated_names_increment_per_country() {
        let owner1 = CountryId(1);
        let owner2 = CountryId(2);
        let military_registry = registry_with_armies(vec![
            make_army(1, owner1),
            make_army(2, owner1),
            make_army(3, owner2),
        ]);
        let mut groups = ArmyGroupRegistry::default();
        let g1 = groups
            .create_group(owner1, &[ArmyId(1)], &military_registry)
            .unwrap();
        let g2 = groups
            .create_group(owner1, &[ArmyId(2)], &military_registry)
            .unwrap();
        let g3 = groups
            .create_group(owner2, &[ArmyId(3)], &military_registry)
            .unwrap();

        assert_eq!(groups.groups[&g1].name, "Army 1");
        assert_eq!(groups.groups[&g2].name, "Army 2");
        // 国家ごとに独立した採番
        assert_eq!(groups.groups[&g3].name, "Army 1");
    }
}
