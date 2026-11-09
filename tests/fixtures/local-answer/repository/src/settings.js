export function readSettings(env) {
  const maxPending = Number(env.MAX_PENDING ?? 3);
  if (!Number.isInteger(maxPending) || maxPending < 1) throw new Error("invalid MAX_PENDING");
  return { maxPending };
}
