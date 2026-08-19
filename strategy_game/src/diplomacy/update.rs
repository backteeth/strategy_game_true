use crate::app::time::{DayChangedMessage, GameDate};
use crate::common::{CountryId, DiplomaticCrisisId};
use crate::country::power::{CountryPowerRegistry, PowerTier};
use crate::country::{CountryData, CountryRegistry, PlayerCountry};
use crate::diplomacy::ai::calculate_demand_acceptance;
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::{
    CrisisPhase, CrisisRegistry, DiplomaticCrisis, ThirdCountryReaction,
};
use crate::diplomacy::crisis_response;
use crate::diplomacy::crisis_response::CrisisSupportSide;
use crate::diplomacy::data::{DiplomacyRegistry, DiplomaticPairKey, TreatyType};
use crate::localization::{CurrentLocale, TranslationCatalog, t, tf};
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use crate::war::justification::WarJustificationRegistry;
use bevy::prelude::*;
use std::collections::HashMap;

pub const ACTIVITY_DURATION_DAYS: u32 = 30;
pub const COOLDOWN_DURATION_DAYS: u32 = 30;
/// P21-015: AI大国のCrisis支持判断における関係値差のしきい値
/// (`relation(ai, initiator) - relation(ai, target)`)。この値以上なら要求国側、
/// この値以下(負)なら対象国側を支持する。ちょうど境界値も支持する側に含める。
pub const AI_CRISIS_SUPPORT_RELATION_MARGIN: f32 = 25.0;

/// P21-015: `evaluate_ai_crisis_support`が実際に何回呼ばれたかを直接検証するための
/// 診断専用Resource。本番コードから挿入されることは無い(`handle_daily_diplomacy`は
/// `Option<ResMut<_>>`として受け取るため、Worldに存在しなければ静かに`None`となり
/// 一切のオーバーヘッドを生まない)。テストがこのResourceを明示的に`insert_resource`
/// した場合にのみ、支持判断を実行するたびに1ずつ増分される — 「同一フレームに
/// 複数の`DayChangedMessage`があっても支持判断はフレームにつき最大1回」という要件を、
/// 結果(`third_party_reactions`の中身)だけでなく実際の呼び出し回数そのもので
/// 検証できるようにする。
#[derive(Resource, Default, Debug)]
pub struct AiSupportEvaluationCount(pub usize);

#[allow(clippy::too_many_arguments)]
pub fn handle_daily_diplomacy(
    mut day_events: MessageReader<DayChangedMessage>,
    mut diplomacy_registry: ResMut<DiplomacyRegistry>,
    player_country: Res<PlayerCountry>,
    mut notif_writer: MessageWriter<GameNotification>,
    country_registry: Res<CountryRegistry>,
    date: Res<GameDate>,
    mut justification_registry: ResMut<WarJustificationRegistry>,
    mut state_registry: ResMut<crate::state::data::StateRegistry>,
    mut crisis_registry: ResMut<CrisisRegistry>,
    mut claim_registry: ResMut<ClaimRegistry>,
    power_registry: Res<CountryPowerRegistry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut support_call_counter: Option<ResMut<AiSupportEvaluationCount>>,
) {
    // P21-015修正: 3aのAI支持判断だけは、同一フレームに`DayChangedMessage`が複数件
    // (高速進行等で1フレームに複数日が進んだ場合)積まれていても、フレームにつき
    // 最大1回しか実行してはならない([要求仕様]「同一フレームで全Crisis評価は
    // 最大1回」)。0/3/3b/3c(正当化進行・days_in_phase増分・P21-012応答・P21-011
    // 期限処理)は既存どおり日ごとに(=`day_events.read()`のイベント1件につき1回)
    // 実行し続ける必要があるため、このガードは3aの呼び出しだけを覆う
    // (`day_events.read()`ループ自体の構造・既存ステップの実行回数は変更しない)。
    let mut ai_support_evaluated_this_frame = false;

    for _event in day_events.read() {
        // 0. 正当化の日次進行 (CountryAiより前に完了判定を行うため)
        justification_registry.process_daily_justifications(&state_registry);

        // 3. 外交危機の日次進行 (P21-010: 既存の状態遷移規則が存在しないため、
        // 接続範囲は`days_in_phase`のインクリメントのみに限定する。フェーズ自動遷移・
        // 自動終結・自動宣戦は実装しない[対象外: 新しい外交ルール]。terminal state
        // [ResolvedPeacefully/WarStarted/Cancelled]は既存実装どおり日数を進めない)。
        for crisis in crisis_registry.crises.values_mut() {
            if !matches!(
                crisis.current_phase,
                CrisisPhase::ResolvedPeacefully | CrisisPhase::WarStarted | CrisisPhase::Cancelled
            ) {
                crisis.days_in_phase += 1;
            }
        }

        // 3a. P21-015: PowerTier::GreatPowerのAI国家によるCrisis支持判断。
        // 「日付更新 → AI大国のCrisis支持判断 → P21-012のAI対象国回答 → Crisis期限切れ等」
        // という要求仕様の実行順序を、System登録順ではなくこの関数内の逐次呼び出し順で
        // 明示的に保証する(day_events.read()の同一ループ内、3bより必ず前)。これにより
        // 同日中に新規追加された支持は、直後に実行される3bのP21-012受諾スコア計算
        // (`crisis_support_adjusted_power`が`third_party_reactions`を都度読み直す)から
        // 必ず見える。`ai_support_evaluated_this_frame`ガードにより、同一フレームに
        // 複数の`DayChangedMessage`があってもこの呼び出しはフレームにつき1回だけになる
        // (2回目以降のイベントではスキップされ、3b/3cは従来どおり毎イベント実行される)。
        if !ai_support_evaluated_this_frame {
            evaluate_ai_crisis_support(
                &mut crisis_registry,
                &diplomacy_registry,
                &country_registry,
                &power_registry,
                &player_country,
                &date,
            );
            ai_support_evaluated_this_frame = true;
            if let Some(counter) = support_call_counter.as_mut() {
                counter.0 += 1;
            }
        }

        // 3b. P21-012: AI対象国による最後通牒への自動応答(期限前のDemandSentのみ)。
        // プレイヤーがtargetの場合は絶対に自動応答しない(P21-011のUIによる手動判断を維持)。
        // 期限に達したCrisisはここでは一切触れず、次の3cブロック(既存のP21-011タイムアウト
        // 処理)だけが処理する — 呼び出し順が入れ替わっても結果が変わらないよう、
        // 「期限前」であることをこのブロック自身の対象条件に含めている。
        evaluate_ai_crisis_responses(
            &mut crisis_registry,
            &mut claim_registry,
            &mut state_registry,
            &mut justification_registry,
            &diplomacy_registry,
            &country_registry,
            &player_country,
            &date,
            &mut notif_writer,
            &locale,
            &catalog,
        );

        // 3c. P21-011: 要求期限(deadline_date)に達したDemandSent中のCrisisを自動拒否する。
        // targetが人間かAIかを問わず一律に扱う(3bで既にAI応答済みのCrisisはDemandSentで
        // なくなっているため、ここでは再度触れられない)。
        let expired_crisis_ids: Vec<_> = crisis_registry
            .crises
            .values()
            .filter(|c| {
                c.current_phase == CrisisPhase::DemandSent
                    && c.deadline_date
                        .as_deref()
                        .and_then(GameDate::from_string)
                        .is_some_and(|deadline| date.is_at_least(&deadline))
            })
            .map(|c| (c.id, c.initiator, c.target))
            .collect();

        for (crisis_id, initiator, target) in expired_crisis_ids {
            crisis_response::apply_rejection(
                &mut crisis_registry,
                &mut justification_registry,
                crisis_id,
                date.display(),
            );

            if player_country.0 == Some(initiator) {
                let target_name = country_registry
                    .get(target)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
                notif_writer.write(GameNotification {
                    message: tf(
                        &catalog,
                        locale.0,
                        "notif.crisis_demand_expired_initiator",
                        vec![("country", target_name)],
                    ),
                });
            } else if player_country.0 == Some(target) {
                let other_name = country_registry
                    .get(initiator)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
                notif_writer.write(GameNotification {
                    message: tf(
                        &catalog,
                        locale.0,
                        "notif.crisis_demand_expired_target",
                        vec![("country", other_name)],
                    ),
                });
            }
        }

        for (&key, relation) in diplomacy_registry.relations.iter_mut() {
            // 1. クールダウン減算
            let mut expired_cooldowns = Vec::new();
            for (&cid, cd) in relation.cooldowns.iter_mut() {
                if *cd > 0 {
                    *cd -= 1;
                }
                if *cd == 0 {
                    expired_cooldowns.push(cid);
                }
            }
            for cid in expired_cooldowns {
                relation.cooldowns.remove(&cid);
            }

            // 2. 実行中外交活動の進行
            let mut activity_finished = false;
            let mut finished_type = None;
            let mut initiator_cid = None;

            if let Some(ref mut act) = relation.active_activity {
                relation.opinion =
                    (relation.opinion + act.daily_opinion_change).clamp(-100.0, 100.0);
                if act.days_remaining > 0 {
                    act.days_remaining -= 1;
                }
                if act.days_remaining == 0 {
                    activity_finished = true;
                    finished_type = Some(act.activity_type);
                    initiator_cid = Some(act.initiator);
                }
            }

            if activity_finished {
                relation.active_activity = None;
                if let Some(init_id) = initiator_cid {
                    relation.cooldowns.insert(init_id, COOLDOWN_DURATION_DAYS);
                }
                relation.last_updated_date = date.display();

                if player_country.0 == Some(key.0) || player_country.0 == Some(key.1) {
                    let other_id = if player_country.0 == Some(key.0) {
                        key.1
                    } else {
                        key.0
                    };
                    let other_name = country_registry
                        .get(other_id)
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
                    let act_name = finished_type
                        .map(|ty| t(&catalog, locale.0, ty.display_name()))
                        .unwrap_or_else(|| {
                            t(&catalog, locale.0, "notif.diplomatic_activity_fallback")
                        });

                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "notif.diplomacy_finished",
                            vec![
                                ("activity", act_name),
                                ("country", other_name),
                                ("opinion", format!("{:.0}", relation.opinion)),
                            ],
                        ),
                    });
                }
            }
        }
    }
}

