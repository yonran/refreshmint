use std::path::{Path, PathBuf};

use rquickjs::loader::{Loader, Resolver};
use rquickjs::{Ctx, Error, Module, Result};

#[derive(Debug, Clone)]
pub(crate) struct RootedScriptModuleResolver {
    root: PathBuf,
    extensions: Vec<&'static str>,
}

impl RootedScriptModuleResolver {
    pub(crate) fn new(root: &Path, extensions: &[&'static str]) -> Self {
        Self {
            root: root.to_path_buf(),
            extensions: extensions.to_vec(),
        }
    }
}

impl Resolver for RootedScriptModuleResolver {
    fn resolve<'js>(&mut self, _ctx: &Ctx<'js>, base: &str, name: &str) -> Result<String> {
        resolve_existing_specifier(&self.root, &self.extensions, base, name)
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
        let source_path = self.root.join(specifier_to_path(name));
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
) -> Option<String> {
    let normalized = if name.starts_with('.') {
        let base_dir = base.rsplit_once('/').map_or("", |(dir, _)| dir);
        normalize_relative_specifier(base_dir, name)?
    } else {
        normalize_path_like_specifier(name)?
    };

    if specifier_exists(root, &normalized) {
        return Some(normalized);
    }

    if normalized.rsplit_once('.').is_some() {
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
    root.join(specifier_to_path(specifier)).is_file()
}

fn specifier_to_path(specifier: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for segment in specifier.split('/') {
        if !segment.is_empty() {
            path.push(segment);
        }
    }
    path
}

fn normalize_relative_specifier(base_dir: &str, name: &str) -> Option<String> {
    let mut segments = Vec::new();
    if !base_dir.is_empty() {
        segments.extend(base_dir.split('/').filter(|segment| !segment.is_empty()));
    }
    normalize_segments(segments, name.split('/'))
}

fn normalize_path_like_specifier(name: &str) -> Option<String> {
    normalize_segments(Vec::new(), name.split('/'))
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
            normalize_relative_specifier("", "./driver.mjs"),
            Some("driver.mjs".into())
        );
        assert_eq!(
            normalize_relative_specifier("nested", "../driver.mjs"),
            Some("driver.mjs".into())
        );
        assert_eq!(normalize_relative_specifier("", "../driver.mjs"), None);
    }
}
