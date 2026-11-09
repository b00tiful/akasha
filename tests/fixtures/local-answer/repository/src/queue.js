export function enqueue(queue, event, limit = 3) {
  if (queue.some(item => item.id === event.id)) return "duplicate";
  if (queue.length >= limit) return "full";
  queue.push({ ...event, attempts: 0 });
  return "queued";
}
