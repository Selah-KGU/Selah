use std::sync::LazyLock;

#[path = "catalog/names.rs"]
mod names;
#[path = "catalog/schema.rs"]
mod schema;
#[path = "catalog/specs.rs"]
mod specs;

#[cfg(test)]
use names::DISPATCH_TOOL_NAMES;
use names::TOOL_ALIASES;
pub(super) use schema::ArgSchema;
pub(super) use specs::TOOL_SPECS;

/// Check if a tool name is in the registry.
#[cfg(test)]
pub fn is_known_tool(name: &str) -> bool {
    canonical_tool_name(name).is_some()
}

#[cfg(test)]
pub fn registered_tool_names() -> impl Iterator<Item = &'static str> {
    TOOL_SPECS.iter().map(|spec| spec.name)
}

#[cfg(test)]
pub fn dispatched_tool_names() -> &'static [&'static str] {
    DISPATCH_TOOL_NAMES
}

pub fn exact_tool_name(name: &str) -> Option<&'static str> {
    let trimmed = name
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'');
    TOOL_SPECS
        .iter()
        .find(|spec| spec.name == trimmed)
        .map(|spec| spec.name)
}

pub fn canonical_tool_name(name: &str) -> Option<&'static str> {
    let trimmed = name
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'');
    if let Some(spec) = TOOL_SPECS.iter().find(|s| s.name == trimmed) {
        return Some(spec.name);
    }
    let trimmed = trimmed
        .rsplit([':', '.', '/'])
        .next()
        .unwrap_or(trimmed)
        .trim();
    if let Some(spec) = TOOL_SPECS.iter().find(|s| s.name == trimmed) {
        return Some(spec.name);
    }
    TOOL_ALIASES
        .iter()
        .find_map(|(alias, target)| (*alias == trimmed).then_some(*target))
}

/// Static description given to the model during the planning phase.
pub fn tool_catalog_prompt() -> &'static str {
    static TOOL_CATALOG_PROMPT: LazyLock<String> = LazyLock::new(build_tool_catalog_prompt);
    &TOOL_CATALOG_PROMPT
}

/// Names and argument shapes only. The full catalog does not fit Apple's 4096-token window.
pub fn tool_catalog_signatures() -> &'static str {
    static TOOL_CATALOG_SIGNATURES: LazyLock<String> = LazyLock::new(|| {
        TOOL_SPECS
            .iter()
            .map(|spec| format!("- {}", spec.signature))
            .collect::<Vec<_>>()
            .join("\n")
    });
    &TOOL_CATALOG_SIGNATURES
}

fn build_tool_catalog_prompt() -> String {
    let mut out = String::new();
    let mut current_category = "";
    for spec in TOOL_SPECS {
        if spec.category != current_category {
            if !out.is_empty() {
                out.push('\n');
            }
            current_category = spec.category;
            out.push_str(&format!("【{}】\n", spec.category));
        }
        out.push_str(&format!("- {}: {}\n", spec.signature, spec.purpose));
    }
    out.trim_end().to_string()
}

#[cfg(test)]
#[path = "catalog/tests.rs"]
mod tests;
