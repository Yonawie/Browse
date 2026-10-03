// Browse — AI-Native Browser Shell Client (WebUI)
// Communicates with Core over JSON-RPC (/api/rpc) and SSE (/api/events)

class BrowseShell {
  constructor() {
    this.tabs = [];
    this.activeTabId = null;
    this.pendingConfirmation = null;

    this.initElements();
    this.initEvents();
    this.connectEventStream();
    this.fetchTabs();
    this.fetchMemoryStats();
  }

  initElements() {
    this.tabsList = document.getElementById('tabs-list');
    this.newTabBtn = document.getElementById('new-tab-btn');
    this.omnibox = document.getElementById('omnibox-input');
    this.pageTitle = document.getElementById('page-title');
    this.profileIndicator = document.getElementById('profile-indicator');
    this.webFrame = document.getElementById('web-frame');
    this.sidebarPane = document.getElementById('sidebar-pane');
    this.toggleSidebarBtn = document.getElementById('btn-toggle-sidebar');

    // Sidebar navigation
    this.sidebarTabs = document.querySelectorAll('.sidebar-tab');
    this.sidebarContents = document.querySelectorAll('.sidebar-content');

    // Intelligence
    this.chipSummarize = document.getElementById('chip-summarize');
    this.chipKeyfacts = document.getElementById('chip-keypoints');
    this.chipTranslate = document.getElementById('chip-translate');
    this.intelligenceOutput = document.getElementById('intelligence-output');
    this.askInput = document.getElementById('ask-input');
    this.btnSendAsk = document.getElementById('btn-send-ask');

    // Agent
    this.agentTaskInput = document.getElementById('agent-task-input');
    this.agentDryRun = document.getElementById('agent-dry-run');
    this.btnStartAgent = document.getElementById('btn-start-agent');
    this.btnStopAgent = document.getElementById('btn-stop-agent');
    this.timelineFeed = document.getElementById('timeline-feed');
    this.confirmationCard = document.getElementById('confirmation-card');
    this.confPreview = document.getElementById('conf-preview');
    this.confReason = document.getElementById('conf-reason');
    this.btnConfApprove = document.getElementById('btn-conf-approve');
    this.btnConfSession = document.getElementById('btn-conf-session');
    this.btnConfReject = document.getElementById('btn-conf-reject');

    // Memory
    this.memoryInput = document.getElementById('memory-search-input');
    this.btnMemorySearch = document.getElementById('btn-memory-search');
    this.memoryStats = document.getElementById('memory-stats-badge');
    this.memoryResults = document.getElementById('memory-results');
  }

