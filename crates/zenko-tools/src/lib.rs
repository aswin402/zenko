pub mod agents;
pub mod batch;
pub mod coerce;
pub mod context_tools;
pub mod fs_tools;
pub mod git;
pub mod glob;
pub mod goals;
pub mod jail;
pub mod jobs;
pub mod lsp;
pub mod mcp_client;
pub mod memory_cron;
pub mod orchestrator_tools;
pub mod outline;
pub mod search;
pub mod shell;
pub mod web;
pub mod web_search;

use reqwest::Client;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use tokio::sync::Notify;
use zenko_core::{
    ContextBridge, GoalLoopBridge, JobSupervisor, OrchestratorBridge, SecretVault, SkillsConfig,
    SubagentBridge, TOOL_TOOL_SEARCH, Tool, ToolAvailabilityContext, ToolBatchBridge, ToolExposure,
    ToolSet, ToolSetSummary, ToolSpec, WebSearchConfig, find_toolset_descriptor,
};
use zenko_memory::{SnapshotManager, SqliteStore};

pub struct ToolRegistry {
    tools: RwLock<BTreeMap<String, Arc<dyn Tool>>>,
}

impl ToolRegistry {
    pub fn empty() -> Self {
        Self {
            tools: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn default_suite(
        http: Client,
        store: SqliteStore,
        snapshots: Arc<SnapshotManager>,
        cron_notify: Arc<Notify>,
        jobs: Option<Arc<dyn JobSupervisor>>,
        orchestrator: Option<Arc<dyn OrchestratorBridge>>,
    ) -> Self {
        let reg = Self::empty();
        reg.register(Arc::new(fs_tools::ReadFileTool));
        reg.register(Arc::new(fs_tools::HashEditTool));
        reg.register(Arc::new(fs_tools::ApplyPatchTool));
        reg.register(Arc::new(fs_tools::WriteFileTool));
        reg.register(Arc::new(fs_tools::ListDirTool));
        reg.register(Arc::new(search::CodeSearchTool));
        reg.register(Arc::new(glob::GlobFindTool));
        reg.register(Arc::new(git::GitTool));
        reg.register(Arc::new(shell::ShellExecTool::new(jobs.clone())));
        reg.register(Arc::new(web::WebFetchTool::new(http)));
        reg.register(Arc::new(memory_cron::MemoryTool::new(
            store.clone(),
            snapshots.clone(),
        )));
        reg.register(Arc::new(memory_cron::SessionSearchTool::new(store.clone())));
        reg.register(Arc::new(memory_cron::CronTool::new(store, cron_notify)));
        reg.register(Arc::new(context_tools::SkillViewTool::new(snapshots)));
        if let Some(sup) = jobs {
            reg.register(Arc::new(jobs::JobListTool::new(sup.clone())));
            reg.register(Arc::new(jobs::JobReadTool::new(sup.clone())));
            reg.register(Arc::new(jobs::JobWaitTool::new(sup.clone())));
            reg.register(Arc::new(jobs::JobInputTool::new(sup.clone())));
            reg.register(Arc::new(jobs::JobStopTool::new(sup.clone())));
            reg.register(Arc::new(jobs::TaskManageTool::new(sup)));
        }
        if let Some(bridge) = orchestrator {
            reg.register(Arc::new(orchestrator_tools::RecordBrainstormTool::new(
                bridge.clone(),
            )));
            reg.register(Arc::new(orchestrator_tools::PlanUpdateTool::new(
                bridge.clone(),
            )));
            reg.register(Arc::new(orchestrator_tools::SessionTaskTool::new(
                bridge.clone(),
            )));
            reg.register(Arc::new(orchestrator_tools::OrchestrateControlTool::new(
                bridge.clone(),
            )));
            reg.register(Arc::new(orchestrator_tools::AskClarificationTool::new(
                bridge,
            )));
        }
        reg
    }

    /// Registers Phase 7 runtime-backed engineering, web search, and MCP resource tools (`web_search`, `lsp_tool`, `mcp_resource`).
    pub fn register_phase7_tools(
        &self,
        http: Client,
        web_search_cfg: Arc<RwLock<WebSearchConfig>>,
        vault: Arc<SecretVault>,
        lsp_pool: Arc<lsp::LspPoolManager>,
        mcp_clients: Arc<mcp_client::McpClientManager>,
    ) {
        self.register(Arc::new(web_search::WebSearchTool::new(
            http,
            web_search_cfg,
            vault,
        )));
        self.register(Arc::new(lsp::LspTool::new(lsp_pool)));
        self.register(Arc::new(mcp_client::McpResourceTool::new(mcp_clients)));
    }

    /// Registers Phase 8 self-evolution & autonomous loop tools (`skill_manage`, `goal_loop`).
    pub fn register_phase8_tools(
        &self,
        snapshots: Arc<SnapshotManager>,
        vault: Arc<SecretVault>,
        skills_cfg: Arc<RwLock<SkillsConfig>>,
        goal_bridge: Arc<dyn GoalLoopBridge>,
    ) {
        self.register(Arc::new(context_tools::SkillManageTool::new(
            snapshots,
            vault,
            skills_cfg,
        )));
        self.register(Arc::new(goals::GoalLoopTool::new(goal_bridge)));
    }

    /// Registers runtime-backed context tools (`tool_search`, `context_inspect`) once the
    /// `ContextBridge` is initialized (cleanly breaks circular dependency between tools & runtime).
    pub fn register_context_tools(&self, bridge: Arc<dyn ContextBridge>) {
        self.register(Arc::new(context_tools::ToolSearchTool::new(bridge.clone())));
        self.register(Arc::new(context_tools::ContextInspectTool::new(bridge)));
    }

    /// Registers wave-aware `batch_tools` backed by `ToolBatchBridge`.
    pub fn register_batch_tool(&self, bridge: Arc<dyn ToolBatchBridge>) {
        self.register(Arc::new(batch::BatchToolsTool::new(bridge)));
    }

    /// Registers Phase 5/5.5 async subagent orchestration tools (`spawn_agent`, `agent_list`,
    /// `agent_message`, `agent_wait`, `agent_kill`, `agent_worktree`) backed by `SubagentBridge`.
    pub fn register_subagent_tools(&self, bridge: Arc<dyn SubagentBridge>) {
        self.register(Arc::new(agents::SpawnAgentTool::new(bridge.clone())));
        self.register(Arc::new(agents::AgentListTool::new(bridge.clone())));
        self.register(Arc::new(agents::AgentMessageTool::new(bridge.clone())));
        self.register(Arc::new(agents::AgentWaitTool::new(bridge.clone())));
        self.register(Arc::new(agents::AgentKillTool::new(bridge.clone())));
        self.register(Arc::new(agents::AgentWorktreeTool::new(bridge)));
    }

    pub fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.spec().name;
        if let Ok(mut map) = self.tools.write() {
            map.insert(name, tool);
        }
    }

    pub fn unregister_prefix(&self, prefix: &str) {
        if let Ok(mut map) = self.tools.write() {
            map.retain(|k, _| !k.starts_with(prefix));
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.read().ok().and_then(|map| map.get(name).cloned())
    }

    pub fn is_deferred(&self, name: &str) -> bool {
        self.tools
            .read()
            .ok()
            .and_then(|map| map.get(name).map(|t| t.exposure() == ToolExposure::Deferred))
            .unwrap_or(false)
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .read()
            .map(|map| map.values().map(|t| t.spec()).collect())
            .unwrap_or_default()
    }

    /// Returns deterministic, cache-friendly `tools[]` for a session (unfiltered convenience).
    pub fn specs_for_session(
        &self,
        deferred_enabled: bool,
        activated_deferred: &BTreeSet<String>,
    ) -> Vec<ToolSpec> {
        self.specs_for_session_filtered(
            deferred_enabled,
            activated_deferred,
            &ToolAvailabilityContext::default(),
            |_| true,
        )
    }

    /// Returns deterministic, cache-friendly `tools[]` filtered by `is_available(&avail)` and `allow(spec)`.
    pub fn specs_for_session_filtered<F>(
        &self,
        deferred_enabled: bool,
        activated_deferred: &BTreeSet<String>,
        avail: &ToolAvailabilityContext,
        allow: F,
    ) -> Vec<ToolSpec>
    where
        F: Fn(&ToolSpec) -> bool,
    {
        let mut eager = Vec::new();
        let mut active_def = Vec::new();
        if let Ok(map) = self.tools.read() {
            for (name, tool) in map.iter() {
                if !tool.is_available(avail) {
                    continue;
                }
                let spec = tool.spec();
                if !allow(&spec) {
                    continue;
                }
                match (deferred_enabled, tool.exposure(), activated_deferred.contains(name)) {
                    (false, _, _) | (true, ToolExposure::Eager, _) => eager.push(spec),
                    (true, ToolExposure::Deferred, true) => active_def.push(spec),
                    (true, ToolExposure::Deferred, false) => {}
                }
            }
        }
        eager.extend(active_def);
        eager
    }

    /// Returns `(eager_count, deferred_available_count)`.
    pub fn exposure_counts(&self) -> (usize, usize) {
        let mut eager = 0usize;
        let mut deferred = 0usize;
        if let Ok(map) = self.tools.read() {
            for tool in map.values() {
                match tool.exposure() {
                    ToolExposure::Eager => eager += 1,
                    ToolExposure::Deferred => deferred += 1,
                }
            }
        }
        (eager, deferred)
    }

    /// Formats a compact `<deferred_tools>` Tier-2 catalog listing all registered deferred tools.
    pub fn deferred_catalog_text(&self) -> Option<String> {
        self.deferred_catalog_text_filtered(&ToolAvailabilityContext {
            has_mcp_servers: true,
            is_subagent: false,
            read_only: false,
        }, |_| true)
    }

    /// Formats a compact ToolSet-grouped `<deferred_tools>` Tier-2 catalog strictly filtered by
    /// `is_available(&avail)` and `allow(&spec)` so subagents and Plan Mode never see denied tools.
    pub fn deferred_catalog_text_filtered<F>(
        &self,
        avail: &ToolAvailabilityContext,
        allow: F,
    ) -> Option<String>
    where
        F: Fn(&ToolSpec) -> bool,
    {
        let mut grouped: BTreeMap<ToolSet, Vec<String>> = BTreeMap::new();
        if let Ok(map) = self.tools.read() {
            for (name, tool) in map.iter() {
                if tool.exposure() != ToolExposure::Deferred || !tool.is_available(avail) {
                    continue;
                }
                let spec = tool.spec();
                if !allow(&spec) {
                    continue;
                }
                let brief = spec
                    .description
                    .split('.')
                    .next()
                    .unwrap_or(&spec.description)
                    .trim();
                grouped
                    .entry(tool.toolset())
                    .or_default()
                    .push(format!("  - `{name}`: {brief}"));
            }
        }
        if grouped.is_empty() {
            return None;
        }
        let mut sections = Vec::with_capacity(grouped.len());
        for (set, items) in grouped {
            let desc = find_toolset_descriptor(set);
            sections.push(format!("[{}] {}\n{}", desc.slug, desc.label, items.join("\n")));
        }
        Some(format!(
            "<deferred_tools>\n\
             Additional specialized tools grouped by ToolSet (invoke `{TOOL_TOOL_SEARCH}` with `query`, `toolset`, `names`, or call any listed tool directly):\n\
             {}\n\
             </deferred_tools>",
            sections.join("\n")
        ))
    }

    /// Lists all `ToolSet` domains containing at least one tool permitted by `avail` + `allow`.
    pub fn list_toolsets_filtered<F>(
        &self,
        avail: &ToolAvailabilityContext,
        allow: F,
    ) -> Vec<ToolSetSummary>
    where
        F: Fn(&ToolSpec) -> bool,
    {
        let mut by_set: BTreeMap<ToolSet, (Vec<String>, Vec<String>)> = BTreeMap::new();
        if let Ok(map) = self.tools.read() {
            for (name, tool) in map.iter() {
                if !tool.is_available(avail) {
                    continue;
                }
                let spec = tool.spec();
                if !allow(&spec) {
                    continue;
                }
                let entry = by_set.entry(tool.toolset()).or_default();
                match tool.exposure() {
                    ToolExposure::Eager => entry.0.push(name.clone()),
                    ToolExposure::Deferred => entry.1.push(name.clone()),
                }
            }
        }
        by_set
            .into_iter()
            .map(|(set, (eager_tools, deferred_tools))| {
                let desc = find_toolset_descriptor(set);
                ToolSetSummary {
                    id: desc.slug.to_string(),
                    label: desc.label.to_string(),
                    description: desc.description.to_string(),
                    eager_tools,
                    deferred_tools,
                }
            })
            .collect()
    }

    /// Returns closest permitted tool names when an unknown/misspelled tool name is called.
    pub fn suggest_permitted_tools<F>(
        &self,
        unknown_name: &str,
        max_results: usize,
        avail: &ToolAvailabilityContext,
        allow: F,
    ) -> Vec<String>
    where
        F: Fn(&ToolSpec) -> bool,
    {
        let names: Vec<String> = self
            .tools
            .read()
            .map(|map| {
                map.iter()
                    .filter(|(_, t)| t.is_available(avail))
                    .filter_map(|(name, t)| match allow(&t.spec()) {
                        true => Some(name.clone()),
                        false => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        coerce::suggest_similar_tools(unknown_name, &refs, max_results)
    }

    /// Deterministic zero-regex BM25 + exact/prefix scorer ranking all deferred tools against `query`.
    pub fn search_deferred_bm25(&self, query: &str, limit: usize) -> Vec<ToolSpec> {
        self.search_deferred_bm25_filtered(
            query,
            limit,
            &ToolAvailabilityContext {
                has_mcp_servers: true,
                is_subagent: false,
                read_only: false,
            },
            |_| true,
        )
    }

    /// Scope-aware BM25 search over permitted deferred tools (also indexing `ToolSet` slug & label).
    pub fn search_deferred_bm25_filtered<F>(
        &self,
        query: &str,
        limit: usize,
        avail: &ToolAvailabilityContext,
        allow: F,
    ) -> Vec<ToolSpec>
    where
        F: Fn(&ToolSpec) -> bool,
    {
        let query_tokens = tokenize_ascii_terms(query);
        if query_tokens.is_empty() || limit == 0 {
            return Vec::new();
        }

        let docs: Vec<(ToolSpec, Vec<String>, String)> = self
            .tools
            .read()
            .map(|map| {
                map.values()
                    .filter(|t| t.exposure() == ToolExposure::Deferred && t.is_available(avail))
                    .filter_map(|t| {
                        let spec = t.spec();
                        if !allow(&spec) {
                            return None;
                        }
                        let ts = find_toolset_descriptor(t.toolset());
                        let mut text = format!(
                            "{} {} {} {} ",
                            spec.name, ts.slug, ts.label, spec.description
                        );
                        if let Some(props) =
                            spec.parameters.get("properties").and_then(|p| p.as_object())
                        {
                            for key in props.keys() {
                                text.push_str(key);
                                text.push(' ');
                            }
                        }
                        let tokens = tokenize_ascii_terms(&text);
                        let lower_name = spec.name.to_ascii_lowercase();
                        Some((spec, tokens, lower_name))
                    })
                    .collect()
            })
            .unwrap_or_default();

        if docs.is_empty() {
            return Vec::new();
        }

        let n_docs = docs.len() as f64;
        let avg_dl = (docs.iter().map(|(_, t, _)| t.len()).sum::<usize>() as f64 / n_docs).max(1.0);
        let mut df: HashMap<&str, usize> = HashMap::new();
        for qt in &query_tokens {
            let count = docs
                .iter()
                .filter(|(_, toks, _)| toks.iter().any(|t| t == qt || t.starts_with(qt.as_str())))
                .count();
            df.insert(qt.as_str(), count);
        }

        const K1: f64 = 1.2;
        const B: f64 = 0.75;
        let mut scored: Vec<(f64, ToolSpec)> = docs
            .into_iter()
            .filter_map(|(spec, doc_tokens, lower_name)| {
                let dl = doc_tokens.len() as f64;
                let mut score = 0.0f64;
                for qt in &query_tokens {
                    let tf_exact = doc_tokens.iter().filter(|t| *t == qt).count() as f64;
                    let tf_prefix = doc_tokens
                        .iter()
                        .filter(|t| *t != qt && t.starts_with(qt.as_str()))
                        .count() as f64
                        * 0.5;
                    let tf = tf_exact + tf_prefix;
                    if tf > 0.0 {
                        let doc_freq = *df.get(qt.as_str()).unwrap_or(&0) as f64;
                        let idf = ((n_docs - doc_freq + 0.5) / (doc_freq + 0.5) + 1.0).ln();
                        score += idf * ((tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * (dl / avg_dl))));
                    }
                    if lower_name == *qt {
                        score += 6.0;
                    } else if lower_name.contains(qt.as_str()) {
                        score += 2.5;
                    }
                }
                match score > 0.0 {
                    true => Some((score, spec)),
                    false => None,
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.name.cmp(&b.1.name))
        });
        scored.into_iter().take(limit).map(|(_, s)| s).collect()
    }

    /// Unified scope-aware selector combining `toolset`, `activate_all`, explicit `names`, and BM25 `query`.
    pub fn select_deferred_by_scope<F>(
        &self,
        toolset: Option<ToolSet>,
        activate_all: bool,
        names: &[String],
        query: Option<&str>,
        limit: usize,
        avail: &ToolAvailabilityContext,
        allow: F,
    ) -> Vec<ToolSpec>
    where
        F: Fn(&ToolSpec) -> bool,
    {
        let mut selected: BTreeMap<String, ToolSpec> = BTreeMap::new();
        if let Ok(map) = self.tools.read() {
            for (name, tool) in map.iter() {
                if tool.exposure() != ToolExposure::Deferred || !tool.is_available(avail) {
                    continue;
                }
                let matches_set = activate_all || toolset.is_some_and(|ts| tool.toolset() == ts);
                let matches_name = names.iter().any(|n| n.eq_ignore_ascii_case(name));
                if !matches_set && !matches_name {
                    continue;
                }
                let spec = tool.spec();
                if allow(&spec) {
                    selected.insert(spec.name.clone(), spec);
                }
            }
        }
        if let Some(q) = query {
            for spec in self.search_deferred_bm25_filtered(q, limit, avail, &allow) {
                selected.entry(spec.name.clone()).or_insert(spec);
            }
        }
        selected.into_values().collect()
    }
}

fn tokenize_ascii_terms(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in input.chars() {
        match ch.is_ascii_alphanumeric() {
            true => cur.push(ch.to_ascii_lowercase()),
            false => {
                if cur.len() >= 2 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use tokio_util::sync::CancellationToken;
    use zenko_core::{Result, RiskLevel, ToolContext, ToolOutput};

    struct DummyTool {
        name: &'static str,
        desc: &'static str,
        exp: ToolExposure,
        set: ToolSet,
        requires_mcp: bool,
    }

    #[async_trait]
    impl Tool for DummyTool {
        fn toolset(&self) -> ToolSet {
            self.set
        }
        fn is_available(&self, ctx: &ToolAvailabilityContext) -> bool {
            match self.requires_mcp {
                true => ctx.has_mcp_servers,
                false => true,
            }
        }
        fn exposure(&self) -> ToolExposure {
            self.exp
        }
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.into(),
                description: self.desc.into(),
                parameters: json!({"type": "object", "properties": {}}),
                risk: RiskLevel::Safe,
            }
        }
        async fn execute(&self, _: &ToolContext, _: Value, _: CancellationToken) -> Result<ToolOutput> {
            Ok(ToolOutput::ok("ok"))
        }
    }

    #[test]
    fn deferred_tool_loading_toolsets_and_availability_filtering() {
        let reg = ToolRegistry::empty();
        reg.register(Arc::new(DummyTool {
            name: "read_file",
            desc: "Read workspace file lines",
            exp: ToolExposure::Eager,
            set: ToolSet::CoreFs,
            requires_mcp: false,
        }));
        reg.register(Arc::new(DummyTool {
            name: "cron_tool",
            desc: "Schedule recurring 24/7 cron jobs and scripts",
            exp: ToolExposure::Deferred,
            set: ToolSet::Automation,
            requires_mcp: false,
        }));
        reg.register(Arc::new(DummyTool {
            name: "web_fetch",
            desc: "Fetch HTTP and HTTPS web pages safely",
            exp: ToolExposure::Deferred,
            set: ToolSet::Web,
            requires_mcp: false,
        }));
        reg.register(Arc::new(DummyTool {
            name: "mcp_resource",
            desc: "Read MCP resources",
            exp: ToolExposure::Deferred,
            set: ToolSet::Mcp,
            requires_mcp: true,
        }));

        let mut active = BTreeSet::new();
        let initial = reg.specs_for_session(true, &active);
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].name, "read_file");

        let hits = reg.search_deferred_bm25("schedule recurring cron", 3);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "cron_tool");

        active.insert(hits[0].name.clone());
        let after = reg.specs_for_session(true, &active);
        assert_eq!(after.len(), 2);
        assert_eq!(after[1].name, "cron_tool");

        // Availability & role filtering hides unavailable MCP and disallowed automation tools.
        let avail_no_mcp = ToolAvailabilityContext {
            has_mcp_servers: false,
            is_subagent: true,
            read_only: true,
        };
        let cat = reg
            .deferred_catalog_text_filtered(&avail_no_mcp, |s| s.name != "cron_tool")
            .expect("catalog");
        assert!(cat.contains("[web]"));
        assert!(cat.contains("web_fetch"));
        assert!(!cat.contains("cron_tool"));
        assert!(!cat.contains("mcp_resource"));

        // Selecting by ToolSet activates all permitted tools in that domain.
        let web_only = reg.select_deferred_by_scope(
            Some(ToolSet::Web),
            false,
            &[],
            None,
            5,
            &avail_no_mcp,
            |_| true,
        );
        assert_eq!(web_only.len(), 1);
        assert_eq!(web_only[0].name, "web_fetch");
    }
}
