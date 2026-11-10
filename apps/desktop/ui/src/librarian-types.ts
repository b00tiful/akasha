/** Existing Rust packet/review schemas. These types contain no retrieval policy. */
export interface LibrarianPacket {
  schema_version: number;
  project: string;
  question: string;
  terms: string[];
  note: string | null;
  scanned_notes: number;
  matched_notes: number;
  omitted_notes: number;
  evidence: Array<{
    citation: number; id: string; fingerprint: string; status: string | null;
    line_start: number; line_end: number; excerpt: string; excerpt_truncated: boolean;
  }>;
}

export interface LibrarianPassage {
  citation: number; id: string; fingerprint: string; status: string | null;
  line_start: number; line_end: number; partial_body: boolean; quote: string;
  links: Array<{ target: string; wikilink: string; line: number }>;
  omitted_links: number;
}

export interface LibrarianNoteProvenance {
  citation: number; evidence_state: string; unmatched_claims: number;
  claims: Array<{
    recorded_kind: string; claim: string; rationale: string | null; note_line: number;
    sources: Array<{
      path: string; recorded_fingerprint: string; observed_fingerprint: string | null;
      line_start: number | null; line_end: number | null; freshness: string;
      excerpt_state: string; excerpt: string | null;
    }>;
  }>;
}

export interface DesktopLibrarianReview {
  packet: LibrarianPacket;
  review: {
    schema_version: number; kind: "source_review"; highlighted_citations: number[];
    scanned_notes: number; matched_notes: number; omitted_notes: number;
    sources: {
      schema_version: number; kind: string; project: string; question: string;
      link_scope: string; matches: LibrarianPassage[];
    };
    provenance?: { association: string; notes: LibrarianNoteProvenance[] };
  };
}

export interface AssistedLibrarianReview {
  initial: DesktopLibrarianReview;
  expanded: DesktopLibrarianReview | null;
  terms: string[];
  raw_expansion: string;
  metrics: import("./onboarding-types").GeneratedOnboarding["metrics"][number];
}
