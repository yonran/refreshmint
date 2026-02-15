pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::APP_VERSION;
    use serde_json::Value;
    use std::fs;

    fn read_version(path: &str) -> String {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(err) => {
                panic!("failed to read {path}: {err}");
            }
        };
        let json: Value = match serde_json::from_str(&contents) {
            Ok(json) => json,
            Err(err) => {
                panic!("failed to parse {path}: {err}");
            }
        };
        match json.get("version").and_then(Value::as_str) {
            Some(version) => version.to_string(),
            None => {
                panic!("missing version field in {path}");
            }
        }
    }

    #[test]
    fn versions_match_package_and_tauri_config() {
        let package_version = read_version("../package.json");
        let tauri_version = read_version("tauri.conf.json");

        if package_version != APP_VERSION {
            panic!("package.json version {package_version} does not match {APP_VERSION}");
        }

        if tauri_version != APP_VERSION {
            panic!("tauri.conf.json version {tauri_version} does not match {APP_VERSION}");
        }
    }
}
