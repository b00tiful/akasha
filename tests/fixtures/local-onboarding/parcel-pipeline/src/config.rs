pub fn capacity(input: Option<&str>) -> Result<usize, &'static str> {
    let Some(input) = input else { return Ok(2) };
    let parsed = input.parse::<usize>().map_err(|_| "invalid capacity")?;
    if !(1..=16).contains(&parsed) { return Err("capacity outside 1..=16"); }
    Ok(parsed)
}
