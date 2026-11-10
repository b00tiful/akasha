// Optional local query planning. Generated terms are retrieval hypotheses, never evidence.
import { readFileSync } from "node:fs";
export const QUERY_POLICY = readFileSync(new URL("./librarian-query-policy.txt", import.meta.url), "utf8").trimEnd();

export const QUERY_SCHEMA = {
  type: "object", additionalProperties: false, required: ["terms"],
  properties: { terms: { type: "array", maxItems: 8, items: { type: "string", minLength: 2, maxLength: 64 } } },
};

export function validateExpansion(value) {
  if (!value || Array.isArray(value) || Object.keys(value).length !== 1 || !Object.hasOwn(value, "terms")
    || !Array.isArray(value.terms) || value.terms.length > 8) throw new Error("Invalid query expansion shape");
  const terms = value.terms.map(term => {
    if (typeof term !== "string" || term !== term.trim() || [...term].length < 2 || [...term].length > 64
      || !/^[\p{L}\p{N}][\p{L}\p{N} _.-]*$/u.test(term)) throw new Error("Invalid literal query expansion term");
    return term;
  });
  // Repeated/case-equivalent terms have the same meaning to the core's folded set.
  return [...new Map(terms.map(term => [term.toLowerCase(), term])).values()];
}

export async function expandQuery(question, complete) {
  if (typeof question !== "string" || !question.trim() || [...question].length > 160
    || /[\p{C}]/u.test(question)) throw new Error("Invalid expansion question");
  const reply = await complete({ question }, QUERY_SCHEMA, QUERY_POLICY);
  try {
    if (typeof reply.answer !== "string" || Buffer.byteLength(reply.answer) > 4096) throw new Error("Oversized query expansion");
    return { ...reply, terms: validateExpansion(JSON.parse(reply.answer)) };
  } catch (error) {
    error.queryExpansion = { response: typeof reply.answer === "string" ? reply.answer.slice(0, 4096) : null, metrics: reply.metrics };
    throw error;
  }
}
