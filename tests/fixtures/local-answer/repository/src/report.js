export function report(queue) {
  return queue.map(item => `${item.id}\t${item.attempts}`).join("\n");
}
