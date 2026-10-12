use crate::app::game_state::GameState;
use crate::app::time::GameDate;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::economy::resources::ResourceType;
use crate::economy::supply_balance::{
    MilitarySupplyBalanceView, MilitarySupplyProductionLedger, MonthlyProductionRecord,
    MonthlySupplyProduction, StockDays, build_military_supply_balance_view,
};
use crate::military::data::MilitaryRegistry;
use crate::localization::{
    CurrentLocale, LocalizedText, TranslationCatalog, localized_text, t, tf,
};
use crate::state::data::StateRegistry;
use crate::ui::notification::NotificationHistory;
use bevy::prelude::*;

#[derive(Component)]
pub struct EconomyPanelRoot;

#[derive(Component)]
pub struct EconomyPanelText;

#[derive(Component)]
pub struct TaxUpButton;

#[derive(Component)]
pub struct TaxDownButton;

pub struct EconomyPanelPlugin;

impl Plugin for EconomyPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), setup_economy_panel)
            .add_systems(
                Update,
                (update_economy_panel, handle_tax_buttons).run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_economy_panel(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    commands
        .spawn((
            EconomyPanelRoot,
            // P21-013: 背景自体もButton化し、子Button以外の余白をクリック/ホバーしても
            // `Interaction`を確実に発行させる(`ui::load_confirm`の既存パターンを踏襲)。
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(40.0),
                bottom: Val::Px(0.0),
                width: Val::Px(300.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.1, 0.85)),
        ))
        .with_children(|parent| {
            let (text, marker) = localized_text(&catalog, locale.0, "economy_panel.title", vec![]);
            parent.spawn((
                text,
                marker,
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.85, 0.6)),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
            ));

            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    let (text, marker) =
                        localized_text(&catalog, locale.0, "economy_panel.tax_rate_label", vec![]);
                    row.spawn((
                        text,
                        marker,
                        TextLayout {
                            linebreak: LineBreak::AnyCharacter,
                            ..default()
                        },
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                    ));

                    row.spawn((
                        TaxDownButton,
                        Button,
                        Node {
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("-1%"),
                            TextLayout {
                                linebreak: LineBreak::AnyCharacter,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(12.0),
                                ..default()
                            },
                        ));
                    });

                    row.spawn((
                        TaxUpButton,
                        Button,
                        Node {
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("+1%"),
                            TextLayout {
                                linebreak: LineBreak::AnyCharacter,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(12.0),
                                ..default()
                            },
                        ));
                    });
                });

            let (text, marker) =
                localized_text(&catalog, locale.0, "economy_panel.loading", vec![]);
            parent.spawn((
                EconomyPanelText,
                text,
                marker,
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.9)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));
        });
}

/// P29-002: 軍用補給物資(MilitarySupplies)の「前回の月次生産」「現在の需要(日次)」
/// 「備蓄日数」を、ローカライズ済みの複数行テキストへ整形する(表示専用、副作用なし)。
/// 備蓄日数は国家備蓄の日数であり、各Divisionへの配送充足率とは別である(注記を添える)。
pub fn format_military_supply_balance(
    view: &MilitarySupplyBalanceView,
    catalog: &TranslationCatalog,
    locale: crate::localization::Locale,
) -> String {
    let mut out = t(catalog, locale, "economy_panel.supply_header");

    match view.last_production {
        Some(MonthlySupplyProduction {
            year,
            month,
            record: MonthlyProductionRecord::Produced(amount),
        }) => out.push_str(&tf(
            catalog,
            locale,
            "economy_panel.supply_production",
            vec![
                ("period", format!("{}/{:02}", year, month)),
                ("amount", format!("{:.1}", amount)),
            ],
        )),
        _ => out.push_str(&t(catalog, locale, "economy_panel.supply_production_pending")),
    }

    match view.daily_demand {
        Some(demand) => out.push_str(&tf(
            catalog,
            locale,
            "economy_panel.supply_demand",
            vec![("amount", format!("{:.1}", demand))],
        )),
        None => out.push_str(&t(catalog, locale, "economy_panel.supply_demand_unavailable")),
    }

    match view.stock_days {
        StockDays::Days(days) => out.push_str(&tf(
            catalog,
            locale,
            "economy_panel.supply_days",
            vec![("days", format!("{:.1}", days))],
        )),
        StockDays::NoConsumption => {
            out.push_str(&t(catalog, locale, "economy_panel.supply_days_none"))
        }
        StockDays::Unavailable => {
            out.push_str(&t(catalog, locale, "economy_panel.supply_days_unavailable"))
        }
    }

    out.push_str(&t(catalog, locale, "economy_panel.supply_note"));
    out
}

