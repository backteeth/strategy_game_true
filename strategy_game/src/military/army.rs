/// P21-004: 複数師団の永続的な集合(軍)を管理するモジュール。
/// 個別師団インスタンスは`DivisionId`(`military::data::Division`)、
/// 師団定義(テンプレート)は`DivisionDefinitionId`(`DivisionDefinition`)、
/// この軍は`ArmyId`(P21-004R: `military::army_group`から改名、階層名を正規化)。
///
/// `war::frontline::FrontlineRegistry`と同じ設計方針を踏襲する:
/// - Bevy Entityではなく、Resource内のプレーンなデータとして管理する
///   (このコードベースの全シミュレーション状態と同じ慣習)
/// - 「1師団は同時に1個の編成のみ所属」を、陸軍→編成の逆引きマップで一元的に保証する
/// - 撃破・消滅した師団の参照は`sanitize_references`で日次に整理する
///   (`FrontlineRegistry::sanitize_references`と対になる)
use crate::common::{ArmyId, CountryId, DivisionId};
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 編成(軍)データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Army {
    pub id: ArmyId,
    pub owner: CountryId,
    pub name: String,
    /// 所属師団(DivisionId昇順で安定保持)
    pub member_division_ids: Vec<DivisionId>,
}

/// UI上で現在「選択中」の編成(軍)。マップの`SelectedState`/`SelectedDivision`と同様、
/// 純粋なUI選択状態でありシミュレーション本体には影響しない。
/// P21-004: 編成一覧のクリックで設定され、追加/除外/解散ボタンの対象を決める。
/// 参照先の編成が消滅(解散・自動解散)した場合はNoneへ戻す必要がある
/// (`ui::military_panel::update_army_ui`が毎更新時に再検証する)。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedArmy(pub Option<ArmyId>);

/// 全編成を集中管理するリソース
#[derive(Resource, Default, Debug, Clone, Serialize, Deserialize)]
pub struct ArmyRegistry {
    pub armies: HashMap<ArmyId, Army>,
    /// DivisionId -> ArmyId のマッピング（1師団は1編成のみ所属）
    pub division_army_map: HashMap<DivisionId, ArmyId>,
    next_id: usize,
    /// 国家ごとの自動採番カウンタ("Army 1", "Army 2", ...)
    next_army_number: HashMap<CountryId, u32>,
}

fn division_is_usable(
    military_registry: &MilitaryRegistry,
    division_id: DivisionId,
    owner: CountryId,
) -> bool {
    military_registry
        .divisions
        .get(&division_id)
        .map(|a| a.owner == owner && a.manpower > 0 && a.status != DivisionStatus::Destroyed)
        .unwrap_or(false)
}

impl ArmyRegistry {
    pub fn next_id(&self) -> usize {
        self.next_id
    }

    /// 保存されていた全4フィールドから`ArmyRegistry`を復元する（P21-SAVE-002D）。
    /// `crate::save`のDTO型ではなく、通常のコレクションとカウンタだけを引数にとる。
    pub(crate) fn from_saved_parts(
        armies: HashMap<ArmyId, Army>,
        division_army_map: HashMap<DivisionId, ArmyId>,
        next_id: usize,
        next_army_number: HashMap<CountryId, u32>,
    ) -> Self {
        Self {
            armies,
            division_army_map,
            next_id,
            next_army_number,
        }
    }

    /// 国家ごとの自動採番カウンタへの読み取り専用アクセサ（P21-SAVE-002B: セーブ用。
    /// `next_army_number`はロード後の命名衝突防止に必要な正規状態だが、既存の`next_id()`
    /// と異なりHashMapなのでコピーではなく借用を返す。呼び出し側[`crate::save::export`]で
    /// `.clone()`してSaved DTOへ複製する）
    pub(crate) fn next_army_number_map(&self) -> &HashMap<CountryId, u32> {
        &self.next_army_number
    }

    /// 陸軍を現在所属している編成(あれば)から取り除く(内部ヘルパー)。
    /// 除去の結果、編成の所属師団が0になった場合は編成自体を自動解散する
    /// (最小仕様: 空編成を放置しない)。
    fn detach_division(&mut self, division_id: DivisionId) {
        if let Some(old_group_id) = self.division_army_map.remove(&division_id)
            && let Some(old_group) = self.armies.get_mut(&old_group_id)
        {
            old_group
                .member_division_ids
                .retain(|&id| id != division_id);
            if old_group.member_division_ids.is_empty() {
                self.armies.remove(&old_group_id);
            }
        }
    }