/// P21-012: 国家ごとの同盟国合計`available_manpower`を一度だけ計算する
/// (`calculate_demand_acceptance`の`target_allies_power`/`initiator_allies_power`引数の実体)。
/// crisis件数に対してO(1)で引けるよう、日次1回だけ`diplomacy_registry.relations`を
/// 1パスして構築する(crisis単位で毎回同盟関係を再スキャンしない)。`country_by_id`
/// (呼び出し元で既に構築済み)を使い、`CountryRegistry::get`のO(n)線形探索を避ける。
fn compute_ally_power_by_country(
    diplomacy_registry: &DiplomacyRegistry,
    country_by_id: &HashMap<CountryId, &CountryData>,
) -> HashMap<CountryId, f32> {
    let mut result: HashMap<CountryId, f32> = HashMap::new();
    for (key, relation) in diplomacy_registry.relations.iter() {
        if !relation.has_treaty(TreatyType::Alliance) {
            continue;
        }
        let DiplomaticPairKey(a, b) = *key;
        if let Some(country_b) = country_by_id.get(&b) {
            *result.entry(a).or_insert(0.0) += country_b.available_manpower as f32;
        }
        if let Some(country_a) = country_by_id.get(&a) {
            *result.entry(b).or_insert(0.0) += country_a.available_manpower as f32;
        }
    }
    result
}

/// P21-013: `ally_power_by_country`(同盟ベース、Crisis非依存)へ、このCrisis固有の
/// 明示的な第三国支持表明(`crisis.third_party_reactions`)を上書き反映した、
/// Crisis単位の支援戦力を計算する。`calculate_demand_acceptance`自体は一切変更しない
/// (このヘルパーが渡す`target_allies_power`/`initiator_allies_power`引数の中身だけを
/// 調整する)。
///
/// 二重計上防止: 明示的支持がある国家については、そのCrisisに限り明示した側だけへ
/// 計上し、反対側との同盟関係があっても両陣営へ二重加算しない。具体的には、支持国が
/// 同盟経由で既に`ally_power_by_country`のどちらか(または両方)に計上済みなら、
/// まずその分を差し引いてから、明示した側だけへ改めて加算する。明示的支持のない
/// 既存同盟国はこの関数に一切触れられないため、P21-012までの計算結果を変えない。
///
/// 性能: `crisis.third_party_reactions`はCrisis1件あたり通常小規模(支持を表明した
/// 国家の数だけ)であり、`country_by_id`/`diplomacy_registry.get`はいずれもO(1)なので、
/// この関数はCrisis単位でO(そのCrisisの支持国数)であり、`crises×countries`の
/// 総当たりにはならない。
fn crisis_support_adjusted_power(
    crisis: &DiplomaticCrisis,
    ally_power_by_country: &HashMap<CountryId, f32>,
    country_by_id: &HashMap<CountryId, &CountryData>,
    diplomacy_registry: &DiplomacyRegistry,
) -> (f32, f32) {
    let mut target_delta = 0.0f32;
    let mut initiator_delta = 0.0f32;

    for (&supporter_id, reaction) in crisis.third_party_reactions.iter() {
        // 不正Save等に対する保険(通常はP21-013のドメインAPI経由でしか書き込まれない
        // ため、当事国自身のエントリは通常発生しない)。
        if supporter_id == crisis.initiator || supporter_id == crisis.target {
            continue;
        }
        let Some(supporter_country) = country_by_id.get(&supporter_id) else {
            continue; // dangling参照、安全にスキップ
        };
        let supporter_power = supporter_country.available_manpower as f32;

        let is_target_ally = diplomacy_registry
            .get(crisis.target, supporter_id)
            .is_some_and(|r| r.has_treaty(TreatyType::Alliance));
        let is_initiator_ally = diplomacy_registry
            .get(crisis.initiator, supporter_id)
            .is_some_and(|r| r.has_treaty(TreatyType::Alliance));

        // 同盟経由で既にグローバル集計へ含まれている分を、このCrisisに限り差し引く
        // (明示的支持の有無に関わらず両陣営へ二重加算しないため)。
        if is_target_ally {
            target_delta -= supporter_power;
        }
        if is_initiator_ally {
            initiator_delta -= supporter_power;
        }

        match reaction {
            ThirdCountryReaction::SupportsInitiator => initiator_delta += supporter_power,
            ThirdCountryReaction::SupportsTarget => target_delta += supporter_power,
            // Neutral/CondemnsInitiatorはP21-013のドメインAPIからは一切書き込まれない
            // (将来の別機能向けに予約された値)。不正Save等で紛れ込んでいた場合も
            // スコアへ影響させない(no-op)。
            ThirdCountryReaction::Neutral | ThirdCountryReaction::CondemnsInitiator => {}
        }
    }

    let target_allies_power = (ally_power_by_country
        .get(&crisis.target)
        .copied()
        .unwrap_or(0.0)
        + target_delta)
        .max(0.0);
    let initiator_allies_power = (ally_power_by_country
        .get(&crisis.initiator)
        .copied()
        .unwrap_or(0.0)
        + initiator_delta)
        .max(0.0);

    (target_allies_power, initiator_allies_power)
}

