use std::collections::VecDeque;

pub struct Queue {
    capacity: usize,
    pending: VecDeque<String>,
}
impl Queue {
    pub fn new(capacity: usize) -> Result<Self, &'static str> {
        if capacity == 0 { return Err("zero capacity"); }
        Ok(Self { capacity, pending: VecDeque::new() })
    }
    pub fn submit(&mut self, id: &str) -> Result<bool, &'static str> {
        if self.pending.iter().any(|item| item == id) { return Ok(false); }
        if self.pending.len() >= self.capacity { return Err("full"); }
        self.pending.push_back(id.to_owned());
        Ok(true)
    }
    pub fn front(&self) -> Option<&str> { self.pending.front().map(String::as_str) }
    pub fn acknowledge(&mut self) { self.pending.pop_front(); }
    pub fn len(&self) -> usize { self.pending.len() }
    pub fn is_empty(&self) -> bool { self.pending.is_empty() }
}
