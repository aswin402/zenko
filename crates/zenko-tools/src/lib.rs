pub mod agents;
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
    SubagentBridge, TOOL_TOOL_SEARCH, Tool, ToolExposure, ToolSpec, WebSearchConfig,
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

    /// Returns deterministic, cache-friendly `tools[]` for a session:
    /// - When `deferred_enabled == false`, returns every registered tool.
    /// - When `deferred_enabled == true`, returns all `Eager` tools in canonical order followed by
    ///   any session-activated `Deferred` tools in canonical order.
    pub fn specs_for_session(
        &self,
        deferred_enabled: bool,
        activated_deferred: &BTreeSet<String>,
    ) -> Vec<ToolSpec> {
        if !deferred_enabled {
            return self.specs();
        }
        let mut eager = Vec::new();
        let mut active_def = Vec::new();
        if let Ok(map) = self.tools.read() {
            for (name, tool) in map.iter() {
                match (tool.exposure(), activated_deferred.contains(name)) {
                    (ToolExposure::Eager, _) => eager.push(tool.spec()),
                    (ToolExposure::Deferred, true) => active_def.push(tool.spec()),
                    (ToolExposure::Deferred, false) => {}
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

    /// Formats a compact 1-line `<deferred_tools>` Tier-2 catalog listing all deferred tools
    /// (keeping initial prompts lean while letting the agent discover capabilities via `tool_search`).
    pub fn deferred_catalog_text(&self) -> Option<String> {
        let mut lines = Vec::new();
        if let Ok(map) = self.tools.read() {
            for (name, tool) in map.iter() {
                if tool.exposure() != ToolExposure::Deferred {
                    continue;
                }
                let spec = tool.spec();
                let brief = spec
                    .description
                    .split('.')
                    .next()
                    .unwrap_or(&spec.description)
                    .trim();
                lines.push(format!("- `{name}`: {brief}"));
            }
        }
        match lines.is_empty() {
            true => None,
            false => Some(format!(
                "<deferred_tools>\n\
                 Additional specialized tools (invoke `{TOOL_TOOL_SEARCH}(query)` to load full schemas on demand):\n\
                 {}\n\
                 </deferred_tools>",
                lines.join("\n")
            )),
        }
    }

    /// Deterministic zero-regex BM25 + exact/prefix scorer ranking deferred tools against `query`.
    pub fn search_deferred_bm25(&self, query: &str, limit: usize) -> Vec<ToolSpec> {
        let query_tokens = tokenize_ascii_terms(query);
        if query_tokens.is_empty() || limit == 0 {
            return Vec::new();
        }

        let docs: Vec<(ToolSpec, Vec<String>, String)> = self
            .tools
            .read()
            .map(|map| {
                map.values()
                    .filter(|t| t.exposure() == ToolExposure::Deferred)
                    .map(|t| {
                        let spec = t.spec();
                        let mut text = format!("{} {} ", spec.name, spec.description);
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
                        (spec, tokens, lower_name)
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
    }

    #[async_trait]
    impl Tool for DummyTool {
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
    fn deferred_tool_loading_and_bm25_search_activate_only_relevant_schemas() {
        let reg = ToolRegistry::empty();
        reg.register(Arc::new(DummyTool {
            name: "read_file",
            desc: "Read workspace file lines",
            exp: ToolExposure::Eager,
        }));
        reg.register(Arc::new(DummyTool {
            name: "cron_tool",
            desc: "Schedule recurring 24/7 cron jobs and scripts",
            exp: ToolExposure::Deferred,
        }));
        reg.register(Arc::new(DummyTool {
            name: "web_fetch",
            desc: "Fetch HTTP and HTTPS web pages safely",
            exp: ToolExposure::Deferred,
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
        assert!(reg.deferred_catalog_text().expect("catalog").contains("web_fetch"));
    }
}