/// P21-015: 同盟・関係値差だけに基づいてAI大国の支持陣営を決定する純粋関数
/// (Bevy非依存、乱数不使用)。`None`は中立(Abstain)を表す — `CrisisSupportSide`が
/// 2値(Initiator/Target)しか持たないため、3値目の「中立」は新しい列挙型を作らず
/// `Option<CrisisSupportSide>`で表現し、既存型の重複定義を避けている。
///
/// 優先順位: (1) 片方とのみ同盟していればその陣営を無条件で支持する(関係値差が
/// 逆方向に大きくても同盟を優先する)。両陣営と同盟、またはどちらとも非同盟なら
/// 関係値差(`relation(ai, initiator) - relation(ai, target)`)へ進む。
/// (2) 関係値差が`+AI_CRISIS_SUPPORT_RELATION_MARGIN`以上なら要求国側、
/// `-AI_CRISIS_SUPPORT_RELATION_MARGIN`以下なら対象国側、その間は中立
/// (境界値ちょうどはどちらも支持する側に含む)。
fn decide_ai_crisis_support(
    ally_of_initiator: bool,
    ally_of_target: bool,
    relation_with_initiator: f32,
    relation_with_target: f32,
) -> Option<CrisisSupportSide> {
    match (ally_of_initiator, ally_of_target) {
        (true, false) => Some(CrisisSupportSide::Initiator),
        (false, true) => Some(CrisisSupportSide::Target),
        _ => {
            let relation_delta = relation_with_initiator - relation_with_target;
            if relation_delta >= AI_CRISIS_SUPPORT_RELATION_MARGIN {
                Some(CrisisSupportSide::Initiator)
            } else if relation_delta <= -AI_CRISIS_SUPPORT_RELATION_MARGIN {
                Some(CrisisSupportSide::Target)
            } else {
                None
            }
        }
    }
}

/// P21-015: `PowerTier::GreatPower`のAI国家が進行中のCrisisへ自動的に支持を表明する。
///
/// 候補抽出(Great Power一覧)は`CountryPowerRegistry`から日次1回だけ行い(最大8か国、
/// `power_registry`未構築[空]なら即return — panicしない)、`CountryId.0`昇順の
/// 安定順で保持する。Crisisの処理順も`DiplomaticCrisisId.0`昇順に固定する。計算量は
/// `O(countries[候補抽出] + active_crises × great_powers[≤8])`であり、
/// `active_crises × all_countries`にも`active_crises × states/military`にもしない
/// (州・建物・師団・国家総合力そのものはP21-014の`CountryPowerRegistry`を読むだけで、
/// ここから再集計しない)。
///
/// 支持適用は必ず`crisis_response::pledge_support`(P21-013の既存ドメインAPI)を経由する
/// — `third_party_reactions`への直接書き込みは行わない。`pledge_support`自身が
/// `crisis_is_awaiting_support_response`(phase == DemandSent && current_date <
/// deadline_date)を再検証するため、AI側で期限判定を複製しない。拒否された場合は
/// (通常は起こらないが、defense-in-depthとして)戻り値を無視するだけで、panicも
/// Registry変更も行わない。
pub fn evaluate_ai_crisis_support(
    crisis_registry: &mut CrisisRegistry,
    diplomacy_registry: &DiplomacyRegistry,
    country_registry: &CountryRegistry,
    power_registry: &CountryPowerRegistry,
    player_country: &PlayerCountry,
    date: &GameDate,
) {
    let mut great_power_candidates: Vec<CountryId> = power_registry
        .ordered_country_ids()
        .iter()
        .copied()
        .filter(|&id| {
            Some(id) != player_country.0
                && country_registry.get(id).is_some()
                && power_registry
                    .get(id)
                    .is_some_and(|a| a.power_tier == PowerTier::GreatPower)
        })
        .collect();
    if great_power_candidates.is_empty() {
        return;
    }
    great_power_candidates.sort_by_key(|id| id.0);

    let mut crisis_ids: Vec<DiplomaticCrisisId> = crisis_registry.crises.keys().copied().collect();
    crisis_ids.sort_by_key(|id| id.0);

    for crisis_id in crisis_ids {
        let Some((initiator, target)) = crisis_registry.crises.get(&crisis_id).and_then(|c| {
            crisis_response::crisis_is_awaiting_support_response(c, date)
                .then_some((c.initiator, c.target))
        }) else {
            continue;
        };

        for &ai_id in &great_power_candidates {
            if ai_id == initiator || ai_id == target {
                continue;
            }
            let already_supports = crisis_registry
                .crises
                .get(&crisis_id)
                .is_some_and(|c| c.third_party_reactions.contains_key(&ai_id));
            if already_supports {
                continue;
            }

            let ally_of_initiator = diplomacy_registry
                .get(ai_id, initiator)
                .is_some_and(|r| r.has_treaty(TreatyType::Alliance));
            let ally_of_target = diplomacy_registry
                .get(ai_id, target)
                .is_some_and(|r| r.has_treaty(TreatyType::Alliance));
            let relation_with_initiator =
                diplomacy_registry.get_or_default(ai_id, initiator).opinion;
            let relation_with_target = diplomacy_registry.get_or_default(ai_id, target).opinion;

            let decision = decide_ai_crisis_support(
                ally_of_initiator,
                ally_of_target,
                relation_with_initiator,
                relation_with_target,
            );

            if let Some(side) = decision {
                let _ = crisis_response::pledge_support(
                    crisis_registry,
                    country_registry,
                    crisis_id,
                    ai_id,
                    side,
                    date,
                );
            }
        }
    }
}

