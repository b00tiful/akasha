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
use serde::Serialize;
use serde_json::{Value, json};
use std::path::PathBuf;

const POLICY: &str = include_str!("onboarding-policy.txt");
const PLANNING_RULE: &str = "This is a filename-role group, not an inferred component. Return exactly this slot with evidence only. The host supplies all metadata. Summarize useful observed behavior from the supplied excerpts: implementation conditions, defaults, return values and boundaries; concrete test inputs/assertions; manifest requirements and overview scope. Prefer behavior over comments describing source selection. Do not generalize a test beyond its actual inputs. Cover distinct useful sources within the four-claim limit; report unknowns when partial source prevents a conclusion.";

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
            let (input, schema) = group_input(&packet, slot);
            let (answer, measurement) = worker.complete(&input, &schema, POLICY).await?;
            notes.push(remap(&answer, slot, &packet)?);
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
) -> Result<GeneratedOnboarding, DesktopError> {
    let lease = jobs.claim(&request_id)?;
    generate_onboarding(
        root,
        project,
        packet_json,
        profile,
        lease.cancellation.clone(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
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
