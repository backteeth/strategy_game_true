//! P32-017: 新ステート経路の**静的入力の項目別ダイジェスト**と、保存時・復元先の不一致の判定表
//! (Bevy非依存)。
//!
//! ## 役割の整理(識別情報は3種類ある)
//! 1. 世界層の地理識別(`world::GeographyFingerprint`, FNV-1a)。**ファイルに記録された値**で、世界層の
//!    検証(`validate_against_geography`)のためのもの。ここでは使わない。
//! 2. P32-015 の内部コンテキスト(`MapEconomySnapshot::context`, FNV-1a 64bit)。**計算結果を左右する静的入力
//!    だけ**の総合的な同一性確認(警告で許す項目は含めない)。直接 `restore` を呼ぶ経路の最後の砦。
//! 3. このモジュールのダイジェスト(BLAKE3-256, 項目別)。ファイル(形式2)に保存し、復元先の静的入力
//!    (実際に読み込んで使っている `HexMap`・`WorldModel`・`BuildingRegistry`・シミュレーション)から
//!    **毎回作り直して**項目ごとに比べる。保存ファイル内の値を復元先の正しい値として採用しない。
//!
//! 「署名」と呼ぶが**内容識別用のダイジェスト**であり、電子署名・改ざん者の認証・暗号学的な真正性の保証では
//! ない(ファイルを書ける者は、自分で計算した値を書ける)。独自の暗号アルゴリズムは実装せず、`blake3` の
//! 標準実装を使う(SHA-256のクレートは依存になく、BLAKE3は既にCargo.lockにあるため新しい依存を増やさない)。
//!
//! ## 正規化
//! 項目ごとに別の `blake3::Hasher` を使い、先頭に方式名・版・項目名(長さ前置)を入れる(項目間の混同を防ぐ)。
//! 整数は固定長リトルエンディアン、文字列と列は長さ(個数)前置、`Option` は 0/1 の目印、浮動小数点は
//! ビット列そのまま(丸めない。非有限値はエラー)。セルは `(r, q)` 昇順、ID付きの集合はID昇順、資源・建物は
//! `ALL` の順で出力する。`HashMap` の列挙順・ファイルの配置場所・更新日時には依存しない。
//!
//! ## 判定表(`RULES`)
//! 既定は**拒否**。警告で許す項目は `RULES` に限定列挙した `Class::Warn` だけで、根拠(現在の保存対象の
//! 計算で読まれないことの確認)を持つ。表にない項目名・欠落した項目・非対応の方式は拒否する。
//! 複数項目が同時に違う場合は、拒否項目が一つでもあれば全体を拒否する。

