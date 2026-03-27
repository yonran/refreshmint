use std::path::{Component, Path, PathBuf};

use rquickjs::loader::{Loader, Resolver};
use rquickjs::{Ctx, Error, Module, Result};

const EXPORTS_CONDITIONS: &[&str] = &["browser", "import", "default"];

#[derive(Debug, Clone)]
pub(crate) struct RootedScriptModuleResolver {
    root: PathBuf,
    extensions: Vec<&'static str>,
    allow_package_resolution: bool,
}

impl RootedScriptModuleResolver {
    pub(crate) fn new(
        root: &Path,
        extensions: &[&'static str],
        allow_package_resolution: bool,
    ) -> Self {
        Self {
            root: root.to_path_buf(),
            extensions: extensions.to_vec(),
            allow_package_resolution,
        }
    }
}

impl Resolver for RootedScriptModuleResolver {
    fn resolve<'js>(&mut self, _ctx: &Ctx<'js>, base: &str, name: &str) -> Result<String> {
        resolve_existing_specifier(
            &self.root,
            &self.extensions,
            base,
            name,
            self.allow_package_resolution,
        )
        .ok_or_else(|| Error::new_resolving(base, name))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RootedScriptModuleLoader {
    root: PathBuf,
}

impl RootedScriptModuleLoader {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }
}

impl Loader for RootedScriptModuleLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str) -> Result<Module<'js>> {
        let source_path = specifier_to_path(&self.root, name);
        let source = std::fs::read(&source_path)?;
        let source = match source_path.extension().and_then(std::ffi::OsStr::to_str) {
            Some("ts" | "mts") => crate::ts_strip::strip_typescript_module(name, &source)
                .map_err(|error| Error::new_loading_message(name, error))?,
            _ => source,
        };
        Module::declare(ctx.clone(), name, source)
    }
}

pub(crate) fn entry_module_specifier(root: &Path, entry_path: &Path) -> Result<String> {
    let relative = entry_path.strip_prefix(root).map_err(|_| {
        Error::new_loading_message(
            entry_path.display().to_string(),
            "entry path must be inside extension root",
        )
    })?;
    normalize_path_like_specifier(relative.to_string_lossy().as_ref()).ok_or_else(|| {
        Error::new_loading_message(
            entry_path.display().to_string(),
            "entry path resolved outside extension root",
        )
    })
}

