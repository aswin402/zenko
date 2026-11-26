//! Zero-LLM, zero-regex Repo-Native Service & Ownership Briefing parser.
//! Reads open-standard CNCF Backstage `catalog-info.yaml` / `catalog-info.yml`
//! and GitHub/GitLab `CODEOWNERS` files through kernel-confined `openat(O_NOFOLLOW)`
//! handles and distills them into a single compact Turn-1 reconnaissance line
//! (0 bytes emitted when neither file exists in the workspace).

use crate::jail::read_jailed_string_bounded;
use std::path::Path;
use zenko_core::{
    SERVICE_BRIEF_ANNOTATION_MAX_CHARS, SERVICE_BRIEF_MAX_APIS, SERVICE_BRIEF_MAX_DEPS,
    SERVICE_BRIEF_MAX_FILE_BYTES, SERVICE_BRIEF_MAX_LINKS, SERVICE_BRIEF_MAX_OWNERS,
    SERVICE_BRIEF_MAX_SCOPED_RULES, SERVICE_CATALOG_ANNOTATION_KEYS, SERVICE_CATALOG_FILES,
    SERVICE_CODEOWNERS_PATHS, SERVICE_YAML_DOC_END, SERVICE_YAML_DOC_START,
    SERVICE_YAML_KEY_ANNOTATIONS, SERVICE_YAML_KEY_DEPENDS_ON, SERVICE_YAML_KEY_KIND,
    SERVICE_YAML_KEY_LIFECYCLE, SERVICE_YAML_KEY_METADATA, SERVICE_YAML_KEY_NAME,
    SERVICE_YAML_KEY_OWNER, SERVICE_YAML_KEY_PROVIDES_APIS, SERVICE_YAML_KEY_SPEC,
    SERVICE_YAML_KEY_SYSTEM, SERVICE_YAML_KEY_TYPE, compact_preview,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CatalogServiceInfo {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub owner: Option<String>,
    pub lifecycle: Option<String>,
    pub system: Option<String>,
    pub service_type: Option<String>,
    pub provides_apis: Vec<String>,
    pub depends_on: Vec<String>,
    pub annotations: Vec<(&'static str, String)>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CodeownersSummary {
    pub default_owners: Vec<String>,
    pub scoped_rules: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YamlSection {
    Root,
    Metadata,
    MetadataAnnotations,
    Spec,
    SpecList(SpecListKind),
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpecListKind {
    DependsOn,
    ProvidesApis,
}

fn unquote_yaml_scalar(raw: &str) -> String {
    let trimmed = raw.trim();
    // Strip trailing inline comment outside quotes
    let cleaned = match (trimmed.starts_with('"'), trimmed.starts_with('\'')) {
        (true, _) => trimmed
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(trimmed),
        (false, true) => trimmed
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .unwrap_or(trimmed),
        (false, false) => match trimmed.find(" #") {
            Some(idx) => trimmed[..idx].trim(),
            None => trimmed,
        },
    };
    cleaned.trim().to_string()
}

fn parse_inline_yaml_list(raw: &str, cap: usize) -> Vec<String> {
    let trimmed = raw.trim();
    let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for part in inner.split(',') {
        if items.len() >= cap {
            break;
        }
        let val = unquote_yaml_scalar(part);
        if !val.is_empty() && !items.contains(&val) {
            items.push(val);
        }
    }
    items
}

fn push_bounded_unique(target: &mut Vec<String>, val: String, cap: usize) {
    if !val.is_empty() && target.len() < cap && !target.contains(&val) {
        target.push(val);
    }
}

/// Deterministic indentation-stack parser for the primary entity in a Backstage `catalog-info.yaml`.
/// Zero regex, zero external YAML dependencies, zero panics.
pub fn parse_catalog_info_yaml(content: &str) -> Option<CatalogServiceInfo> {
    let mut info = CatalogServiceInfo::default();
    let mut stack: Vec<(usize, YamlSection)> = vec![(0, YamlSection::Root)];
    let mut saw_entity = false;

    for raw_line in content.lines() {
        let stripped_cr = raw_line.trim_end_matches('\r');
        let trimmed = stripped_cr.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == SERVICE_YAML_DOC_START || trimmed == SERVICE_YAML_DOC_END {
            if saw_entity {
                break;
            }
            stack.truncate(1);
            continue;
        }

        let indent = stripped_cr.bytes().take_while(|&b| b == b' ').count();

        // Handle sequence items (`- item`) before popping equal-indent section frames
        if let Some(rest) = trimmed.strip_prefix('-')
            && (rest.is_empty() || rest.starts_with(' '))
        {
            while stack.len() > 1 {
                let Some(&(top_indent, top_sec)) = stack.last() else {
                    break;
                };
                let keep = match top_sec {
                    YamlSection::SpecList(_) => indent >= top_indent,
                    _ => indent > top_indent,
                };
                if keep {
                    break;
                }
                stack.pop();
            }
            let val = unquote_yaml_scalar(rest);
            if let Some(&(_, YamlSection::SpecList(list_kind))) = stack.last() {
                match list_kind {
                    SpecListKind::DependsOn => {
                        push_bounded_unique(&mut info.depends_on, val, SERVICE_BRIEF_MAX_DEPS)
                    }
                    SpecListKind::ProvidesApis => {
                        push_bounded_unique(&mut info.provides_apis, val, SERVICE_BRIEF_MAX_APIS)
                    }
                }
            }
            continue;
        }

        while stack.len() > 1 && stack.last().is_some_and(|&(d, _)| indent <= d) {
            stack.pop();
        }
        let parent = stack.last().map(|&(_, s)| s).unwrap_or(YamlSection::Root);

        let Some((raw_key, raw_val)) = trimmed.split_once(':') else {
            continue;
        };
        let key = unquote_yaml_scalar(raw_key);
        let val_trimmed = raw_val.trim();
        let val = unquote_yaml_scalar(val_trimmed);

        match (parent, key.as_str()) {
            (YamlSection::Root, SERVICE_YAML_KEY_KIND) => {
                if !val.is_empty() {
                    info.kind = Some(val);
                    saw_entity = true;
                }
            }
            (YamlSection::Root, SERVICE_YAML_KEY_METADATA) => {
                stack.push((indent, YamlSection::Metadata));
            }
            (YamlSection::Root, SERVICE_YAML_KEY_SPEC) => {
                stack.push((indent, YamlSection::Spec));
            }
            (YamlSection::Root, _) if val_trimmed.is_empty() => {
                stack.push((indent, YamlSection::Ignored));
            }
            (YamlSection::Metadata, SERVICE_YAML_KEY_NAME) if !val.is_empty() => {
                info.name = Some(val);
                saw_entity = true;
            }
            (YamlSection::Metadata, SERVICE_YAML_KEY_ANNOTATIONS) => {
                stack.push((indent, YamlSection::MetadataAnnotations));
            }
            (YamlSection::Metadata, _) if val_trimmed.is_empty() => {
                stack.push((indent, YamlSection::Ignored));
            }
            (YamlSection::MetadataAnnotations, ann_key) => {
                if !val.is_empty()
                    && info.annotations.len() < SERVICE_BRIEF_MAX_LINKS
                    && let Some(&(_, label)) = SERVICE_CATALOG_ANNOTATION_KEYS
                        .iter()
                        .find(|&&(k, _)| k.eq_ignore_ascii_case(ann_key))
                    && !info.annotations.iter().any(|&(l, _)| l == label)
                {
                    info.annotations.push((
                        label,
                        compact_preview(&val, SERVICE_BRIEF_ANNOTATION_MAX_CHARS),
                    ));
                }
            }
            (YamlSection::Spec, SERVICE_YAML_KEY_OWNER) if !val.is_empty() => {
                info.owner = Some(val);
            }
            (YamlSection::Spec, SERVICE_YAML_KEY_LIFECYCLE) if !val.is_empty() => {
                info.lifecycle = Some(val);
            }
            (YamlSection::Spec, SERVICE_YAML_KEY_SYSTEM) if !val.is_empty() => {
                info.system = Some(val);
            }
            (YamlSection::Spec, SERVICE_YAML_KEY_TYPE) if !val.is_empty() => {
                info.service_type = Some(val);
            }
            (YamlSection::Spec, SERVICE_YAML_KEY_DEPENDS_ON) => {
                for item in parse_inline_yaml_list(val_trimmed, SERVICE_BRIEF_MAX_DEPS) {
                    push_bounded_unique(&mut info.depends_on, item, SERVICE_BRIEF_MAX_DEPS);
                }
                if val_trimmed.is_empty() {
                    stack.push((indent, YamlSection::SpecList(SpecListKind::DependsOn)));
                }
            }
            (YamlSection::Spec, SERVICE_YAML_KEY_PROVIDES_APIS) => {
                for item in parse_inline_yaml_list(val_trimmed, SERVICE_BRIEF_MAX_APIS) {
                    push_bounded_unique(&mut info.provides_apis, item, SERVICE_BRIEF_MAX_APIS);
                }
                if val_trimmed.is_empty() {
                    stack.push((indent, YamlSection::SpecList(SpecListKind::ProvidesApis)));
                }
            }
            (YamlSection::Spec, _) if val_trimmed.is_empty() => {
                stack.push((indent, YamlSection::Ignored));
            }
            _ => {}
        }
    }

    let has_signal = info.name.is_some()
        || info.owner.is_some()
        || info.lifecycle.is_some()
        || info.system.is_some()
        || !info.depends_on.is_empty()
        || !info.provides_apis.is_empty();
    match has_signal {
        true => Some(info),
        false => None,
    }
}

/// Deterministic single-pass `CODEOWNERS` parser (GitHub/GitLab compatible, zero regex).
pub fn parse_codeowners(content: &str) -> CodeownersSummary {
    let mut summary = CodeownersSummary::default();
    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(pattern) = parts.next() else {
            continue;
        };
        let owners: Vec<String> = parts
            .take_while(|tok| !tok.starts_with('#'))
            .filter(|tok| tok.starts_with('@') || tok.contains('@'))
            .take(SERVICE_BRIEF_MAX_OWNERS)
            .map(String::from)
            .collect();
        if owners.is_empty() {
            continue;
        }
        match pattern {
            "*" | "/*" => {
                // Later wildcard rule overrides earlier ones per GitHub CODEOWNERS semantics
                summary.default_owners = owners;
            }
            path_pat => {
                if summary.scoped_rules.len() < SERVICE_BRIEF_MAX_OWNERS {
                    summary.scoped_rules.push((path_pat.to_string(), owners));
                }
            }
        }
    }
    summary
}

/// Formats parsed `catalog-info.yaml` and `CODEOWNERS` metadata into one compact Turn-1 line.
/// Returns `None` (0 bytes) when neither source yields meaningful service metadata.
pub fn format_service_brief(
    catalog: Option<&CatalogServiceInfo>,
    codeowners: Option<&CodeownersSummary>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::with_capacity(8);

    if let Some(cat) = catalog {
        match (cat.kind.as_deref(), cat.name.as_deref()) {
            (Some(k), Some(n)) => parts.push(format!("{k}/{n}")),
            (None, Some(n)) => parts.push(n.to_string()),
            (Some(k), None) => parts.push(k.to_string()),
            (None, None) => {}
        }
        if let Some(ref owner) = cat.owner {
            parts.push(format!("owner={owner}"));
        }
        if let Some(ref lifecycle) = cat.lifecycle {
            parts.push(format!("lifecycle={lifecycle}"));
        }
        if let Some(ref system) = cat.system {
            parts.push(format!("system={system}"));
        }
        if let Some(ref svc_type) = cat.service_type {
            parts.push(format!("type={svc_type}"));
        }
        if !cat.provides_apis.is_empty() {
            parts.push(format!("provides=[{}]", cat.provides_apis.join(", ")));
        }
        if !cat.depends_on.is_empty() {
            parts.push(format!("deps=[{}]", cat.depends_on.join(", ")));
        }
        for (label, val) in &cat.annotations {
            parts.push(format!("{label}={val}"));
        }
    }

    if let Some(co) = codeowners {
        match (co.default_owners.is_empty(), co.scoped_rules.is_empty()) {
            (false, _) => {
                parts.push(format!("codeowners={}", co.default_owners.join(",")));
            }
            (true, false) => {
                let rendered: Vec<String> = co
                    .scoped_rules
                    .iter()
                    .take(SERVICE_BRIEF_MAX_SCOPED_RULES)
                    .map(|(p, owners)| format!("{p}->{}", owners.join(",")))
                    .collect();
                parts.push(format!("codeowners=[{}]", rendered.join("; ")));
            }
            (true, true) => {}
        }
    }

    match parts.is_empty() {
        true => None,
        false => Some(parts.join(" · ")),
    }
}

/// Scans `workspace_dir` for open-standard `catalog-info.yaml` and `CODEOWNERS` using
/// kernel-confined `openat(O_NOFOLLOW)` reads (strictly fails open; 0 bytes if absent).
pub fn scan_workspace_service_brief(workspace_dir: &Path) -> Option<String> {
    let catalog = SERVICE_CATALOG_FILES.iter().find_map(|rel| {
        let target = workspace_dir.join(rel);
        let raw =
            read_jailed_string_bounded(workspace_dir, &target, SERVICE_BRIEF_MAX_FILE_BYTES, rel)
                .ok()?;
        parse_catalog_info_yaml(&raw)
    });

    let codeowners = SERVICE_CODEOWNERS_PATHS.iter().find_map(|rel| {
        let target = workspace_dir.join(rel);
        let raw =
            read_jailed_string_bounded(workspace_dir, &target, SERVICE_BRIEF_MAX_FILE_BYTES, rel)
                .ok()?;
        let parsed = parse_codeowners(&raw);
        match parsed.default_owners.is_empty() && parsed.scoped_rules.is_empty() {
            true => None,
            false => Some(parsed),
        }
    });

    format_service_brief(catalog.as_ref(), codeowners.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_backstage_catalog_and_codeowners_deterministically() {
        let yaml = r#"
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: checkout-api
  annotations:
    backstage.io/techdocs-ref: dir:.
    pagerduty.com/service-id: P123456
    unrelated.io/noise: ignore-me
spec:
  type: service
  lifecycle: production
  owner: group:payments-team
  system: commerce
  providesApis: [checkout-grpc, checkout-http]
  dependsOn:
    - resource:orders-db
    - component:fraud-scorer
---
apiVersion: backstage.io/v1alpha1
kind: API
metadata:
  name: secondary-entity-ignored
"#;
        let cat = parse_catalog_info_yaml(yaml).expect("catalog parsed");
        assert_eq!(cat.kind.as_deref(), Some("Component"));
        assert_eq!(cat.name.as_deref(), Some("checkout-api"));
        assert_eq!(cat.owner.as_deref(), Some("group:payments-team"));
        assert_eq!(cat.lifecycle.as_deref(), Some("production"));
        assert_eq!(cat.system.as_deref(), Some("commerce"));
        assert_eq!(cat.service_type.as_deref(), Some("service"));
        assert_eq!(cat.provides_apis, vec!["checkout-grpc", "checkout-http"]);
        assert_eq!(
            cat.depends_on,
            vec!["resource:orders-db", "component:fraud-scorer"]
        );
        assert_eq!(
            cat.annotations,
            vec![
                ("docs", "dir:.".to_string()),
                ("pagerduty", "P123456".to_string())
            ]
        );

        let co = parse_codeowners(
            "# Global owners\n* @payments-team @platform-eng # comment\n/crates/ @rust-core\n",
        );
        assert_eq!(co.default_owners, vec!["@payments-team", "@platform-eng"]);
        assert_eq!(co.scoped_rules.len(), 1);

        let brief = format_service_brief(Some(&cat), Some(&co)).expect("brief");
        assert!(brief.contains("Component/checkout-api"));
        assert!(brief.contains("owner=group:payments-team"));
        assert!(brief.contains("lifecycle=production"));
        assert!(brief.contains("deps=[resource:orders-db, component:fraud-scorer]"));
        assert!(brief.contains("codeowners=@payments-team,@platform-eng"));
    }

    #[test]
    fn empty_workspace_emits_zero_bytes_and_blocks_symlink_escape() {
        let base = std::env::temp_dir().join(format!("zenko_svcbrief_{}", zenko_core::new_id()));
        let ws = base.join("ws");
        let _ = std::fs::create_dir_all(&ws);
        assert!(scan_workspace_service_brief(&ws).is_none());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = base.join("outside.yaml");
            let _ = std::fs::write(
                &outside,
                "kind: Component\nmetadata:\n  name: leaked-secret\nspec:\n  owner: evil\n",
            );
            let _ = symlink(&outside, ws.join("catalog-info.yaml"));
            assert!(
                scan_workspace_service_brief(&ws).is_none(),
                "symlinked catalog-info.yaml outside workspace must be ignored"
            );
        }

        let _ = std::fs::remove_dir_all(&base);
    }
}