use super::MapStateEconomyRegistry;
use super::snapshot::{ECONOMY_RULES_REVISION, MapEconomySimulation, building_index, resource_index};
use crate::building::data::{BuildingRegistry, BuildingType};
use crate::economy::resources::ResourceType;
use crate::hexmap::data::{HexMap, SUPPORTED_ORIENTATION};
use crate::world::{
    Coverage, LocationFacilityKind, LocationFacilityRegistry, ProvinceFacilitySetup, WorldModel,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// ダイジェストの方式名。
pub const IDENTITY_SCHEME: &str = "blake3-256";
/// 正規化の版。正規化の手順(出力するバイト列)を変えたら上げる(保存値との比較は非対応として拒否される)。
pub const IDENTITY_SCHEME_VERSION: u32 = 1;
const DOMAIN: &[u8] = b"strategy_game/map_economy/static_identity";

// ---------------------------------------------------------------- 項目

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKey {
    Grid,
    CellCoordinates,
    LandSet,
    WaterKinds,
    LandTerrain,
    Climate,
    Vegetation,
    Rivers,
    ProvinceTable,
    StateMembership,
    StatePresentation,
    StateConstraints,
    ScenarioStructure,
    StateEconomyStatic,
    ResourceDeposits,
    ResourceSetup,
    MilitaryInputs,
    BuildingComputation,
    BuildingPresentation,
    LocationFacilities,
    EconomyRules,
}

impl ItemKey {
    pub const ALL: [ItemKey; 21] = [
        ItemKey::Grid,
        ItemKey::CellCoordinates,
        ItemKey::LandSet,
        ItemKey::WaterKinds,
        ItemKey::LandTerrain,
        ItemKey::Climate,
        ItemKey::Vegetation,
        ItemKey::Rivers,
        ItemKey::ProvinceTable,
        ItemKey::StateMembership,
        ItemKey::StatePresentation,
        ItemKey::StateConstraints,
        ItemKey::ScenarioStructure,
        ItemKey::StateEconomyStatic,
        ItemKey::ResourceDeposits,
        ItemKey::ResourceSetup,
        ItemKey::MilitaryInputs,
        ItemKey::BuildingComputation,
        ItemKey::BuildingPresentation,
        ItemKey::LocationFacilities,
        ItemKey::EconomyRules,
    ];

    /// ファイルに書く項目名(変えない。変える場合は方式の版を上げる)。
    pub fn name(self) -> &'static str {
        match self {
            ItemKey::Grid => "grid",
            ItemKey::CellCoordinates => "cell_coordinates",
            ItemKey::LandSet => "land_set",
            ItemKey::WaterKinds => "water_kinds",
            ItemKey::LandTerrain => "land_terrain",
            ItemKey::Climate => "climate",
            ItemKey::Vegetation => "vegetation",
            ItemKey::Rivers => "rivers",
            ItemKey::ProvinceTable => "province_table",
            ItemKey::StateMembership => "state_membership",
            ItemKey::StatePresentation => "state_presentation",
            ItemKey::StateConstraints => "state_constraints",
            ItemKey::ScenarioStructure => "scenario_structure",
            ItemKey::StateEconomyStatic => "state_economy_static",
            ItemKey::ResourceDeposits => "resource_deposits",
            ItemKey::ResourceSetup => "resource_setup",
            ItemKey::MilitaryInputs => "military_inputs",
            ItemKey::BuildingComputation => "building_computation",
            ItemKey::BuildingPresentation => "building_presentation",
            ItemKey::LocationFacilities => "location_facilities",
            ItemKey::EconomyRules => "economy_rules",
        }
    }

    pub fn parse(name: &str) -> Option<ItemKey> {
        ItemKey::ALL.into_iter().find(|k| k.name() == name)
    }
}

impl std::fmt::Display for ItemKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// 一致しなければ復元しない(既定)。
    Reject,
    /// 一致しなくても、警告を返したうえで復元できる(根拠を `RULES` に記録した項目だけ)。
    Warn,
}

/// 判定表の1行。`used_by`・`normalization`・`impact`・`rationale` は開発用の記録(報告書・テストが参照する)。
#[derive(Debug, Clone, Copy)]
pub struct ItemRule {
    pub key: ItemKey,
    pub class: Class,
    /// 利用する処理(現在のコードで、この入力を読む処理。「なし」は読む処理がないことの確認結果)。
    pub used_by: &'static str,
    pub normalization: &'static str,
    /// 変更したときの影響。
    pub impact: &'static str,
    /// 判定の根拠。
    pub rationale: &'static str,
}

const fn rule(
    key: ItemKey,
    class: Class,
    used_by: &'static str,
    normalization: &'static str,
    impact: &'static str,
    rationale: &'static str,
) -> ItemRule {
    ItemRule { key, class, used_by, normalization, impact, rationale }
}

