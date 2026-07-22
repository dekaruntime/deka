pub fn parse_package_spec(spec: &str) -> (String, Option<String>) {
    if spec.starts_with('@') {
        if let Some(pos) = spec[1..].find('@') {
            let split = pos + 1;
            let name = spec[..split].to_string();
            let version = spec[split + 1..].to_string();
            return (name, Some(version));
        }
        return (spec.to_string(), None);
    }

    if let Some(pos) = spec.rfind('@') {
        if pos > 0 {
            let name = spec[..pos].to_string();
            let version = spec[pos + 1..].to_string();
            return (name, Some(version));
        }
    }

    (spec.to_string(), None)
}
