//! Native draft orchestration; source identities, metadata and publication remain in core.
#[cfg(feature = "desktop")]
use crate::inference::InferenceJobs;
use crate::{
    DesktopError,
    inference::{self, Cancellation, CompletionMetrics, NativeInference, Profile},
    onboarding::{DesktopOnboardingReview, review_onboarding_plan},
    request,
};
use akasha_core::onboarding_local::{
    LocalOnboardingPacket, LocalOnboardingSlot, plan_local_onboarding,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

const POLICY: &str = include_str!("onboarding-policy.txt");
const PLANNING_RULE: &str = "This is a filename-role group, not an inferred component. Return exactly this slot with evidence only. The host supplies all metadata. Summarize useful observed behavior from the supplied excerpts: implementation conditions, defaults, return values and boundaries; concrete test inputs/assertions; manifest requirements and overview scope. Prefer behavior over comments describing source selection. Do not generalize a test beyond its actual inputs. Cover distinct useful sources within the four-claim limit; report unknowns when partial source prevents a conclusion.";

/// Ephemeral opt-in generation strategy. Existing callers retain the compact role summary.
#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftMode {
    #[default]
    Summary,
    PerFile,
    BehaviorMap,
}

const PER_FILE_POLICY: &str = include_str!("onboarding-per-file-policy.txt");
const BEHAVIOR_POLICY: &str = include_str!("onboarding-behavior-policy.txt");

// Every source gets room for one observation. Share the remaining eight-claim budget
// by captured text size, rather than letting a verbose first file starve later files.
fn behavior_quotas(packet: &LocalOnboardingPacket, slot: &LocalOnboardingSlot) -> Vec<usize> {
    let mut quotas = vec![1; slot.sources.len()];
    for _ in slot.sources.len()..8 {
        let next = slot
            .sources
            .iter()
            .enumerate()
            .filter(|(index, _)| quotas[*index] < 4)
            .max_by_key(|(index, handle)| {
                (
                    packet.sources[**handle - 1].text.len() / (quotas[*index] + 1),
                    std::cmp::Reverse(*index),
                )
            });
        let Some((index, _)) = next else { break };
        quotas[index] += 1;
    }
    quotas
}

fn behavior_input(packet: &LocalOnboardingPacket, slot: &LocalOnboardingSlot) -> (Value, Value) {
    let (mut input, mut schema) = per_file_input(packet, slot);
    input["planning_rule"] = "For every source return an array of distinct, complete observations, from one up to its claim_limit. Describe operations with their conditions and results; include error precedence, retained/changed state and boundary assertions when visible. Use short complete sentences and separate independent behaviors. For tests, cover distinct concrete scenarios across all supplied excerpts without generalizing. For manifests and overviews record only useful scope/configuration. Never pad to the limit or repeat declarations. An unknown with rationale and no citations is valid. Cite only that file's excerpts covering the entire observation, including adjacent windows when needed.".into();
    input["source_role"] = slot.role.clone().into();
    for (index, quota) in behavior_quotas(packet, slot).iter().enumerate() {
        input["sources"][index]["claim_limit"] = (*quota).into();
        let item = &mut schema["properties"]["observations"]["properties"][(index + 1).to_string()];
        *item = json!({"type":"array","minItems":1,"maxItems":quota,"items":item.clone()});
    }
    (input, schema)
}

fn per_file_input(packet: &LocalOnboardingPacket, slot: &LocalOnboardingSlot) -> (Value, Value) {
    let (mut input, _) = group_input(packet, slot);
    input.as_object_mut().unwrap().remove("planned_note");
    input["planning_rule"] = "Return observations with exactly one claim per supplied source handle. Read each source independently. Prefer concrete conditions, exceptions, state changes and test assertions over declarations or imports. Keep each claim concise and cite only that source's supporting excerpts. Use unknown with a rationale and no sources when no useful supported observation is possible. A file observation does not imply complete behavior coverage.".into();
    let mut observations = serde_json::Map::new();
    for (local, handle) in slot.sources.iter().enumerate() {
        let source = object(
            json!({"source":{"const":local+1},"excerpt":{"type":"integer","enum":(1..=packet.sources[handle-1].excerpts.len()).collect::<Vec<_>>()}}),
        );
        let text = json!({"type":"string","minLength":1,"maxLength":512});
        let claims = ["fact", "inference", "unknown"].iter().map(|kind| {
            let mut properties = json!({"kind":{"const":kind},"claim":text,"sources":{"type":"array","minItems":if *kind == "unknown" {0} else {1},"maxItems":if *kind == "unknown" {0} else {4},"items":source}});
            if *kind != "fact" { properties["rationale"] = text.clone(); }
            object(properties)
        }).collect::<Vec<_>>();
        observations.insert((local + 1).to_string(), json!({"oneOf":claims}));
    }
    (
        input,
        object(json!({"observations":object(observations.into())})),
    )
}