/// 判定表。**ここに書いた `Warn` だけが警告で許される。** 将来、`Warn` の項目を計算が読むようになったら、
/// 同じ変更で `Reject` へ変え、`ECONOMY_RULES_REVISION` を上げること(`rules_revision_policy` 参照)。
pub const RULES: [ItemRule; 21] = [
    rule(ItemKey::Grid, Class::Reject,
        "座標→位置の対応(プロヴィンス表のセル解決、地理集計)。経済計算は直接読まない",
        "orientation・size・origin・画像寸法・extent(f64はビット列)",
        "同じ座標が別の場所を指す。プロヴィンスの位置の意味が変わる",
        "グリッドの構造変更は座標・プロヴィンスの意味を変える(基本方針で拒否)"),
    rule(ItemKey::CellCoordinates, Class::Reject,
        "プロヴィンス表の所在地の存在確認、陸水の判定の母集団",
        "全セルの(q,r)を(r,q)昇順に",
        "セル集合が変わるとプロヴィンスの所在地・地図の範囲が変わる",
        "座標集合の構造変更(基本方針で拒否)"),
    rule(ItemKey::LandSet, Class::Reject,
        "プロヴィンスは陸セルだけに置ける(世界層の検証)。陸水の別",
        "陸セル(水域でないセル)の(q,r)を(r,q)昇順に",
        "陸水が変わるとプロヴィンスの有効性が変わる",
        "陸水の変更(基本方針で拒否)"),
    rule(ItemKey::WaterKinds, Class::Reject,
        "なし(現在の経済は読まない)。ただし海・湖の扱いは未決のゲーム規則",
        "水域セルの(q,r)と地形ID(r,q昇順)",
        "現在の経済結果には影響しない。海と湖の区別は将来の規則に関わりうる",
        "海・湖の扱いが未決のため、陸水の区別として保守的に拒否する"),
    rule(ItemKey::LandTerrain, Class::Warn,
        "なし。経済・AI・建設・月次処理は地形を読まない(`economy/`・`country/`・`building/`に参照なし。戦闘の地形補正は引数を無視する)",
        "陸セルの(q,r)と地形ID(r,q昇順)。水域は `water_kinds` と `land_set` が別に守る",
        "現在の保存対象(経済・AI・建設)の結果には影響しない",
        "読まれないことをコード参照と自動テスト(再開後の結果の一致)で確認した。将来、地形を計算が読むなら拒否へ変える"),
    rule(ItemKey::Climate, Class::Warn,
        "なし。経済・AI・建設・月次処理は気候を読まない",
        "全セルの(q,r)と気候ID(r,q昇順)",
        "現在の保存対象の結果には影響しない",
        "地形と同じ。コード参照と自動テストで確認した"),
    rule(ItemKey::Vegetation, Class::Warn,
        "なし。経済・AI・建設・月次処理は植生を読まない",
        "全セルの(q,r)と植生ID(r,q昇順)",
        "現在の保存対象の結果には影響しない",
        "地形と同じ。コード参照と自動テストで確認した"),
    rule(ItemKey::Rivers, Class::Warn,
        "なし。河川はステート境界の描画・地理集計(再生成可能な導出)だけが読む。経済・AI・建設は読まない",
        "正規化済みの辺(q,r,direction)昇順",
        "現在の保存対象の結果には影響しない(世界層も情報(Info)扱い)",
        "世界層の検証が既に情報扱い。経済側に参照がないことを確認した。河川を補給等に使うなら拒否へ変える"),
    rule(ItemKey::ProvinceTable, Class::Reject,
        "プロヴィンスIDの対象範囲・所在地。ステート所属の解決",
        "coverage・next_province_id・退役ID・(id,q,r)をID昇順に",
        "プロヴィンスの識別・所在地・IDの再利用防止が変わる",
        "プロヴィンスの識別や所属の変更(基本方針で拒否)"),
    rule(ItemKey::StateMembership, Class::Reject,
        "資源の集計(ステート別鉱床)、国家とステートの対応の前提",
        "ステートID昇順に所属プロヴィンスID昇順",
        "ステートの鉱床・規模の意味が変わる",
        "所属の変更(基本方針で拒否)"),
    rule(ItemKey::StatePresentation, Class::Warn,
        "表示名の翻訳(`world_view`)と代表プロヴィンスの表示位置だけ。経済・AI・建設は読まない",
        "ステートID昇順に name_key と代表プロヴィンス",
        "表示だけが変わる",
        "表示用メタデータ。経済・AI・建設に参照がないことを確認した"),
    rule(ItemKey::StateConstraints, Class::Reject,
        "世界層の検証(連結性。許可フラグ)。経済は読まない",
        "ステートID昇順に allow_disconnected",
        "検証の通り方が変わる。ステートの連結性規則は未決",
        "未決の規則に関わるため保守的に拒否する"),
    rule(ItemKey::ScenarioStructure, Class::Reject,
        "日次処理(国家ごとの建設・月次経済・AI評価)、国家とステートの経済上の対応、AI国家か否か",
        "経済ステートID集合、国家(ラベル・AIか否か・所属ステート)をシミュレーションの順に",
        "国家構造・経済上の対応・AIの有無が変わる",
        "国家構造の意味的変更(基本方針で拒否)。国家の並びは処理順に関わるため正規化しない"),
    rule(ItemKey::StateEconomyStatic, Class::Reject,
        "月次経済(労働力・物流容量)",
        "ステートID昇順に workforce_ratio・base_logistics_capacity(ビット列)",
        "雇用・物流・稼働率が変わる",
        "経済結果へ影響する属性(基本方針で拒否)"),
    rule(ItemKey::ResourceDeposits, Class::Reject,
        "生産処理(鉱床による産出)、Mineの注文可否",
        "ステートID昇順に鉱床(資源・base_output・discovered・development_level)を保持順に",
        "産出が変わる",
        "資源定義の変更(基本方針で拒否)。鉱床の発見が実行時に変わる場合は責務の変更(残件)"),
    rule(ItemKey::ResourceSetup, Class::Reject,
        "Mineの注文受付・完成時の整合確認",
        "ステートID昇順に設定状況",
        "注文の可否が変わる",
        "資源設定の変更(基本方針で拒否)"),
    rule(ItemKey::MilitaryInputs, Class::Reject,
        "AIの優先度2.5・2.9(軍事AIへの明示入力)",
        "国家をシミュレーションの順に、軍事入力の有無と各値",
        "AIの建設判断が変わる",
        "軍事AI入力の意味的変更(基本方針で拒否)"),
    rule(ItemKey::BuildingComputation, Class::Reject,
        "建設費・必要進捗・労働力・物流・維持費・産出・投入・最大レベル・科学/魔法・鉄道容量",
        "建物種別の `ALL` 順に、定義の有無と計算に使う全項目(資源は `ALL` 順)",
        "建設・生産・経済の結果が変わる",
        "建物定義の計算に関係する変更(基本方針で拒否)"),
    rule(ItemKey::BuildingPresentation, Class::Warn,
        "なし。`name` は表示(UI)だけで、経済・AI・建設は読まない",
        "建物種別の `ALL` 順に name",
        "表示だけが変わる",
        "表示用メタデータ。経済・AI・建設の計算経路に `name` の参照がないことを確認した"),
    rule(ItemKey::LocationFacilities, Class::Warn,
        "なし。位置施設(港・要塞・鉄道駅)は入力層だけで、M4の経済・AI・建設は読まない(効果・費用・規模は未実装)",
        "提供の有無、範囲、施設(id,種別,プロヴィンス)をID昇順、設定済み空プロヴィンス昇順",
        "現在の保存対象の結果には影響しない",
        "計算に使われていないことを確認した。施設が計算に使われるようになったら拒否へ変える"),
    rule(ItemKey::EconomyRules, Class::Reject,
        "経済計算の式・定数の全体(手で管理する改訂番号)",
        "ECONOMY_RULES_REVISION",
        "同じ入力でも結果が変わりうる",
        "経済規則の改訂(基本方針で拒否)"),
];

