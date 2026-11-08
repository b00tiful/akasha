/// Format one tab-separated summary. This is formatting only, with no file I/O.
pub fn summary(accepted: usize, rejected: usize) -> String {
    format!("accepted\t{accepted}\nrejected\t{rejected}\n")
}
