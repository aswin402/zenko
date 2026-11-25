//! Durable per-repository SQLite store for Zenko's native Token Economy Engine
//! (Micro-Skills, Experience-Driven Tool Pruning, P90 Turn Baselines, and Net ROI Ledger).

use super::{SqliteStore, parse_dt};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeSet;
use zenko_core::{
    ECONOMY_ESTIMATED_USD_PER_1M_TOKENS, ECONOMY_LEDGER_MAX_EVENTS,
    ECONOMY_LOOP_MAX_BASELINE_SAMPLES, ECONOMY_LOOP_MIN_ENVELOPE_CALLS,
    ECONOMY_LOOP_P90_MIN_SAMPLES, ECONOMY_MICRO_SKILL_MAX_CHARS,
    ECONOMY_MICRO_SKILL_MAX_CONTRADICTIONS, ECONOMY_MICRO_SKILL_MIN_EXPOSURES_FOR_RATE,
    ECONOMY_MICRO_SKILL_MIN_NEED_RATE, ECONOMY_PROTECTED_CORE_TOOLS, EconomyConfig,
    EconomyLedgerKind, EconomyLedgerSummary, MicroSkillStatus, RepoMicroSkill, RepoToolUsage,
    Result, new_id,
};

fn truncate_utf8_chars(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect()
}

fn is_protected_core_tool(tool_name: &str) -> bool {
    ECONOMY_PROTECTED_CORE_TOOLS.contains(&tool_name)
}

fn row_to_micro_skill(r: &rusqlite::Row<'_>) -> rusqlite::Result<RepoMicroSkill> {
    let sess_raw: String = r.get(7)?;
    let session_ids: Vec<String> = serde_json::from_str(&sess_raw).unwrap_or_default();
    Ok(RepoMicroSkill {
        id: r.get(0)?,
        repo_scope: r.get(1)?,
        signature: r.get(2)?,
        fact: r.get(3)?,
        failed_cmd: r.get(4)?,
        fixed_cmd: r.get(5)?,
        corroborations: r.get::<_, i64>(6)?.max(0) as u32,
        session_ids,
        need_hits: r.get::<_, i64>(8)?.max(0) as u32,
        exposures: r.get::<_, i64>(9)?.max(0) as u32,
        contradictions: r.get::<_, i64>(10)?.max(0) as u32,
        status: MicroSkillStatus::from_str_loose(&r.get::<_, String>(11)?),
        created_at: parse_dt(&r.get::<_, String>(12)?),
        updated_at: parse_dt(&r.get::<_, String>(13)?),
    })
}

fn row_to_tool_usage(r: &rusqlite::Row<'_>) -> rusqlite::Result<RepoToolUsage> {
    Ok(RepoToolUsage {
        repo_scope: r.get(0)?,
        tool_name: r.get(1)?,
        call_count: r.get::<_, i64>(2)?.max(0) as u64,
        prompts_since_used: r.get::<_, i64>(3)?.max(0) as u32,
        sessions_observed: r.get::<_, i64>(4)?.max(0) as u32,
        estimated_schema_tokens: r.get::<_, i64>(5)?.max(0) as u32,
        pruned_to_deferred: r.get::<_, i64>(6)? != 0,
        manually_pinned: r.get::<_, i64>(7)? != 0,
        last_used_at: r.get::<_, Option<String>>(8)?.map(|s| parse_dt(&s)),
        updated_at: parse_dt(&r.get::<_, String>(9)?),
    })
}