pub fn rule_of(key: ItemKey) -> &'static ItemRule {
    RULES.iter().find(|r| r.key == key).expect("every item has a rule")
}

// ---------------------------------------------------------------- 入力

/// 識別情報を作る(保存時)・照合する(復元時)ための静的入力。いずれも実際に読み込んで使っているデータ。
pub struct StaticInputs<'a> {
    pub map: &'a HexMap,
    pub world: &'a WorldModel,
    pub buildings: &'a BuildingRegistry,
    /// 位置施設の定義(使っていなければ `None`。提供の有無も項目に含める)。
    pub facilities: Option<&'a LocationFacilityRegistry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// 浮動小数点が有限でない(識別と検証を整合させるため、作らない)。
    NonFinite { item: ItemKey, what: &'static str },
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::NonFinite { item, what } => {
                write!(f, "static input `{item}` contains a non-finite number ({what}); its identity is not defined")
            }
        }
    }
}

// ---------------------------------------------------------------- 正規化したバイト列

struct Enc {
    h: blake3::Hasher,
    item: ItemKey,
    bad: Option<&'static str>,
}

impl Enc {
    fn new(item: ItemKey) -> Self {
        let mut e = Enc { h: blake3::Hasher::new(), item, bad: None };
        e.h.update(DOMAIN);
        e.str(IDENTITY_SCHEME);
        e.u32(IDENTITY_SCHEME_VERSION);
        e.str(item.name());
        e
    }
    fn u8(&mut self, v: u8) {
        self.h.update(&[v]);
    }
    fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    fn u32(&mut self, v: u32) {
        self.h.update(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.h.update(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.h.update(&v.to_le_bytes());
    }
    fn count(&mut self, n: usize) {
        self.u64(n as u64);
    }
    fn str(&mut self, s: &str) {
        self.count(s.len());
        self.h.update(s.as_bytes());
    }
    fn f64(&mut self, v: f64, what: &'static str) {
        if !v.is_finite() {
            self.bad.get_or_insert(what);
        }
        self.h.update(&v.to_bits().to_le_bytes());
    }
    fn f32(&mut self, v: f32, what: &'static str) {
        if !v.is_finite() {
            self.bad.get_or_insert(what);
        }
        self.h.update(&v.to_bits().to_le_bytes());
    }
    fn opt_u64(&mut self, v: Option<u64>) {
        match v {
            None => self.u8(0),
            Some(x) => {
                self.u8(1);
                self.u64(x);
            }
        }
    }
    fn finish(self) -> Result<String, IdentityError> {
        match self.bad {
            Some(what) => Err(IdentityError::NonFinite { item: self.item, what }),
            None => Ok(self.h.finalize().to_hex().to_string()),
        }
    }
}

/// 全セルのインデックスを `(r, q)` 昇順に(入力の並び順に依存しない)。
fn sorted_cells(map: &HexMap) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..map.cells().len()).collect();
    idx.sort_by_key(|&i| (map.cells()[i].r, map.cells()[i].q));
    idx
}

