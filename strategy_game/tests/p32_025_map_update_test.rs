//! P32-025: 地図更新(`hex-atlas-map_2.json`、schemaVersion 10)の完了確認。
//! 更新後の地図の読込と検証、既存区画(ID・所属)の維持、m4d/m4e の組み立てと経済進行、新地図での保存・復元・継続、
//! 旧地図で作った保存(`tests/fixtures/p32_025/old_map_*.mapeco.ron`、更新前の静的入力から生成した固定fixture)
//! の判定(既存方針どおり拒否されること)。ユーザーの保存は使わない。生成ファイルはすべて一時ディレクトリ。

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use strategy_game::economy::map_economy_file::{FileError, SaveLocation};
use strategy_game::economy::map_session::{EconomySession, GeographyData, SessionPaths, scenario_by_id};
use strategy_game::economy::map_state::static_identity::ItemKey;
use strategy_game::hexmap::data::{HexMapError, parse_hex_map};
use strategy_game::world::{MapStateId, ProvinceId};

fn geo() -> &'static GeographyData {
    static G: OnceLock<GeographyData> = OnceLock::new();
    G.get_or_init(|| GeographyData::load(&SessionPaths::default()).unwrap())
}

fn session(id: &str) -> EconomySession {
    EconomySession::load_files(scenario_by_id(id).unwrap(), geo().model(), &SessionPaths::default()).unwrap()
}

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("p32_025_{tag}_{}_{nanos}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn loc(&self) -> SaveLocation {
        SaveLocation::new(&self.0)
    }
    fn dir(&self) -> PathBuf {
        self.0.join("map_economy")
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------- 更新後の地図

#[test]
fn the_updated_map_loads_with_the_expected_facts() {
    let map = &geo().map;
    assert_eq!(map.cells().len(), 96_760, "same grid and cell count as before the update");
    let land = map.cells().iter().filter(|c| !c.terrain.is_water()).count();
    assert_eq!(land, 35_221, "34,503 land cells before + 718 sea/lake cells that became land, none lost");
}

#[test]
fn unknown_schema_versions_are_still_rejected() {
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read("assets/data/hex_atlas_map.json").unwrap()).unwrap();
    assert_eq!(v["schemaVersion"], 10);
    for bad in [8u32, 11, 0] {
        v["schemaVersion"] = serde_json::json!(bad);
        let r = parse_hex_map(serde_json::to_string(&v).unwrap().as_bytes());
        assert!(matches!(r, Err(HexMapError::UnsupportedSchema(x)) if x == bad), "schema {bad} must be rejected");
    }
    // 9も受け付ける(従来形式)
    v["schemaVersion"] = serde_json::json!(9);
    assert!(parse_hex_map(serde_json::to_string(&v).unwrap().as_bytes()).is_ok());
}

#[test]
fn the_preview_region_is_unchanged_and_ids_are_kept() {
    let m = geo().model();
    // 検証地域の29プロヴィンス・3ステートは存続し、IDは据え置き。地図更新で新しいIDは発行していない。
    assert_eq!(m.provinces().len(), 29);
    assert_eq!(m.states().len(), 3);
    assert_eq!(m.next_province_id(), 34_503);
    let ids: BTreeSet<u32> = m.provinces().iter().map(|p| p.id.0).collect();
    assert_eq!(ids.len(), 29);
    assert_eq!(m.province_at(83, 78).unwrap().id, ProvinceId(17266));
    assert_eq!(m.state_of(ProvinceId(17266)), Some(MapStateId(22)));
    // 各ステートのプロヴィンスは連結(既存の検証がAuthoringで通っていること)
    assert!(geo().preview.report.is_ok());
}

// ---------------------------------------------------------------- m4d/m4e の組み立てと経済進行、保存・復元・継続

