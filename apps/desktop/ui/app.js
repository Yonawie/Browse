// Browse — AI-Native Browser Shell Client (WebUI)
// Communicates with Core over JSON-RPC (/api/rpc) and SSE (/api/events)

class BrowseShell {
  constructor() {
    this.tabs = [];
    this.activeTabId = null;
    this.pendingConfirmation = null;

    this.focusMode = false;
    this.currentSelection = null;

    this.initElements();
    this.initEvents();
    this.connectEventStream();
    this.fetchTabs();
    this.fetchMemoryStats();
    this.checkPruneCandidates();
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

    // Focus Mode (AT-4) & Selection Context (IN-4)
    this.btnToggleFocus = document.getElementById('btn-toggle-focus');
    this.selectionBar = document.getElementById('selection-bar');
    this.selectionPreview = document.getElementById('selection-text-preview');
    this.btnAskSelection = document.getElementById('btn-ask-selection');
    this.btnSummarizeSelection = document.getElementById('btn-summarize-selection');
    this.btnClearSelection = document.getElementById('btn-clear-selection');

    // Tab Hygiene (AT-2)
    this.tabPruneBadge = document.getElementById('tab-prune-badge');
    this.pruneCount = document.getElementById('prune-count');
    this.pruneModal = document.getElementById('prune-modal');
    this.pruneCandidatesContainer = document.getElementById('prune-candidates-container');
    this.btnDismissPrune = document.getElementById('btn-dismiss-prune');
    this.btnArchiveAllPrune = document.getElementById('btn-archive-all-prune');
    this.toastBanner = document.getElementById('toast-banner');

    // Sidebar navigation
    this.sidebarTabs = document.querySelectorAll('.sidebar-tab');
    this.sidebarContents = document.querySelectorAll('.sidebar-content');

    // Intelligence
    this.chipSummarize = document.getElementById('chip-summarize');
    this.chipKeyfacts = document.getElementById('chip-keypoints');
    this.chipDiff = document.getElementById('chip-diff');
    this.chipTranslate = document.getElementById('chip-translate');
    this.intelligenceOutput = document.getElementById('intelligence-output');
    this.askInput = document.getElementById('ask-input');
    this.btnSendAsk = document.getElementById('btn-send-ask');

    // Agent
    this.agentTaskInput = document.getElementById('agent-task-input');
    this.agentSkillSelect = document.getElementById('agent-skill-select');
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
    this.kgTags = document.getElementById('knowledge-graph-tags');
    this.btnRefreshKg = document.getElementById('btn-refresh-kg');
  }

