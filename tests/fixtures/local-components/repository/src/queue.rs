use std::collections::VecDeque;

/// In-memory FIFO. No disk persistence or network transport is implemented here.
#[derive(Default)]
pub struct JobQueue {
    pending: VecDeque<String>,
    capacity: usize,
}
impl JobQueue {
    pub fn new(capacity: usize) -> Self {
        Self { pending: VecDeque::new(), capacity }
    }
    pub fn enqueue(&mut self, job: String) -> Result<(), String> {
        if self.pending.len() >= self.capacity { return Err(job); }
        self.pending.push_back(job);
        Ok(())
    }
    pub fn dequeue(&mut self) -> Option<String> { self.pending.pop_front() }
}