  initEvents() {
    this.newTabBtn.addEventListener('click', () => this.createTab('https://example.com'));
    
    this.omnibox.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        const val = this.omnibox.value.trim();
        if (val.startsWith('http://') || val.startsWith('https://')) {
          this.navigate(val);
        } else if (val.startsWith('browser://')) {
          this.openInternalPage(val);
        } else {
          this.navigate('https://duckduckgo.com/?q=' + encodeURIComponent(val));
        }
      }
    });

    document.getElementById('btn-reload').addEventListener('click', () => {
      if (this.webFrame.src) this.webFrame.src = this.webFrame.src;
    });

    this.toggleSidebarBtn.addEventListener('click', () => {
      this.sidebarPane.classList.toggle('collapsed');
    });

    // Sidebar tab switching
    this.sidebarTabs.forEach((tab) => {
      tab.addEventListener('click', () => {
        const target = tab.dataset.tab;
        this.sidebarTabs.forEach(t => t.classList.remove('active'));
        this.sidebarContents.forEach(c => c.classList.remove('active'));
        tab.classList.add('active');
        document.getElementById(`tab-${target}`).classList.add('active');
      });
    });

    // Page Intelligence triggers
    this.chipSummarize.addEventListener('click', () => this.triggerPageAction('summarize'));
    this.chipKeyfacts.addEventListener('click', () => this.triggerPageAction('ask', 'Extract key facts and main takeaways'));
    this.chipTranslate.addEventListener('click', () => this.triggerPageAction('translate', 'Russian'));
    this.btnSendAsk.addEventListener('click', () => {
      const q = this.askInput.value.trim();
      if (q) {
        this.triggerPageAction('ask', q);
        this.askInput.value = '';
      }
    });
    this.askInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') this.btnSendAsk.click();
    });

    // Agent controls
    this.btnStartAgent.addEventListener('click', () => this.startAgentTask());
    this.btnStopAgent.addEventListener('click', () => this.stopAgentTask());

    // Confirmation card
    this.btnConfApprove.addEventListener('click', () => this.respondConfirmation('approved'));
    this.btnConfSession.addEventListener('click', () => this.respondConfirmation('approved_for_session'));
    this.btnConfReject.addEventListener('click', () => this.respondConfirmation('rejected'));

    // Memory search
    this.btnMemorySearch.addEventListener('click', () => this.searchMemory());
    this.memoryInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') this.searchMemory();
    });
  }

  async rpc(method, params = {}) {
    try {
      const res = await fetch('/api/rpc', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: Date.now(), method, params })
      });
      const data = await res.json();
      if (data.error) throw new Error(data.error.message || JSON.stringify(data.error));
      return data.result;
    } catch (err) {
      console.error(`RPC error on ${method}:`, err);
      return null;
    }
  }

  connectEventStream() {
    const es = new EventSource('/api/events');
    es.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data);
        this.handleServerEvent(msg);
      } catch (err) {
        console.error('SSE parse error:', err);
      }
    };
  }

  handleServerEvent(msg) {
    if (msg.type === 'agent_step') {
      this.appendTimelineStep(msg.data);
    } else if (msg.type === 'agent_confirmation') {
      this.showConfirmation(msg.data);
    } else if (msg.type === 'tab_navigated') {
      this.onTabNavigated(msg.data);
    } else if (msg.type === 'intelligence_delta') {
      this.appendIntelligenceDelta(msg.data);
    }
  }

  async fetchTabs() {
    const tabs = await this.rpc('tabs.list') || [
      { id: 'tab-1', url: 'https://example.com', title: 'Example Domain', profile: 'User', active: true }
    ];
    this.tabs = tabs;
    this.renderTabs();
  }

  renderTabs() {
    this.tabsList.innerHTML = '';
    this.tabs.forEach((tab) => {
      const el = document.createElement('div');
      el.className = `tab-item ${tab.active ? 'active' : ''} ${tab.profile === 'Agent' ? 'agent-profile' : ''}`;
      
      const badge = tab.profile === 'Agent' ? '<span class="tab-badge">Agent</span>' : '';
      const groupBadge = tab.group ? `<span class="tab-group-tag">${this.escapeHtml(tab.group)}</span>` : '';
      el.innerHTML = `
        ${badge}
        ${groupBadge}
        <span class="tab-title">${this.escapeHtml(tab.title || tab.url)}</span>
        <button class="tab-close" title="Close Tab">×</button>
      `;

      el.addEventListener('click', (e) => {
        if (!e.target.classList.contains('tab-close')) {
          this.switchTab(tab.id);
        }
      });

      el.querySelector('.tab-close').addEventListener('click', (e) => {
        e.stopPropagation();
        this.closeTab(tab.id);
      });

      this.tabsList.appendChild(el);

      if (tab.active) {
        this.activeTabId = tab.id;
        this.omnibox.value = tab.url;
        this.pageTitle.textContent = tab.title || tab.url;
        this.profileIndicator.textContent = `Profile: ${tab.profile}`;
        if (this.webFrame.src !== tab.url) {
          this.webFrame.src = tab.url;
        }
        this.analyzeSafety(tab.url);
      }
    });
  }

  async createTab(url, profile = 'User') {
    const newTab = await this.rpc('tabs.create', { url, profile }) || {
      id: `tab-${Date.now()}`,
      url,
      title: 'New Tab',
      profile,
      active: true,
      group: 'General'
    };
    this.tabs.forEach(t => t.active = false);
    this.tabs.push(newTab);
    this.renderTabs();
  }

  switchTab(tabId) {
    this.tabs.forEach(t => t.active = (t.id === tabId));
    this.renderTabs();
  }

  closeTab(tabId) {
    this.tabs = this.tabs.filter(t => t.id !== tabId);
    if (this.tabs.length === 0) {
      this.createTab('https://example.com');
    } else {
      if (!this.tabs.some(t => t.active)) {
        this.tabs[0].active = true;
      }
      this.renderTabs();
    }
  }

  navigate(url) {
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    if (activeTab) {
      activeTab.url = url;
      activeTab.title = url;
      this.renderTabs();
      this.webFrame.src = url;
      this.analyzeSafety(url);
      this.rpc('tabs.navigate', { tab_id: activeTab.id, url });
    }
  }

  openInternalPage(schemeUrl) {
    if (schemeUrl === 'browser://memory') {
      const memTab = document.querySelector('[data-tab="memory"]');
      if (memTab) memTab.click();
    } else if (schemeUrl === 'browser://ai') {
      const aiTab = document.querySelector('[data-tab="settings"]');
      if (aiTab) aiTab.click();
    } else if (schemeUrl === 'browser://safety') {
      const safetyTab = document.querySelector('[data-tab="safety"]');
      if (safetyTab) safetyTab.click();
    }
  }

  async analyzeSafety(url) {
    const safetyTag = document.getElementById('safety-tag');
    const safetyBadge = document.getElementById('safety-badge');
    const safetyDomain = document.getElementById('safety-domain-name');
    const phishingDetails = document.getElementById('phishing-details');
    const darkPatternsDetails = document.getElementById('dark-patterns-details');

    if (!url || url === 'about:blank' || !safetyTag) return;

    try {
      const res = await this.rpc('page.analyze_safety', { url, text: '' });
      if (!res) return;

      const { phishing, dark_patterns } = res;
      if (safetyDomain) safetyDomain.textContent = `Domain: ${phishing.domain}`;

      if (phishing.severity === 'Dangerous') {
        safetyTag.className = 'safety-tag dangerous';
        safetyTag.textContent = 'Phishing Risk ⚠';
        if (safetyBadge) {
          safetyBadge.textContent = '🚨 High Risk: Phishing / Spoofing Detected';
          safetyBadge.style.color = '#ef4444';
        }
        if (phishingDetails) {
          phishingDetails.innerHTML = phishing.reasons.map(r => `<div class="threat-alert">⚠ ${this.escapeHtml(r)}</div>`).join('');
        }
      } else if (phishing.severity === 'Suspicious') {
        safetyTag.className = 'safety-tag suspicious';
        safetyTag.textContent = 'Suspicious ⚠';
        if (safetyBadge) {
          safetyBadge.textContent = '⚠ Warning: Suspicious Domain Pattern';
          safetyBadge.style.color = '#f59e0b';
        }
        if (phishingDetails) {
          phishingDetails.innerHTML = phishing.reasons.map(r => `<div class="warning-alert">⚠ ${this.escapeHtml(r)}</div>`).join('');
        }
      } else {
        safetyTag.className = 'safety-tag safe';
        safetyTag.textContent = 'Safe 🛡️';
        if (safetyBadge) {
          safetyBadge.textContent = '🛡️ Domain Security Verified';
          safetyBadge.style.color = '#10b981';
        }
        if (phishingDetails) {
          phishingDetails.innerHTML = '<span class="safe-tag">✔ No homograph attacks or brand spoofing detected</span>';
        }
      }

      if (darkPatternsDetails) {
        if (dark_patterns && dark_patterns.length > 0) {
          darkPatternsDetails.innerHTML = dark_patterns.map(dp => `
            <div class="dark-pattern-item">
              <strong>${this.escapeHtml(dp.title)}:</strong> "${this.escapeHtml(dp.snippet)}"
              <small>${this.escapeHtml(dp.explanation)}</small>
            </div>
          `).join('');
        } else {
          darkPatternsDetails.innerHTML = '<p class="empty-hint">✔ No deceptive urgency, countdowns, or concealed charges found.</p>';
        }
      }
    } catch (e) {
      console.warn('Safety analysis error:', e);
    }
  }

  async triggerPageAction(action, query = '') {
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    if (!activeTab) return;

    this.intelligenceOutput.innerHTML += `
      <div class="chat-message user"><strong>You:</strong> ${action} ${query}</div>
      <div class="chat-message bot"><em>Thinking and analyzing page...</em></div>
    `;
    this.intelligenceOutput.scrollTop = this.intelligenceOutput.scrollHeight;

    const res = await this.rpc('page.execute', {
      tab_id: activeTab.id,
      url: activeTab.url,
      action,
      query
    });

    if (res && res.answer) {
      // Remove temporary thinking message
      const msgs = this.intelligenceOutput.querySelectorAll('.chat-message.bot');
      if (msgs.length > 0) msgs[msgs.length - 1].remove();

      let formatted = this.escapeHtml(res.answer);
      // Highlight citations
      formatted = formatted.replace(/\[(c\d+)\]/g, '<span class="citation-tag">[$1]</span>');

      let citationsHtml = '';
      if (res.citations && res.citations.verified && res.citations.verified.length > 0) {
        citationsHtml = '<div class="citations-box"><strong>Verified Sources:</strong><br>';
        res.citations.verified.forEach(c => {
          citationsHtml += `[${c.obs_id}] "${this.escapeHtml(c.snippet)}" <em>(${c.heading_path || 'Page'})</em><br>`;
        });
        citationsHtml += '</div>';
      }

      this.intelligenceOutput.innerHTML += `
        <div class="chat-message bot">
          <div>${formatted}</div>
          ${citationsHtml}
        </div>
      `;
      this.intelligenceOutput.scrollTop = this.intelligenceOutput.scrollHeight;
    }
  }

  async startAgentTask() {
    const task = this.agentTaskInput.value.trim();
    if (!task) return;

    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    const dryRun = this.agentDryRun.checked;

    this.btnStartAgent.disabled = true;
    this.btnStopAgent.disabled = false;
    this.timelineFeed.innerHTML = '<div class="timeline-step">Starting agent session...</div>';

    await this.rpc('agent.start', {
      url: activeTab ? activeTab.url : 'https://example.com',
      task,
      dry_run: dryRun
    });
  }

  async stopAgentTask() {
    await this.rpc('agent.stop');
    this.btnStartAgent.disabled = false;
    this.btnStopAgent.disabled = true;
    this.timelineFeed.innerHTML += '<div class="timeline-step rejected">Agent task stopped by user.</div>';
  }

  appendTimelineStep(step) {
    const el = document.createElement('div');
    el.className = `timeline-step ${step.outcome_class || ''}`;
    el.innerHTML = `
      <strong>Step ${step.step_num}:</strong> ${this.escapeHtml(step.action)}<br>
      <small style="color:var(--text-muted)">Policy: ${step.policy_rule || 'allow'} | Critic: ${step.critic_verdict || 'allow'}</small>
    `;
    this.timelineFeed.appendChild(el);
    this.timelineFeed.scrollTop = this.timelineFeed.scrollHeight;

    if (step.terminal) {
      this.btnStartAgent.disabled = false;
      this.btnStopAgent.disabled = true;
    }
  }

  showConfirmation(data) {
    this.pendingConfirmation = data;
    this.confPreview.textContent = data.preview || 'Action confirmation needed';
    this.confReason.textContent = data.reason || '';
    this.confirmationCard.classList.remove('hidden');
  }

  async respondConfirmation(answer) {
    this.confirmationCard.classList.add('hidden');
    if (this.pendingConfirmation) {
      await this.rpc('agent.confirm', {
        session_id: this.pendingConfirmation.session_id,
        answer
      });
      this.pendingConfirmation = null;
    }
  }

  async fetchMemoryStats() {
    const stats = await this.rpc('memory.stats');
    if (stats) {
      this.memoryStats.textContent = `Memory: ${stats.pages || 0} pages, ${stats.chunks || 0} chunks indexed.`;
    }
  }

  async searchMemory() {
    const q = this.memoryInput.value.trim();
    if (!q) return;

    this.memoryResults.innerHTML = '<div style="color:var(--text-dim);padding:8px">Searching memory...</div>';
    const hits = await this.rpc('memory.search', { query: q }) || [];

    if (hits.length === 0) {
      this.memoryResults.innerHTML = '<div style="color:var(--text-dim);padding:8px">No matching memory items found.</div>';
      return;
    }

    this.memoryResults.innerHTML = '';
    hits.forEach((hit) => {
      const card = document.createElement('div');
      card.className = 'memory-hit-card';
      card.innerHTML = `
        <div class="memory-hit-header">${this.escapeHtml(hit.title || 'Untitled')} (Score: ${(hit.score || 0).toFixed(2)})</div>
        <div class="memory-hit-url">${this.escapeHtml(hit.url)} &bull; ${this.escapeHtml(hit.heading_path || 'Page')}</div>
        <div>"${this.escapeHtml(hit.text)}"</div>
      `;
      this.memoryResults.appendChild(card);
    });
  }

  escapeHtml(str) {
    if (!str) return '';
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }
}

// Start browser shell client on load
window.addEventListener('DOMContentLoaded', () => {
  window.browseShell = new BrowseShell();
});