  initEvents() {
    this.newTabBtn.addEventListener('click', () => this.createTab('https://example.com'));

    // Theme toggle & persistence
    const themeBtn = document.getElementById('btn-toggle-theme');
    const savedTheme = localStorage.getItem('browse_theme') || 'dark';
    if (savedTheme === 'light') {
      document.body.classList.add('light-theme');
      if (themeBtn) themeBtn.textContent = '☀️';
    }
    if (themeBtn) {
      themeBtn.addEventListener('click', () => {
        const isLight = document.body.classList.toggle('light-theme');
        themeBtn.textContent = isLight ? '☀️' : '🌙';
        localStorage.setItem('browse_theme', isLight ? 'light' : 'dark');
      });
    }

    // Tab Hygiene interactions
    if (this.tabPruneBadge) {
      this.tabPruneBadge.addEventListener('click', () => {
        if (this.pruneModal) this.pruneModal.classList.remove('hidden');
      });
    }
    if (this.btnDismissPrune) {
      this.btnDismissPrune.addEventListener('click', () => {
        if (this.pruneModal) this.pruneModal.classList.add('hidden');
      });
    }
    if (this.btnArchiveAllPrune) {
      this.btnArchiveAllPrune.addEventListener('click', async () => {
        const res = await this.rpc('tabs.archive', {});
        if (this.pruneModal) this.pruneModal.classList.add('hidden');
        if (res) {
          this.showToast(`Archived ${res.archived_count || 0} stale tab(s) to memory`);
        }
        await this.fetchTabs();
        await this.checkPruneCandidates();
      });
    }

    // Focus Mode (AT-4)
    if (this.btnToggleFocus) {
      this.btnToggleFocus.addEventListener('click', () => this.toggleFocusMode());
    }

    // Selection Context (IN-4)
    if (this.btnAskSelection) {
      this.btnAskSelection.addEventListener('click', () => {
        if (this.currentSelection) {
          this.sidebarPane.classList.remove('collapsed');
          const intelTab = document.querySelector('.sidebar-tab[data-tab="intelligence"]');
          if (intelTab) intelTab.click();
          this.askInput.value = `Regarding "${this.currentSelection}": `;
          this.askInput.focus();
        }
      });
    }
    if (this.btnSummarizeSelection) {
      this.btnSummarizeSelection.addEventListener('click', () => {
        if (this.currentSelection) {
          this.sidebarPane.classList.remove('collapsed');
          const intelTab = document.querySelector('.sidebar-tab[data-tab="intelligence"]');
          if (intelTab) intelTab.click();
          this.triggerPageAction('ask', `Summarize this specific text: "${this.currentSelection}"`);
        }
      });
    }
    if (this.btnClearSelection) {
      this.btnClearSelection.addEventListener('click', () => {
        this.clearSelection();
      });
    }

    // Listen for text selection changes
    document.addEventListener('selectionchange', () => {
      const sel = window.getSelection().toString().trim();
      if (sel && sel.length > 3) {
        this.setSelection(sel);
      }
    });
    window.addEventListener('message', (e) => {
      if (e.data && e.data.type === 'page_selection') {
        this.setSelection(e.data.text);
      }
    });


    // First-run onboarding wizard
    const onboardingModal = document.getElementById('onboarding-modal');
    const finishOnboardingBtn = document.getElementById('btn-finish-onboarding');
    const hasOnboarded = localStorage.getItem('browse_onboarded');
    if (!hasOnboarded && onboardingModal) {
      onboardingModal.classList.remove('hidden');
    }
    if (finishOnboardingBtn) {
      finishOnboardingBtn.addEventListener('click', () => {
        localStorage.setItem('browse_onboarded', 'true');
        if (onboardingModal) onboardingModal.classList.add('hidden');
      });
    }

    // Global keyboard shortcuts (Phase S9)
    window.addEventListener('keydown', (e) => {
      const isCmd = e.ctrlKey || e.metaKey;

      if (isCmd && e.key.toLowerCase() === 't') {
        e.preventDefault();
        this.createTab('https://example.com');
      } else if (isCmd && e.key.toLowerCase() === 'w') {
        e.preventDefault();
        if (this.activeTabId) this.closeTab(this.activeTabId);
      } else if (isCmd && e.key.toLowerCase() === 'l') {
        e.preventDefault();
        this.omnibox.focus();
        this.omnibox.select();
      } else if (isCmd && e.key.toLowerCase() === 'b') {
        e.preventDefault();
        this.sidebarPane.classList.toggle('collapsed');
      } else if (isCmd && e.key >= '1' && e.key <= '9') {
        const idx = parseInt(e.key, 10) - 1;
        if (idx < this.tabs.length) {
          e.preventDefault();
          this.switchTab(this.tabs[idx].id);
        }
      } else if (isCmd && e.key === 'Tab') {
        e.preventDefault();
        const curIdx = this.tabs.findIndex(t => t.id === this.activeTabId);
        if (curIdx !== -1 && this.tabs.length > 1) {
          const nextIdx = (curIdx + (e.shiftKey ? this.tabs.length - 1 : 1)) % this.tabs.length;
          this.switchTab(this.tabs[nextIdx].id);
        }
      } else if (e.key === 'Escape') {
        if (!this.confirmationCard.classList.contains('hidden')) {
          this.respondConfirmation('rejected');
        } else if (this.pruneModal && !this.pruneModal.classList.contains('hidden')) {
          this.pruneModal.classList.add('hidden');
        } else if (onboardingModal && !onboardingModal.classList.contains('hidden')) {
          onboardingModal.classList.add('hidden');
          localStorage.setItem('browse_onboarded', 'true');
        } else {
          this.omnibox.blur();
        }
      }
    });
    
    this.omnibox.addEventListener('keydown', async (e) => {
      if (e.key === 'Enter') {
        const val = this.omnibox.value.trim();
        if (!val) return;

        const classification = await this.rpc('omnibox.classify', { input: val });
        const kind = classification ? classification.kind : 'search';
        const query = classification ? classification.query : val;

        if (kind === 'url') {
          this.navigate(query);
        } else if (val.startsWith('browser://')) {
          this.openInternalPage(val);
        } else if (kind === 'tab_command') {
          const res = await this.rpc('tabs.execute_command', { command: val });
          if (res) {
            if (res.action === 'closed') {
              this.showToast(`Closed ${res.count} tab(s) matching "${res.topic || val}"`);
            } else if (res.action === 'grouped') {
              this.showToast(`Organized ${res.count} tabs into task clusters`);
            } else if (res.action === 'closed_stale') {
              this.showToast(`Closed ${res.count} inactive tab(s)`);
            } else if (res.action === 'archived_stale') {
              this.showToast(`Archived ${res.count} stale tab(s) to local memory`);
            } else {
              this.showToast(`Executed: ${val}`);
            }
            await this.fetchTabs();
            await this.checkPruneCandidates();
          }
        } else if (kind === 'agent_task') {
          // Switch to Agent tab, prefill task, open sidebar
          this.sidebarPane.classList.remove('collapsed');
          const agentTabBtn = document.querySelector('.sidebar-tab[data-tab="agent"]');
          if (agentTabBtn) agentTabBtn.click();
          this.agentTaskInput.value = query;
        } else if (kind === 'memory_query') {
          // Switch to Memory tab and trigger search
          this.sidebarPane.classList.remove('collapsed');
          const memoryTabBtn = document.querySelector('.sidebar-tab[data-tab="memory"]');
          if (memoryTabBtn) memoryTabBtn.click();
          this.memoryInput.value = query;
          this.searchMemory();
        } else {
          // Default: Search via DuckDuckGo
          this.navigate('https://duckduckgo.com/?q=' + encodeURIComponent(query));
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
    if (this.chipDiff) this.chipDiff.addEventListener('click', () => this.triggerPageDiff());
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
    if (this.agentSkillSelect) {
      this.agentSkillSelect.addEventListener('change', () => this.onSkillSelected());
    }

    // Confirmation card
    this.btnConfApprove.addEventListener('click', () => this.respondConfirmation('approved'));
    this.btnConfSession.addEventListener('click', () => this.respondConfirmation('approved_for_session'));
    this.btnConfReject.addEventListener('click', () => this.respondConfirmation('rejected'));

    // Memory search
    this.btnMemorySearch.addEventListener('click', () => this.searchMemory());
    this.memoryInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') this.searchMemory();
    });
    if (this.btnRefreshKg) {
      this.btnRefreshKg.addEventListener('click', () => this.loadKnowledgeGraph());
    }

    // Load available skills
    this.fetchSkills();
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
    const tabs = await this.rpc('tabs.list', { focused_only: this.focusMode }) || [
      { id: 'tab-1', url: 'https://example.com', title: 'Example Domain', profile: 'User', active: true }
    ];
    this.tabs = tabs;
    this.renderTabs();
  }

  async toggleFocusMode() {
    const res = await this.rpc('focus.toggle', {});
    if (res) {
      this.focusMode = res.focus_mode;
      if (this.btnToggleFocus) {
        this.btnToggleFocus.classList.toggle('active', this.focusMode);
      }
      this.showToast(this.focusMode ? '🎯 Focus Mode ON: Filtering tabs for active task' : '🎯 Focus Mode OFF: All tabs visible');
      await this.fetchTabs();
    }
  }

  setSelection(text) {
    if (!text) return;
    this.currentSelection = text;
    if (this.selectionBar && this.selectionPreview) {
      const preview = text.length > 60 ? text.substring(0, 57) + '...' : text;
      this.selectionPreview.textContent = preview;
      this.selectionBar.classList.remove('hidden');
    }
    this.rpc('page.selection.set', { text });
  }

  clearSelection() {
    this.currentSelection = null;
    if (this.selectionBar) {
      this.selectionBar.classList.add('hidden');
    }
    this.rpc('page.selection.set', { text: null });
  }

  renderTabs() {
    this.tabsList.innerHTML = '';
    this.tabs.forEach((tab) => {
      const el = document.createElement('div');
      el.className = `tab-item ${tab.active ? 'active' : ''} ${tab.profile === 'Agent' ? 'agent-profile' : ''}`;
      
      const badge = tab.profile === 'Agent' ? '<span class="tab-badge">Agent</span>' : '';
      const groupBadge = tab.group ? `<span class="tab-group-tag">${this.escapeHtml(tab.group)}</span>` : '';
      const taskBadge = tab.task_id ? `<span class="tab-task-tag">🎯 ${this.escapeHtml(tab.task_id)}</span>` : '';
      el.innerHTML = `
        ${badge}
        ${groupBadge}
        ${taskBadge}
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
    this.rpc('tabs.switch', { tab_id: tabId });
  }

  closeTab(tabId) {
    this.rpc('tabs.close', { tab_id: tabId });
    this.tabs = this.tabs.filter(t => t.id !== tabId);
    if (this.tabs.length === 0) {
      this.createTab('https://example.com');
    } else {
      if (!this.tabs.some(t => t.active)) {
        this.tabs[0].active = true;
      }
      this.renderTabs();
    }
    this.checkPruneCandidates();
  }

  showToast(msg) {
    if (!this.toastBanner) return;
    this.toastBanner.textContent = msg;
    this.toastBanner.classList.remove('hidden');
    clearTimeout(this.toastTimeout);
    this.toastTimeout = setTimeout(() => {
      this.toastBanner.classList.add('hidden');
    }, 3500);
  }

  async checkPruneCandidates() {
    const candidates = await this.rpc('tabs.suggest_pruning', {}) || [];
    if (!this.tabPruneBadge) return;
    if (candidates.length > 0) {
      if (this.pruneCount) this.pruneCount.textContent = candidates.length;
      this.tabPruneBadge.classList.remove('hidden');
      if (this.pruneCandidatesContainer) {
        this.pruneCandidatesContainer.innerHTML = '';
        candidates.forEach(cand => {
          const item = document.createElement('div');
          item.className = 'prune-candidate-item';
          const indexedBadge = cand.is_indexed ? '<span class="indexed-pill">Indexed in Memory</span>' : '';
          item.innerHTML = `
            <div class="prune-cand-info">
              <span class="prune-cand-title">${this.escapeHtml(cand.title || cand.url)}</span>
              <span class="prune-cand-meta">${this.escapeHtml(cand.reason)}</span>
            </div>
            <div style="display:flex; align-items:center; gap:8px;">
              ${indexedBadge}
              <button class="btn-secondary btn-archive-single" data-id="${cand.tab_id}" style="padding:4px 8px; font-size:11px;">Archive</button>
            </div>
          `;
          item.querySelector('.btn-archive-single').addEventListener('click', async () => {
            await this.rpc('tabs.archive', { tab_ids: [cand.tab_id] });
            this.showToast('Archived tab to memory');
            await this.fetchTabs();
            await this.checkPruneCandidates();
          });
          this.pruneCandidatesContainer.appendChild(item);
        });
      }
    } else {
      this.tabPruneBadge.classList.add('hidden');
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

  async triggerPageDiff() {
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    if (!activeTab) return;

    this.intelligenceOutput.innerHTML += `
      <div class="chat-message user"><strong>You:</strong> What changed on this page since my last visit?</div>
      <div class="chat-message bot"><em>Analyzing historical snapshots from memory...</em></div>
    `;
    this.intelligenceOutput.scrollTop = this.intelligenceOutput.scrollHeight;

    const res = await this.rpc('page.diff', { url: activeTab.url });

    // Remove thinking message
    const msgs = this.intelligenceOutput.querySelectorAll('.chat-message.bot');
    if (msgs.length > 0) msgs[msgs.length - 1].remove();

    if (res) {
      let diffHtml = '';
      if (!res.has_changes) {
        diffHtml = `<div style="color:var(--success);padding:4px 0;">✔ No changes detected on this page since your previous visit.</div>`;
      } else {
        diffHtml = `<div style="margin-bottom:6px;"><strong>⚡ Page Diff Summary:</strong> ${this.escapeHtml(res.summary)}</div>`;
        if (res.items && res.items.length > 0) {
          diffHtml += '<div class="diff-items-container" style="display:flex;flex-direction:column;gap:6px;font-size:12px;">';
          res.items.forEach(item => {
            if (item.kind === 'Modified') {
              diffHtml += `<div style="border-left:3px solid var(--warning);padding-left:8px;"><span style="color:var(--warning);font-weight:600;">Modified:</span> <del style="opacity:0.6">${this.escapeHtml(item.old_text)}</del> &rarr; <ins style="color:var(--text-main);background:rgba(234,179,8,0.1);">${this.escapeHtml(item.new_text)}</ins></div>`;
            } else if (item.kind === 'Added') {
              diffHtml += `<div style="border-left:3px solid var(--success);padding-left:8px;"><span style="color:var(--success);font-weight:600;">Added:</span> <ins style="color:var(--text-main);background:rgba(34,197,94,0.1);">${this.escapeHtml(item.new_text)}</ins></div>`;
            } else if (item.kind === 'Removed') {
              diffHtml += `<div style="border-left:3px solid var(--danger);padding-left:8px;"><span style="color:var(--danger);font-weight:600;">Removed:</span> <del style="opacity:0.7">${this.escapeHtml(item.old_text)}</del></div>`;
            }
          });
          diffHtml += '</div>';
        }
      }

      this.intelligenceOutput.innerHTML += `
        <div class="chat-message bot">
          ${diffHtml}
        </div>
      `;
      this.intelligenceOutput.scrollTop = this.intelligenceOutput.scrollHeight;
    }
  }

  async fetchSkills() {
    if (!this.agentSkillSelect) return;
    const skills = await this.rpc('skills.list') || [];
    this.skills = skills;
    skills.forEach(s => {
      const opt = document.createElement('option');
      opt.value = s.id;
      opt.textContent = `✦ ${s.name} (${s.category})`;
      this.agentSkillSelect.appendChild(opt);
    });
  }

  onSkillSelected() {
    const skillId = this.agentSkillSelect.value;
    if (!skillId) return;
    const skill = (this.skills || []).find(s => s.id === skillId);
    if (skill) {
      this.agentTaskInput.value = skill.prompt_template;
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
    this.loadKnowledgeGraph();
  }

  async loadKnowledgeGraph() {
    if (!this.kgTags) return;
    const entities = await this.rpc('memory.entities', { limit: 15 }) || [];
    if (entities.length === 0) {
      this.kgTags.innerHTML = '<span class="empty-hint" style="font-size:0.8rem;color:var(--text-dim);">No entities indexed yet. Browse documentation or repositories to build the graph.</span>';
      return;
    }
    this.kgTags.innerHTML = entities.map(e => `
      <span class="entity-tag" style="display:inline-flex;align-items:center;gap:4px;padding:3px 8px;border-radius:12px;font-size:0.75rem;background:var(--bg-card);border:1px solid var(--border-color);color:var(--text-primary);cursor:pointer;" title="Type: ${this.escapeHtml(e.kind)} (${e.mentions} mentions)">
        <span style="opacity:0.6">${e.kind === 'repo' ? '📦' : e.kind === 'topic' ? '🏷️' : '✦'}</span>
        <strong>${this.escapeHtml(e.name)}</strong>
        <small style="opacity:0.5">${e.mentions}</small>
      </span>
    `).join('');

    // Clicking an entity tag searches memory
    this.kgTags.querySelectorAll('.entity-tag').forEach(tag => {
      tag.addEventListener('click', () => {
        const strong = tag.querySelector('strong');
        if (strong) {
          this.memoryInput.value = strong.textContent;
          this.searchMemory();
        }
      });
    });
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
