/// Parses a positive queue capacity. Missing values default to 32.
/// Callers supply the string; this module does not read environment variables.
pub fn parse_capacity(value: Option<&str>) -> Result<usize, &'static str> {
    match value {
        None => Ok(32),
        Some(raw) => raw.parse::<usize>().ok().filter(|n| *n > 0).ok_or("invalid capacity"),
    }
}