fn registry_of(sim: &MapEconomySimulation) -> &MapStateEconomyRegistry {
    &sim.registry
}

// ---------------------------------------------------------------- 識別情報

/// 項目別のダイジェスト(16進小文字64桁)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticIdentity {
    items: BTreeMap<ItemKey, String>,
}

impl StaticIdentity {
    pub fn digest(&self, key: ItemKey) -> Option<&str> {
        self.items.get(&key).map(String::as_str)
    }

    /// 現在の静的入力から作る。
    pub fn compute(sim: &MapEconomySimulation, inputs: &StaticInputs<'_>) -> Result<StaticIdentity, IdentityError> {
        let mut items = BTreeMap::new();
        for key in ItemKey::ALL {
            items.insert(key, compute_item(key, sim, inputs)?);
        }
        Ok(StaticIdentity { items })
    }

    /// ファイルへ書く形(項目は判定表の順)。
    pub fn to_saved(&self) -> SavedIdentity {
        SavedIdentity {
            scheme: IDENTITY_SCHEME.to_string(),
            scheme_version: IDENTITY_SCHEME_VERSION,
            items: ItemKey::ALL
                .iter()
                .map(|k| SavedItem { key: k.name().to_string(), digest: self.items[k].clone() })
                .collect(),
        }
    }
}