impl SqliteStore {
    /// Records corroborating cross-session evidence for a repository micro-skill.
    /// Promotes `Candidate -> Active` once observed across `>= min_sessions` distinct sessions.
    /// Never resurrects a previously `Retired` (contradicted) skill.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_micro_skill_evidence(
        &self,
        repo_scope: &str,
        signature: &str,
        fact: &str,
        failed_cmd: &str,
        fixed_cmd: &str,
        session_id: &str,
        min_sessions: u32,
    ) -> Result<RepoMicroSkill> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let clean_fact = truncate_utf8_chars(fact, ECONOMY_MICRO_SKILL_MAX_CHARS);
        let threshold = min_sessions.max(1);

        self.with_conn(|conn| {
            let existing: Option<RepoMicroSkill> = conn
                .query_row(
                    "SELECT id, repo_scope, signature, fact, failed_cmd, fixed_cmd, corroborations,
                            session_ids_json, need_hits, exposures, contradictions, status,
                            created_at, updated_at
                     FROM repo_micro_skills
                     WHERE repo_scope = ?1 AND signature = ?2",
                    params![repo_scope, signature],
                    row_to_micro_skill,
                )
                .optional()?;

            match existing {
                Some(mut rec) => {
                    if rec.status == MicroSkillStatus::Retired {
                        return Ok(rec);
                    }
                    if !session_id.trim().is_empty()
                        && !rec.session_ids.iter().any(|s| s == session_id)
                    {
                        rec.session_ids.push(session_id.to_string());
                        if rec.session_ids.len() > 32 {
                            let drop_n = rec.session_ids.len() - 32;
                            rec.session_ids.drain(0..drop_n);
                        }
                        rec.corroborations = rec.corroborations.saturating_add(1);
                    }
                    if rec.corroborations >= threshold && rec.contradictions == 0 {
                        rec.status = MicroSkillStatus::Active;
                    }
                    rec.updated_at = now;
                    let sess_json =
                        serde_json::to_string(&rec.session_ids).unwrap_or_else(|_| "[]".into());
                    conn.execute(
                        "UPDATE repo_micro_skills
                         SET corroborations = ?1, session_ids_json = ?2, status = ?3, updated_at = ?4
                         WHERE id = ?5",
                        params![
                            rec.corroborations as i64,
                            sess_json,
                            rec.status.as_str(),
                            now_str,
                            rec.id
                        ],
                    )?;
                    Ok(rec)
                }
                None => {
                    let id = format!("ms_{}", new_id());
                    let session_ids = match session_id.trim().is_empty() {
                        true => Vec::new(),
                        false => vec![session_id.to_string()],
                    };
                    let status = match 1 >= threshold {
                        true => MicroSkillStatus::Active,
                        false => MicroSkillStatus::Candidate,
                    };
                    let sess_json =
                        serde_json::to_string(&session_ids).unwrap_or_else(|_| "[]".into());
                    conn.execute(
                        "INSERT INTO repo_micro_skills
                         (id, repo_scope, signature, fact, failed_cmd, fixed_cmd, corroborations,
                          session_ids_json, need_hits, exposures, contradictions, status,
                          created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, 0, 0, 0, ?8, ?9, ?9)",
                        params![
                            id,
                            repo_scope,
                            signature,
                            clean_fact,
                            failed_cmd.trim(),
                            fixed_cmd.trim(),
                            sess_json,
                            status.as_str(),
                            now_str
                        ],
                    )?;
                    Ok(RepoMicroSkill {
                        id,
                        repo_scope: repo_scope.to_string(),
                        signature: signature.to_string(),
                        fact: clean_fact,
                        failed_cmd: failed_cmd.trim().to_string(),
                        fixed_cmd: fixed_cmd.trim().to_string(),
                        corroborations: 1,
                        session_ids,
                        need_hits: 0,
                        exposures: 0,
                        contradictions: 0,
                        status,
                        created_at: now,
                        updated_at: now,
                    })
                }
            }
        })
    }

    /// Manually inserts or updates a verified `Active` repository micro-skill.
    pub fn insert_manual_micro_skill(
        &self,
        repo_scope: &str,
        signature: &str,
        fact: &str,
        failed_cmd: &str,
        fixed_cmd: &str,
    ) -> Result<RepoMicroSkill> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let clean_fact = truncate_utf8_chars(fact, ECONOMY_MICRO_SKILL_MAX_CHARS);

        self.with_conn(|conn| {
            let existing: Option<RepoMicroSkill> = conn
                .query_row(
                    "SELECT id, repo_scope, signature, fact, failed_cmd, fixed_cmd, corroborations,
                            session_ids_json, need_hits, exposures, contradictions, status,
                            created_at, updated_at
                     FROM repo_micro_skills
                     WHERE repo_scope = ?1 AND signature = ?2",
                    params![repo_scope, signature],
                    row_to_micro_skill,
                )
                .optional()?;

            match existing {
                Some(mut rec) => {
                    rec.fact = clean_fact.clone();
                    rec.failed_cmd = failed_cmd.trim().to_string();
                    rec.fixed_cmd = fixed_cmd.trim().to_string();
                    rec.corroborations = rec.corroborations.max(2);
                    rec.contradictions = 0;
                    rec.status = MicroSkillStatus::Active;
                    rec.updated_at = now;
                    conn.execute(
                        "UPDATE repo_micro_skills
                         SET fact = ?1, failed_cmd = ?2, fixed_cmd = ?3, corroborations = ?4,
                             contradictions = 0, status = 'active', updated_at = ?5
                         WHERE id = ?6",
                        params![
                            rec.fact,
                            rec.failed_cmd,
                            rec.fixed_cmd,
                            rec.corroborations as i64,
                            now_str,
                            rec.id
                        ],
                    )?;
                    Ok(rec)
                }
                None => {
                    let id = format!("ms_{}", new_id());
                    conn.execute(
                        "INSERT INTO repo_micro_skills
                         (id, repo_scope, signature, fact, failed_cmd, fixed_cmd, corroborations,
                          session_ids_json, need_hits, exposures, contradictions, status,
                          created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 2, '[]', 0, 0, 0, 'active', ?7, ?7)",
                        params![
                            id,
                            repo_scope,
                            signature,
                            clean_fact,
                            failed_cmd.trim(),
                            fixed_cmd.trim(),
                            now_str
                        ],
                    )?;
                    Ok(RepoMicroSkill {
                        id,
                        repo_scope: repo_scope.to_string(),
                        signature: signature.to_string(),
                        fact: clean_fact,
                        failed_cmd: failed_cmd.trim().to_string(),
                        fixed_cmd: fixed_cmd.trim().to_string(),
                        corroborations: 2,
                        session_ids: Vec::new(),
                        need_hits: 0,
                        exposures: 0,
                        contradictions: 0,
                        status: MicroSkillStatus::Active,
                        created_at: now,
                        updated_at: now,
                    })
                }
            }
        })
    }

    /// Records a contradiction against an active/candidate micro-skill, retiring it on 1 strike.
    pub fn record_micro_skill_contradiction(&self, id: &str) -> Result<bool> {
        let now_str = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            let cur: Option<(i64, String)> = conn
                .query_row(
                    "SELECT contradictions, status FROM repo_micro_skills WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((contradictions, status)) = cur else {
                return Ok(false);
            };
            let next_c = contradictions.saturating_add(1);
            let next_status = match (next_c as u32) >= ECONOMY_MICRO_SKILL_MAX_CONTRADICTIONS {
                true => MicroSkillStatus::Retired.as_str(),
                false => status.as_str(),
            };
            conn.execute(
                "UPDATE repo_micro_skills SET contradictions = ?1, status = ?2, updated_at = ?3 WHERE id = ?4",
                params![next_c, next_status, now_str, id],
            )?;
            Ok(next_status == MicroSkillStatus::Retired.as_str())
        })
    }

    /// Records exposure and need-hit counts for injected active micro-skills, auto-retiring
    /// low-value skills whose need rate drops below `ECONOMY_MICRO_SKILL_MIN_NEED_RATE`.
    pub fn record_micro_skill_exposures(
        &self,
        injected_ids: &[String],
        hit_ids: &BTreeSet<String>,
    ) -> Result<usize> {
        if injected_ids.is_empty() {
            return Ok(0);
        }
        let now_str = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            let mut retired_count = 0usize;
            for id in injected_ids {
                let hit_inc: i64 = match hit_ids.contains(id) {
                    true => 1,
                    false => 0,
                };
                conn.execute(
                    "UPDATE repo_micro_skills
                     SET exposures = exposures + 1, need_hits = need_hits + ?1, updated_at = ?2
                     WHERE id = ?3 AND status = 'active'",
                    params![hit_inc, now_str, id],
                )?;
                let stats: Option<(i64, i64)> = conn
                    .query_row(
                        "SELECT need_hits, exposures FROM repo_micro_skills WHERE id = ?1 AND status = 'active'",
                        params![id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                if let Some((hits, exp)) = stats
                    && (exp as u32) >= ECONOMY_MICRO_SKILL_MIN_EXPOSURES_FOR_RATE
                    && (hits as f64) / (exp.max(1) as f64) < ECONOMY_MICRO_SKILL_MIN_NEED_RATE
                {
                    conn.execute(
                        "UPDATE repo_micro_skills SET status = 'retired', updated_at = ?1 WHERE id = ?2",
                        params![now_str, id],
                    )?;
                    retired_count += 1;
                }
            }
            Ok(retired_count)
        })
    }

    /// Explicitly retires a repository micro-skill by ID.
    pub fn retire_micro_skill(&self, id: &str) -> Result<bool> {
        let now_str = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            let changed = conn.execute(
                "UPDATE repo_micro_skills SET status = 'retired', updated_at = ?1 WHERE id = ?2",
                params![now_str, id],
            )?;
            Ok(changed > 0)
        })
    }

    /// Lists micro-skills for `repo_scope` ordered Active -> Candidate -> Retired, highest corroborations first.
    pub fn list_repo_micro_skills(
        &self,
        repo_scope: &str,
        include_retired: bool,
    ) -> Result<Vec<RepoMicroSkill>> {
        self.with_conn(|conn| {
            let sql = match include_retired {
                true => {
                    "SELECT id, repo_scope, signature, fact, failed_cmd, fixed_cmd, corroborations,
                            session_ids_json, need_hits, exposures, contradictions, status,
                            created_at, updated_at
                     FROM repo_micro_skills
                     WHERE repo_scope = ?1
                     ORDER BY CASE status WHEN 'active' THEN 0 WHEN 'candidate' THEN 1 ELSE 2 END,
                              corroborations DESC, updated_at DESC
                     LIMIT 64"
                }
                false => {
                    "SELECT id, repo_scope, signature, fact, failed_cmd, fixed_cmd, corroborations,
                            session_ids_json, need_hits, exposures, contradictions, status,
                            created_at, updated_at
                     FROM repo_micro_skills
                     WHERE repo_scope = ?1 AND status != 'retired'
                     ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END,
                              corroborations DESC, updated_at DESC
                     LIMIT 64"
                }
            };
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt
                .query_map(params![repo_scope], row_to_micro_skill)?
                .filter_map(std::result::Result::ok)
                .collect();
            Ok(rows)
        })
    }

    /// Updates per-repo tool utilization after a completed turn.
    /// Soft-demotes persistently unused non-core tools (`pruned_to_deferred = 1`) when `cfg.auto_tool_prune`
    /// is enabled and automatically restores any demoted tool that gets invoked via `invoke_tool`.
    /// Returns the list of newly soft-pruned tool names.
    pub fn record_turn_tool_usage(
        &self,
        repo_scope: &str,
        session_id: &str,
        eager_tools: &[(String, u32)],
        used_tools: &BTreeSet<String>,
        cfg: &EconomyConfig,
    ) -> Result<Vec<String>> {
        let now_str = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            let mut newly_pruned = Vec::new();
            let mut seen_names: BTreeSet<String> = BTreeSet::new();

            for (tool_name, schema_tokens) in eager_tools {
                seen_names.insert(tool_name.clone());
                let used = used_tools.contains(tool_name);
                let existing: Option<(i64, i64, i64, String, i64, i64)> = conn
                    .query_row(
                        "SELECT call_count, prompts_since_used, sessions_observed, last_session_id,
                                pruned_to_deferred, manually_pinned
                         FROM repo_tool_usage
                         WHERE repo_scope = ?1 AND tool_name = ?2",
                        params![repo_scope, tool_name],
                        |r| {
                            Ok((
                                r.get(0)?,
                                r.get(1)?,
                                r.get(2)?,
                                r.get(3)?,
                                r.get(4)?,
                                r.get(5)?,
                            ))
                        },
                    )
                    .optional()?;

                match (existing, used) {
                    (Some((calls, _, sess_obs, last_sid, _, pinned)), true) => {
                        let next_sess = match !session_id.is_empty() && last_sid != session_id {
                            true => sess_obs + 1,
                            false => sess_obs.max(1),
                        };
                        conn.execute(
                            "UPDATE repo_tool_usage
                             SET call_count = ?1, prompts_since_used = 0, sessions_observed = ?2,
                                 last_session_id = ?3, estimated_schema_tokens = ?4,
                                 pruned_to_deferred = 0, manually_pinned = ?5,
                                 last_used_at = ?6, updated_at = ?6
                             WHERE repo_scope = ?7 AND tool_name = ?8",
                            params![
                                calls + 1,
                                next_sess,
                                session_id,
                                *schema_tokens as i64,
                                pinned,
                                now_str,
                                repo_scope,
                                tool_name
                            ],
                        )?;
                    }
                    (Some((_, prompts_unused, sess_obs, last_sid, was_pruned, pinned)), false) => {
                        let next_prompts = prompts_unused + 1;
                        let next_sess = match !session_id.is_empty() && last_sid != session_id {
                            true => sess_obs + 1,
                            false => sess_obs.max(1),
                        };
                        let should_prune = cfg.auto_tool_prune
                            && pinned == 0
                            && !is_protected_core_tool(tool_name)
                            && (next_prompts as u32) >= cfg.tool_prune_min_prompts.max(1)
                            && (next_sess as u32) >= cfg.tool_prune_min_sessions.max(1);
                        let next_pruned = match (was_pruned != 0, should_prune) {
                            (true, _) | (_, true) => 1i64,
                            (false, false) => 0i64,
                        };
                        if was_pruned == 0 && next_pruned == 1 {
                            newly_pruned.push(tool_name.clone());
                        }
                        conn.execute(
                            "UPDATE repo_tool_usage
                             SET prompts_since_used = ?1, sessions_observed = ?2,
                                 last_session_id = ?3, estimated_schema_tokens = ?4,
                                 pruned_to_deferred = ?5, updated_at = ?6
                             WHERE repo_scope = ?7 AND tool_name = ?8",
                            params![
                                next_prompts,
                                next_sess,
                                session_id,
                                *schema_tokens as i64,
                                next_pruned,
                                now_str,
                                repo_scope,
                                tool_name
                            ],
                        )?;
                    }
                    (None, true) => {
                        conn.execute(
                            "INSERT INTO repo_tool_usage
                             (repo_scope, tool_name, call_count, prompts_since_used, sessions_observed,
                              last_session_id, estimated_schema_tokens, pruned_to_deferred,
                              manually_pinned, last_used_at, updated_at)
                             VALUES (?1, ?2, 1, 0, 1, ?3, ?4, 0, 0, ?5, ?5)",
                            params![
                                repo_scope,
                                tool_name,
                                session_id,
                                *schema_tokens as i64,
                                now_str
                            ],
                        )?;
                    }
                    (None, false) => {
                        conn.execute(
                            "INSERT INTO repo_tool_usage
                             (repo_scope, tool_name, call_count, prompts_since_used, sessions_observed,
                              last_session_id, estimated_schema_tokens, pruned_to_deferred,
                              manually_pinned, last_used_at, updated_at)
                             VALUES (?1, ?2, 0, 1, 1, ?3, ?4, 0, 0, NULL, ?5)",
                            params![
                                repo_scope,
                                tool_name,
                                session_id,
                                *schema_tokens as i64,
                                now_str
                            ],
                        )?;
                    }
                }
            }

            // Self-healing: if a previously soft-pruned tool was called via `invoke_tool`, restore it!
            for used_name in used_tools {
                if seen_names.contains(used_name) {
                    continue;
                }
                conn.execute(
                    "UPDATE repo_tool_usage
                     SET call_count = call_count + 1, prompts_since_used = 0,
                         pruned_to_deferred = 0, last_used_at = ?1, updated_at = ?1
                     WHERE repo_scope = ?2 AND tool_name = ?3",
                    params![now_str, repo_scope, used_name],
                )?;
            }

            Ok(newly_pruned)
        })
    }

    /// Manually soft-prunes or restores (and optionally pins) a tool for `repo_scope`.
    /// Refuses to prune protected core tools (`ECONOMY_PROTECTED_CORE_TOOLS`).
    pub fn set_tool_pruned(
        &self,
        repo_scope: &str,
        tool_name: &str,
        pruned: bool,
        pin: bool,
    ) -> Result<bool> {
        let name = tool_name.trim();
        if name.is_empty() || (pruned && is_protected_core_tool(name)) {
            return Ok(false);
        }
        let now_str = Utc::now().to_rfc3339();
        let pruned_i = i64::from(pruned);
        let pin_i = i64::from(pin && !pruned);
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO repo_tool_usage
                 (repo_scope, tool_name, call_count, prompts_since_used, sessions_observed,
                  last_session_id, estimated_schema_tokens, pruned_to_deferred,
                  manually_pinned, last_used_at, updated_at)
                 VALUES (?1, ?2, 0, 0, 1, '', 0, ?3, ?4, NULL, ?5)
                 ON CONFLICT(repo_scope, tool_name) DO UPDATE SET
                    pruned_to_deferred = excluded.pruned_to_deferred,
                    manually_pinned = CASE WHEN excluded.pruned_to_deferred = 1 THEN 0 ELSE excluded.manually_pinned END,
                    prompts_since_used = CASE WHEN excluded.pruned_to_deferred = 0 THEN 0 ELSE repo_tool_usage.prompts_since_used END,
                    updated_at = excluded.updated_at",
                params![repo_scope, name, pruned_i, pin_i, now_str],
            )?;
            Ok(true)
        })
    }

    /// Periodic re-audit: restores non-pinned pruned tools back to eager with reset counters so
    /// changing project needs never suffer permanent tool lockouts.
    pub fn reaudit_pruned_tools(&self, repo_scope: &str) -> Result<usize> {
        let now_str = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            let restored = conn.execute(
                "UPDATE repo_tool_usage
                 SET pruned_to_deferred = 0, prompts_since_used = 0, updated_at = ?1
                 WHERE repo_scope = ?2 AND pruned_to_deferred = 1",
                params![now_str, repo_scope],
            )?;
            Ok(restored)
        })
    }

    pub fn list_repo_tool_usage(&self, repo_scope: &str) -> Result<Vec<RepoToolUsage>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT repo_scope, tool_name, call_count, prompts_since_used, sessions_observed,
                        estimated_schema_tokens, pruned_to_deferred, manually_pinned,
                        last_used_at, updated_at
                 FROM repo_tool_usage
                 WHERE repo_scope = ?1
                 ORDER BY pruned_to_deferred DESC, prompts_since_used DESC, call_count DESC, tool_name ASC",
            )?;
            let rows = stmt
                .query_map(params![repo_scope], row_to_tool_usage)?
                .filter_map(std::result::Result::ok)
                .collect();
            Ok(rows)
        })
    }

    pub fn list_pruned_tools(&self, repo_scope: &str) -> Result<BTreeSet<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT tool_name FROM repo_tool_usage
                 WHERE repo_scope = ?1 AND pruned_to_deferred = 1",
            )?;
            let set: BTreeSet<String> = stmt
                .query_map(params![repo_scope], |r| r.get::<_, String>(0))?
                .filter_map(std::result::Result::ok)
                .filter(|name| !is_protected_core_tool(name))
                .collect();
            Ok(set)
        })
    }

    /// Records a completed turn's tool-call count for `repo_scope` and prunes old rows beyond the
    /// sliding baseline window. Returns total completed task count for periodic re-audit cadence.
    pub fn record_turn_baseline(
        &self,
        repo_scope: &str,
        tool_calls: usize,
        iterations: usize,
        succeeded: bool,
    ) -> Result<u64> {
        let now_str = Utc::now().to_rfc3339();
        let id = format!("tb_{}", new_id());
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO repo_turn_baselines (id, repo_scope, tool_calls, iterations, succeeded, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    repo_scope,
                    tool_calls as i64,
                    iterations as i64,
                    i64::from(succeeded),
                    now_str
                ],
            )?;
            // Bound baseline table per repo to a compact sliding window
            conn.execute(
                "DELETE FROM repo_turn_baselines
                 WHERE repo_scope = ?1 AND id NOT IN (
                     SELECT id FROM repo_turn_baselines
                     WHERE repo_scope = ?1
                     ORDER BY created_at DESC
                     LIMIT ?2
                 )",
                params![repo_scope, (ECONOMY_LOOP_MAX_BASELINE_SAMPLES * 2) as i64],
            )?;
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM repo_turn_baselines WHERE repo_scope = ?1",
                params![repo_scope],
                |r| r.get(0),
            )?;
            Ok(count.max(0) as u64)
        })
    }

    /// Computes deterministic nearest-rank P90 successful tool-call count and adaptive turn envelope
    /// (`max(MIN_ENVELOPE_CALLS, ceil(P90 * multiplier))`) for `repo_scope`.
    /// Returns `(p90_calls, envelope_calls, sample_count)` (`envelope_calls == 0` if under minimum samples).
    pub fn compute_repo_p90_envelope(
        &self,
        repo_scope: &str,
        multiplier: f64,
    ) -> Result<(usize, usize, usize)> {
        let mut samples: Vec<usize> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT tool_calls FROM repo_turn_baselines
                 WHERE repo_scope = ?1 AND succeeded = 1 AND tool_calls > 0
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(
                    params![repo_scope, ECONOMY_LOOP_MAX_BASELINE_SAMPLES as i64],
                    |r| Ok(r.get::<_, i64>(0)?.max(0) as usize),
                )?
                .filter_map(std::result::Result::ok)
                .collect();
            Ok(rows)
        })?;

        let n = samples.len();
        if n == 0 {
            return Ok((0, 0, 0));
        }
        samples.sort_unstable();
        // Deterministic nearest-rank 90th percentile index: ceil(0.9 * n) - 1
        let idx = ((n * 9).div_ceil(10)).saturating_sub(1).min(n - 1);
        let p90 = samples[idx];
        let envelope = match n >= ECONOMY_LOOP_P90_MIN_SAMPLES {
            true => {
                let scaled = ((p90 as f64) * multiplier.clamp(1.0, 3.0)).ceil() as usize;
                scaled.max(ECONOMY_LOOP_MIN_ENVELOPE_CALLS)
            }
            false => 0,
        };
        Ok((p90, envelope, n))
    }

    /// Appends a categorized savings or overhead event to `economy_ledger` (bounded ring buffer).
    pub fn record_economy_ledger_event(
        &self,
        repo_scope: &str,
        kind: EconomyLedgerKind,
        tokens_delta: i64,
        cost_usd_delta: f64,
        detail: &str,
    ) -> Result<()> {
        let id = format!("el_{}", new_id());
        let now_str = Utc::now().to_rfc3339();
        let clean_detail = truncate_utf8_chars(detail, 180);
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO economy_ledger (id, repo_scope, kind, tokens_delta, cost_usd_delta, detail, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    repo_scope,
                    kind.as_str(),
                    tokens_delta,
                    cost_usd_delta,
                    clean_detail,
                    now_str
                ],
            )?;
            conn.execute(
                "DELETE FROM economy_ledger
                 WHERE repo_scope = ?1 AND id NOT IN (
                     SELECT id FROM economy_ledger
                     WHERE repo_scope = ?1
                     ORDER BY created_at DESC
                     LIMIT ?2
                 )",
                params![repo_scope, ECONOMY_LEDGER_MAX_EVENTS as i64],
            )?;
            Ok(())
        })
    }

    /// Aggregates honest per-repo Token Economy ROI (gross savings minus background overhead).
    pub fn summarize_economy_ledger(
        &self,
        repo_scope: &str,
        p90_multiplier: f64,
    ) -> Result<EconomyLedgerSummary> {
        let (p90_turn_calls, envelope_calls, baseline_samples) =
            self.compute_repo_p90_envelope(repo_scope, p90_multiplier)?;

        self.with_conn(|conn| {
            let mut summary = EconomyLedgerSummary {
                repo_scope: repo_scope.to_string(),
                p90_turn_calls,
                envelope_calls,
                baseline_samples,
                ..EconomyLedgerSummary::default()
            };

            let mut stmt = conn.prepare(
                "SELECT kind, COUNT(*), COALESCE(SUM(tokens_delta), 0), COALESCE(SUM(cost_usd_delta), 0.0)
                 FROM economy_ledger
                 WHERE repo_scope = ?1
                 GROUP BY kind",
            )?;
            let mut explicit_usd = 0.0f64;
            let rows = stmt.query_map(params![repo_scope], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?.max(0) as u64,
                    r.get::<_, i64>(2)?,
                    r.get::<_, f64>(3)?,
                ))
            })?;
            for row in rows.flatten() {
                let (kind_str, count, tok_sum, usd_sum) = row;
                explicit_usd += usd_sum;
                match EconomyLedgerKind::from_str_loose(&kind_str) {
                    Some(EconomyLedgerKind::ReadLess) => {
                        summary.read_less_tokens_saved =
                            summary.read_less_tokens_saved.saturating_add(tok_sum.max(0) as u64);
                    }
                    Some(EconomyLedgerKind::ToolPrune) => {
                        summary.tool_prune_tokens_saved =
                            summary.tool_prune_tokens_saved.saturating_add(tok_sum.max(0) as u64);
                    }
                    Some(EconomyLedgerKind::MicroSkill) => {
                        summary.micro_skill_injections =
                            summary.micro_skill_injections.saturating_add(count);
                    }
                    Some(EconomyLedgerKind::LoopGuard) => {
                        summary.loop_guard_interventions =
                            summary.loop_guard_interventions.saturating_add(count);
                    }
                    Some(EconomyLedgerKind::OverheadDream | EconomyLedgerKind::OverheadJudge) => {
                        summary.overhead_tokens =
                            summary.overhead_tokens.saturating_add(tok_sum.unsigned_abs());
                    }
                    None => {}
                }
            }
            drop(stmt);

            let gross_saved = summary
                .read_less_tokens_saved
                .saturating_add(summary.tool_prune_tokens_saved);
            summary.net_tokens_saved = (gross_saved as i64) - (summary.overhead_tokens as i64);
            let heuristic_usd = ((summary.net_tokens_saved as f64) / 1_000_000.0)
                * ECONOMY_ESTIMATED_USD_PER_1M_TOKENS;
            summary.estimated_usd_saved = match explicit_usd.abs() > f64::EPSILON {
                true => explicit_usd,
                false => heuristic_usd,
            };

            let mut ms_stmt = conn.prepare(
                "SELECT status, COUNT(*) FROM repo_micro_skills WHERE repo_scope = ?1 GROUP BY status",
            )?;
            for row in ms_stmt
                .query_map(params![repo_scope], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as usize))
                })?
                .flatten()
            {
                match MicroSkillStatus::from_str_loose(&row.0) {
                    MicroSkillStatus::Active => summary.active_micro_skills = row.1,
                    MicroSkillStatus::Candidate => summary.candidate_micro_skills = row.1,
                    MicroSkillStatus::Retired => summary.retired_micro_skills = row.1,
                }
            }
            drop(ms_stmt);

            summary.pruned_tools_count = conn
                .query_row(
                    "SELECT COUNT(*) FROM repo_tool_usage WHERE repo_scope = ?1 AND pruned_to_deferred = 1",
                    params![repo_scope],
                    |r| Ok(r.get::<_, i64>(0)?.max(0) as usize),
                )
                .unwrap_or(0);

            summary.completed_tasks = conn
                .query_row(
                    "SELECT COUNT(*) FROM repo_turn_baselines WHERE repo_scope = ?1",
                    params![repo_scope],
                    |r| Ok(r.get::<_, i64>(0)?.max(0) as u64),
                )
                .unwrap_or(0);

            Ok(summary)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::TOOL_READ_FILE;

    fn temp_store() -> (SqliteStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("zenko_econ_db_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open sqlite");
        (store, dir)
    }

    #[test]
    fn micro_skill_corroborates_across_sessions_and_retires_on_one_contradiction() {
        let (store, dir) = temp_store();
        let scope = "project:demo";
        // First session -> Candidate
        let s1 = store
            .upsert_micro_skill_evidence(
                scope,
                "sig_pytest",
                "In this repo, `pytest` failed; use `python -m pytest` instead.",
                "pytest",
                "python -m pytest",
                "sess-1",
                2,
            )
            .expect("s1");
        assert_eq!(s1.status, MicroSkillStatus::Candidate);
        assert_eq!(s1.corroborations, 1);

        // Same session again does not double-count corroboration
        let s1_dup = store
            .upsert_micro_skill_evidence(
                scope,
                "sig_pytest",
                "In this repo, `pytest` failed; use `python -m pytest` instead.",
                "pytest",
                "python -m pytest",
                "sess-1",
                2,
            )
            .expect("s1 dup");
        assert_eq!(s1_dup.status, MicroSkillStatus::Candidate);
        assert_eq!(s1_dup.corroborations, 1);

        // Second distinct session -> promotes to Active
        let s2 = store
            .upsert_micro_skill_evidence(
                scope,
                "sig_pytest",
                "In this repo, `pytest` failed; use `python -m pytest` instead.",
                "pytest",
                "python -m pytest",
                "sess-2",
                2,
            )
            .expect("s2");
        assert_eq!(s2.status, MicroSkillStatus::Active);
        assert_eq!(s2.corroborations, 2);

        // 1 contradiction -> immediately Retired, and never resurrected
        let retired = store
            .record_micro_skill_contradiction(&s2.id)
            .expect("contradict");
        assert!(retired);
        let s3 = store
            .upsert_micro_skill_evidence(
                scope,
                "sig_pytest",
                "In this repo, `pytest` failed; use `python -m pytest` instead.",
                "pytest",
                "python -m pytest",
                "sess-3",
                2,
            )
            .expect("s3");
        assert_eq!(s3.status, MicroSkillStatus::Retired);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tool_pruning_protects_core_tools_soft_demotes_unused_and_self_heals_on_use() {
        let (store, dir) = temp_store();
        let scope = "project:demo";
        let cfg = EconomyConfig {
            tool_prune_min_prompts: 3,
            tool_prune_min_sessions: 2,
            ..EconomyConfig::default()
        };
        let eager = vec![
            (TOOL_READ_FILE.to_string(), 140),
            ("web_search".to_string(), 220),
        ];
        let empty = BTreeSet::new();

        // Session 1 (2 prompts unused)
        assert!(
            store
                .record_turn_tool_usage(scope, "s1", &eager, &empty, &cfg)
                .expect("t1")
                .is_empty()
        );
        assert!(
            store
                .record_turn_tool_usage(scope, "s1", &eager, &empty, &cfg)
                .expect("t2")
                .is_empty()
        );
        // Session 2 (3rd prompt unused across 2nd session -> prunes web_search, never read_file!)
        let pruned = store
            .record_turn_tool_usage(scope, "s2", &eager, &empty, &cfg)
            .expect("t3");
        assert_eq!(pruned, vec!["web_search".to_string()]);
        let pruned_set = store.list_pruned_tools(scope).expect("set");
        assert!(pruned_set.contains("web_search"));
        assert!(!pruned_set.contains(TOOL_READ_FILE));

        // Self-healing: invoking web_search restores it cleanly
        let mut used = BTreeSet::new();
        used.insert("web_search".to_string());
        store
            .record_turn_tool_usage(scope, "s3", &[(TOOL_READ_FILE.to_string(), 140)], &used, &cfg)
            .expect("t4");
        assert!(store.list_pruned_tools(scope).expect("restored").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn p90_baseline_and_net_economy_ledger_compute_honest_roi() {
        let (store, dir) = temp_store();
        let scope = "project:demo";
        for calls in [4usize, 6, 8, 10, 12] {
            store
                .record_turn_baseline(scope, calls, 3, true)
                .expect("baseline");
        }
        let (p90, env, samples) = store
            .compute_repo_p90_envelope(scope, 1.25)
            .expect("p90");
        assert_eq!(samples, 5);
        assert_eq!(p90, 12);
        assert_eq!(env, 15);

        store
            .record_economy_ledger_event(scope, EconomyLedgerKind::ReadLess, 4_000, 0.0, "test")
            .expect("rl");
        store
            .record_economy_ledger_event(scope, EconomyLedgerKind::ToolPrune, 1_500, 0.0, "prune")
            .expect("tp");
        store
            .record_economy_ledger_event(scope, EconomyLedgerKind::OverheadDream, 500, 0.0, "dream")
            .expect("oh");
        let sum = store.summarize_economy_ledger(scope, 1.25).expect("sum");
        assert_eq!(sum.read_less_tokens_saved, 4_000);
        assert_eq!(sum.tool_prune_tokens_saved, 1_500);
        assert_eq!(sum.overhead_tokens, 500);
        assert_eq!(sum.net_tokens_saved, 5_000);
        let _ = std::fs::remove_dir_all(dir);
    }
}