    /// 選択中の陸軍から新しい編成を作る。所有者不一致・撃破済み・存在しない陸軍は
    /// 黙って除外する(選択自体が所有者不問のため`map::division_selection`側から
    /// 敵国陸軍が混ざって渡され得る)。有効な陸軍が1件もなければNoneを返す。
    pub fn create_army(
        &mut self,
        owner: CountryId,
        member_division_ids: &[DivisionId],
        military_registry: &MilitaryRegistry,
    ) -> Option<ArmyId> {
        let mut valid_ids: Vec<DivisionId> = member_division_ids
            .iter()
            .copied()
            .filter(|&id| division_is_usable(military_registry, id, owner))
            .collect();

        if valid_ids.is_empty() {
            return None;
        }
        valid_ids.sort_by_key(|id| id.0);

        let group_id = ArmyId(self.next_id);
        self.next_id += 1;

        let number = self.next_army_number.entry(owner).or_insert(1);
        let name = format!("Army {number}");
        *number += 1;

        for &division_id in &valid_ids {
            self.detach_division(division_id);
        }
        for &division_id in &valid_ids {
            self.division_army_map.insert(division_id, group_id);
        }

        self.armies.insert(
            group_id,
            Army {
                id: group_id,
                owner,
                name,
                member_division_ids: valid_ids,
            },
        );

        Some(group_id)
    }

    /// 陸軍を既存の編成へ追加する(既に別編成にいた場合はそちらから外れる)。
    pub fn add_division(
        &mut self,
        group_id: ArmyId,
        division_id: DivisionId,
        owner: CountryId,
        military_registry: &MilitaryRegistry,
    ) -> Result<(), &'static str> {
        let group = self
            .armies
            .get(&group_id)
            .ok_or("Division group not found")?;
        if group.owner != owner {
            return Err("Division group belongs to a different country");
        }
        if !division_is_usable(military_registry, division_id, owner) {
            return Err("Division not found, foreign-owned, or destroyed");
        }

        // 既にこの編成へ所属済みなら何もしない。ここで無条件にdetach_divisionを
        // 呼ぶと、対象師団がこの編成の唯一の所属だった場合、detach直後の
        // 自動解散(空編成を残さない仕様)でgroup_id自体が消えてしまい、
        // 直後のget_mutが失敗する自己参照トラップになる。
        if self.division_army_map.get(&division_id) == Some(&group_id) {
            return Ok(());
        }

        self.detach_division(division_id);

