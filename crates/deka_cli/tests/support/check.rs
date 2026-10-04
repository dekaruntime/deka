use std::process::Output;

pub fn checked(output: Output, source: &str) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("[check] {source} - ok\n")
    );
}