fn remap_per_file(
    answer: &str,
    slot: &LocalOnboardingSlot,
    packet: &LocalOnboardingPacket,
) -> Result<Value, DesktopError> {
    remap_observations(answer, slot, packet, false)
}

fn remap_observations(
    answer: &str,
    slot: &LocalOnboardingSlot,
    packet: &LocalOnboardingPacket,
    behavior: bool,
) -> Result<Value, DesktopError> {
    let value: Value = serde_json::from_str(answer)
        .map_err(|_| inference::error("Invalid per-file draft JSON"))?;
    let root = value
        .as_object()
        .ok_or_else(|| inference::error("Invalid per-file draft"))?;
    let observations = value["observations"]
        .as_object()
        .filter(|items| root.len() == 1 && items.len() == slot.sources.len())
        .ok_or_else(|| {
            inference::error("Per-file draft must address every assigned source exactly once")
        })?;
    let mut claims = Vec::new();
    let quotas = behavior_quotas(packet, slot);
    for (local, handle) in slot.sources.iter().enumerate() {
        let observation = observations
            .get(&(local + 1).to_string())
            .ok_or_else(|| inference::error("Missing per-file observation"))?;
        let items = if behavior {
            observation
                .as_array()
                .filter(|items| !items.is_empty() && items.len() <= quotas[local])
                .ok_or_else(|| {
                    inference::error("Behavior observations exceed their source claim limit")
                })?
                .iter()
                .collect::<Vec<_>>()
        } else {
            vec![observation]
        };
        for claim in items {
            let sources = claim["sources"]
                .as_array()
                .ok_or_else(|| inference::error("Invalid per-file sources"))?;
            if sources
                .iter()
                .any(|source| source["source"].as_u64() != Some((local + 1) as u64))
            {
                return Err(inference::error(
                    "Per-file observation cites another source",
                ));
            }
            let mut claim = claim.clone();
            // Keep an unknown bound to its assigned path even though it has no evidence references.
            // Only host-owned paths are inserted. Core still validates all text and claim fields.
            if claim["kind"] == "unknown" {
                if !sources.is_empty() {
                    return Err(inference::error(
                        "Unknown observations cannot cite evidence",
                    ));
                }
                let text = claim["claim"]
                    .as_str()
                    .ok_or_else(|| inference::error("Invalid unknown claim"))?;
                claim["claim"] = format!("{}: {text}", packet.sources[handle - 1].path).into();
            } else if sources.is_empty() {
                return Err(inference::error(
                    "Supported observations require their assigned source",
                ));
            }
            claims.push(claim);
        }
    }
    remap(
        &json!({"schema_version":5,"notes":[{"slot":slot.slot,"evidence":claims}]}).to_string(),
        slot,
        packet,
    )
}

