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
