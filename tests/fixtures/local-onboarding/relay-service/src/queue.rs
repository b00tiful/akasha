use std::collections::VecDeque;
/// Queue rejects duplicate messages and refuses insertion when capacity is reached.
pub struct Queue {
    messages: VecDeque<String>,
    capacity: usize,
}
impl Queue {
    pub fn new(capacity: usize) -> Self {
        Self {
            messages: VecDeque::new(),
            capacity,
        }
    }
    pub fn push(&mut self, message: String) -> bool {
        if self.messages.len() >= self.capacity || self.messages.contains(&message) {
            return false;
        }
        self.messages.push_back(message);
        true
    }
    pub fn front(&self) -> Option<&str> {
        self.messages.front().map(String::as_str)
    }
    pub fn ack(&mut self) {
        self.messages.pop_front();
    }
    pub fn len(&self) -> usize {
        self.messages.len()
    }
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}