/// P21-012: AI対象国による最後通牒への自動応答を評価・実行する。対象は
/// `phase == DemandSent` かつ target が AI国家(プレイヤーではない)、かつ期限前
/// (`current_date < deadline_date`)のCrisisのみ。判定は既存の
/// `diplomacy::ai::calculate_demand_acceptance`のみを使用し、新しい受諾確率・補正値は
/// 一切追加しない。スコアの解釈は`score > 0.0`を受諾、それ以外(0を含む)を拒否とする
/// (関数自身のdocコメント「正の値なら受諾、負の値なら拒否に傾く」、`-100.0..100.0`の
/// 同型スコアを使う既存コード`war/capitulation.rs`の`war_score > 0.0`分岐、このスコアを
/// 最初に使ったテスト自身の`score > 0.0`アサーションのいずれとも整合する解釈であり、
/// 独自の閾値を新設していない)。
///
/// 受諾・拒否の実処理は`crisis_response::accept_demand`/`reject_demand`(P21-011の共通
/// ドメインAPI)をそのまま呼ぶだけで、Registry/構造体フィールドを直接書き換えない。
/// 対象Crisis idは呼び出し時点で一度収集してから処理するため、`CrisisRegistry`の
/// HashMap反復順に関わらず、各Crisisの受諾・拒否結果は他のCrisisの処理順に依存しない
/// (evaluate_ai_crisis_responses自身が読む入力[対象国データ・同盟戦力・関係値]は、
/// 同一tick内の他Crisisの受諾/拒否によって変化しないため)。
#[allow(clippy::too_many_arguments)]
fn evaluate_ai_crisis_responses(
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &mut ClaimRegistry,
    state_registry: &mut StateRegistry,
    justification_registry: &mut WarJustificationRegistry,
    diplomacy_registry: &DiplomacyRegistry,
    country_registry: &CountryRegistry,
    player_country: &PlayerCountry,
    date: &GameDate,
    notif_writer: &mut MessageWriter<GameNotification>,
    locale: &CurrentLocale,
    catalog: &TranslationCatalog,
) {
    // crisis件数 × country数の総当たりを避けるため、O(1)引き用のマップを日次1回だけ構築する。
    let country_by_id: HashMap<CountryId, &CountryData> = country_registry
        .countries
        .iter()
        .map(|c| (c.id, c))
        .collect();
    let ally_power_by_country = compute_ally_power_by_country(diplomacy_registry, &country_by_id);

    let eligible_crisis_ids: Vec<DiplomaticCrisisId> = crisis_registry
        .crises
        .values()
        .filter(|c| {
            c.current_phase == CrisisPhase::DemandSent
                && player_country.0 != Some(c.target)
                && c.deadline_date
                    .as_deref()
                    .and_then(GameDate::from_string)
                    .is_some_and(|deadline| !date.is_at_least(&deadline))
        })
        .map(|c| c.id)
        .collect();

    for crisis_id in eligible_crisis_ids {
        // 参照不整合(対象国・州が既に存在しない等)ではpanicせず、この1件だけを
        // 安全にスキップする(他のCrisisやCrisis自体には触れない。翌日以降、
        // 参照が復元されるか期限切れになるまでDemandSentのまま残る)。
        let Some(crisis) = crisis_registry.crises.get(&crisis_id) else {
            continue;
        };
        if crisis.current_phase != CrisisPhase::DemandSent {
            continue;
        }
        let Some(target_country) = country_by_id.get(&crisis.target) else {
            continue;
        };
        let Some(initiator_country) = country_by_id.get(&crisis.initiator) else {
            continue;
        };
        let Some(target_state_id) = crisis
            .war_goals
            .first()
            .and_then(|g| g.target_states.first().copied())
        else {
            continue;
        };
        let Some(target_state) = state_registry.get(target_state_id) else {
            continue;
        };
        let relation = diplomacy_registry.get_or_default(crisis.initiator, crisis.target);
        let (target_allies_power, initiator_allies_power) = crisis_support_adjusted_power(
            crisis,
            &ally_power_by_country,
            &country_by_id,
            diplomacy_registry,
        );

        let score = calculate_demand_acceptance(
            crisis,
            target_country,
            initiator_country,
            &[target_state],
            &relation,
            target_allies_power,
            initiator_allies_power,
        );
        let accept = score > 0.0;
        let initiator = crisis.initiator;
        let target = crisis.target;

        if accept {
            let _ = crisis_response::accept_demand(
                crisis_registry,
                claim_registry,
                state_registry,
                crisis_id,
                target,
            );
        } else {
            let _ = crisis_response::reject_demand(
                crisis_registry,
                justification_registry,
                crisis_id,
                target,
                date.display(),
            );
        }

        if player_country.0 == Some(initiator) {
            let target_name = country_registry
                .get(target)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| t(catalog, locale.0, "common.unknown"));
            notif_writer.write(GameNotification {
                message: tf(
                    catalog,
                    locale.0,
                    if accept {
                        "notif.crisis_ai_accepted"
                    } else {
                        "notif.crisis_ai_rejected"
                    },
                    vec![("country", target_name)],
                ),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::diplomacy::data::{ActiveTreaty, DiplomaticRelation};
    use crate::state::data::StateData;

    fn country(id: usize, manpower: u64) -> CountryData {
        CountryData {
            id: CountryId(id),
            available_manpower: manpower,
            capital_state_id: crate::common::StateId(999), // どの州も首都ではない
            ..Default::default()
        }
    }

    fn country_by_id_of(countries: &[CountryData]) -> HashMap<CountryId, &CountryData> {
        countries.iter().map(|c| (c.id, c)).collect()
    }

    fn alliance_registry(pairs: &[(usize, usize)]) -> DiplomacyRegistry {
        let mut registry = DiplomacyRegistry::default();
        for &(a, b) in pairs {
            let key = DiplomaticPairKey::new(CountryId(a), CountryId(b)).unwrap();
            registry.relations.insert(
                key,
                DiplomaticRelation {
                    treaties: vec![ActiveTreaty {
                        treaty_type: TreatyType::Alliance,
                        countries: (CountryId(a), CountryId(b)),
                        signed_date: "1800/01/01".to_string(),
                        is_active: true,
                    }],
                    ..Default::default()
                },
            );
        }
        registry
    }

    fn crisis_with_support(
        initiator: usize,
        target: usize,
        supports: &[(usize, ThirdCountryReaction)],
    ) -> DiplomaticCrisis {
        DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(initiator),
            target: CountryId(target),
            war_goals: vec![WarGoal {
                attacker: CountryId(initiator),
                defender: CountryId(target),
                goal_type: WarGoalType::ConquerState,
                target_states: vec![crate::common::StateId(1)],
                base_peace_cost: 0.0,
                international_concern: 0.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: "1800/01/01".to_string(),
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1800/01/31".to_string()),
            international_concern: 0.0,
            third_party_reactions: supports
                .iter()
                .map(|&(id, reaction)| (CountryId(id), reaction))
                .collect(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        }
    }

    /// 要求テスト項目15: 支持なしなら`ally_power_by_country`(P21-012までの計算結果)を
    /// そのまま返す(全く変えない)。
    #[test]
    fn no_support_matches_p21_012_baseline() {
        let countries = [country(0, 100_000), country(1, 10_000)];
        let country_by_id = country_by_id_of(&countries);
        let diplomacy_registry = DiplomacyRegistry::default();
        let mut ally_power_by_country = HashMap::new();
        ally_power_by_country.insert(CountryId(0), 5_000.0);
        ally_power_by_country.insert(CountryId(1), 3_000.0);

        let crisis = crisis_with_support(0, 1, &[]);
        let (target_power, initiator_power) = crisis_support_adjusted_power(
            &crisis,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );

        assert_eq!(target_power, 3_000.0);
        assert_eq!(initiator_power, 5_000.0);
    }

    /// 要求テスト項目18: initiator/target自身が(不正データ等で)third_party_reactionsに
    /// 紛れ込んでいても、支援戦力へ加算されない。
    #[test]
    fn initiator_and_target_are_never_added_to_support_power() {
        let countries = [country(0, 100_000), country(1, 10_000)];
        let country_by_id = country_by_id_of(&countries);
        let diplomacy_registry = DiplomacyRegistry::default();
        let ally_power_by_country = HashMap::new();

        // 通常のドメインAPI経由では起こらないが、不正データに対する保険を検証する。
        let crisis = crisis_with_support(
            0,
            1,
            &[
                (0, ThirdCountryReaction::SupportsInitiator),
                (1, ThirdCountryReaction::SupportsTarget),
            ],
        );
        let (target_power, initiator_power) = crisis_support_adjusted_power(
            &crisis,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );

        assert_eq!(target_power, 0.0);
        assert_eq!(initiator_power, 0.0);
    }

    /// 要求テスト項目13: 要求国側支持で受諾スコアが上昇する(支持ありの方がスコアが高い)。
    #[test]
    fn supporting_initiator_increases_the_acceptance_score() {
        let initiator_country = country(0, 60_000);
        let target_country = country(1, 60_000);
        let state = StateData {
            id: crate::common::StateId(1),
            owner_country_id: CountryId(1),
            population: 200_000,
            integration: 100.0,
            ..Default::default()
        };
        let relation = DiplomaticRelation::default();

        let crisis_no_support = crisis_with_support(0, 1, &[]);
        let score_no_support = calculate_demand_acceptance(
            &crisis_no_support,
            &target_country,
            &initiator_country,
            &[&state],
            &relation,
            0.0,
            0.0,
        );

        let crisis_supported =
            crisis_with_support(0, 1, &[(2, ThirdCountryReaction::SupportsInitiator)]);
        let countries = [
            initiator_country.clone(),
            target_country.clone(),
            country(2, 500_000),
        ];
        let country_by_id = country_by_id_of(&countries);
        let diplomacy_registry = DiplomacyRegistry::default();
        let ally_power_by_country = HashMap::new();
        let (target_power, initiator_power) = crisis_support_adjusted_power(
            &crisis_supported,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );
        let score_with_support = calculate_demand_acceptance(
            &crisis_supported,
            &target_country,
            &initiator_country,
            &[&state],
            &relation,
            target_power,
            initiator_power,
        );

        assert!(
            score_with_support > score_no_support,
            "supporting the initiator must raise the acceptance score \
             (no_support={score_no_support}, with_support={score_with_support})"
        );
    }

    /// 要求テスト項目14: 対象国側支持で受諾スコアが低下する(支持ありの方がスコアが低い)。
    #[test]
    fn supporting_target_decreases_the_acceptance_score() {
        let initiator_country = country(0, 60_000);
        let target_country = country(1, 60_000);
        let state = StateData {
            id: crate::common::StateId(1),
            owner_country_id: CountryId(1),
            population: 200_000,
            integration: 100.0,
            ..Default::default()
        };
        let relation = DiplomaticRelation::default();

        let crisis_no_support = crisis_with_support(0, 1, &[]);
        let score_no_support = calculate_demand_acceptance(
            &crisis_no_support,
            &target_country,
            &initiator_country,
            &[&state],
            &relation,
            0.0,
            0.0,
        );

        let crisis_supported =
            crisis_with_support(0, 1, &[(2, ThirdCountryReaction::SupportsTarget)]);
        let countries = [
            initiator_country.clone(),
            target_country.clone(),
            country(2, 500_000),
        ];
        let country_by_id = country_by_id_of(&countries);
        let diplomacy_registry = DiplomacyRegistry::default();
        let ally_power_by_country = HashMap::new();
        let (target_power, initiator_power) = crisis_support_adjusted_power(
            &crisis_supported,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );
        let score_with_support = calculate_demand_acceptance(
            &crisis_supported,
            &target_country,
            &initiator_country,
            &[&state],
            &relation,
            target_power,
            initiator_power,
        );

        assert!(
            score_with_support < score_no_support,
            "supporting the target must lower the acceptance score \
             (no_support={score_no_support}, with_support={score_with_support})"
        );
    }

    /// 要求テスト項目16/17: 支持国が既に対象国の同盟国であっても、明示的にinitiator側を
    /// 支持していれば、その側だけへ一度だけ計上され(二重計上なし)、反対側(同盟のある方)
    /// との同盟関係は無視される(明示的支持が優先)。
    #[test]
    fn explicit_support_overrides_and_deduplicates_existing_alliance() {
        let countries = [country(0, 100_000), country(1, 100_000), country(2, 40_000)];
        let country_by_id = country_by_id_of(&countries);
        // 支持国2は「対象国1の同盟国」だが、このCrisisでは明示的にinitiator(0)を支持する。
        let diplomacy_registry = alliance_registry(&[(1, 2)]);
        let mut ally_power_by_country = HashMap::new();
        // P21-012のグローバル集計では、同盟国2の戦力(40,000)がtargetの合計へ既に含まれている。
        ally_power_by_country.insert(CountryId(1), 40_000.0);
        ally_power_by_country.insert(CountryId(0), 0.0);

        let crisis = crisis_with_support(0, 1, &[(2, ThirdCountryReaction::SupportsInitiator)]);
        let (target_power, initiator_power) = crisis_support_adjusted_power(
            &crisis,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );

        assert_eq!(
            target_power, 0.0,
            "the ally's power must be removed from target's total once they explicitly support initiator instead"
        );
        assert_eq!(
            initiator_power, 40_000.0,
            "the ally's power must be added to initiator's total exactly once (no double counting)"
        );
    }

    /// 要求テスト項目16: 支持国が既に要求国の同盟国で、要求国側へ支持表明した場合
    /// (同盟と支持が同じ側)、二重計上されない(戦力は1回分だけ計上される)。
    #[test]
    fn same_side_alliance_and_explicit_support_do_not_double_count() {
        let countries = [country(0, 100_000), country(1, 100_000), country(2, 40_000)];
        let country_by_id = country_by_id_of(&countries);
        let diplomacy_registry = alliance_registry(&[(0, 2)]);
        let mut ally_power_by_country = HashMap::new();
        ally_power_by_country.insert(CountryId(0), 40_000.0);
        ally_power_by_country.insert(CountryId(1), 0.0);

        let crisis = crisis_with_support(0, 1, &[(2, ThirdCountryReaction::SupportsInitiator)]);
        let (_, initiator_power) = crisis_support_adjusted_power(
            &crisis,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );

        assert_eq!(
            initiator_power, 40_000.0,
            "an ally who explicitly (re-)supports the same side they're already allied with \
             must be counted exactly once, not twice"
        );
    }

    /// 要求テスト項目19: `third_party_reactions`の走査順(HashMap反復順)が異なっても、
    /// 結果の支援戦力は変わらない。
    #[test]
    fn iteration_order_does_not_affect_the_computed_power() {
        let countries = [
            country(0, 100_000),
            country(1, 100_000),
            country(2, 20_000),
            country(3, 30_000),
        ];
        let country_by_id = country_by_id_of(&countries);
        let diplomacy_registry = DiplomacyRegistry::default();
        let ally_power_by_country = HashMap::new();

        let crisis_a = crisis_with_support(
            0,
            1,
            &[
                (2, ThirdCountryReaction::SupportsInitiator),
                (3, ThirdCountryReaction::SupportsTarget),
            ],
        );
        let crisis_b = crisis_with_support(
            0,
            1,
            &[
                (3, ThirdCountryReaction::SupportsTarget),
                (2, ThirdCountryReaction::SupportsInitiator),
            ],
        );

        let result_a = crisis_support_adjusted_power(
            &crisis_a,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );
        let result_b = crisis_support_adjusted_power(
            &crisis_b,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );

        assert_eq!(result_a, result_b);
        assert_eq!(result_a, (30_000.0, 20_000.0));
    }

    /// 要求テスト項目20: 支持だけを変えて閾値0を跨ぐ受諾ケース
    /// (支持なしでは拒否だが、要求国側支持を追加すると受諾になる)。
    #[test]
    fn support_alone_can_flip_a_rejection_into_an_acceptance() {
        let initiator_country = country(0, 50_000);
        let target_country = country(1, 50_000);
        let state = StateData {
            id: crate::common::StateId(1),
            owner_country_id: CountryId(1),
            population: 100_000,
            integration: 100.0,
            ..Default::default()
        };
        let relation = DiplomaticRelation::default();

        let crisis_no_support = crisis_with_support(0, 1, &[]);
        let score_no_support = calculate_demand_acceptance(
            &crisis_no_support,
            &target_country,
            &initiator_country,
            &[&state],
            &relation,
            0.0,
            0.0,
        );
        assert!(
            score_no_support <= 0.0,
            "test setup must start as a rejection without support (got {score_no_support})"
        );

        let crisis_supported =
            crisis_with_support(0, 1, &[(2, ThirdCountryReaction::SupportsInitiator)]);
        let countries = [
            initiator_country.clone(),
            target_country.clone(),
            country(2, 1_000_000),
        ];
        let country_by_id = country_by_id_of(&countries);
        let diplomacy_registry = DiplomacyRegistry::default();
        let ally_power_by_country = HashMap::new();
        let (target_power, initiator_power) = crisis_support_adjusted_power(
            &crisis_supported,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );
        let score_with_support = calculate_demand_acceptance(
            &crisis_supported,
            &target_country,
            &initiator_country,
            &[&state],
            &relation,
            target_power,
            initiator_power,
        );

        assert!(
            score_with_support > 0.0,
            "sufficient initiator-side support must flip the verdict to acceptance (score={score_with_support})"
        );
    }

    /// 要求テスト項目21: 支持だけを変えて閾値0を跨ぐ拒否ケース
    /// (支持なしでは受諾だが、対象国側支持を追加すると拒否になる)。
    #[test]
    fn support_alone_can_flip_an_acceptance_into_a_rejection() {
        let initiator_country = country(0, 100_000);
        let target_country = country(1, 10_000);
        let state = StateData {
            id: crate::common::StateId(1),
            owner_country_id: CountryId(1),
            population: 50_000,
            integration: 100.0,
            ..Default::default()
        };
        let relation = DiplomaticRelation::default();

        let crisis_no_support = crisis_with_support(0, 1, &[]);
        let score_no_support = calculate_demand_acceptance(
            &crisis_no_support,
            &target_country,
            &initiator_country,
            &[&state],
            &relation,
            0.0,
            0.0,
        );
        assert!(
            score_no_support > 0.0,
            "test setup must start as an acceptance without support (got {score_no_support})"
        );

        let crisis_supported =
            crisis_with_support(0, 1, &[(2, ThirdCountryReaction::SupportsTarget)]);
        let countries = [
            initiator_country.clone(),
            target_country.clone(),
            country(2, 1_000_000),
        ];
        let country_by_id = country_by_id_of(&countries);
        let diplomacy_registry = DiplomacyRegistry::default();
        let ally_power_by_country = HashMap::new();
        let (target_power, initiator_power) = crisis_support_adjusted_power(
            &crisis_supported,
            &ally_power_by_country,
            &country_by_id,
            &diplomacy_registry,
        );
        let score_with_support = calculate_demand_acceptance(
            &crisis_supported,
            &target_country,
            &initiator_country,
            &[&state],
            &relation,
            target_power,
            initiator_power,
        );

        assert!(
            score_with_support <= 0.0,
            "sufficient target-side support must flip the verdict to rejection (score={score_with_support})"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-015: AI大国のCrisis支持判断
    // ─────────────────────────────────────────────────────────────────────

    // ─── decide_ai_crisis_support (純粋関数、要求テスト1-12) ─────────────────

    /// 要求テスト1: 要求国とのみ同盟なら要求国側。
    /// 要求テスト2: 対象国とのみ同盟なら対象国側。
    #[test]
    fn alliance_with_only_one_side_determines_the_side() {
        assert_eq!(
            decide_ai_crisis_support(true, false, 0.0, 0.0),
            Some(CrisisSupportSide::Initiator)
        );
        assert_eq!(
            decide_ai_crisis_support(false, true, 0.0, 0.0),
            Some(CrisisSupportSide::Target)
        );
    }

    /// 要求テスト3: 片側同盟は逆方向の大きな関係値差より優先される。
    #[test]
    fn single_side_alliance_overrides_a_large_opposite_relation_delta() {
        // 対象国への関係値が要求国よりはるかに高くても、要求国側とのみ同盟していれば
        // 要求国側を支持する。
        assert_eq!(
            decide_ai_crisis_support(true, false, -100.0, 100.0),
            Some(CrisisSupportSide::Initiator)
        );
        assert_eq!(
            decide_ai_crisis_support(false, true, 100.0, -100.0),
            Some(CrisisSupportSide::Target)
        );
    }

    /// 要求テスト4: 両国と同盟している場合は関係値差へ進む。
    #[test]
    fn alliance_with_both_sides_falls_through_to_relation_delta() {
        assert_eq!(
            decide_ai_crisis_support(true, true, 50.0, 0.0),
            Some(CrisisSupportSide::Initiator)
        );
        assert_eq!(decide_ai_crisis_support(true, true, 0.0, 0.0), None);
    }

    /// 要求テスト5-10: 非同盟時の関係値差境界(+25/-25/中立域/同値)。
    #[test]
    fn relation_delta_boundaries_without_any_alliance() {
        // 5: delta > 25 → 要求国側。 6: delta == 25 → 要求国側(境界含む)。
        assert_eq!(
            decide_ai_crisis_support(false, false, 30.0, 0.0),
            Some(CrisisSupportSide::Initiator)
        );
        assert_eq!(
            decide_ai_crisis_support(false, false, 25.0, 0.0),
            Some(CrisisSupportSide::Initiator)
        );
        // 7: delta < -25 → 対象国側。 8: delta == -25 → 対象国側(境界含む)。
        assert_eq!(
            decide_ai_crisis_support(false, false, 0.0, 30.0),
            Some(CrisisSupportSide::Target)
        );
        assert_eq!(
            decide_ai_crisis_support(false, false, 0.0, 25.0),
            Some(CrisisSupportSide::Target)
        );
        // 9: -24〜24は中立。
        assert_eq!(decide_ai_crisis_support(false, false, 24.0, 0.0), None);
        assert_eq!(decide_ai_crisis_support(false, false, 0.0, 24.0), None);
        // 10: 関係値が同値(delta==0)なら中立。
        assert_eq!(decide_ai_crisis_support(false, false, 10.0, 10.0), None);
    }

    /// 要求テスト12: 同一入力から常に同一結果になる(純粋関数の決定論性)。
    #[test]
    fn decide_ai_crisis_support_is_deterministic() {
        for _ in 0..5 {
            assert_eq!(
                decide_ai_crisis_support(false, true, 12.0, -40.0),
                Some(CrisisSupportSide::Target)
            );
        }
    }

    // ─── evaluate_ai_crisis_support (資格・接続・順序、要求テスト11,13-27,34-36) ──

    fn power_registry_from_population(entries: &[(usize, u64)]) -> CountryPowerRegistry {
        let countries: Vec<CountryData> = entries
            .iter()
            .map(|&(id, _)| CountryData {
                id: CountryId(id),
                ..CountryData::default()
            })
            .collect();
        let states: Vec<StateData> = entries
            .iter()
            .enumerate()
            .map(|(i, &(id, pop))| StateData {
                id: crate::common::StateId(2000 + i),
                owner_country_id: CountryId(id),
                population: pop,
                ..Default::default()
            })
            .collect();
        crate::country::power::evaluate_country_power(
            &CountryRegistry { countries },
            &StateRegistry::build(states),
            &crate::military::data::MilitaryRegistry::default(),
            &crate::building::data::BuildingRegistry::default(),
            "1800/01/01".to_string(),
        )
    }

    /// 1(=initiator)を圧倒的なGreat Power、2,3,4,5を小規模国にした5か国Registry。
    /// `compute_tier_counts(5) == (great=1, regional=2, minor=2)`なので、
    /// 2,3は地域大国、4,5は小国になる(同点はCountryId昇順で決まる)。
    fn standard_power_registry() -> CountryPowerRegistry {
        power_registry_from_population(&[(1, 10_000_000), (2, 100), (3, 100), (4, 100), (5, 100)])
    }

    fn empty_diplomacy() -> DiplomacyRegistry {
        DiplomacyRegistry::default()
    }

    fn before_deadline() -> GameDate {
        GameDate::new(1800, 1, 15)
    }

    fn crisis_registry_with(crisis: DiplomaticCrisis) -> CrisisRegistry {
        let mut registry = CrisisRegistry::default();
        registry.add_crisis(crisis);
        registry
    }

    /// 要求テスト13: AI大国は支持できる。
    #[test]
    fn ai_great_power_can_support() {
        // 6か国: 0=initiator, 10=target, 1=Great Power候補。1がinitiator/targetに
        // 干渉しないよう、standard_power_registryの国0/1/2/3/4/5とはIDを分けて
        // 別の国家群(initiator=0, target=10)を使い、Great Power自体はid=1とする。
        let power_registry = standard_power_registry(); // id=1がGreatPower
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
            ],
        };
        let mut crisis_registry = crisis_registry_with(crisis_with_support(0, 10, &[]));
        let diplomacy_registry = alliance_registry(&[(1, 0)]); // GreatPower(1)がinitiator(0)と同盟
        let player_country = PlayerCountry(None);

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        );

        let crisis = crisis_registry.crises.values().next().unwrap();
        assert_eq!(
            crisis.third_party_reactions.get(&CountryId(1)),
            Some(&ThirdCountryReaction::SupportsInitiator)
        );
    }

    /// 要求テスト14: AI地域大国は自動支持しない。
    /// 要求テスト15: AI小国は自動支持しない。
    #[test]
    fn regional_and_minor_powers_never_auto_support() {
        let power_registry = standard_power_registry();
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(2),
                    ..CountryData::default()
                }, // RegionalPower
                CountryData {
                    id: CountryId(4),
                    ..CountryData::default()
                }, // MinorPower
            ],
        };
        // 2(地域大国)・4(小国)ともに要求国(0)と同盟し、関係値差も要求国側で
        // 決定的なはずの状況を作るが、Great Powerでないため一切支持しないはず。
        let diplomacy_registry = alliance_registry(&[(2, 0), (4, 0)]);
        let mut crisis_registry = crisis_registry_with(crisis_with_support(0, 10, &[]));
        let player_country = PlayerCountry(None);

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        );

        let crisis = crisis_registry.crises.values().next().unwrap();
        assert!(crisis.third_party_reactions.is_empty());
    }

    /// 要求テスト16: プレイヤー大国は自動支持しない。
    #[test]
    fn player_controlled_great_power_never_auto_supports() {
        let power_registry = standard_power_registry(); // id=1がGreatPower
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
            ],
        };
        let diplomacy_registry = alliance_registry(&[(1, 0)]);
        let mut crisis_registry = crisis_registry_with(crisis_with_support(0, 10, &[]));
        let player_country = PlayerCountry(Some(CountryId(1))); // プレイヤー自身がGreatPower

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        );

        let crisis = crisis_registry.crises.values().next().unwrap();
        assert!(crisis.third_party_reactions.is_empty());
    }

    /// 要求テスト17: 要求国自身は支持者にならない。
    /// 要求テスト18: 対象国自身は支持者にならない。
    #[test]
    fn belligerents_are_never_added_as_their_own_supporters() {
        // Great Powerが1件だけの5か国Registryを使い、initiator/targetをそのGreat Power
        // 自身のID(1)にする(=1が自分自身のCrisisの当事国)。
        let power_registry = standard_power_registry(); // id=1がGreatPower
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
            ],
        };
        let diplomacy_registry = empty_diplomacy();
        let player_country = PlayerCountry(None);

        // ケースA: Great Power(1)がinitiator。
        let mut crisis_registry_a = crisis_registry_with(crisis_with_support(1, 10, &[]));
        evaluate_ai_crisis_support(
            &mut crisis_registry_a,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        );
        assert!(
            crisis_registry_a
                .crises
                .values()
                .next()
                .unwrap()
                .third_party_reactions
                .is_empty(),
            "the initiator itself (even if a Great Power) must never be added as its own supporter"
        );

        // ケースB: Great Power(1)がtarget。
        let mut crisis_registry_b = crisis_registry_with(crisis_with_support(10, 1, &[]));
        evaluate_ai_crisis_support(
            &mut crisis_registry_b,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        );
        assert!(
            crisis_registry_b
                .crises
                .values()
                .next()
                .unwrap()
                .third_party_reactions
                .is_empty(),
            "the target itself (even if a Great Power) must never be added as its own supporter"
        );
    }

    /// 要求テスト19: 既に要求国側を支持している国を重複追加しない。
    /// 要求テスト20: 既に対象国側を支持している国を重複追加しない。
    /// 要求テスト21: 反対陣営へ切り替えない。
    #[test]
    fn already_supporting_countries_are_not_reevaluated_or_switched() {
        let power_registry = standard_power_registry(); // id=1がGreatPower
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
            ],
        };
        // 1は既に対象国(10)側を支持済み。同時に要求国(0)と同盟していて、放置すれば
        // 「要求国側を支持すべき」という決定が出るはずの状況を作る — それでも
        // 既存の支持(対象国側)へは一切触れない(切り替えない・重複追加しない)ことを確認する。
        let diplomacy_registry = alliance_registry(&[(1, 0)]);
        let mut crisis_registry = crisis_registry_with(crisis_with_support(
            0,
            10,
            &[(1, ThirdCountryReaction::SupportsTarget)],
        ));
        let player_country = PlayerCountry(None);

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        );

        let crisis = crisis_registry.crises.values().next().unwrap();
        assert_eq!(
            crisis.third_party_reactions.get(&CountryId(1)),
            Some(&ThirdCountryReaction::SupportsTarget),
            "an already-supporting Great Power must neither be re-added nor switched to the other side"
        );
        assert_eq!(crisis.third_party_reactions.len(), 1);
    }

    /// 要求テスト22: `current_date < deadline_date`なら支持可能。
    /// 要求テスト23: `current_date == deadline_date`なら拒否。
    /// 要求テスト24: 期限超過かつphaseが`DemandSent`でも拒否。
    #[test]
    fn support_respects_the_shared_deadline_predicate() {
        let power_registry = standard_power_registry();
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
            ],
        };
        let diplomacy_registry = alliance_registry(&[(1, 0)]);
        let player_country = PlayerCountry(None);

        // deadline_date == "1800/01/31" (crisis_with_supportの既定値)。
        for (label, date, must_support) in [
            ("before_deadline", GameDate::new(1800, 1, 15), true),
            ("on_deadline_day", GameDate::new(1800, 1, 31), false),
            ("after_deadline", GameDate::new(1800, 2, 5), false),
        ] {
            let mut crisis_registry = crisis_registry_with(crisis_with_support(0, 10, &[]));
            evaluate_ai_crisis_support(
                &mut crisis_registry,
                &diplomacy_registry,
                &countries,
                &power_registry,
                &player_country,
                &date,
            );
            let supported = crisis_registry
                .crises
                .values()
                .next()
                .unwrap()
                .third_party_reactions
                .contains_key(&CountryId(1));
            assert_eq!(supported, must_support, "case={label}");
        }
    }

    /// 要求テスト25: `DemandSent`以外では支持しない。
    #[test]
    fn non_demand_sent_crises_are_never_supported() {
        let power_registry = standard_power_registry();
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
            ],
        };
        let diplomacy_registry = alliance_registry(&[(1, 0)]);
        let player_country = PlayerCountry(None);

        for phase in [
            CrisisPhase::Escalating,
            CrisisPhase::ResolvedPeacefully,
            CrisisPhase::WarStarted,
            CrisisPhase::Cancelled,
        ] {
            let mut crisis = crisis_with_support(0, 10, &[]);
            crisis.current_phase = phase;
            let mut crisis_registry = crisis_registry_with(crisis);
            evaluate_ai_crisis_support(
                &mut crisis_registry,
                &diplomacy_registry,
                &countries,
                &power_registry,
                &player_country,
                &before_deadline(),
            );
            assert!(
                crisis_registry
                    .crises
                    .values()
                    .next()
                    .unwrap()
                    .third_party_reactions
                    .is_empty(),
                "phase {phase:?} must never receive AI support"
            );
        }
    }

    /// 要求テスト26: Registry未構築(空)時にpanicしない。
    #[test]
    fn empty_power_registry_does_not_panic_and_skips_support() {
        let power_registry = CountryPowerRegistry::default();
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
            ],
        };
        let diplomacy_registry = empty_diplomacy();
        let mut crisis_registry = crisis_registry_with(crisis_with_support(0, 10, &[]));
        let player_country = PlayerCountry(None);

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        ); // panicしないこと自体がテスト

        assert!(
            crisis_registry
                .crises
                .values()
                .next()
                .unwrap()
                .third_party_reactions
                .is_empty()
        );
    }

    /// 要求テスト27: 壊れた国家参照(存在しない当事国)を含むCrisisでもpanicしない。
    #[test]
    fn crisis_with_dangling_belligerent_reference_does_not_panic() {
        let power_registry = standard_power_registry(); // id=1がGreatPower
        // countryRegistryにはGreatPower(1)しか登録しない。initiator(999)/target(998)は
        // 実在しない参照だが、evaluate_ai_crisis_supportはこれをそのままCountryIdの
        // 値として比較・DiplomacyRegistry参照するだけなのでpanicしないはず。
        let countries = CountryRegistry {
            countries: vec![CountryData {
                id: CountryId(1),
                ..CountryData::default()
            }],
        };
        let diplomacy_registry = empty_diplomacy();
        let mut crisis_registry = crisis_registry_with(crisis_with_support(999, 998, &[]));
        let player_country = PlayerCountry(None);

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        ); // panicしないこと自体がテスト
    }

    /// 要求テスト34: 複数の大国がCountryId順で決定論的に追加される。
    #[test]
    fn multiple_great_powers_are_processed_in_country_id_order_deterministically() {
        // 2件のGreat Power(1,1の代わりに複数国を作る必要があるため、6か国構成で
        // n=6 → compute_tier_counts(6)==(2,2,2)となるRegistryを使う)。
        let power_registry = power_registry_from_population(&[
            (1, 10_000_000),
            (2, 9_000_000),
            (3, 100),
            (4, 100),
            (5, 100),
            (6, 100),
        ]);
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(2),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
            ],
        };
        // 1・2の双方がGreat Powerで、いずれも要求国(0)と同盟しているとする。
        let diplomacy_registry = alliance_registry(&[(1, 0), (2, 0)]);
        let mut crisis_registry = crisis_registry_with(crisis_with_support(0, 10, &[]));
        let player_country = PlayerCountry(None);

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        );

        let crisis = crisis_registry.crises.values().next().unwrap();
        assert_eq!(
            crisis.third_party_reactions.get(&CountryId(1)),
            Some(&ThirdCountryReaction::SupportsInitiator)
        );
        assert_eq!(
            crisis.third_party_reactions.get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsInitiator)
        );
    }

    /// 要求テスト35: 一つの大国が複数の異なるCrisisを支持できる。
    #[test]
    fn one_great_power_can_support_multiple_different_crises() {
        let power_registry = standard_power_registry(); // id=1がGreatPower
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(11),
                    ..CountryData::default()
                },
            ],
        };
        let diplomacy_registry = alliance_registry(&[(1, 0)]); // 1はcountry 0とのみ同盟
        let mut crisis_registry = CrisisRegistry::default();
        let crisis_a_id = crisis_registry.add_crisis(crisis_with_support(0, 10, &[]));
        let crisis_b_id = crisis_registry.add_crisis(crisis_with_support(0, 11, &[]));
        let player_country = PlayerCountry(None);

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &player_country,
            &before_deadline(),
        );

        assert_eq!(
            crisis_registry.crises[&crisis_a_id]
                .third_party_reactions
                .get(&CountryId(1)),
            Some(&ThirdCountryReaction::SupportsInitiator)
        );
        assert_eq!(
            crisis_registry.crises[&crisis_b_id]
                .third_party_reactions
                .get(&CountryId(1)),
            Some(&ThirdCountryReaction::SupportsInitiator)
        );
    }

    /// 要求テスト36: Crisis数やHashMap挿入順を変えても各Crisisの判断結果が変わらない。
    #[test]
    fn crisis_processing_order_does_not_affect_individual_outcomes() {
        let power_registry = standard_power_registry(); // id=1がGreatPower
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(11),
                    ..CountryData::default()
                },
            ],
        };
        // 1はcountry 0(crisis Aのinitiator)とのみ同盟、country 11(crisis Bのtarget)とは
        // 非同盟・中立関係(delta=0)なので、crisis Aでは支持しcrisis Bでは中立のはず。
        let diplomacy_registry = alliance_registry(&[(1, 0)]);

        // 挿入順序A: crisis Aを先に追加。
        let mut registry_order_a = CrisisRegistry::default();
        let a1 = registry_order_a.add_crisis(crisis_with_support(0, 10, &[]));
        let b1 = registry_order_a.add_crisis(crisis_with_support(11, 0, &[]));
        evaluate_ai_crisis_support(
            &mut registry_order_a,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &PlayerCountry(None),
            &before_deadline(),
        );

        // 挿入順序B: crisis Bを先に追加(=別のDiplomaticCrisisIdが採番される)。
        let mut registry_order_b = CrisisRegistry::default();
        let b2 = registry_order_b.add_crisis(crisis_with_support(11, 0, &[]));
        let a2 = registry_order_b.add_crisis(crisis_with_support(0, 10, &[]));
        evaluate_ai_crisis_support(
            &mut registry_order_b,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &PlayerCountry(None),
            &before_deadline(),
        );

        assert_eq!(
            registry_order_a.crises[&a1].third_party_reactions,
            registry_order_b.crises[&a2].third_party_reactions,
            "crisis A's outcome must not depend on insertion/processing order"
        );
        assert_eq!(
            registry_order_a.crises[&b1].third_party_reactions,
            registry_order_b.crises[&b2].third_party_reactions,
            "crisis B's outcome must not depend on insertion/processing order"
        );
    }

    /// 要求テスト11: 未登録の外交関係は既存の中立値(opinion=0.0)として扱われ、
    /// 同盟も無い場合は中立(支持なし)になる。
    #[test]
    fn unregistered_relation_is_treated_as_the_existing_neutral_default() {
        let power_registry = standard_power_registry(); // id=1がGreatPower
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(10),
                    ..CountryData::default()
                },
            ],
        };
        let diplomacy_registry = empty_diplomacy(); // 1-0, 1-10 とも一切未登録
        let mut crisis_registry = crisis_registry_with(crisis_with_support(0, 10, &[]));

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            &countries,
            &power_registry,
            &PlayerCountry(None),
            &before_deadline(),
        );

        assert!(
            crisis_registry
                .crises
                .values()
                .next()
                .unwrap()
                .third_party_reactions
                .is_empty(),
            "unregistered relations must be treated as opinion=0.0 (neutral), yielding Abstain"
        );
    }
}
