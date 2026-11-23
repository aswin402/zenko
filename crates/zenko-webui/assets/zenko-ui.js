/* Zenko Ultra-Light Vanilla Client Engine (<12KB, zero runtime frameworks) */
(function () {
  const root = document.documentElement;

  window.zenkoToast = function (msg, kind = 'info') {
    let stack = document.getElementById('z-toast-stack');
    if (!stack) {
      stack = document.createElement('div');
      stack.id = 'z-toast-stack';
      stack.className = 'z-toast-stack';
      document.body.appendChild(stack);
    }
    const el = document.createElement('div');
    el.className = 'z-toast';
    if (kind === 'error') el.style.borderColor = 'var(--destructive)';
    if (kind === 'success') el.style.borderColor = 'var(--success)';
    el.textContent = msg;
    stack.appendChild(el);
    setTimeout(() => { el.style.opacity = '0'; setTimeout(() => el.remove(), 220); }, 3400);
  };

  async function api(url, opts = {}) {
    const res = await fetch(url, {
      headers: { 'Content-Type': 'application/json', ...(opts.headers || {}) },
      ...opts,
    });
    const text = await res.text();
    let data = {};
    try { data = text ? JSON.parse(text) : {}; } catch { data = { raw: text }; }
    if (!res.ok) throw new Error(data.error || `HTTP ${res.status}`);
    return data;
  }
  window.zenkoApi = api;

  // ── Live Zero-Reload UI Customization Engine ───────────────────────────────
  window.zenkoUiState = window.__ZENKO_UI_CONFIG__ || null;

  window.zenkoApplyUi = function (cfg) {
    if (!cfg) return;
    window.zenkoUiState = cfg;
    const c = cfg.colors || {};
    const setVar = (k, v) => { if (v !== undefined && v !== null) root.style.setProperty(k, v); };
    setVar('--bg', c.background);
    setVar('--surface', c.surface);
    setVar('--card', c.card);
    setVar('--border', c.border);
    setVar('--fg', c.foreground);
    setVar('--muted', c.muted);
    setVar('--primary', c.primary);
    setVar('--primary-fg', c.primary_foreground);
    setVar('--accent', c.accent);
    setVar('--destructive', c.destructive);
    setVar('--success', c.success);
    setVar('--warning', c.warning);

    if (cfg.surface) {
      const r = Math.min(24, Math.max(0, Number(cfg.surface.radius_px ?? 10)));
      setVar('--radius', r + 'px');
      setVar('--radius-sm', Math.max(2, r - 4) + 'px');
      setVar('--radius-lg', Math.min(28, r + 4) + 'px');
      root.dataset.surface = cfg.surface.card_style || 'bordered';
      root.dataset.bgPattern = cfg.surface.bg_pattern || 'dot_grid';
    }
    if (cfg.buttons) {
      root.dataset.btnStyle = cfg.buttons.style || 'solid';
      root.dataset.btnShape = cfg.buttons.shape || 'rounded';
      root.dataset.btnSize = cfg.buttons.size || 'default';
      root.dataset.btnMotion = cfg.buttons.motion || 'scale_press';
    }
    if (cfg.icons) {
      setVar('--icon-size', (cfg.icons.size_px || 16) + 'px');
      setVar('--icon-stroke', String(cfg.icons.stroke_width || 1.75));
      root.dataset.iconStyle = cfg.icons.style || 'outline';
      root.dataset.iconPack = cfg.icons.pack || 'lucide';
    }
    if (cfg.typography) {
      setVar('--text-base', (cfg.typography.base_size_px || 14) + 'px');
      const uiMap = {
        inter: "'Inter', ui-sans-serif, system-ui, sans-serif",
        geist: "'Geist', 'Inter', ui-sans-serif, system-ui, sans-serif",
        jakarta: "'Plus Jakarta Sans', 'Inter', ui-sans-serif, system-ui, sans-serif",
        mono: "'JetBrains Mono', ui-monospace, monospace",
      };
      const monoMap = {
        jetbrains: "'JetBrains Mono', ui-monospace, monospace",
        fira: "'Fira Code', ui-monospace, monospace",
        cascadia: "'Cascadia Code', ui-monospace, monospace",
        system: "ui-monospace, SFMono-Regular, Menlo, monospace",
      };
      setVar('--font-sans', uiMap[cfg.typography.ui_font] || uiMap.inter);
      setVar('--font-mono', monoMap[cfg.typography.mono_font] || monoMap.jetbrains);
    }
    if (cfg.layout) {
      root.dataset.density = cfg.layout.density || 'balanced';
      root.dataset.sidebar = cfg.layout.sidebar_mode || 'fixed';
      root.dataset.chatLayout = cfg.layout.chat_layout || 'editorial';
      root.dataset.inspector = String(cfg.layout.show_inspector !== false);
    }
    if (cfg.motion) {
      const mMap = {
        snappy: ['140ms', 'cubic-bezier(0.2, 0.8, 0.2, 1)'],
        fluid_spring: ['260ms', 'cubic-bezier(0.34, 1.4, 0.64, 1)'],
        reduced: ['80ms', 'linear'],
        instant: ['0ms', 'linear'],
      };
      const [ms, ease] = mMap[cfg.motion.preset] || mMap.snappy;
      setVar('--motion-ms', ms);
      setVar('--motion-ease', ease);
      root.dataset.orbStyle = cfg.motion.orb_style || 'liquid_halo';
    }
  };

  let saveTimer = null;
  window.zenkoPatchUi = function (mutator, immediate = false) {
    if (!window.zenkoUiState) return;
    mutator(window.zenkoUiState);
    window.zenkoApplyUi(window.zenkoUiState);
    clearTimeout(saveTimer);
    const persist = async () => {
      try {
        await api('/api/ui/active', { method: 'PUT', body: JSON.stringify(window.zenkoUiState) });
      } catch (e) {
        zenkoToast('Failed to save appearance: ' + e.message, 'error');
      }
    };
    if (immediate) persist();
    else saveTimer = setTimeout(persist, 260);
  };

  window.zenkoSelectPreset = async function (presetId) {
    try {
      const res = await api('/api/ui/preset/' + encodeURIComponent(presetId), { method: 'POST' });
      if (res.config) {
        window.zenkoApplyUi(res.config);
        zenkoToast('Applied theme preset: ' + presetId.replaceAll('_', ' '), 'success');
        if (location.pathname.startsWith('/settings/appearance')) location.reload();
      }
    } catch (e) {
      zenkoToast(e.message, 'error');
    }
  };

  window.zenkoSaveNamedProfile = async function () {
    const nameInput = document.getElementById('custom-profile-name');
    const name = nameInput ? nameInput.value.trim() : '';
    if (!name) return zenkoToast('Enter a profile name first', 'error');
    try {
      await api('/api/ui/profiles', {
        method: 'POST',
        body: JSON.stringify({ name, config: window.zenkoUiState }),
      });
      zenkoToast('Saved UI profile "' + name + '"', 'success');
      location.reload();
    } catch (e) {
      zenkoToast(e.message, 'error');
    }
  };

  window.zenkoActivateProfile = async function (id) {
    try {
      await api('/api/ui/profiles/' + encodeURIComponent(id) + '/activate', { method: 'POST' });
      location.reload();
    } catch (e) { zenkoToast(e.message, 'error'); }
  };

  window.zenkoExportUiJson = function () {
    const blob = new Blob([JSON.stringify(window.zenkoUiState, null, 2)], { type: 'application/json' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = 'zenko-theme-profile.json';
    a.click();
  };

  window.zenkoImportUiJson = async function (input) {
    const file = input.files && input.files[0];
    if (!file) return;
    try {
      const cfg = JSON.parse(await file.text());
      window.zenkoApplyUi(cfg);
      await api('/api/ui/active', { method: 'PUT', body: JSON.stringify(cfg) });
      zenkoToast('Imported theme profile!', 'success');
      location.reload();
    } catch (e) { zenkoToast('Invalid JSON: ' + e.message, 'error'); }
  };

  // ── Quick Appearance Drawer & Cmd+K Palette ────────────────────────────────
  window.zenkoToggleCustomizer = function (force) {
    const el = document.getElementById('z-customizer-sheet');
    if (!el) return;
    el.classList.toggle('open', force);
  };
  window.zenkoToggleCmdK = function (force) {
    const el = document.getElementById('z-cmdk-modal');
    if (!el) return;
    const isOpen = el.classList.toggle('open', force);
    if (isOpen) {
      const inp = document.getElementById('z-cmdk-input');
      if (inp) { inp.value = ''; inp.focus(); window.zenkoFilterCmdK(''); }
    }
  };
  window.zenkoFilterCmdK = function (q) {
    const term = (q || '').toLowerCase();
    document.querySelectorAll('#z-cmdk-list [data-cmd-label]').forEach((item) => {
      const ok = item.dataset.cmdLabel.toLowerCase().includes(term);
      item.style.display = ok ? 'flex' : 'none';
    });
  };

  document.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault(); window.zenkoToggleCmdK();
    } else if ((e.metaKey || e.ctrlKey) && e.key === '.') {
      e.preventDefault(); window.zenkoToggleCustomizer();
    } else if (e.key === 'Escape') {
      window.zenkoToggleCmdK(false);
      window.zenkoToggleCustomizer(false);
    }
  });

  // ── Workbench Chat, Branching, Approvals & SSE Live Stream ─────────────────
  window.zenkoNewSession = async function () {
    try {
      const s = await api('/api/sessions', { method: 'POST', body: JSON.stringify({ title: 'New Session' }) });
      location.href = '/sessions/' + encodeURIComponent(s.id);
    } catch (e) { zenkoToast(e.message, 'error'); }
  };

  window.zenkoDeleteSession = async function (id) {
    try {
      await api('/api/sessions/' + encodeURIComponent(id), { method: 'DELETE' });
      location.href = '/';
    } catch (e) { zenkoToast(e.message, 'error'); }
  };

  window.zenkoSendMessage = async function (e) {
    if (e) e.preventDefault();
    const sid = document.body.dataset.sessionId;
    const input = document.getElementById('composer-input');
    const text = input ? input.value.trim() : '';
    if (!sid || !text) return;
    const parentOverride = input.dataset.branchParent || null;
    input.value = '';
    delete input.dataset.branchParent;
    const banner = document.getElementById('branch-edit-banner');
    if (banner) banner.style.display = 'none';

    try {
      const payload = { text };
      if (parentOverride !== null) payload.parent_event_id = parentOverride === '__ROOT__' ? null : parentOverride;
      await api('/api/sessions/' + encodeURIComponent(sid) + '/messages', {
        method: 'POST',
        body: JSON.stringify(payload),
      });
    } catch (err) {
      zenkoToast(err.message, 'error');
    }
  };

  window.zenkoStopSession = async function () {
    const sid = document.body.dataset.sessionId;
    if (!sid) return;
    try {
      await api('/api/sessions/' + encodeURIComponent(sid) + '/stop', { method: 'POST' });
      zenkoToast('Cancellation requested');
    } catch (e) { zenkoToast(e.message, 'error'); }
  };

  window.zenkoUpdateSessionMeta = async function (patch) {
    const sid = document.body.dataset.sessionId;
    if (!sid) return;
    try {
      await api('/api/sessions/' + encodeURIComponent(sid), { method: 'PATCH', body: JSON.stringify(patch) });
      zenkoToast('Updated session settings', 'success');
    } catch (e) { zenkoToast(e.message, 'error'); }
  };

  window.zenkoEditBranch = function (parentId, oldText) {
    const input = document.getElementById('composer-input');
    const banner = document.getElementById('branch-edit-banner');
    if (!input) return;
    input.value = oldText;
    input.dataset.branchParent = parentId || '__ROOT__';
    if (banner) banner.style.display = 'flex';
    input.focus();
  };

  window.zenkoSwitchBranch = async function (eventId) {
    const sid = document.body.dataset.sessionId;
    if (!sid) return;
    try {
      await api('/api/sessions/' + encodeURIComponent(sid) + '/branch', {
        method: 'POST',
        body: JSON.stringify({ event_id: eventId }),
      });
      location.reload();
    } catch (e) { zenkoToast(e.message, 'error'); }
  };

  window.zenkoResolveApproval = async function (approvalId, decision) {
    try {
      await api('/api/approvals/' + encodeURIComponent(approvalId), {
        method: 'POST',
        body: JSON.stringify({ decision }),
      });
      const el = document.getElementById('approval-' + approvalId);
      if (el) el.remove();
      zenkoToast('Approval ' + decision, 'success');
    } catch (e) { zenkoToast(e.message, 'error'); }
  };

  function escapeHtml(str) {
    return String(str || '')
      .replaceAll('&', '&amp;').replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;').replaceAll('"', '&quot;');
  }

  function renderSimpleMarkdown(md) {
    const parts = String(md || '').split(/```/);
    return parts.map((seg, i) => {
      if (i % 2 === 1) {
        const nl = seg.indexOf('\n');
        const code = nl >= 0 ? seg.slice(nl + 1) : seg;
        return '<pre class="code-block"><code>' + escapeHtml(code) + '</code></pre>';
      }
      return escapeHtml(seg)
        .replace(/`([^`]+)`/g, '<code class="font-mono" style="background:var(--surface);padding:1px 5px;border-radius:4px;">$1</code>')
        .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
        .replace(/\n/g, '<br>');
    }).join('');
  }

  function renderDiffHtml(diff) {
    return String(diff || '').split('\n').map((line) => {
      if (line.startsWith('+') && !line.startsWith('+++')) return '<span class="diff-line-add">' + escapeHtml(line) + '</span>';
      if (line.startsWith('-') && !line.startsWith('---')) return '<span class="diff-line-del">' + escapeHtml(line) + '</span>';
      if (line.startsWith('@@')) return '<span class="diff-line-hunk">' + escapeHtml(line) + '</span>';
      return '<span>' + escapeHtml(line) + '\n</span>';
    }).join('');
  }

  // Subscribe to Daemon SSE stream
  if (document.body.dataset.enableSse === 'true') {
    const activeSid = document.body.dataset.sessionId || '';
    const streamEl = document.getElementById('chat-stream-inner');
    const scrollWrap = document.getElementById('chat-stream');
    const orbEl = document.getElementById('zenko-status-orb');
    const stopBtn = document.getElementById('composer-stop-btn');
    const logBox = document.getElementById('live-log-stream');
    let liveTurnBox = null;
    let liveThinkBuf = '';
    let liveTextBuf = '';

    const stickBottom = () => { if (scrollWrap) scrollWrap.scrollTop = scrollWrap.scrollHeight; };

    const es = new EventSource('/api/events');
    es.onmessage = (msg) => {
      let ev;
      try { ev = JSON.parse(msg.data); } catch { return; }

      if (ev.event === 'session_updated' && ev.session_id === activeSid) {
        if (orbEl) orbEl.dataset.state = ev.status;
        if (stopBtn) stopBtn.style.display = (ev.status === 'running' || ev.status === 'waiting_approval') ? 'inline-flex' : 'none';
      }

      if (ev.event === 'user_message_accepted' && ev.session_id === activeSid && streamEl) {
        const div = document.createElement('div');
        div.className = 'msg-card msg-user';
        div.innerHTML = '<div class="text-xs text-muted font-semibold" style="margin-bottom:4px;">You</div><div>' + renderSimpleMarkdown(ev.text) + '</div>';
        streamEl.appendChild(div);
        liveTurnBox = null; liveThinkBuf = ''; liveTextBuf = '';
        stickBottom();
      }

      if ((ev.event === 'thinking_delta' || ev.event === 'text_delta') && ev.session_id === activeSid && streamEl) {
        if (!liveTurnBox) {
          liveTurnBox = document.createElement('div');
          liveTurnBox.className = 'msg-card msg-assistant';
          liveTurnBox.innerHTML =
            '<div class="text-xs text-muted font-semibold" style="margin-bottom:6px;">Zenko</div>' +
            '<details class="thinking-box" style="display:none;"><summary>Thinking trace...</summary><div class="thinking-content"></div></details>' +
            '<div class="assistant-body"></div>';
          streamEl.appendChild(liveTurnBox);
        }
        if (ev.event === 'thinking_delta') {
          liveThinkBuf += ev.text;
          const tb = liveTurnBox.querySelector('.thinking-box');
          tb.style.display = 'block';
          tb.querySelector('.thinking-content').textContent = liveThinkBuf;
        } else {
          liveTextBuf += ev.text;
          liveTurnBox.querySelector('.assistant-body').innerHTML = renderSimpleMarkdown(liveTextBuf);
        }
        stickBottom();
      }

      if (ev.event === 'tool_call_started' && ev.session_id === activeSid && streamEl) {
        liveTurnBox = null; liveThinkBuf = ''; liveTextBuf = '';
        const card = document.createElement('details');
        card.id = 'tool-' + ev.call_id;
        card.className = 'tool-card';
        card.innerHTML =
          '<summary class="tool-card-head"><span class="font-mono font-semibold">' + escapeHtml(ev.tool_name) + '</span>' +
          '<span class="z-badge z-badge-accent">running...</span></summary>' +
          '<div class="tool-card-body">Args: ' + escapeHtml(ev.args_preview) + '</div>';
        streamEl.appendChild(card);
        stickBottom();
      }

      if (ev.event === 'tool_call_finished' && ev.session_id === activeSid) {
        const card = document.getElementById('tool-' + ev.call_id);
        if (card) {
          const badge = card.querySelector('.z-badge');
          if (badge) {
            badge.className = 'z-badge ' + (ev.is_error ? 'z-badge-danger' : 'z-badge-success');
            badge.textContent = (ev.is_error ? 'error ' : 'ok ') + ev.duration_ms + 'ms';
          }
          const body = card.querySelector('.tool-card-body');
          if (body) {
            body.innerHTML += '\n\n' + (ev.diff ? renderDiffHtml(ev.diff) : escapeHtml(ev.output_preview));
          }
        }
        stickBottom();
      }

      if (ev.event === 'tool_approval_requested') {
        const a = ev.approval;
        zenkoToast('Approval needed for ' + a.tool_name, 'info');
        if (a.session_id === activeSid && streamEl) {
          const box = document.createElement('div');
          box.id = 'approval-' + a.id;
          box.className = 'approval-banner';
          box.innerHTML =
            '<div class="flex items-center justify-between"><strong class="text-sm">Approval Required: <code class="font-mono">' + escapeHtml(a.tool_name) + '</code></strong>' +
            '<span class="z-badge z-badge-warning">' + escapeHtml(a.risk) + '</span></div>' +
            '<div class="font-mono text-xs text-muted">' + escapeHtml(a.args_preview) + '</div>' +
            '<div class="flex gap-2">' +
            '<button class="z-btn z-btn-success z-btn-sm" onclick="zenkoResolveApproval(\'' + a.id + '\', \'approve\')">Approve</button>' +
            '<button class="z-btn z-btn-secondary z-btn-sm" onclick="zenkoResolveApproval(\'' + a.id + '\', \'always_allow\')">Always Allow</button>' +
            '<button class="z-btn z-btn-danger z-btn-sm" onclick="zenkoResolveApproval(\'' + a.id + '\', \'deny\')">Deny</button>' +
            '</div>';
          streamEl.appendChild(box);
          stickBottom();
        }
      }

      if (ev.event === 'tool_approval_resolved') {
        const el = document.getElementById('approval-' + ev.approval_id);
        if (el) el.remove();
      }

      if (ev.event === 'turn_completed' && ev.session_id === activeSid) {
        liveTurnBox = null; liveThinkBuf = ''; liveTextBuf = '';
      }

      if (ev.event === 'turn_error' && ev.session_id === activeSid) {
        zenkoToast(ev.message, 'error');
      }

      if (ev.event === 'gateway_paired') {
        zenkoToast('Telegram paired: ' + (ev.peer.username || ev.peer.peer_id), 'success');
        if (location.pathname.startsWith('/gateways') || location.pathname.startsWith('/setup')) {
          setTimeout(() => location.reload(), 600);
        }
      }

      if (ev.event === 'log' && logBox) {
        const row = document.createElement('div');
        row.className = 'text-xs font-mono';
        row.style.padding = '3px 0';
        row.textContent = '[' + ev.level + '] ' + ev.target + ': ' + ev.message;
        logBox.appendChild(row);
        if (logBox.children.length > 250) logBox.firstChild.remove();
        logBox.scrollTop = logBox.scrollHeight;
      }
    };
  }
})();
