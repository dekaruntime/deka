pub fn parse_exit_code(message: &str) -> Option<i32> {
    let marker = "DekaExit:";
    let idx = message.find(marker)?;
    let tail = &message[idx + marker.len()..];
    let digits: String = tail
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<i32>().ok()
}