fn compute_item(key: ItemKey, sim: &MapEconomySimulation, inputs: &StaticInputs<'_>) -> Result<String, IdentityError> {
    let mut e = Enc::new(key);
    let map = inputs.map;
    let world = inputs.world;
    match key {
        ItemKey::Grid => {
            let g = &map.grid;
            e.str(SUPPORTED_ORIENTATION);
            e.f64(g.size, "grid.size");
            e.f64(g.origin_x, "grid.origin_x");
            e.f64(g.origin_y, "grid.origin_y");
            e.u32(g.image_width);
            e.u32(g.image_height);
            e.i32(g.extent.min_q);
            e.i32(g.extent.max_q);
            e.i32(g.extent.min_r);
            e.i32(g.extent.max_r);
        }
        ItemKey::CellCoordinates => {
            let order = sorted_cells(map);
            e.count(order.len());
            for &i in &order {
                let c = &map.cells()[i];
                e.i32(c.q);
                e.i32(c.r);
            }
        }
        ItemKey::LandSet => {
            let land: Vec<_> = sorted_cells(map).into_iter().map(|i| &map.cells()[i]).filter(|c| !c.terrain.is_water()).collect();
            e.count(land.len());
            for c in land {
                e.i32(c.q);
                e.i32(c.r);
            }
        }
        ItemKey::WaterKinds => {
            let water: Vec<_> = sorted_cells(map).into_iter().map(|i| &map.cells()[i]).filter(|c| c.terrain.is_water()).collect();
            e.count(water.len());
            for c in water {
                e.i32(c.q);
                e.i32(c.r);
                e.str(c.terrain.id());
            }
        }
        ItemKey::LandTerrain => {
            let land: Vec<_> = sorted_cells(map).into_iter().map(|i| &map.cells()[i]).filter(|c| !c.terrain.is_water()).collect();
            e.count(land.len());
            for c in land {
                e.i32(c.q);
                e.i32(c.r);
                e.str(c.terrain.id());
            }
        }
        ItemKey::Climate => {
            let order = sorted_cells(map);
            e.count(order.len());
            for &i in &order {
                let c = &map.cells()[i];
                e.i32(c.q);
                e.i32(c.r);
                e.str(c.climate.id());
            }
        }
        ItemKey::Vegetation => {
            let order = sorted_cells(map);
            e.count(order.len());
            for &i in &order {
                let c = &map.cells()[i];
                e.i32(c.q);
                e.i32(c.r);
                e.str(c.vegetation.id());
            }
        }
        ItemKey::Rivers => {
            // `river_edges` は正規化済み(両側表現を統合)で(q,r,direction)昇順。
            let edges = map.river_edges();
            e.count(edges.len());
            for r in edges {
                e.i32(r.q);
                e.i32(r.r);
                e.u8(r.direction);
            }
        }
        ItemKey::ProvinceTable => {
            e.u8(match world.coverage() {
                Coverage::World => 0,
                Coverage::Partial => 1,
            });
            e.u32(world.next_province_id());
            let mut retired: Vec<u32> = world.retired_ids().iter().map(|p| p.0).collect();
            retired.sort_unstable();
            e.count(retired.len());
            for id in retired {
                e.u32(id);
            }
            let mut provinces: Vec<(u32, i32, i32)> = world.provinces().iter().map(|p| (p.id.0, p.cell.q, p.cell.r)).collect();
            provinces.sort_unstable();
            e.count(provinces.len());
            for (id, q, r) in provinces {
                e.u32(id);
                e.i32(q);
                e.i32(r);
            }
        }
        ItemKey::StateMembership => {
            let mut states: Vec<_> = world.states().iter().collect();
            states.sort_by_key(|s| s.id);
            e.count(states.len());
            for s in states {
                e.u32(s.id.0);
                let mut ps: Vec<u32> = s.provinces.iter().map(|p| p.0).collect();
                ps.sort_unstable();
                e.count(ps.len());
                for p in ps {
                    e.u32(p);
                }
            }
        }
        ItemKey::StatePresentation => {
            let mut states: Vec<_> = world.states().iter().collect();
            states.sort_by_key(|s| s.id);
            e.count(states.len());
            for s in states {
                e.u32(s.id.0);
                e.str(&s.name_key);
                e.opt_u64(s.representative.map(|p| p.0 as u64));
            }
        }
        ItemKey::StateConstraints => {
            let mut states: Vec<_> = world.states().iter().collect();
            states.sort_by_key(|s| s.id);
            e.count(states.len());
            for s in states {
                e.u32(s.id.0);
                e.bool(s.allow_disconnected);
            }
        }
        ItemKey::ScenarioStructure => {
            let reg = registry_of(sim);
            e.count(reg.states.len());
            for id in reg.states.keys() {
                e.u32(id.0);
            }
            e.count(sim.countries.len());
            for c in &sim.countries {
                e.u32(c.label);
                e.bool(c.ai.is_some());
                e.count(c.states.len());
                for s in &c.states {
                    e.u32(s.0);
                }
            }
        }
        ItemKey::StateEconomyStatic => {
            let reg = registry_of(sim);
            e.count(reg.states.len());
            for (id, s) in &reg.states {
                e.u32(id.0);
                e.f32(s.workforce_ratio, "state.workforce_ratio");
                e.f32(s.base_logistics_capacity, "state.base_logistics_capacity");
            }
        }
        ItemKey::ResourceDeposits => {
            let reg = registry_of(sim);
            e.count(reg.states.len());
            for (id, s) in &reg.states {
                e.u32(id.0);
                e.count(s.resource_deposits.len());
                for d in &s.resource_deposits {
                    e.u64(resource_index(d.resource_type));
                    e.f64(d.base_output, "deposit.base_output");
                    e.bool(d.discovered);
                    e.u64(d.development_level as u64);
                }
            }
        }
        ItemKey::ResourceSetup => match &registry_of(sim).resource_setup {
            None => e.u8(0),
            Some(map) => {
                e.u8(1);
                e.count(map.len());
                for (id, setup) in map {
                    e.u32(id.0);
                    e.u64(*setup as u64);
                }
            }
        },
        ItemKey::MilitaryInputs => {
            e.count(sim.countries.len());
            for c in &sim.countries {
                e.u32(c.label);
                match &c.military {
                    None => e.u8(0),
                    Some(m) => {
                        e.u8(1);
                        e.bool(m.at_war);
                        e.opt_u64(m.daily_demand_units);
                        e.u64(m.living_divisions);
                        e.u64(m.recruitment_queue_len);
                        e.opt_u64(m.infantry_required_manpower);
                    }
                }
            }
        }
        ItemKey::BuildingComputation => {
            e.count(BuildingType::ALL.len());
            for b in BuildingType::ALL {
                e.u64(building_index(b));
                match inputs.buildings.get(b) {
                    None => e.u8(0),
                    Some(d) => {
                        e.u8(1);
                        e.u64(building_index(d.building_type));
                        e.f64(d.construction_cost, "building.construction_cost");
                        e.f64(d.required_progress, "building.required_progress");
                        e.f32(d.required_workforce, "building.required_workforce");
                        e.f32(d.logistics_cost, "building.logistics_cost");
                        e.f64(d.maintenance_cost, "building.maintenance_cost");
                        e.u64(d.max_level as u64);
                        e.f64(d.science_output, "building.science_output");
                        e.f64(d.magic_output, "building.magic_output");
                        e.f32(d.railway_capacity_bonus, "building.railway_capacity_bonus");
                        for map in [&d.input_resources, &d.output_resources] {
                            let present: Vec<ResourceType> = ResourceType::ALL.into_iter().filter(|r| map.contains_key(r)).collect();
                            e.count(present.len());
                            for r in present {
                                e.u64(resource_index(r));
                                e.f64(map[&r], "building.resource_amount");
                            }
                        }
                    }
                }
            }
        }
        ItemKey::BuildingPresentation => {
            e.count(BuildingType::ALL.len());
            for b in BuildingType::ALL {
                e.u64(building_index(b));
                match inputs.buildings.get(b) {
                    None => e.u8(0),
                    Some(d) => {
                        e.u8(1);
                        e.str(&d.name);
                    }
                }
            }
        }
        ItemKey::LocationFacilities => match inputs.facilities {
            None => e.u8(0),
            Some(reg) => {
                e.u8(1);
                e.u8(match reg.scope() {
                    crate::world::FacilityScope::World => 0,
                    crate::world::FacilityScope::Partial => 1,
                });
                let facilities: Vec<_> = reg.iter().collect();
                e.count(facilities.len());
                for f in facilities {
                    e.u32(f.id);
                    e.u8(LocationFacilityKind::ALL.iter().position(|k| *k == f.kind).unwrap_or(usize::MAX) as u8);
                    e.u32(f.province);
                }
                let mut empty: Vec<u32> = world
                    .provinces()
                    .iter()
                    .filter(|p| reg.province_setup(p.id) == ProvinceFacilitySetup::ConfirmedEmpty)
                    .map(|p| p.id.0)
                    .collect();
                empty.sort_unstable();
                e.count(empty.len());
                for id in empty {
                    e.u32(id);
                }
            }
        },
        ItemKey::EconomyRules => e.u32(ECONOMY_RULES_REVISION),
    }
    e.finish()
}

