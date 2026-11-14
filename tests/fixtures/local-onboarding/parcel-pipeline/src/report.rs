#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub sent: usize,
    pub failed: bool,
    pub remaining: usize,
}
impl Report {
    pub fn render(&self) -> String {
        format!("sent={} failed={} remaining={}", self.sent, self.failed, self.remaining)
    }
}
