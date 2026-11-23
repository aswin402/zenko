use super::*;

impl SqliteStore {
    // ─── Approvals ─────────────────────────────────────────────────────────────

    pub fn insert_approval(&self, record: &ApprovalRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO approvals (id, session_id, call_id, tool_name, args_preview, risk, status, resolved_by, created_at, resolved_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    record.id,
                    record.session_id,
                    record.call_id,
                    record.tool_name,
                    record.args_preview,
                    record.risk.as_str(),
                    record.status.as_str(),
                    record.resolved_by,
                    record.created_at.to_rfc3339(),
                    record.resolved_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn resolve_approval(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
        resolved_by: &str,
    ) -> Result<Option<ApprovalRecord>> {
        let status = match decision {
            ApprovalDecision::Approve => ApprovalStatus::Approved,
            ApprovalDecision::Deny => ApprovalStatus::Denied,
            ApprovalDecision::AlwaysAllow => ApprovalStatus::AlwaysAllowed,
        };
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE approvals SET status = ?1, resolved_by = ?2, resolved_at = ?3 WHERE id = ?4 AND status = 'pending'",
                params![status.as_str(), resolved_by, now, approval_id],
            )?;
            conn.query_row(
                "SELECT id, session_id, call_id, tool_name, args_preview, risk, status, resolved_by, created_at, resolved_at
                 FROM approvals WHERE id = ?1",
                params![approval_id],
                row_to_approval,
            )
            .optional()
        })
    }

    pub fn list_pending_approvals(&self) -> Result<Vec<ApprovalRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, call_id, tool_name, args_preview, risk, status, resolved_by, created_at, resolved_at
                 FROM approvals WHERE status = 'pending' ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], row_to_approval)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    // ─── Gateways & Pairing ────────────────────────────────────────────────────

    pub fn create_pairing_code(
        &self,
        channel: &str,
        role: PeerRole,
        ttl_mins: i64,
    ) -> Result<PairingCode> {
        let code = zenko_core::SecretVault::generate_pairing_code();
        let expires_at = Utc::now() + Duration::minutes(ttl_mins.max(1));
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO pairing_codes (code, channel, role, expires_at, used_at) VALUES (?1, ?2, ?3, ?4, NULL)",
                params![code, channel, role.as_str(), expires_at.to_rfc3339()],
            )?;
            Ok(())
        })?;
        Ok(PairingCode {
            code,
            channel: channel.to_string(),
            role,
            expires_at,
            used_at: None,
        })
    }

    pub fn list_active_pairing_codes(&self, channel: &str) -> Result<Vec<PairingCode>> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT code, channel, role, expires_at, used_at
                 FROM pairing_codes
                 WHERE channel = ?1 AND used_at IS NULL AND expires_at > ?2
                 ORDER BY expires_at DESC",
            )?;
            let rows = stmt.query_map(params![channel, now], |r| {
                Ok(PairingCode {
                    code: r.get(0)?,
                    channel: r.get(1)?,
                    role: PeerRole::from_str_loose(&r.get::<_, String>(2)?),
                    expires_at: parse_dt(&r.get::<_, String>(3)?),
                    used_at: r.get::<_, Option<String>>(4)?.map(|s| parse_dt(&s)),
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn redeem_pairing_code(
        &self,
        channel: &str,
        code: &str,
        peer_id: &str,
        username: Option<&str>,
        display_name: Option<&str>,
    ) -> Result<Option<GatewayPeer>> {
        let normalized = code.trim().to_ascii_uppercase();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let redeemed_role: Option<PeerRole> = self.with_conn(|conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            let role_opt: Option<String> = tx
                .query_row(
                    "SELECT role FROM pairing_codes WHERE code = ?1 AND channel = ?2 AND used_at IS NULL AND expires_at > ?3",
                    params![normalized, channel, now_str],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(role_str) = role_opt else {
                tx.rollback()?;
                return Ok(None);
            };
            let updated = tx.execute(
                "UPDATE pairing_codes SET used_at = ?1 WHERE code = ?2 AND channel = ?3 AND used_at IS NULL AND expires_at > ?1",
                params![now_str, normalized, channel],
            )?;
            if updated != 1 {
                tx.rollback()?;
                return Ok(None);
            }
            let role = PeerRole::from_str_loose(&role_str);
            tx.execute(
                "INSERT OR REPLACE INTO gateway_peers (channel, peer_id, username, display_name, role, paired_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![channel, peer_id, username, display_name, role.as_str(), now_str],
            )?;
            tx.commit()?;
            Ok(Some(role))
        })?;
        Ok(redeemed_role.map(|role| GatewayPeer {
            channel: channel.to_string(),
            peer_id: peer_id.to_string(),
            username: username.map(String::from),
            display_name: display_name.map(String::from),
            role,
            paired_at: now,
        }))
    }

    pub fn get_peer(&self, channel: &str, peer_id: &str) -> Result<Option<GatewayPeer>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT channel, peer_id, username, display_name, role, paired_at
                 FROM gateway_peers WHERE channel = ?1 AND peer_id = ?2",
                params![channel, peer_id],
                row_to_peer,
            )
            .optional()
        })
    }

    pub fn list_peers(&self, channel: Option<&str>) -> Result<Vec<GatewayPeer>> {
        self.with_conn(|conn| {
            if let Some(ch) = channel {
                let mut stmt = conn.prepare(
                    "SELECT channel, peer_id, username, display_name, role, paired_at
                     FROM gateway_peers WHERE channel = ?1 ORDER BY paired_at DESC",
                )?;
                let rows = stmt.query_map(params![ch], row_to_peer)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            } else {
                let mut stmt = conn.prepare(
                    "SELECT channel, peer_id, username, display_name, role, paired_at
                     FROM gateway_peers ORDER BY paired_at DESC",
                )?;
                let rows = stmt.query_map([], row_to_peer)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            }
        })
    }

    pub fn update_peer_role(&self, channel: &str, peer_id: &str, role: PeerRole) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE gateway_peers SET role = ?1 WHERE channel = ?2 AND peer_id = ?3",
                params![role.as_str(), channel, peer_id],
            )?;
            Ok(())
        })
    }

    pub fn remove_peer(&self, channel: &str, peer_id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM gateway_peers WHERE channel = ?1 AND peer_id = ?2",
                params![channel, peer_id],
            )
        })?;
        Ok(n > 0)
    }

}