// ---------------------------------------------------------------- ファイル上の識別情報

/// ファイルに保存する識別情報(未知のフィールドは拒否。項目の欠落は判定で拒否)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedIdentity {
    pub scheme: String,
    pub scheme_version: u32,
    pub items: Vec<SavedItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedItem {
    pub key: String,
    pub digest: String,
}

// ---------------------------------------------------------------- 判定

/// 項目の不一致(`RULES` の根拠つき)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub key: ItemKey,
    pub saved: String,
    pub current: String,
    /// 判定表の根拠。
    pub rationale: &'static str,
    pub impact: &'static str,
}

/// 識別情報そのものが使えない理由(項目の比較へ進む前に、または比較と併せて拒否する)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Structural {
    UnsupportedScheme { found: String },
    UnsupportedSchemeVersion { found: u32 },
    MissingItem(ItemKey),
    UnknownItem(String),
    DuplicateItem(String),
    MalformedDigest { key: String },
}

impl std::fmt::Display for Structural {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Structural::UnsupportedScheme { found } => {
                write!(f, "unsupported identity scheme \"{found}\" (supported: \"{IDENTITY_SCHEME}\")")
            }
            Structural::UnsupportedSchemeVersion { found } => {
                write!(f, "unsupported identity scheme version {found} (supported: {IDENTITY_SCHEME_VERSION})")
            }
            Structural::MissingItem(k) => write!(f, "required identity item `{k}` is missing"),
            Structural::UnknownItem(k) => write!(f, "identity item `{k}` is not in the judgement table (unknown items are refused)"),
            Structural::DuplicateItem(k) => write!(f, "identity item `{k}` appears more than once"),
            Structural::MalformedDigest { key } => write!(f, "identity item `{key}` has a malformed digest (expected 64 lowercase hex digits)"),
        }
    }
}