#[allow(clippy::too_many_arguments)]
fn update_economy_panel(
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    notif_history: Res<NotificationHistory>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    military_registry: Res<MilitaryRegistry>,
    supply_ledger: Res<MilitarySupplyProductionLedger>,
    date: Res<GameDate>,
    mut supply_text_cache: Local<Option<String>>,
    mut text_q: Query<(&mut Text, &mut LocalizedText), With<EconomyPanelText>>,
) {
    let Ok((mut text, mut marker)) = text_q.single_mut() else {
        return;
    };

    let Some(country_id) = player_country.0 else {
        let key = "economy_panel.no_country";
        let rendered = t(&catalog, locale.0, key);
        if text.0 != rendered {
            *text = Text::new(rendered);
        }
        marker.key = key;
        marker.args = vec![];
        return;
    };

    let Some(country) = country_registry.get(country_id) else {
        return;
    };

    let mut stockpile_str = String::new();
    for res in ResourceType::ALL {
        let amount = country.stockpile.get(res);
        stockpile_str.push_str(&tf(
            &catalog,
            locale.0,
            "economy_panel.stockpile_line",
            vec![
                ("resource", t(&catalog, locale.0, res.display_name())),
                ("amount", format!("{:.0}", amount)),
            ],
        ));
    }

    // P29-002: 軍用補給物資の収支。日付・軍事登録・実績・言語・プレイヤー国家が変わった
    // ときだけ再計算する(毎フレームの全Division走査を避ける)。国家備蓄・需要・実績は同じ
    // フレームのスナップショットから読むため、異なる日の値は混在しない。
    let needs_supply_refresh = supply_text_cache.is_none()
        || date.is_changed()
        || military_registry.is_changed()
        || supply_ledger.is_changed()
        || locale.is_changed()
        || player_country.is_changed();
    if needs_supply_refresh {
        let view = build_military_supply_balance_view(
            country_id,
            country.stockpile.get(ResourceType::MilitarySupplies),
            &military_registry.divisions,
            &military_registry.definitions,
            &supply_ledger,
        );
        *supply_text_cache = Some(format_military_supply_balance(&view, &catalog, locale.0));
    }
    let supply_balance_str = supply_text_cache.clone().unwrap_or_default();

    let mut queue_str = String::new();
    if country.construction_queue.is_empty() {
        queue_str.push_str(&t(&catalog, locale.0, "economy_panel.queue_empty"));
    } else {
        for (i, item) in country.construction_queue.iter().enumerate() {
            let state_name = state_registry
                .get(item.state_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
            queue_str.push_str(&tf(
                &catalog,
                locale.0,
                "economy_panel.queue_line",
                vec![
                    ("index", (i + 1).to_string()),
                    (
                        "building",
                        t(&catalog, locale.0, item.building_type.display_name()),
                    ),
                    ("state", state_name),
                    ("progress", format!("{:.0}", item.progress)),
                    ("required", format!("{:.0}", item.required_progress)),
                ],
            ));
        }
    }

    let last_notif = notif_history
        .recent
        .last()
        .cloned()
        .unwrap_or_else(|| t(&catalog, locale.0, "common.none"));

    let args = vec![
        (
            "state",
            t(
                &catalog,
                locale.0,
                country.economic_state.display_name_short(),
            ),
        ),
        ("tax_rate", format!("{:.1}", country.tax_rate * 100.0)),
        ("income", format!("{:.1}", country.monthly_income)),
        ("expenses", format!("{:.1}", country.monthly_expenses)),
        ("balance", format!("{:.1}", country.monthly_balance)),
        (
            "sci_cap",
            format!("{:.1}", country.science_research_capacity),
        ),
        ("mag_cap", format!("{:.1}", country.magic_research_capacity)),
        ("stockpile", stockpile_str),
        ("supply_balance", supply_balance_str),
        ("queue", queue_str),
        ("last_notification", last_notif),
    ];
    let info = tf(&catalog, locale.0, "economy_panel.info", args.clone());

    if text.0 != info {
        *text = Text::new(info);
    }
    marker.key = "economy_panel.info";
    marker.args = args;
}

fn handle_tax_buttons(
    up_q: Query<&Interaction, (With<TaxUpButton>, Changed<Interaction>)>,
    down_q: Query<&Interaction, (With<TaxDownButton>, Changed<Interaction>)>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
) {
    let Some(country_id) = player_country.0 else {
        return;
    };
    let Some(country) = country_registry.get_mut(country_id) else {
        return;
    };

    for interaction in up_q.iter() {
        if *interaction == Interaction::Pressed {
            country.tax_rate = crate::economy::finance::apply_tax_rate_delta(
                country.tax_rate,
                crate::economy::finance::TAX_RATE_STEP,
            );
        }
    }
    for interaction in down_q.iter() {
        if *interaction == Interaction::Pressed {
            country.tax_rate = crate::economy::finance::apply_tax_rate_delta(
                country.tax_rate,
                -crate::economy::finance::TAX_RATE_STEP,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P21-013: `EconomyPanelRoot`自身が`Button`であることを確認する回帰テスト。これにより
    /// 子Button(税率+/-等)以外の余白をクリック/ホバーしても`Interaction`が確実に発行され、
    /// `map::selection::handle_state_click`等の既存「UIのHovered/Pressed中はマップ操作を
    /// スキップする」ガードがこの領域にも効くようになる(`ui::load_confirm`の既存パターンと
    /// 同じ)。`EconomyPanelRoot`は常時表示の左サイドバーであり、マップ左端に常に重なるため、
    /// この抜け穴の実質的な主要因だったと考えられる。
    #[test]
    fn economy_panel_root_background_is_itself_a_button() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, setup_economy_panel);
        app.update();

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<EconomyPanelRoot>>()
            .single(app.world())
            .expect("EconomyPanelRoot must be spawned");
        assert!(
            app.world().entity(root).contains::<Button>(),
            "EconomyPanelRoot's own background must be a Button so hovering it registers Interaction"
        );
    }

    // ─── P29-002: 軍用補給物資の収支表示 ───────────────────────────────────────

    use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
    use crate::country::CountryData;
    use crate::localization::Locale;
    use crate::military::data::{
        Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType,
    };

    fn supply_def() -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(0),
            name: "t".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment: 1000.0,
            recruitment_days: 30,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 5.0,
            organization: 50.0,
            morale: 50.0,
            supply_usage: 1.0,
            maintenance_cost: 5.0,
        }
    }

    fn supply_division(id: usize, owner: usize, manpower: u64) -> Division {
        Division {
            id: DivisionId(id),
            owner: CountryId(owner),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(0),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower: 10_000,
            equipment: 1000.0,
            max_equipment: 1000.0,
            organization: 50.0,
            max_organization: 50.0,
            morale: 50.0,
            max_morale: 50.0,
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

    fn supply_app(locale: Locale) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale(locale));
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.insert_resource(PlayerCountry(Some(CountryId(1))));
        let mut c1 = CountryData {
            id: CountryId(1),
            ..CountryData::default()
        };
        c1.stockpile.set(ResourceType::MilitarySupplies, 180.0);
        c1.stockpile.set(ResourceType::MilitaryEquipment, 16.0);
        let mut c2 = CountryData {
            id: CountryId(2),
            ..CountryData::default()
        };
        c2.stockpile.set(ResourceType::MilitarySupplies, 30.0);
        app.insert_resource(CountryRegistry {
            countries: vec![c1, c2],
        });
        app.insert_resource(StateRegistry::default());
        app.insert_resource(NotificationHistory::default());
        app.insert_resource(GameDate::default());
        let mut mil = MilitaryRegistry::default();
        mil.definitions.insert(DivisionDefinitionId(0), supply_def());
        for (id, owner) in [(0, 1), (1, 1), (2, 2)] {
            mil.divisions
                .insert(DivisionId(id), supply_division(id, owner, 10_000));
        }
        app.insert_resource(mil);
        app.insert_resource(MilitarySupplyProductionLedger::default());
        app.add_systems(Startup, setup_economy_panel);
        app.add_systems(Update, update_economy_panel);
        app.update();
        app
    }

    fn panel_text(app: &mut App) -> String {
        app.world_mut()
            .query_filtered::<&Text, With<EconomyPanelText>>()
            .single(app.world())
            .expect("economy panel text")
            .0
            .clone()
    }

    #[test]
    fn supply_balance_shows_production_demand_and_stock_days_in_japanese() {
        let mut app = supply_app(Locale::JaJp);
        app.world_mut()
            .resource_mut::<MilitarySupplyProductionLedger>()
            .record(CountryId(1), 1800, 3, Some(36.0));
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("[軍用補給物資の収支]"), "{text}");
        assert!(text.contains("前回の月次生産(実績・1800/03分): 36.0"), "{text}");
        assert!(text.contains("現在の需要: 2.0 / 日"), "{text}");
        assert!(text.contains("備蓄: 90.0日分"), "{text}");
        assert!(text.contains("補給充足率とは別"), "{text}");
    }

    #[test]
    fn supply_balance_is_available_in_english_and_names_military_supplies() {
        let mut app = supply_app(Locale::EnUs);
        app.world_mut()
            .resource_mut::<MilitarySupplyProductionLedger>()
            .record(CountryId(1), 1800, 3, Some(36.0));
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("[Military Supplies Balance]"), "{text}");
        assert!(text.contains("Last monthly production (actual, 1800/03): 36.0"), "{text}");
        assert!(text.contains("Current demand: 2.0 / day"), "{text}");
        assert!(text.contains("Stock: 90.0 days"), "{text}");
    }

    #[test]
    fn missing_or_failed_production_is_pending_not_zero() {
        let mut app = supply_app(Locale::JaJp);
        let text = panel_text(&mut app);
        assert!(text.contains("前回の月次生産(実績): 未集計"), "{text}");
        assert!(!text.contains("実績・"), "{text}");
        app.world_mut()
            .resource_mut::<MilitarySupplyProductionLedger>()
            .record(CountryId(1), 1800, 3, None);
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("未集計"), "{text}");
        app.world_mut()
            .resource_mut::<MilitarySupplyProductionLedger>()
            .record(CountryId(1), 1800, 4, Some(0.0));
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("前回の月次生産(実績・1800/04分): 0.0"), "{text}");
    }

    #[test]
    fn display_follows_date_stock_and_division_changes_without_any_click() {
        let mut app = supply_app(Locale::JaJp);
        assert!(panel_text(&mut app).contains("備蓄: 90.0日分"));

        // 日次消費で備蓄が減る(日付が進む)→ 日数が更新される。
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .countries[0]
            .stockpile
            .set(ResourceType::MilitarySupplies, 100.0);
        app.world_mut().resource_mut::<GameDate>().day = 2;
        app.update();
        assert!(panel_text(&mut app).contains("備蓄: 50.0日分"));

        // 募兵完成(Division +1)→ 需要が増え、日数が減る。
        app.world_mut()
            .resource_mut::<MilitaryRegistry>()
            .divisions
            .insert(DivisionId(9), supply_division(9, 1, 10_000));
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("現在の需要: 3.0 / 日"), "{text}");
        assert!(text.contains("備蓄: 33.3日分"), "{text}");

        // Division消滅 → 需要が減る。
        {
            let mut mil = app.world_mut().resource_mut::<MilitaryRegistry>();
            mil.divisions.remove(&DivisionId(9));
            mil.divisions.remove(&DivisionId(1));
        }
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("現在の需要: 1.0 / 日"), "{text}");
        assert!(text.contains("備蓄: 100.0日分"), "{text}");
    }

    #[test]
    fn empty_stock_still_shows_demand_and_zero_demand_is_no_consumption() {
        let mut app = supply_app(Locale::JaJp);
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .countries[0]
            .stockpile
            .set(ResourceType::MilitarySupplies, 0.0);
        app.world_mut().resource_mut::<GameDate>().day = 2;
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("現在の需要: 2.0 / 日"), "demand is not zeroed by an empty stock: {text}");
        assert!(text.contains("備蓄: 0.0日分"), "{text}");

        app.world_mut()
            .resource_mut::<MilitaryRegistry>()
            .divisions
            .clear();
        app.world_mut().resource_mut::<GameDate>().day = 3;
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("現在の需要: 0.0 / 日"), "{text}");
        assert!(text.contains("備蓄: 消費なし"), "{text}");
        assert!(!text.contains("NaN") && !text.contains("inf"), "{text}");
    }

    #[test]
    fn switching_the_player_country_shows_only_that_countrys_values() {
        let mut app = supply_app(Locale::JaJp);
        {
            let mut ledger = app.world_mut().resource_mut::<MilitarySupplyProductionLedger>();
            ledger.record(CountryId(1), 1800, 3, Some(36.0));
            ledger.record(CountryId(2), 1800, 3, Some(500.0));
        }
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("実績・1800/03分): 36.0") && !text.contains("500.0"), "{text}");

        app.world_mut().insert_resource(PlayerCountry(Some(CountryId(2))));
        app.update();
        let text = panel_text(&mut app);
        assert!(text.contains("実績・1800/03分): 500.0") && !text.contains("36.0"), "{text}");
        assert!(text.contains("現在の需要: 1.0 / 日"), "{text}");
        assert!(text.contains("備蓄: 30.0日分"), "{text}");
    }

    #[test]
    fn locale_toggle_updates_the_supply_block() {
        let mut app = supply_app(Locale::JaJp);
        assert!(panel_text(&mut app).contains("[軍用補給物資の収支]"));
        app.insert_resource(CurrentLocale(Locale::EnUs));
        app.update();
        assert!(panel_text(&mut app).contains("[Military Supplies Balance]"));
    }

    #[test]
    fn updating_the_panel_has_no_side_effects_on_resources_or_divisions() {
        let mut app = supply_app(Locale::JaJp);
        let snapshot = |app: &App| {
            let countries: Vec<(usize, Vec<u64>)> = app
                .world()
                .resource::<CountryRegistry>()
                .countries
                .iter()
                .map(|c| {
                    (
                        c.id.0,
                        ResourceType::ALL
                            .iter()
                            .map(|r| c.stockpile.get(*r).to_bits())
                            .collect(),
                    )
                })
                .collect();
            let mut divisions: Vec<(usize, u64, u32)> = app
                .world()
                .resource::<MilitaryRegistry>()
                .divisions
                .values()
                .map(|d| (d.id.0, d.manpower, d.supply_ratio.to_bits()))
                .collect();
            divisions.sort();
            let date = app.world().resource::<GameDate>().display();
            (countries, divisions, date)
        };
        let before = snapshot(&app);
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(before, snapshot(&app));
    }

}