fn resolve_existing_specifier(
    root: &Path,
    extensions: &[&'static str],
    base: &str,
    name: &str,
    allow_package_resolution: bool,
) -> Option<String> {
    let normalized = if name.starts_with('.') {
        normalize_relative_specifier(base, name)?
    } else if allow_package_resolution {
        resolve_package_specifier(root, base, name)
            .or_else(|| normalize_path_like_specifier(name))?
    } else {
        normalize_path_like_specifier(name)?
    };

    if specifier_exists(root, &normalized) {
        return Some(normalized);
    }

    if is_absolute_specifier(&normalized) || normalized.rsplit_once('.').is_some() {
        return None;
    }

    for extension in extensions {
        let candidate = format!("{normalized}.{extension}");
        if specifier_exists(root, &candidate) {
            return Some(candidate);
        }
    }

    None
}

fn specifier_exists(root: &Path, specifier: &str) -> bool {
    specifier_to_path(root, specifier).is_file()
}

fn specifier_to_path(root: &Path, specifier: &str) -> PathBuf {
    if is_absolute_specifier(specifier) {
        return PathBuf::from(specifier);
    }

    let mut path = root.to_path_buf();
    for segment in specifier.split('/') {
        if !segment.is_empty() {
            path.push(segment);
        }
    }
    path
}

fn normalize_relative_specifier(base: &str, name: &str) -> Option<String> {
    if is_absolute_specifier(base) {
        let base_dir = Path::new(base).parent()?;
        let resolved = normalize_relative_path(base_dir, name)?;
        return Some(resolved.to_string_lossy().into_owned());
    }

    let base_dir = base.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut segments = Vec::new();
    if !base_dir.is_empty() {
        segments.extend(base_dir.split('/').filter(|segment| !segment.is_empty()));
    }
    normalize_segments(segments, name.split('/'))
}

fn normalize_path_like_specifier(name: &str) -> Option<String> {
    normalize_segments(Vec::new(), name.split('/'))
}

fn is_absolute_specifier(specifier: &str) -> bool {
    Path::new(specifier).is_absolute()
}

fn normalize_relative_path(base_dir: &Path, name: &str) -> Option<PathBuf> {
    let mut resolved = base_dir.to_path_buf();
    for segment in Path::new(name).components() {
        match segment {
            Component::Normal(value) => resolved.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolved.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(resolved)
}

fn resolve_package_specifier(root: &Path, base: &str, name: &str) -> Option<String> {
    let (package_name, package_subpath) = split_package_specifier(name)?;
    let mut candidate_dir = if is_absolute_specifier(base) {
        Path::new(base).parent()?.to_path_buf()
    } else {
        let base_path = specifier_to_path(root, base);
        base_path.parent()?.to_path_buf()
    };

    loop {
        let package_dir = candidate_dir.join("node_modules").join(package_name);
        if let Some(entry) = resolve_package_entry(&package_dir, package_subpath) {
            return Some(entry.to_string_lossy().into_owned());
        }
        if !candidate_dir.pop() {
            return None;
        }
    }
}

fn split_package_specifier(name: &str) -> Option<(&str, Option<&str>)> {
    if name.starts_with('.') || name.starts_with('/') || name.is_empty() {
        return None;
    }

    if let Some(remainder) = name.strip_prefix('@') {
        let (scope, tail) = remainder.split_once('/')?;
        let (package, subpath) = match tail.split_once('/') {
            Some((package, subpath)) => (package, Some(subpath)),
            None => (tail, None),
        };
        if package.is_empty() {
            return None;
        }
        return Some((&name[..1 + scope.len() + 1 + package.len()], subpath));
    }

    match name.split_once('/') {
        Some((package, subpath)) => Some((package, Some(subpath))),
        None => Some((name, None)),
    }
}

fn resolve_package_entry(package_dir: &Path, subpath: Option<&str>) -> Option<PathBuf> {
    let package_json_path = package_dir.join("package.json");
    let package_json = std::fs::read_to_string(&package_json_path).ok()?;
    let package_value: serde_json::Value = serde_json::from_str(&package_json).ok()?;
    let package_type = package_value
        .get("type")
        .and_then(serde_json::Value::as_str);

    if let Some(target) = package_value
        .get("exports")
        .and_then(|exports| resolve_exports_target(exports, subpath))
        .and_then(|target| resolve_package_target(package_dir, package_type, target, true))
    {
        return Some(target);
    }

    if subpath.is_some() {
        return None;
    }

    if let Some(target) = package_value
        .get("module")
        .and_then(serde_json::Value::as_str)
        .and_then(|target| resolve_package_target(package_dir, package_type, target, true))
    {
        return Some(target);
    }

    package_value
        .get("main")
        .and_then(serde_json::Value::as_str)
        .and_then(|target| resolve_package_target(package_dir, package_type, target, false))
}

fn resolve_exports_target<'a>(
    exports: &'a serde_json::Value,
    subpath: Option<&str>,
) -> Option<&'a str> {
    let key = subpath
        .map(|value| format!("./{value}"))
        .unwrap_or_else(|| ".".to_string());
    if exports.is_string() {
        return (subpath.is_none()).then(|| exports.as_str()).flatten();
    }
    let export_value = exports
        .get(&key)
        .or_else(|| (subpath.is_none()).then(|| exports.get(".")).flatten())?;
    select_exports_condition(export_value)
}

fn select_exports_condition(value: &serde_json::Value) -> Option<&str> {
    if let Some(target) = value.as_str() {
        return Some(target);
    }
    let object = value.as_object()?;
    for condition in EXPORTS_CONDITIONS {
        if let Some(selected) = object.get(*condition).and_then(select_exports_condition) {
            return Some(selected);
        }
    }
    None
}

fn resolve_package_target(
    package_dir: &Path,
    package_type: Option<&str>,
    target: &str,
    allow_js_without_module_type: bool,
) -> Option<PathBuf> {
    if !(target.starts_with("./") || target.starts_with("../")) {
        return None;
    }
    let resolved = normalize_relative_path(package_dir, target)?;
    let extension = resolved.extension().and_then(std::ffi::OsStr::to_str);
    let is_esm = match extension {
        Some("mjs" | "mts") => true,
        Some("js" | "ts") => allow_js_without_module_type || package_type == Some("module"),
        _ => false,
    };
    (is_esm && resolved.is_file()).then_some(resolved)
}

fn normalize_segments<'a, I>(mut segments: Vec<&'a str>, tail: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    for segment in tail {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            _ => segments.push(segment),
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_relative_specifier_rejects_parent_escape() {
        assert_eq!(
            normalize_relative_specifier("driver.mjs", "./driver.mjs"),
            Some("driver.mjs".into())
        );
        assert_eq!(
            normalize_relative_specifier("nested/entry.mjs", "../driver.mjs"),
            Some("driver.mjs".into())
        );
        assert_eq!(
            normalize_relative_specifier("driver.mjs", "../driver.mjs"),
            None
        );
    }

    #[test]
    fn normalize_relative_specifier_supports_absolute_base_paths() -> std::io::Result<()> {
        let base = std::env::temp_dir()
            .join("refreshmint-js-loader")
            .join("driver.mjs");
        let helper = base
            .parent()
            .ok_or_else(|| std::io::Error::other("base path should have parent"))?
            .join("shared")
            .join("helper.mjs");
        assert_eq!(
            normalize_relative_specifier(base.to_string_lossy().as_ref(), "./shared/helper.mjs"),
            Some(helper.to_string_lossy().into_owned())
        );
        Ok(())
    }

    #[test]
    fn resolve_package_entry_prefers_browser_import_default_then_module() -> std::io::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "refreshmint-js-loader-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(std::io::Error::other)?
                .as_nanos()
        ));
        let package_dir = root.join("node_modules").join("demo");
        std::fs::create_dir_all(package_dir.join("dist"))?;
        std::fs::write(
            package_dir.join("package.json"),
            r#"{
  "name": "demo",
  "exports": {
    ".": {
      "default": "./dist/default.mjs",
      "import": "./dist/import.mjs",
      "browser": "./dist/browser.mjs"
    }
  },
  "module": "./dist/module.mjs",
  "main": "./dist/main.cjs"
}"#,
        )?;
        for file in [
            "browser.mjs",
            "import.mjs",
            "default.mjs",
            "module.mjs",
            "main.cjs",
        ] {
            std::fs::write(package_dir.join("dist").join(file), "// test\n")?;
        }

        let entry = resolve_package_entry(&package_dir, None)
            .ok_or_else(|| std::io::Error::other("package entry should resolve"))?;
        assert_eq!(entry, package_dir.join("dist/browser.mjs"));

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn resolve_package_specifier_searches_ancestor_node_modules() -> std::io::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "refreshmint-js-loader-ancestor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(std::io::Error::other)?
                .as_nanos()
        ));
        let extension_dir = root.join("builtin-extensions").join("demo");
        let node_modules = root.join("node_modules").join("demo-pkg").join("dist");
        std::fs::create_dir_all(extension_dir.join("src"))?;
        std::fs::create_dir_all(&node_modules)?;
        std::fs::write(
            node_modules
                .parent()
                .ok_or_else(|| std::io::Error::other("node_modules path should have parent"))?
                .join("package.json"),
            r#"{"name":"demo-pkg","module":"./dist/index.js"}"#,
        )?;
        std::fs::write(node_modules.join("index.js"), "export const ok = true;\n")?;

        let base = extension_dir.join("src").join("extract.mts");
        let resolved =
            resolve_package_specifier(&extension_dir, base.to_string_lossy().as_ref(), "demo-pkg");
        assert_eq!(
            resolved,
            Some(node_modules.join("index.js").display().to_string())
        );

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }
}