/// 判定の結果。`is_compatible()` が偽なら復元しない。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Verdict {
    /// 一致した項目(判定表の順)。
    pub matched: Vec<ItemKey>,
    /// 一致しないが、判定表が警告で許す項目。復元できるが、呼び出し側へ返す。
    pub warnings: Vec<Finding>,
    /// 一致せず、拒否の項目。
    pub rejections: Vec<Finding>,
    pub structural: Vec<Structural>,
}

impl Verdict {
    pub fn is_compatible(&self) -> bool {
        self.rejections.is_empty() && self.structural.is_empty()
    }
    pub fn is_exact(&self) -> bool {
        self.is_compatible() && self.warnings.is_empty()
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        let mut sep = |f: &mut std::fmt::Formatter<'_>| -> std::fmt::Result {
            if !first {
                f.write_str("; ")?;
            }
            first = false;
            Ok(())
        };
        for s in &self.structural {
            sep(f)?;
            write!(f, "{s}")?;
        }
        for r in &self.rejections {
            sep(f)?;
            write!(f, "`{}` differs: {}", r.key, r.rationale)?;
        }
        Ok(())
    }
}

fn is_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 保存側の識別情報を、復元先の識別情報と項目別に比べる。**ファイル内の値は復元先の値として採用しない。**
pub fn judge(saved: &SavedIdentity, current: &StaticIdentity) -> Verdict {
    let mut v = Verdict::default();
    if saved.scheme != IDENTITY_SCHEME {
        v.structural.push(Structural::UnsupportedScheme { found: saved.scheme.clone() });
        return v;
    }
    if saved.scheme_version != IDENTITY_SCHEME_VERSION {
        v.structural.push(Structural::UnsupportedSchemeVersion { found: saved.scheme_version });
        return v;
    }
    let mut seen: BTreeMap<ItemKey, &str> = BTreeMap::new();
    for item in &saved.items {
        match ItemKey::parse(&item.key) {
            None => v.structural.push(Structural::UnknownItem(item.key.clone())),
            Some(k) => {
                if !is_digest(&item.digest) {
                    v.structural.push(Structural::MalformedDigest { key: item.key.clone() });
                } else if seen.insert(k, item.digest.as_str()).is_some() {
                    v.structural.push(Structural::DuplicateItem(item.key.clone()));
                }
            }
        }
    }
    for key in ItemKey::ALL {
        let Some(saved_digest) = seen.get(&key) else {
            if !saved.items.iter().any(|i| i.key == key.name()) {
                v.structural.push(Structural::MissingItem(key));
            }
            continue;
        };
        let cur = current.digest(key).unwrap_or_default();
        if *saved_digest == cur {
            v.matched.push(key);
            continue;
        }
        let r = rule_of(key);
        let finding = Finding {
            key,
            saved: saved_digest.to_string(),
            current: cur.to_string(),
            rationale: r.rationale,
            impact: r.impact,
        };
        match r.class {
            Class::Warn => v.warnings.push(finding),
            Class::Reject => v.rejections.push(finding),
        }
    }
    v
}