#[test]
fn m4d_and_m4e_assemble_and_run_on_the_updated_map() {
    for id in ["m4d", "m4e"] {
        let mut s = session(id);
        let start = s.sim.date;
        for _ in 0..90 {
            s.advance_day();
        }
        assert!(s.sim.date > start, "{id}: advanced");
        assert!(!s.sim.registry.is_empty(), "{id}: has states");
    }
}

#[test]
fn save_restore_and_continuation_work_on_the_updated_map() {
    for id in ["m4d", "m4e"] {
        let tmp = Tmp::new(id);
        let mut a = session(id);
        for _ in 0..45 {
            a.advance_day();
        }
        let r = a.save(&geo().map, geo().model(), &tmp.loc(), "new_map").unwrap();
        assert!(r.path.exists());
        // 保存した状態へ、別の(新規)セッションで復元
        let mut b = session(id);
        let report = a.inspect(&geo().map, geo().model(), &tmp.loc(), "new_map").unwrap();
        assert!(report.verdict.is_exact(), "{id}: a save made on this map is an exact match");
        assert!(matches!(report.restorable, Some(Ok(()))));
        b.load(&geo().map, geo().model(), &tmp.loc(), "new_map").unwrap();
        assert!(a.sim == b.sim, "{id}: restored state is identical");
        for _ in 0..60 {
            a.advance_day();
            b.advance_day();
        }
        assert!(a.sim == b.sim, "{id}: continuation is identical to the uninterrupted run");
    }
}

// ---------------------------------------------------------------- 旧地図で作った保存

const OLD_FIXTURES: [(&str, &str); 2] = [
    ("m4d", "tests/fixtures/p32_025/old_map_m4d.mapeco.ron"),
    ("m4e", "tests/fixtures/p32_025/old_map_m4e.mapeco.ron"),
];

#[test]
fn saves_made_on_the_old_map_are_rejected_without_changing_anything() {
    for (id, fixture) in OLD_FIXTURES {
        let tmp = Tmp::new(&format!("old_{id}"));
        fs::create_dir_all(tmp.dir()).unwrap();
        let bytes = fs::read(fixture).unwrap();
        fs::write(tmp.dir().join("old_map.mapeco.ron"), &bytes).unwrap();
        let mut s = session(id);
        for _ in 0..10 {
            s.advance_day();
        }
        let before = s.sim.clone();

        // inspect: 確認自体は成功し、拒否の理由が返る。復元可能性の確認までは進まない
        let rep = s.inspect(&geo().map, geo().model(), &tmp.loc(), "old_map").unwrap();
        assert!(!rep.verdict.is_compatible(), "{id}: old-map save must be rejected");
        assert!(rep.restorable.is_none());
        let rejected: BTreeSet<ItemKey> = rep.verdict.rejections.iter().map(|f| f.key).collect();
        assert!(rejected.contains(&ItemKey::LandSet), "{id}: land set changed -> rejected ({rejected:?})");
        assert!(rejected.contains(&ItemKey::WaterKinds), "{id}: lake count changed -> rejected ({rejected:?})");
        // IDと所属・シナリオは維持されているので、そこは一致する(ただし互換の根拠にはならない)
        for k in [ItemKey::ProvinceTable, ItemKey::StateMembership, ItemKey::ScenarioStructure, ItemKey::Grid, ItemKey::CellCoordinates] {
            assert!(rep.verdict.matched.contains(&k), "{id}: {k:?} matches");
        }
        assert!(!rep.verdict.to_string().is_empty());

        // load: 拒否され、現在の状態は変わらない
        let err = s.load(&geo().map, geo().model(), &tmp.loc(), "old_map").unwrap_err();
        assert!(matches!(err, FileError::Incompatible(_)), "{id}: {err:?}");
        assert!(s.sim == before, "{id}: the current state is untouched by the rejected load");

        // ファイルも変わらない
        assert_eq!(fs::read(tmp.dir().join("old_map.mapeco.ron")).unwrap(), bytes);
        assert_eq!(fs::read_dir(tmp.dir()).unwrap().count(), 1);
    }
}
