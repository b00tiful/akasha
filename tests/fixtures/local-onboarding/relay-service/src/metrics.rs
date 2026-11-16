/// Dispatch metrics count only messages successfully encoded and stored in the current drain.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub stored: usize,
    pub blocked: bool,
    pub remaining: usize,
}