#[derive(Serialize)]
pub struct GeneratedOnboarding {
    pub plan_json: String,
    pub review: DesktopOnboardingReview,
    pub metrics: Vec<CompletionMetrics>,
}
fn object(properties: Value) -> Value {
    json!({"type":"object", "additionalProperties":false, "required":properties.as_object().unwrap().keys().collect::<Vec<_>>(), "properties":properties})
}
pub(crate) fn group_input(
    packet: &LocalOnboardingPacket,
    slot: &LocalOnboardingSlot,
) -> (Value, Value) {
    let sources = slot.sources.iter().enumerate().map(|(local, handle)| {
        let source = &packet.sources[handle - 1];
        let mut value = json!({"handle":local+1,"path":source.path,"excerpts": source.excerpts.iter().enumerate().map(|(index, excerpt)| json!({"handle":index+1,"line_start":excerpt.line_start,"line_end":excerpt.line_end,"text":source.text[excerpt.byte_start..excerpt.byte_end]})).collect::<Vec<_>>()});
        if let Some(sampling) = &source.sampling { value["sampling"] = serde_json::to_value(sampling).unwrap(); value["sampling"]["total_lines"] = source.lines.into(); } value
    }).collect::<Vec<_>>();
    let mut input = json!({"planned_note":{"slot":slot.slot,"role":slot.role},"planning_rule":PLANNING_RULE,"sources":sources});
    if let Some(focus) = &packet.focus {
        input["source_focus"] = json!(focus.terms);
        input["focus_rule"] = "Literal focus guides source targeting, not factual support. Overview and manifests may supply general context; omitted lines and files remain unknown.".into();
    }
    let text = json!({"type":"string","minLength":1,"maxLength":1024});
    let source = json!({"oneOf":slot.sources.iter().enumerate().map(|(local, handle)| object(json!({"source":{"const":local+1},"excerpt":{"type":"integer","enum":(1..=packet.sources[handle-1].excerpts.len()).collect::<Vec<_>>()}}))).collect::<Vec<_>>()});
    let claims = ["fact","inference","unknown"].iter().map(|kind| {
        let mut properties = json!({"kind":{"const":kind},"claim":text,"sources":{"type":"array","minItems":if *kind == "unknown" {0} else {1},"maxItems":if *kind == "unknown" {0} else {4},"items":source}});
        if *kind != "fact" { properties["rationale"] = text.clone(); } object(properties)
    }).collect::<Vec<_>>();
    let note = object(
        json!({"slot":{"const":slot.slot},"evidence":{"type":"array","minItems":1,"maxItems":4,"items":{"oneOf":claims}}}),
    );
    (
        input,
        object(
            json!({"schema_version":{"const":5},"notes":{"type":"array","minItems":1,"maxItems":1,"items":note}}),
        ),
    )
}
pub(crate) fn remap(
    answer: &str,
    slot: &LocalOnboardingSlot,
    packet: &LocalOnboardingPacket,
) -> Result<Value, DesktopError> {
    // Strict decoding, full claim validation and group membership are enforced by core after
    // remapping. Reject a local handle before indexing; never trust schema enforcement alone.
    let mut value: Value =
        serde_json::from_str(answer).map_err(|_| inference::error("Invalid local draft JSON"))?;
    let root = value
        .as_object()
        .ok_or_else(|| inference::error("Invalid local plan"))?;
    if root.len() != 2
        || value["schema_version"] != 5
        || value["notes"]
            .as_array()
            .is_none_or(|notes| notes.len() != 1)
        || value["notes"][0]["slot"] != slot.slot
    {
        return Err(inference::error(
            "Local completion must produce exactly its assigned slot",
        ));
    }
    let note = &mut value["notes"][0];
    let claims = note["evidence"]
        .as_array_mut()
        .ok_or_else(|| inference::error("Missing local evidence claims"))?;
    for claim in claims {
        for reference in claim["sources"]
            .as_array_mut()
            .ok_or_else(|| inference::error("Invalid local source references"))?
        {
            let local = reference["source"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .and_then(|n| n.checked_sub(1))
                .and_then(|n| slot.sources.get(n))
                .ok_or_else(|| inference::error("Unknown completion-local source handle"))?;
            let excerpt = reference["excerpt"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n >= 1 && *n <= packet.sources[local - 1].excerpts.len())
                .ok_or_else(|| inference::error("Unknown completion-local excerpt handle"))?;
            reference["source"] = (*local).into();
            reference["excerpt"] = excerpt.into();
        }
    }
    Ok(note.clone())
}

pub async fn generate_onboarding(
    root: Option<PathBuf>,
    project: String,
    packet_json: String,
    profile: Profile,
    cancellation: Cancellation,
) -> Result<GeneratedOnboarding, DesktopError> {
    generate_onboarding_with_mode(
        root,
        project,
        packet_json,
        profile,
        cancellation,
        DraftMode::Summary,
    )
    .await
}

pub async fn generate_onboarding_with_mode(
    root: Option<PathBuf>,
    project: String,
    packet_json: String,
    profile: Profile,
    cancellation: Cancellation,
    mode: DraftMode,
) -> Result<GeneratedOnboarding, DesktopError> {
    cancellation.check()?;
    if packet_json.len() > 16000 {
        return Err(inference::error("Captured packet exceeds 16000 bytes"));
    }
    let packet: LocalOnboardingPacket = serde_json::from_str(&packet_json)
        .map_err(|_| inference::error("Invalid captured packet"))?;
    let req = request(root.clone(), Some(project.clone()))?;
    // Rebuild before any project evidence can reach a process.
    let identities = plan_local_onboarding(&req, &packet).map_err(|e| DesktopError {
        code: e.exit_code(),
        message: e.to_string(),
    })?;
    if identities
        .slots
        .iter()
        .any(|slot| !slot.unresolved_fields.is_empty())
    {
        return Err(inference::error(
            "Automatic metadata has unresolved required fields; use a reviewed explicit plan",
        ));
    }
    let mut worker = NativeInference::open(profile, cancellation.clone()).await?;
    let result = async {
        let mut notes = Vec::new();
        let mut metrics = Vec::new();
        for slot in &identities.slots {
            cancellation.check()?;
            if matches!(mode, DraftMode::BehaviorMap) {
                let mut evidence = Vec::new();
                for (handle, quota) in slot.sources.iter().zip(behavior_quotas(&packet, slot)) {
                    cancellation.check()?;
                    // No neighboring source text enters this completion. Preserve the original
                    // packet handle when assembling the ordinary core-owned slot afterwards.
                    let isolated = LocalOnboardingSlot {
                        slot: slot.slot,
                        role: slot.role.clone(),
                        note_type: slot.note_type.clone(),
                        name: slot.name.clone(),
                        path: slot.path.clone(),
                        sources: vec![*handle],
                        fields: slot.fields.clone(),
                        unresolved_fields: slot.unresolved_fields.clone(),
                    };
                    let (mut input, mut schema) = behavior_input(&packet, &isolated);
                    input["sources"][0]["claim_limit"] = quota.into();
                    schema["properties"]["observations"]["properties"]["1"]["maxItems"] =
                        quota.into();
                    let (answer, measurement) =
                        worker.complete(&input, &schema, BEHAVIOR_POLICY).await?;
                    let note = remap_observations(&answer, &isolated, &packet, true)?;
                    let claims = note["evidence"].as_array().unwrap();
                    if claims.len() > quota {
                        return Err(inference::error(
                            "Behavior draft exceeds its source claim limit",
                        ));
                    }
                    evidence.extend(claims.iter().cloned());
                    metrics.push(measurement);
                }
                notes.push(json!({"slot":slot.slot,"evidence":evidence}));
                continue;
            }
            let (input, schema, policy) = match mode {
                DraftMode::Summary => {
                    let (input, schema) = group_input(&packet, slot);
                    (input, schema, POLICY)
                }
                DraftMode::PerFile => {
                    let (input, schema) = per_file_input(&packet, slot);
                    (input, schema, PER_FILE_POLICY)
                }
                DraftMode::BehaviorMap => unreachable!("handled with source-isolated completions"),
            };
            let (answer, measurement) = worker.complete(&input, &schema, policy).await?;
            notes.push(match mode {
                DraftMode::Summary => remap(&answer, slot, &packet)?,
                DraftMode::PerFile => remap_per_file(&answer, slot, &packet)?,
                DraftMode::BehaviorMap => unreachable!("handled with source-isolated completions"),
            });
            metrics.push(measurement);
        }
        let plan_json = json!({"schema_version":5,"notes":notes}).to_string();
        Ok::<_, DesktopError>((plan_json, metrics))
    }
    .await;
    // Always reap/remove before freshness checking and returning a draft to the UI.
    worker.close().await?;
    let (plan_json, metrics) = result?;
    cancellation.check()?;
    let review = review_onboarding_plan(root, project, &packet_json, &plan_json)?;
    cancellation.check()?;
    Ok(GeneratedOnboarding {
        plan_json,
        review,
        metrics,
    })
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn local_inference_availability() -> inference::InferenceAvailability {
    inference::availability()
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn reserve_local_generation(
    jobs: tauri::State<'_, InferenceJobs>,
) -> Result<String, DesktopError> {
    jobs.reserve()
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn cancel_local_generation(
    jobs: tauri::State<'_, InferenceJobs>,
    request_id: String,
) -> Result<(), DesktopError> {
    jobs.cancel(&request_id)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn generate_local_onboarding(
    jobs: tauri::State<'_, InferenceJobs>,
    request_id: String,
    root: Option<PathBuf>,
    project: String,
    packet_json: String,
    profile: Profile,
    mode: Option<DraftMode>,
) -> Result<GeneratedOnboarding, DesktopError> {
    let lease = jobs.claim(&request_id)?;
    generate_onboarding_with_mode(
        root,
        project,
        packet_json,
        profile,
        lease.cancellation.clone(),
        mode.unwrap_or_default(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn per_file_protocol_requires_every_source_and_keeps_unknowns_uncited() {
        let packet: LocalOnboardingPacket = serde_json::from_value(json!({"schema_version":2,"preparation":{},"sources":[{"path":"a.rs","fingerprint":"a","text":"a\n","lines":1,"excerpts":[{"byte_start":0,"byte_end":2,"line_start":1,"line_end":1}]},{"path":"b.rs","fingerprint":"b","text":"b\n","lines":1,"excerpts":[{"byte_start":0,"byte_end":2,"line_start":1,"line_end":1}]}],"index":"","roadmap":""})).unwrap();
        let slot = LocalOnboardingSlot {
            slot: 2,
            role: "implementation".into(),
            note_type: "entity".into(),
            name: "group".into(),
            path: "group.md".into(),
            sources: vec![2, 1],
            fields: Default::default(),
            unresolved_fields: vec![],
        };
        let (input, schema) = per_file_input(&packet, &slot);
        assert_eq!(input["sources"][0]["path"], "b.rs");
        assert!(input.get("planned_note").is_none());
        assert_eq!(
            schema["properties"]["observations"]["required"],
            json!(["1", "2"])
        );
        let fact = json!({"kind":"fact","claim":"b behavior","sources":[{"source":1,"excerpt":1}]});
        let unknown = json!({"kind":"unknown","claim":"Behavior cannot be determined.","rationale":"Insufficient evidence.","sources":[]});
        let answer = json!({"observations":{"1":fact,"2":unknown}});
        let note = remap_per_file(&answer.to_string(), &slot, &packet).unwrap();
        assert_eq!(note["slot"], 2);
        assert_eq!(note["evidence"][0]["sources"][0]["source"], 2);
        assert_eq!(
            note["evidence"][1]["claim"],
            "a.rs: Behavior cannot be determined."
        );
        assert_eq!(note["evidence"][1]["sources"], json!([]));
        let (behavior_input, behavior_schema) = behavior_input(&packet, &slot);
        assert_eq!(behavior_input["sources"][0]["claim_limit"], 4);
        assert_eq!(
            behavior_schema["properties"]["observations"]["properties"]["1"]["maxItems"],
            4
        );
        let behavior =
            json!({"observations":{"1":[fact.clone(), fact.clone()],"2":[unknown.clone()]}});
        let mapped = remap_observations(&behavior.to_string(), &slot, &packet, true).unwrap();
        assert_eq!(mapped["evidence"].as_array().unwrap().len(), 3);
        assert_eq!(mapped["evidence"][1]["sources"][0]["source"], 2);
        assert_eq!(
            mapped["evidence"][2]["claim"],
            "a.rs: Behavior cannot be determined."
        );
        for value in [json!([]), json!(vec![fact.clone(); 5]), fact.clone()] {
            let mut invalid = behavior.clone();
            invalid["observations"]["1"] = value;
            assert!(remap_observations(&invalid.to_string(), &slot, &packet, true).is_err());
        }
        let mut cases = vec![json!({}), json!({"observations":[]})];
        for key in ["1", "2"] {
            let mut missing = answer.clone();
            missing["observations"].as_object_mut().unwrap().remove(key);
            cases.push(missing);
        }
        let mut extra = answer.clone();
        extra["observations"]["3"] = fact.clone();
        cases.push(extra);
        let mut replaced = answer.clone();
        replaced["observations"]
            .as_object_mut()
            .unwrap()
            .remove("2");
        replaced["observations"]["0"] = unknown;
        cases.push(replaced);
        let mut extra = answer.clone();
        extra["metadata"] = json!({});
        cases.push(extra);
        for reference in [
            json!({"source":2,"excerpt":1}),
            json!({"source":1,"excerpt":0}),
            json!({"source":1,"excerpt":2}),
            json!({"source":1.5,"excerpt":1}),
        ] {
            let mut invalid = answer.clone();
            invalid["observations"]["1"]["sources"] = json!([reference]);
            cases.push(invalid);
        }
        let mut empty_fact = answer.clone();
        empty_fact["observations"]["1"]["sources"] = json!([]);
        cases.push(empty_fact);
        let mut cited_unknown = answer;
        cited_unknown["observations"]["2"]["sources"] = json!([{"source":2,"excerpt":1}]);
        cases.push(cited_unknown);
        for invalid in cases {
            // The behavior strategy must reject the same missing/foreign/altered handles.
            let mut behavior = invalid.clone();
            if let Some(observations) = behavior["observations"].as_object_mut() {
                for value in observations.values_mut() {
                    *value = json!([value.clone()]);
                }
            }
            assert!(
                remap_observations(&behavior.to_string(), &slot, &packet, true).is_err(),
                "{behavior}"
            );
            assert!(
                remap_per_file(&invalid.to_string(), &slot, &packet).is_err(),
                "{invalid}"
            );
        }
        assert!(serde_json::from_str::<DraftMode>("\"automatic_publish\"").is_err());
        let mut slot = slot;
        for count in 1..=8 {
            let mut packet = packet.clone();
            packet.sources = (0..count)
                .map(|i| {
                    let mut source = packet.sources[0].clone();
                    source.text = "x".repeat((i + 1) * 100);
                    source
                })
                .collect();
            slot.sources = (1..=count).collect();
            let quotas = behavior_quotas(&packet, &slot);
            assert_eq!(quotas.len(), count);
            assert!(quotas.iter().all(|q| (1..=4).contains(q)));
            assert_eq!(quotas.iter().sum::<usize>(), 8.min(count * 4));
            slot.sources.reverse();
            assert_eq!(
                behavior_quotas(&packet, &slot).iter().sum::<usize>(),
                quotas.iter().sum::<usize>()
            );
        }
    }

    #[test]
    fn local_source_remapping_refuses_wrong_slot_handles_and_unexpected_fields() {
        let packet: LocalOnboardingPacket = serde_json::from_value(json!({"schema_version":2,"preparation":{},"sources":[{"path":"a.rs","fingerprint":"a","text":"a\n","lines":1,"excerpts":[{"byte_start":0,"byte_end":2,"line_start":1,"line_end":1}]},{"path":"b.rs","fingerprint":"b","text":"b\n","lines":1,"excerpts":[{"byte_start":0,"byte_end":2,"line_start":1,"line_end":1}]}],"index":"","roadmap":""})).unwrap();
        let slot = LocalOnboardingSlot {
            slot: 2,
            role: "implementation".into(),
            note_type: "entity".into(),
            name: "group".into(),
            path: "group.md".into(),
            sources: vec![2],
            fields: Default::default(),
            unresolved_fields: vec![],
        };
        let answer = json!({"schema_version":5,"notes":[{"slot":2,"evidence":[{"kind":"fact","claim":"b","sources":[{"source":1,"excerpt":1}]}]}]});
        let (input, schema) = group_input(&packet, &slot);
        assert_eq!(input["sources"][0]["path"], "b.rs");
        assert_eq!(input["sources"][0]["handle"], 1);
        assert_eq!(schema["properties"]["schema_version"]["const"], 5);
        assert_eq!(
            remap(&answer.to_string(), &slot, &packet).unwrap()["evidence"][0]["sources"][0]["source"],
            2
        );
        for (key, value) in [
            ("source", json!(0)),
            ("source", json!(2)),
            ("excerpt", json!(2)),
            ("source", json!(1.5)),
        ] {
            let mut invalid = answer.clone();
            invalid["notes"][0]["evidence"][0]["sources"][0][key] = value;
            assert!(remap(&invalid.to_string(), &slot, &packet).is_err());
        }
        let mut invalid = answer.clone();
        invalid["notes"][0]["slot"] = 1.into();
        assert!(remap(&invalid.to_string(), &slot, &packet).is_err());
        let mut invalid = answer;
        invalid["extra"] = true.into();
        assert!(remap(&invalid.to_string(), &slot, &packet).is_err());
    }
}
