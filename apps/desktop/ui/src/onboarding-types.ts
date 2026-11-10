/** Existing core plan/report shapes; validation and publication belong to Rust. */
export interface OnboardingClaim {
  kind: string; claim: string; rationale?: string;
  sources: Array<{ source: number; excerpt: number }>;
}
export interface OnboardingPlan {
  schema_version: number;
  notes: Array<{ slot?: number; name?: string; note_type?: string; fields?: Record<string, string>; evidence: OnboardingClaim[] }>;
}
export interface OnboardingReview {
  draft: {
    proposal: { notes: Array<{ note_type: string; path: string; source: string }>; index: string; roadmap: string };
    preview: {
      root: string; project: string; preview_id: string;
      notes: Array<{ path: string; action: string; evidence_claims: number }>;
      index_changed: boolean; roadmap_changed: boolean; state_changed: boolean;
    };
    review: {
      requires_review: boolean;
      notes: Array<{ path: string; fields: Record<string, unknown> }>;
      uncited_sources: string[]; sampled_sources: unknown[]; uncited_focus_excerpts?: unknown[];
    };
  };
  identities: { slots: Array<{ slot: number; role: string; sources: number[] }> } | null;
  excerpts: Array<{ source: number; excerpt: number; path: string; line_start: number; line_end: number; text: string }>;
}
export interface OnboardingResult {
  root: string; project: string; created_notes: string[]; unchanged_notes: string[];
  updated_projections: string[]; state: string;
}

export interface OnboardingSelection {
  mode: "discover" | "files"; paths: string[]; focus: string[]; sample_large_files: boolean;
}
export interface OnboardingSources {
  root: string; project: string; packet_json: string;
  identities: { slots: Array<{ slot: number; role: string; sources: number[]; path: string; fields: Record<string, string>; unresolved_fields?: string[] }> };
  excerpts: OnboardingReview["excerpts"];
  sampled_sources: unknown[];
  coverage: {
    directories: string[]; entries_inspected: number; candidates: number;
    selected: Array<{ path: string; role: string; matched_terms?: string[] }>;
    implementation_groups: Array<{ directory: string; candidates: number; selected: number }>;
    omitted: Array<{ path: string; reason: string }>;
    excluded_entries: number; excluded_examples: Array<{ path: string; reason: string }>;
  } | null;
}

export interface InferenceAvailability {
  supported: boolean; model_present: boolean; vulkan_present: boolean; cpu_present: boolean;
  location: string; model: string;
}
export interface GeneratedOnboarding {
  plan_json: string; review: OnboardingReview;
  metrics: Array<{ context: number; prompt_tokens: number; model_sha256: string; server_sha256: string; profile: "vulkan" | "cpu"; worker_starts: number }>;
}