        let group = self
            .armies
            .get_mut(&group_id)
            .expect("group existence already checked above");
        if !group.member_division_ids.contains(&division_id) {
            group.member_division_ids.push(division_id);
            group.member_division_ids.sort_by_key(|id| id.0);
        }
        self.division_army_map.insert(division_id, group_id);
        Ok(())
    }

    /// 陸軍を所属編成から除外する(未所属に戻す)。所有者検証込み。
    pub fn remove_division(
        &mut self,
        division_id: DivisionId,
        owner: CountryId,
        military_registry: &MilitaryRegistry,
    ) -> Result<(), &'static str> {
        let division = military_registry
            .divisions
            .get(&division_id)
            .ok_or("Division not found")?;
        if division.owner != owner {
            return Err("Division belongs to a different country");
        }
        self.detach_division(division_id);
        Ok(())
    }

    /// 編成を解散する。所属していた陸軍は全員未所属へ戻る。
    pub fn disband(&mut self, group_id: ArmyId, owner: CountryId) -> Result<(), &'static str> {
        let group = self
            .armies
            .get(&group_id)
            .ok_or("Division group not found")?;
        if group.owner != owner {
            return Err("Division group belongs to a different country");
        }
        if let Some(group) = self.armies.remove(&group_id) {
            for division_id in group.member_division_ids {
                self.division_army_map.remove(&division_id);
            }
        }
        Ok(())
    }

    /// 陸軍が所属する編成のIDを取得
    pub fn army_for_division(&self, division_id: DivisionId) -> Option<ArmyId> {
        self.division_army_map.get(&division_id).copied()
    }

    /// 選択中陸軍(複数可)のうち、いずれかの編成に所属しているものがあれば、
    /// DivisionId昇順で最初に見つかったものが属する編成を「操作対象の編成」として返す。
    /// UI(追加/除外/軍を選択/解散ボタン)と実行ハンドラの両方がこれを使い、
    /// 「対象編成をどう決めるか」の判断を1箇所に集約する。
    pub fn target_army_for_selection(
        &self,
        selected_division_ids: &[DivisionId],
    ) -> Option<ArmyId> {
        let mut ids: Vec<DivisionId> = selected_division_ids.to_vec();
        ids.sort_by_key(|id| id.0);
        ids.iter().find_map(|id| self.army_for_division(*id))
    }

    /// 消滅・撃破済み陸軍の参照を全編成から整理する
    /// (`war::frontline::FrontlineRegistry::sanitize_references`と対になる日次処理)。
    /// 整理の結果、所属師団が0になった編成は自動解散する(最小仕様: 空編成を放置しない)。
    pub fn sanitize_references(&mut self, military_registry: &MilitaryRegistry) {
        for group in self.armies.values_mut() {
            let owner = group.owner;
            group
                .member_division_ids
                .retain(|&division_id| division_is_usable(military_registry, division_id, owner));
        }

        self.armies
            .retain(|_, group| !group.member_division_ids.is_empty());

        let armies = &self.armies;
        self.division_army_map.retain(|division_id, group_id| {
            armies
                .get(group_id)
                .map(|g| division_is_usable(military_registry, *division_id, g.owner))
                .unwrap_or(false)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DivisionDefinitionId, DivisionId};
    use crate::military::data::{Division, DivisionSize, DivisionType};

    fn make_division(id: usize, owner: CountryId) -> Division {
        Division {
            id: DivisionId(id),
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
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn registry_with_divisions(divisions: Vec<Division>) -> MilitaryRegistry {
        let mut reg = MilitaryRegistry::default();
        for division in divisions {
            reg.divisions.insert(division.id, division);
        }
        reg
    }

    #[test]
    fn create_army_from_selection_ignores_foreign_and_destroyed_divisions() {
        let owner = CountryId(1);
        let mut destroyed = make_division(3, owner);
        destroyed.manpower = 0;
        let military_registry = registry_with_divisions(vec![
            make_division(1, owner),
            make_division(2, CountryId(2)), // 他国
            destroyed,
        ]);

        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(
                owner,
                &[DivisionId(1), DivisionId(2), DivisionId(3)],
                &military_registry,
            )
            .expect("at least one valid division should form a group");

        let group = groups.armies.get(&group_id).unwrap();
        assert_eq!(group.member_division_ids, vec![DivisionId(1)]);
        assert_eq!(group.owner, owner);
        assert_eq!(groups.army_for_division(DivisionId(1)), Some(group_id));
        assert_eq!(groups.army_for_division(DivisionId(2)), None);
    }

    #[test]
    fn create_army_with_no_valid_divisions_returns_none() {
        let owner = CountryId(1);
        let military_registry = registry_with_divisions(vec![make_division(1, CountryId(2))]);
        let mut groups = ArmyRegistry::default();
        assert!(
            groups
                .create_army(owner, &[DivisionId(1)], &military_registry)
                .is_none()
        );
        assert!(groups.armies.is_empty());
    }

    #[test]
    fn add_division_moves_it_from_previous_group() {
        let owner = CountryId(1);
        // group_aは師団1・3の2体編成にしておく(師団1が抜けても空にならず、
        // 「移動元編成からの除外」だけを空編成の自動解散から切り分けて検証できる)
        let military_registry = registry_with_divisions(vec![
            make_division(1, owner),
            make_division(2, owner),
            make_division(3, owner),
        ]);
        let mut groups = ArmyRegistry::default();
        let group_a = groups
            .create_army(owner, &[DivisionId(1), DivisionId(3)], &military_registry)
            .unwrap();
        let group_b = groups
            .create_army(owner, &[DivisionId(2)], &military_registry)
            .unwrap();

        groups
            .add_division(group_b, DivisionId(1), owner, &military_registry)
            .unwrap();

        assert!(
            !groups.armies[&group_a]
                .member_division_ids
                .contains(&DivisionId(1))
        );
        assert_eq!(
            groups.armies[&group_a].member_division_ids,
            vec![DivisionId(3)],
            "移動元編成は残りの師団を保持したまま存続する"
        );
        assert_eq!(
            groups.armies[&group_b].member_division_ids,
            vec![DivisionId(1), DivisionId(2)]
        );
        assert_eq!(groups.army_for_division(DivisionId(1)), Some(group_b));
    }

    /// 自己参照トラップの回帰テスト: 既に唯一の所属師団である師団を、同じ編成へ
    /// 「追加」しても(=UIで選択中編成に既存メンバーごと選択して追加を押す等)、
    /// 空編成の自動解散と衝突してpanicしたり編成が消えたりしない。
    #[test]
    fn add_division_already_sole_member_of_target_is_a_safe_no_op() {
        let owner = CountryId(1);
        let military_registry = registry_with_divisions(vec![make_division(1, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1)], &military_registry)
            .unwrap();

        let result = groups.add_division(group_id, DivisionId(1), owner, &military_registry);

        assert!(result.is_ok());
        assert!(groups.armies.contains_key(&group_id));
        assert_eq!(
            groups.armies[&group_id].member_division_ids,
            vec![DivisionId(1)]
        );
        assert_eq!(groups.army_for_division(DivisionId(1)), Some(group_id));
    }

    #[test]
    fn add_division_rejects_foreign_division() {
        let owner = CountryId(1);
        let military_registry = registry_with_divisions(vec![
            make_division(1, owner),
            make_division(2, CountryId(2)),
        ]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1)], &military_registry)
            .unwrap();

        let result = groups.add_division(group_id, DivisionId(2), owner, &military_registry);
        assert!(result.is_err());
        assert_eq!(groups.army_for_division(DivisionId(2)), None);
    }

    #[test]
    fn remove_division_returns_it_to_unassigned() {
        let owner = CountryId(1);
        let military_registry =
            registry_with_divisions(vec![make_division(1, owner), make_division(2, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1), DivisionId(2)], &military_registry)
            .unwrap();

        groups
            .remove_division(DivisionId(1), owner, &military_registry)
            .unwrap();

        assert_eq!(groups.army_for_division(DivisionId(1)), None);
        assert_eq!(
            groups.armies[&group_id].member_division_ids,
            vec![DivisionId(2)]
        );
    }

    #[test]
    fn remove_division_rejects_non_owner() {
        let owner = CountryId(1);
        let military_registry = registry_with_divisions(vec![make_division(1, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1)], &military_registry)
            .unwrap();

        let result = groups.remove_division(DivisionId(1), CountryId(2), &military_registry);
        assert!(result.is_err());
        assert_eq!(groups.army_for_division(DivisionId(1)), Some(group_id));
    }

    #[test]
    fn disband_returns_all_members_to_unassigned_and_removes_group() {
        let owner = CountryId(1);
        let military_registry =
            registry_with_divisions(vec![make_division(1, owner), make_division(2, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1), DivisionId(2)], &military_registry)
            .unwrap();

        groups.disband(group_id, owner).unwrap();

        assert!(!groups.armies.contains_key(&group_id));
        assert_eq!(groups.army_for_division(DivisionId(1)), None);
        assert_eq!(groups.army_for_division(DivisionId(2)), None);
    }

    #[test]
    fn disband_rejects_non_owner_and_keeps_group_intact() {
        let owner = CountryId(1);
        let military_registry = registry_with_divisions(vec![make_division(1, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1)], &military_registry)
            .unwrap();

        let result = groups.disband(group_id, CountryId(2));
        assert!(result.is_err());
        assert!(groups.armies.contains_key(&group_id));
    }

    #[test]
    fn sanitize_references_removes_destroyed_divisions_from_group_and_map() {
        let owner = CountryId(1);
        let mut military_registry =
            registry_with_divisions(vec![make_division(1, owner), make_division(2, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1), DivisionId(2)], &military_registry)
            .unwrap();

        // 師団1が戦闘で撃破された想定(MilitaryRegistryから削除される)
        military_registry.remove_division(DivisionId(1));

        groups.sanitize_references(&military_registry);

        assert_eq!(
            groups.armies[&group_id].member_division_ids,
            vec![DivisionId(2)]
        );
        assert_eq!(groups.army_for_division(DivisionId(1)), None);
        assert_eq!(groups.army_for_division(DivisionId(2)), Some(group_id));
    }

    #[test]
    fn target_army_for_selection_picks_lowest_division_id_with_a_group() {
        let owner = CountryId(1);
        let military_registry = registry_with_divisions(vec![
            make_division(1, owner),
            make_division(2, owner),
            make_division(3, owner),
        ]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(2)], &military_registry)
            .unwrap();

        // 師団1(未所属)・師団2(所属)・師団3(未所属)を選択 → 師団2の編成が対象になる
        assert_eq!(
            groups.target_army_for_selection(&[DivisionId(1), DivisionId(2), DivisionId(3)]),
            Some(group_id)
        );
        // 誰も編成に属していなければNone
        assert_eq!(
            groups.target_army_for_selection(&[DivisionId(1), DivisionId(3)]),
            None
        );
    }

    #[test]
    fn army_ids_are_unique_across_creations() {
        let owner = CountryId(1);
        let military_registry =
            registry_with_divisions(vec![make_division(1, owner), make_division(2, owner)]);
        let mut groups = ArmyRegistry::default();
        let g1 = groups
            .create_army(owner, &[DivisionId(1)], &military_registry)
            .unwrap();
        let g2 = groups
            .create_army(owner, &[DivisionId(2)], &military_registry)
            .unwrap();

        assert_ne!(g1, g2);
        assert_eq!(groups.armies.len(), 2);
    }

    /// P21-004 spec #11/#14: 最後の所属師団を除外(手動)すると、その編成は自動解散される。
    #[test]
    fn removing_last_division_auto_disbands_the_army() {
        let owner = CountryId(1);
        let military_registry = registry_with_divisions(vec![make_division(1, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1)], &military_registry)
            .unwrap();

        groups
            .remove_division(DivisionId(1), owner, &military_registry)
            .unwrap();

        assert!(
            !groups.armies.contains_key(&group_id),
            "空になった編成は自動解散されるはず"
        );
        assert_eq!(groups.army_for_division(DivisionId(1)), None);
    }

    /// P21-004 spec #14: 最後の所属師団が戦闘等で消滅した場合も、日次整理
    /// (`sanitize_references`)で編成が自動解散される。
    #[test]
    fn sanitize_references_auto_disbands_army_that_becomes_empty() {
        let owner = CountryId(1);
        let mut military_registry = registry_with_divisions(vec![make_division(1, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1)], &military_registry)
            .unwrap();

        // 唯一の所属師団が消滅
        military_registry.remove_division(DivisionId(1));
        groups.sanitize_references(&military_registry);

        assert!(
            !groups.armies.contains_key(&group_id),
            "最後の所属師団が消滅した編成は自動解散されるはず"
        );
        assert_eq!(groups.army_for_division(DivisionId(1)), None);
    }

    /// 移動によって旧編成が空になった場合も、旧編成は自動解散される
    /// (作成直後の除外だけでなく、他編成への移動でも同じ不変条件が保たれることを確認)。
    #[test]
    fn moving_last_division_to_another_army_auto_disbands_the_source_army() {
        let owner = CountryId(1);
        let military_registry =
            registry_with_divisions(vec![make_division(1, owner), make_division(2, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_a = groups
            .create_army(owner, &[DivisionId(1)], &military_registry)
            .unwrap();
        let group_b = groups
            .create_army(owner, &[DivisionId(2)], &military_registry)
            .unwrap();

        groups
            .add_division(group_b, DivisionId(1), owner, &military_registry)
            .unwrap();

        assert!(
            !groups.armies.contains_key(&group_a),
            "唯一の所属師団を失った旧編成は自動解散されるはず"
        );
        assert_eq!(
            groups.armies[&group_b].member_division_ids,
            vec![DivisionId(1), DivisionId(2)]
        );
    }

    /// P21-004 spec #15: 存在しないDivisionIdを対象にした操作は安全に無視/拒否される。
    #[test]
    fn add_and_remove_reject_nonexistent_division_id_safely() {
        let owner = CountryId(1);
        let military_registry = registry_with_divisions(vec![make_division(1, owner)]);
        let mut groups = ArmyRegistry::default();
        let group_id = groups
            .create_army(owner, &[DivisionId(1)], &military_registry)
            .unwrap();

        let add_result = groups.add_division(group_id, DivisionId(999), owner, &military_registry);
        assert!(add_result.is_err());
        assert_eq!(groups.army_for_division(DivisionId(999)), None);

        let remove_result = groups.remove_division(DivisionId(999), owner, &military_registry);
        assert!(remove_result.is_err());
        // 既存編成には影響しない
        assert_eq!(
            groups.armies[&group_id].member_division_ids,
            vec![DivisionId(1)]
        );
    }

    #[test]
    fn auto_generated_names_increment_per_country() {
        let owner1 = CountryId(1);
        let owner2 = CountryId(2);
        let military_registry = registry_with_divisions(vec![
            make_division(1, owner1),
            make_division(2, owner1),
            make_division(3, owner2),
        ]);
        let mut groups = ArmyRegistry::default();
        let g1 = groups
            .create_army(owner1, &[DivisionId(1)], &military_registry)
            .unwrap();
        let g2 = groups
            .create_army(owner1, &[DivisionId(2)], &military_registry)
            .unwrap();
        let g3 = groups
            .create_army(owner2, &[DivisionId(3)], &military_registry)
            .unwrap();

        assert_eq!(groups.armies[&g1].name, "Army 1");
        assert_eq!(groups.armies[&g2].name, "Army 2");
        // 国家ごとに独立した採番
        assert_eq!(groups.armies[&g3].name, "Army 1");
    }
}
