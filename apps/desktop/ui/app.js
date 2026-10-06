// Browse — AI-Native Browser Shell Client (WebUI)
// Communicates with Core over JSON-RPC (/api/rpc) and SSE (/api/events)

class BrowseShell {
  constructor() {
    this.tabs = [];
    this.activeTabId = null;
    this.pendingConfirmation = null;

    this.focusMode = false;
    this.currentSelection = null;
    this.recordedActions = [];
    this.playwrightLang = 'typescript';

    this.initElements();
    this.initEvents();
    this.connectEventStream();
    this.fetchTabs();
    this.fetchBookmarks();
    this.fetchHistory();
    this.fetchDownloads();
    this.fetchVaultCredentials();
    this.fetchModelCatalog();
    this.fetchMemoryStats();
    this.checkPruneCandidates();
    this.updateProfileIndicator();
    this.checkCrashRecovery();
    this.fetchPromptTemplates();
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
    this.btnExportMemory = document.getElementById('btn-export-memory');
    this.btnImportMemory = document.getElementById('btn-import-memory');
    this.memoryImportFile = document.getElementById('memory-import-file');

    // Privacy (D-5)
    this.privacyGrade = document.getElementById('privacy-grade-badge');
    this.privacyDetails = document.getElementById('privacy-details');

    // Auto-Group (AT-1)
    this.btnAutoGroup = document.getElementById('btn-auto-group');

    // DevTools Explainer (D-1)
    this.btnExplainDiag = document.getElementById('btn-explain-diag');
    this.diagInput = document.getElementById('diag-input');
    this.diagCard = document.getElementById('diag-explanation-card');
    this.diagTitle = document.getElementById('diag-title');
    this.diagSummary = document.getElementById('diag-summary');
    this.diagCause = document.getElementById('diag-cause');
    this.diagFix = document.getElementById('diag-fix');

    // Bookmarks & AdBlock Shield
    this.btnAddBookmark = document.getElementById('btn-add-bookmark');
    this.btnShield = document.getElementById('btn-adblock-shield');
    this.bookmarksList = document.getElementById('bookmarks-list');

    // History & Models
    this.historySearch = document.getElementById('history-search-input');
    this.btnClearHistory = document.getElementById('btn-clear-history');
    this.historyList = document.getElementById('history-list');
    this.modelCatalogList = document.getElementById('model-catalog-list');

    // Distraction-Free Reader Mode
    this.btnReaderMode = document.getElementById('btn-reader-mode');
    this.readerPane = document.getElementById('reader-pane');
    this.readerBody = document.getElementById('reader-article-body');
    this.btnCloseReader = document.getElementById('btn-close-reader');
    this.btnReaderFontDec = document.getElementById('btn-reader-font-dec');
    this.btnReaderFontInc = document.getElementById('btn-reader-font-inc');
    this.readerFontSize = 17;

    // Site Data Cleaner
    this.btnForgetSite = document.getElementById('btn-forget-site');

    // Downloads Manager
    this.btnToggleDownloads = document.getElementById('btn-toggle-downloads');
    this.downloadsList = document.getElementById('downloads-list');
    this.btnClearDownloads = document.getElementById('btn-clear-downloads');

    // Playwright export (D-2)
    this.btnExportPlaywright = document.getElementById('btn-export-playwright');
    this.playwrightModal = document.getElementById('playwright-modal');
    this.playwrightCodePreview = document.getElementById('playwright-code-preview');
    this.btnLangTs = document.getElementById('btn-lang-ts');
    this.btnLangPy = document.getElementById('btn-lang-py');
    this.btnCopyPlaywright = document.getElementById('btn-copy-playwright');
    this.btnDismissPlaywright = document.getElementById('btn-dismiss-playwright');

    // In-Page Find Bar (Ctrl+F)
    this.findBar = document.getElementById('find-bar');
    this.findInput = document.getElementById('find-input');
    this.findCounter = document.getElementById('find-counter');
    this.btnFindPrev = document.getElementById('btn-find-prev');
    this.btnFindNext = document.getElementById('btn-find-next');
    this.btnFindCase = document.getElementById('btn-find-case');
    this.btnFindClose = document.getElementById('btn-find-close');
    this.findMatches = [];
    this.currentFindIndex = -1;
    this.findMatchCase = false;

    // Cookie & Storage Inspector
    this.btnInspectCookies = document.getElementById('btn-inspect-cookies');
    this.btnClearStorage = document.getElementById('btn-clear-storage');
    this.cookiesDetails = document.getElementById('cookies-audit-details');
    this.cookiesBreakdown = document.getElementById('cookies-breakdown');
    this.cookiesList = document.getElementById('cookies-list');

    // Password & Credentials Vault
    this.btnSaveCred = document.getElementById('btn-save-cred');
    this.vaultCredsList = document.getElementById('vault-creds-list');

    // Command Palette (Ctrl+K / Ctrl+P)
    this.btnPalette = document.getElementById('btn-palette');
    this.paletteModal = document.getElementById('palette-modal');
    this.paletteInput = document.getElementById('palette-input');
    this.paletteResults = document.getElementById('palette-results');
    this.paletteItems = [];
    this.selectedPaletteIndex = 0;

    // Profiles Manager
    this.profileIndicator = document.getElementById('profile-indicator');
    this.profileModal = document.getElementById('profile-modal');
    this.btnDismissProfile = document.getElementById('btn-dismiss-profile');
    this.btnCreateProfile = document.getElementById('btn-create-profile');
    this.newProfileName = document.getElementById('new-profile-name');
    this.newProfileKind = document.getElementById('new-profile-kind');
    this.profilesList = document.getElementById('profiles-list');
    this.currentProfile = localStorage.getItem('browse_active_profile') || 'default';

    // Search Engine Configuration
    this.settingSearchEngine = document.getElementById('setting-search-engine');

    // Site Security & Permissions
    this.securityIndicator = document.getElementById('security-indicator');
    this.securityModal = document.getElementById('security-modal');
    this.secModalOrigin = document.getElementById('sec-modal-origin');
    this.secModalConn = document.getElementById('sec-modal-conn');
    this.secPermissionsList = document.getElementById('sec-permissions-list');
    this.btnSecClose = document.getElementById('btn-sec-close');
    this.btnSecClearData = document.getElementById('btn-sec-clear-data');
    this.btnSecAuditCookies = document.getElementById('btn-sec-audit-cookies');

    // Page Zoom
    this.zoomLevelBadge = document.getElementById('zoom-level-badge');
    this.zoomLevel = 1.0;

    // Saved Sessions & Workspaces
    this.btnSessions = document.getElementById('btn-sessions');
    this.sessionsModal = document.getElementById('sessions-modal');
    this.newSessionName = document.getElementById('new-session-name');
    this.btnSaveSession = document.getElementById('btn-save-current-session');
    this.btnDismissSessions = document.getElementById('btn-dismiss-sessions');
    this.savedSessionsList = document.getElementById('saved-sessions-list');
    this.crashRecoveryAlert = document.getElementById('crash-recovery-alert');
    this.btnRestoreCrashTabs = document.getElementById('btn-restore-crash-tabs');

    // Prompt Templates & Tab Dedup
    this.promptTemplatesBar = document.getElementById('prompt-templates-bar');
    this.btnDedupPrune = document.getElementById('btn-dedup-prune');

    // User Scripts & Custom Styles
    this.userscriptsModal = document.getElementById('userscripts-modal');
    this.newScriptName = document.getElementById('new-script-name');
    this.newScriptDomain = document.getElementById('new-script-domain');
    this.newScriptType = document.getElementById('new-script-type');
    this.newScriptContent = document.getElementById('new-script-content');
    this.btnSaveUserscript = document.getElementById('btn-save-userscript');
    this.btnDismissUserscripts = document.getElementById('btn-dismiss-userscripts');
    this.userscriptsList = document.getElementById('userscripts-list');

    // Reading List & Network Inspector
    this.btnReadingList = document.getElementById('btn-reading-list');
    this.readingListModal = document.getElementById('reading-list-modal');
    this.btnDismissReadingList = document.getElementById('btn-dismiss-reading-list');
    this.btnAddCurrentReading = document.getElementById('btn-add-current-to-reading');
    this.readingListItems = document.getElementById('reading-list-items');
    this.btnReadingFilterAll = document.getElementById('btn-reading-filter-all');
    this.btnReadingFilterUnread = document.getElementById('btn-reading-filter-unread');
    this.readingListUnreadOnly = false;

    this.btnNetworkMonitor = document.getElementById('btn-network-monitor');
    this.networkModal = document.getElementById('network-modal');
    this.btnDismissNetwork = document.getElementById('btn-dismiss-network');
    this.btnNetworkFilterBlocked = document.getElementById('btn-network-filter-blocked');
    this.btnClearNetworkLog = document.getElementById('btn-clear-network-log');
    this.networkRequestsTbody = document.getElementById('network-requests-tbody');
    this.netTotalCount = document.getElementById('net-total-count');
    this.netBlockedCount = document.getElementById('net-blocked-count');
    this.netTotalBytes = document.getElementById('net-total-bytes');
    this.networkBlockedOnly = false;
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

    // Auto-Group Tabs (AT-1)
    if (this.btnAutoGroup) {
      this.btnAutoGroup.addEventListener('click', () => this.autoGroupTabs());
    }

    // DevTools Explainer (D-1)
    if (this.btnExplainDiag) {
      this.btnExplainDiag.addEventListener('click', () => this.explainDiagnostic());
    }
    if (this.diagInput) {
      this.diagInput.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') this.explainDiagnostic();
      });
    }

    // Bookmarks (⭐) & AdBlock Shield
    if (this.btnAddBookmark) {
      this.btnAddBookmark.addEventListener('click', () => this.bookmarkCurrentPage());
    }
    if (this.btnShield) {
      this.btnShield.addEventListener('click', () => this.toggleShield());
    }

    // History (🕒)
    if (this.historySearch) {
      this.historySearch.addEventListener('input', () => this.fetchHistory(this.historySearch.value.trim()));
    }
    if (this.btnClearHistory) {
      this.btnClearHistory.addEventListener('click', () => this.clearHistory());
    }

    // Reader Mode (📖)
    if (this.btnReaderMode) {
      this.btnReaderMode.addEventListener('click', () => this.toggleReaderMode());
    }
    if (this.btnCloseReader) {
      this.btnCloseReader.addEventListener('click', () => this.toggleReaderMode());
    }
    if (this.btnReaderFontDec) {
      this.btnReaderFontDec.addEventListener('click', () => {
        this.readerFontSize = Math.max(12, this.readerFontSize - 2);
        if (this.readerBody) this.readerBody.style.fontSize = `${this.readerFontSize}px`;
      });
    }
    if (this.btnReaderFontInc) {
      this.btnReaderFontInc.addEventListener('click', () => {
        this.readerFontSize = Math.min(32, this.readerFontSize + 2);
        if (this.readerBody) this.readerBody.style.fontSize = `${this.readerFontSize}px`;
      });
    }

    // Site Data Cleaner (🗑️)
    if (this.btnForgetSite) {
      this.btnForgetSite.addEventListener('click', () => this.forgetCurrentSite());
    }

    // Downloads Manager (📥)
    if (this.btnToggleDownloads) {
      this.btnToggleDownloads.addEventListener('click', () => this.showDownloadsTab());
    }
    if (this.btnClearDownloads) {
      this.btnClearDownloads.addEventListener('click', () => this.clearDownloads());
    }

    // In-Page Find Bar (Ctrl+F)
    if (this.findInput) {
      this.findInput.addEventListener('input', () => this.performFind());
      this.findInput.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          if (e.shiftKey) this.prevFindMatch();
          else this.nextFindMatch();
        } else if (e.key === 'Escape') {
          this.closeFind();
        }
      });
    }
    if (this.btnFindNext) this.btnFindNext.addEventListener('click', () => this.nextFindMatch());
    if (this.btnFindPrev) this.btnFindPrev.addEventListener('click', () => this.prevFindMatch());
    if (this.btnFindCase) {
      this.btnFindCase.addEventListener('click', () => {
        this.findMatchCase = !this.findMatchCase;
        this.btnFindCase.style.opacity = this.findMatchCase ? '1' : '0.6';
        this.performFind();
      });
    }
    if (this.btnFindClose) this.btnFindClose.addEventListener('click', () => this.closeFind());

    // Cookie & Storage Inspector
    if (this.btnInspectCookies) this.btnInspectCookies.addEventListener('click', () => this.inspectCookies());
    if (this.btnClearStorage) this.btnClearStorage.addEventListener('click', () => this.clearOriginStorage());

    // Password & Credentials Vault
    if (this.btnSaveCred) this.btnSaveCred.addEventListener('click', () => this.promptSaveCredential());

    // Command Palette (Ctrl+K / Ctrl+P)
    if (this.btnPalette) this.btnPalette.addEventListener('click', () => this.openCommandPalette());
    if (this.paletteInput) {
      this.paletteInput.addEventListener('input', () => this.searchCommandPalette());
      this.paletteInput.addEventListener('keydown', (e) => this.handlePaletteKeydown(e));
    }
    if (this.paletteModal) {
      this.paletteModal.addEventListener('click', (e) => {
        if (e.target === this.paletteModal) this.closeCommandPalette();
      });
    }

    // Profiles Management
    if (this.profileIndicator) this.profileIndicator.addEventListener('click', () => this.openProfileModal());
    if (this.btnDismissProfile) this.btnDismissProfile.addEventListener('click', () => this.closeProfileModal());
    if (this.btnCreateProfile) this.btnCreateProfile.addEventListener('click', () => this.createProfile());
    if (this.profileModal) {
      this.profileModal.addEventListener('click', (e) => {
        if (e.target === this.profileModal) this.closeProfileModal();
      });
    }

    // Search Engine Configuration
    if (this.settingSearchEngine) {
      const savedEngine = localStorage.getItem('browse_search_engine') || 'duckduckgo';
      this.settingSearchEngine.value = savedEngine;
      this.settingSearchEngine.addEventListener('change', () => {
        localStorage.setItem('browse_search_engine', this.settingSearchEngine.value);
        this.showToast(`Default search set to ${this.settingSearchEngine.options[this.settingSearchEngine.selectedIndex].text}`);
      });
    }

    // Site Security & Permissions
    if (this.securityIndicator) {
      this.securityIndicator.addEventListener('click', () => this.openSecurityModal());
    }
    if (this.btnSecClose) {
      this.btnSecClose.addEventListener('click', () => this.closeSecurityModal());
    }
    if (this.btnSecClearData) {
      this.btnSecClearData.addEventListener('click', () => {
        this.closeSecurityModal();
        this.forgetCurrentSite();
      });
    }
    if (this.btnSecAuditCookies) {
      this.btnSecAuditCookies.addEventListener('click', () => {
        this.closeSecurityModal();
        this.inspectCookies();
      });
    }
    if (this.securityModal) {
      this.securityModal.addEventListener('click', (e) => {
        if (e.target === this.securityModal) this.closeSecurityModal();
      });
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

    // Sessions modal interactions
    if (this.btnSessions) {
      this.btnSessions.addEventListener('click', () => this.openSessionsModal());
    }
    if (this.btnDismissSessions) {
      this.btnDismissSessions.addEventListener('click', () => this.closeSessionsModal());
    }
    if (this.btnSaveSession) {
      this.btnSaveSession.addEventListener('click', () => this.saveCurrentSession());
    }
    if (this.btnRestoreCrashTabs) {
      this.btnRestoreCrashTabs.addEventListener('click', () => this.restoreCrashSession());
    }

    if (this.btnDedupPrune) {
      this.btnDedupPrune.addEventListener('click', () => this.dedupTabs());
    }
    if (this.btnDismissUserscripts) {
      this.btnDismissUserscripts.addEventListener('click', () => this.closeUserScriptsModal());
    }
    if (this.btnSaveUserscript) {
      this.btnSaveUserscript.addEventListener('click', () => this.saveUserScript());
    }

    // Reading list event bindings
    if (this.btnReadingList) {
      this.btnReadingList.addEventListener('click', () => this.openReadingListModal());
    }
    if (this.btnDismissReadingList) {
      this.btnDismissReadingList.addEventListener('click', () => this.closeReadingListModal());
    }
    if (this.btnAddCurrentReading) {
      this.btnAddCurrentReading.addEventListener('click', () => this.addCurrentPageToReadingList());
    }
    if (this.btnReadingFilterAll) {
      this.btnReadingFilterAll.addEventListener('click', () => {
        this.readingListUnreadOnly = false;
        this.btnReadingFilterAll.classList.add('active');
        if (this.btnReadingFilterUnread) this.btnReadingFilterUnread.classList.remove('active');
        this.fetchReadingList();
      });
    }
    if (this.btnReadingFilterUnread) {
      this.btnReadingFilterUnread.addEventListener('click', () => {
        this.readingListUnreadOnly = true;
        this.btnReadingFilterUnread.classList.add('active');
        if (this.btnReadingFilterAll) this.btnReadingFilterAll.classList.remove('active');
        this.fetchReadingList();
      });
    }

    // Network monitor bindings
    if (this.btnNetworkMonitor) {
      this.btnNetworkMonitor.addEventListener('click', () => this.openNetworkModal());
    }
    if (this.btnDismissNetwork) {
      this.btnDismissNetwork.addEventListener('click', () => this.closeNetworkModal());
    }
    if (this.btnClearNetworkLog) {
      this.btnClearNetworkLog.addEventListener('click', () => this.clearNetworkLog());
    }
    if (this.btnNetworkFilterBlocked) {
      this.btnNetworkFilterBlocked.addEventListener('click', () => {
        this.networkBlockedOnly = !this.networkBlockedOnly;
        this.btnNetworkFilterBlocked.classList.toggle('active', this.networkBlockedOnly);
        this.fetchNetworkLog();
      });
    }

    // Zoom badge click
    if (this.zoomLevelBadge) {
      this.zoomLevelBadge.addEventListener('click', () => this.resetZoom());
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
      } else if (isCmd && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        this.openFind();
      } else if (isCmd && (e.key.toLowerCase() === 'k' || e.key.toLowerCase() === 'p')) {
        e.preventDefault();
        this.openCommandPalette();
      } else if (isCmd && e.key.toLowerCase() === 'm') {
        e.preventDefault();
        if (this.activeTabId) this.toggleMuteTab(this.activeTabId);
      } else if (isCmd && e.key.toLowerCase() === 's') {
        e.preventDefault();
        this.openSessionsModal();
      } else if (isCmd && (e.key === '=' || e.key === '+')) {
        e.preventDefault();
        this.zoomIn();
      } else if (isCmd && (e.key === '-' || e.key === '_')) {
        e.preventDefault();
        this.zoomOut();
      } else if (isCmd && e.key === '0') {
        e.preventDefault();
        this.resetZoom();
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
        } else if (this.paletteModal && !this.paletteModal.classList.contains('hidden')) {
          this.closeCommandPalette();
        } else if (this.sessionsModal && !this.sessionsModal.classList.contains('hidden')) {
          this.closeSessionsModal();
        } else if (this.userscriptsModal && !this.userscriptsModal.classList.contains('hidden')) {
          this.closeUserScriptsModal();
        } else if (this.readingListModal && !this.readingListModal.classList.contains('hidden')) {
          this.closeReadingListModal();
        } else if (this.networkModal && !this.networkModal.classList.contains('hidden')) {
          this.closeNetworkModal();
        } else if (this.profileModal && !this.profileModal.classList.contains('hidden')) {
          this.closeProfileModal();
        } else if (this.securityModal && !this.securityModal.classList.contains('hidden')) {
          this.closeSecurityModal();
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
          // Default: Search via configured search engine
          const engine = localStorage.getItem('browse_search_engine') || 'duckduckgo';
          let searchUrl = `https://duckduckgo.com/?q=${encodeURIComponent(query)}`;
          if (engine === 'google') searchUrl = `https://www.google.com/search?q=${encodeURIComponent(query)}`;
          else if (engine === 'brave') searchUrl = `https://search.brave.com/search?q=${encodeURIComponent(query)}`;
          else if (engine === 'kagi') searchUrl = `https://kagi.com/search?q=${encodeURIComponent(query)}`;
          else if (engine === 'startpage') searchUrl = `https://www.startpage.com/sp/search?query=${encodeURIComponent(query)}`;
          this.navigate(searchUrl);
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
    if (this.btnExportMemory) {
      this.btnExportMemory.addEventListener('click', () => this.exportMemory());
    }
    if (this.btnImportMemory && this.memoryImportFile) {
      this.btnImportMemory.addEventListener('click', () => this.memoryImportFile.click());
      this.memoryImportFile.addEventListener('change', (e) => this.handleMemoryFileImport(e));
    }

    // Playwright export (D-2)
    if (this.btnExportPlaywright) {
      this.btnExportPlaywright.addEventListener('click', () => this.showPlaywrightModal());
    }
    if (this.btnDismissPlaywright) {
      this.btnDismissPlaywright.addEventListener('click', () => {
        if (this.playwrightModal) this.playwrightModal.classList.add('hidden');
      });
    }
    if (this.btnLangTs) {
      this.btnLangTs.addEventListener('click', () => {
        this.playwrightLang = 'typescript';
        this.btnLangTs.classList.add('active');
        if (this.btnLangPy) this.btnLangPy.classList.remove('active');
        this.renderPlaywrightScript();
      });
    }
    if (this.btnLangPy) {
      this.btnLangPy.addEventListener('click', () => {
        this.playwrightLang = 'python';
        this.btnLangPy.classList.add('active');
        if (this.btnLangTs) this.btnLangTs.classList.remove('active');
        this.renderPlaywrightScript();
      });
    }
    if (this.btnCopyPlaywright) {
      this.btnCopyPlaywright.addEventListener('click', () => {
        if (this.playwrightCodePreview) {
          navigator.clipboard.writeText(this.playwrightCodePreview.textContent);
          this.showToast('📋 Playwright script copied to clipboard!');
        }
      });
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
      el.className = `tab-item ${tab.active ? 'active' : ''} ${tab.profile === 'Agent' ? 'agent-profile' : ''} ${tab.discarded ? 'sleeping' : ''}`;
      
      const badge = tab.profile === 'Agent' ? '<span class="tab-badge">Agent</span>' : '';
      const groupBadge = tab.group ? `<span class="tab-group-tag">${this.escapeHtml(tab.group)}</span>` : '';
      const taskBadge = tab.task_id ? `<span class="tab-task-tag">🎯 ${this.escapeHtml(tab.task_id)}</span>` : '';
      const muteBtn = `<button class="tab-mute-btn" title="${tab.muted ? 'Unmute Tab (Ctrl+M)' : 'Mute Tab (Ctrl+M)'}">${tab.muted ? '🔇' : '🔊'}</button>`;
      el.innerHTML = `
        ${badge}
        ${groupBadge}
        ${taskBadge}
        <span class="tab-title">${this.escapeHtml(tab.title || tab.url)}</span>
        ${muteBtn}
        <button class="tab-close" title="Close Tab">×</button>
      `;

      el.addEventListener('click', (e) => {
        if (!e.target.classList.contains('tab-close') && !e.target.classList.contains('tab-mute-btn')) {
          this.switchTab(tab.id);
        }
      });

      const muteEl = el.querySelector('.tab-mute-btn');
      if (muteEl) {
        muteEl.addEventListener('click', (e) => {
          e.stopPropagation();
          this.toggleMuteTab(tab.id);
        });
      }

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
    this.saveActiveSessionTabs();
  }

  async createTab(url, profile = 'User') {
    const newTab = await this.rpc('tabs.create', { url, profile }) || {
      id: `tab-${Date.now()}`,
      url,
      title: 'New Tab',
      profile,
      active: true,
      group: 'General',
      discarded: false
    };
    this.tabs.forEach(t => t.active = false);
    this.tabs.push(newTab);
    this.renderTabs();
  }

  async switchTab(tabId) {
    const tab = this.tabs.find(t => t.id === tabId);
    if (tab && tab.discarded) {
      tab.discarded = false;
      this.rpc('tabs.wake', { id: tabId });
      this.showToast(`Woke tab: ${tab.title || tab.url}`);
    }
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
      this.applyUserScriptsForUrl(url);
      this.recordNetworkTraffic(url, 'GET', 200, 'document', 14200, 45, false, null);
      this.rpc('tabs.navigate', { tab_id: activeTab.id, url });
      this.recordedActions.push({ tool: 'navigate', url });
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

      if (res.privacy && this.privacyGrade && this.privacyDetails) {
        const { score, grade, trackers, fingerprinting } = res.privacy;
        this.privacyGrade.textContent = `Grade ${grade} (${score}/100)`;
        if (grade === 'A') {
          this.privacyGrade.className = 'safe-tag';
          this.privacyGrade.style.color = '#10b981';
        } else if (grade === 'B') {
          this.privacyGrade.className = 'safe-tag';
          this.privacyGrade.style.color = '#3b82f6';
        } else if (grade === 'C') {
          this.privacyGrade.className = 'warning-alert';
          this.privacyGrade.style.color = '#f59e0b';
        } else {
          this.privacyGrade.className = 'threat-alert';
          this.privacyGrade.style.color = '#ef4444';
        }

        let pDetailsHtml = '';
        if (trackers.length === 0 && fingerprinting.length === 0) {
          pDetailsHtml = '<span class="safe-tag">✔ No third-party tracking or fingerprinting scripts detected</span>';
        } else {
          if (trackers.length > 0) {
            pDetailsHtml += '<div style="margin-bottom:4px;font-weight:600;font-size:11px;">Trackers detected:</div>';
            pDetailsHtml += trackers.map(t => `
              <div class="warning-alert" style="margin-bottom:4px;font-size:11px;">
                ⚠ <strong>${this.escapeHtml(t.category)}:</strong> ${this.escapeHtml(t.pattern)} (risk: -${t.risk_weight}pts)
              </div>
            `).join('');
          }
          if (fingerprinting.length > 0) {
            pDetailsHtml += '<div style="margin-top:6px;margin-bottom:4px;font-weight:600;font-size:11px;">Fingerprinting heuristics:</div>';
            pDetailsHtml += fingerprinting.map(f => `
              <div class="threat-alert" style="margin-bottom:4px;font-size:11px;">
                🚨 <strong>${this.escapeHtml(f.heuristic)}:</strong> ${this.escapeHtml(f.explanation)}
              </div>
            `).join('');
          }
        }
        this.privacyDetails.innerHTML = pDetailsHtml;
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

  async exportMemory() {
    try {
      this.showToast('Generating memory backup...');
      const res = await this.rpc('memory.export', { format: 'json' });
      if (res && res.files && res.files['browse_memory_export.json']) {
        const content = res.files['browse_memory_export.json'];
        const blob = new Blob([content], { type: 'application/json' });
        const a = document.createElement('a');
        a.href = URL.createObjectURL(blob);
        a.download = `browse_memory_backup_${new Date().toISOString().slice(0, 10)}.json`;
        a.click();
        URL.revokeObjectURL(a.href);
        const kb = ((res.total_bytes || content.length) / 1024).toFixed(1);
        this.showToast(`Memory backup downloaded (${kb} KB)`);
      } else {
        this.showToast('Memory export completed.');
      }
    } catch (e) {
      console.error('Export memory error:', e);
      this.showToast('Failed to export memory archive');
    }
  }

  async handleMemoryFileImport(event) {
    const file = event.target.files && event.target.files[0];
    if (!file) return;

    try {
      this.showToast(`Reading backup file ${file.name}...`);
      const reader = new FileReader();
      reader.onload = async (e) => {
        try {
          const content = e.target.result;
          const res = await this.rpc('memory.import', {
            content,
            profile: this.currentProfile || 'default'
          });

          if (res) {
            const count = res.total_items || (res.imported_bookmarks + res.imported_reading_items);
            this.showToast(`Successfully restored ${count} items from backup!`);
            await this.fetchMemoryStats();
            await this.fetchBookmarks();
            if (this.fetchReadingList) await this.fetchReadingList();
          }
        } catch (err) {
          console.error('Failed to import memory JSON:', err);
          this.showToast('Failed to restore backup: ' + (err.message || 'invalid format'));
        } finally {
          event.target.value = '';
        }
      };
      reader.readAsText(file);
    } catch (err) {
      console.error('File read error:', err);
      this.showToast('Failed to read file');
      event.target.value = '';
    }
  }

  async autoGroupTabs() {
    try {
      this.showToast('Analyzing tabs for task auto-grouping...');
      const res = await this.rpc('tabs.auto_group', { apply: true });
      if (res && res.groups && res.groups.length > 0) {
        res.groups.forEach(g => {
          g.tab_ids.forEach(tid => {
            const tab = this.tabs.find(t => t.id === tid);
            if (tab) tab.group = g.title;
          });
        });
        this.renderTabs();
        this.showToast(`✨ Organized ${res.grouped_tab_count} tab(s) into ${res.groups.length} task group(s)!`);
      } else {
        this.showToast('No related tabs found to form new groups.');
      }
    } catch (e) {
      console.error('Auto-group error:', e);
      this.showToast('Failed to auto-group tabs');
    }
  }

  async explainDiagnostic() {
    const input = this.diagInput ? this.diagInput.value.trim() : '';
    if (!input) return;

    try {
      const res = await this.rpc('devtools.explain', { message: input });
      if (res && this.diagCard) {
        this.diagCard.classList.remove('hidden');
        if (this.diagTitle) this.diagTitle.textContent = res.title || 'Diagnostic Explanation';
        if (this.diagSummary) this.diagSummary.textContent = res.summary || '';
        if (this.diagCause) this.diagCause.innerHTML = `<strong>Root cause:</strong> ${this.escapeHtml(res.root_cause || '')}`;
        if (this.diagFix) this.diagFix.innerHTML = `<strong>Suggested fix:</strong> ${this.escapeHtml(res.suggested_fix || '')}`;
      }
    } catch (e) {
      console.error('Explain diagnostic error:', e);
    }
  }

  async fetchBookmarks() {
    if (!this.bookmarksList) return;
    try {
      const list = await this.rpc('bookmarks.list', {}) || [];
      if (list.length === 0) {
        this.bookmarksList.innerHTML = '<span class="empty-hint" style="font-size:11px;margin:0;">No bookmarks yet. Click ⭐ to bookmark current page.</span>';
        return;
      }
      this.bookmarksList.innerHTML = list.map(b => `
        <span class="bookmark-item" style="cursor:pointer;padding:2px 8px;border-radius:4px;background:var(--bg-card);border:1px solid var(--border-color);display:inline-flex;align-items:center;gap:4px;" title="${this.escapeHtml(b.url)}">
          <span>🔖</span>
          <strong>${this.escapeHtml(b.title || b.url)}</strong>
        </span>
      `).join('');

      this.bookmarksList.querySelectorAll('.bookmark-item').forEach((item, idx) => {
        item.addEventListener('click', () => {
          const bm = list[idx];
          if (bm) {
            this.omnibox.value = bm.url;
            this.handleOmniboxSubmit();
          }
        });
      });
    } catch (e) {
      console.warn('Fetch bookmarks error:', e);
    }
  }

  async bookmarkCurrentPage() {
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    if (!activeTab || !activeTab.url || activeTab.url === 'about:blank') return;

    try {
      await this.rpc('bookmarks.add', {
        url: activeTab.url,
        title: activeTab.title || activeTab.url,
        folder: 'Bookmarks Bar'
      });
      await this.fetchBookmarks();
      this.showToast(`⭐ Saved to bookmarks: ${activeTab.title || activeTab.url}`);
    } catch (e) {
      console.error('Bookmark add error:', e);
      this.showToast('Failed to add bookmark');
    }
  }

  toggleShield() {
    this.shieldActive = !this.shieldActive;
    if (this.btnShield) {
      if (this.shieldActive) {
        this.btnShield.textContent = '🛡️ Shield: On';
        this.btnShield.className = 'badge focus-badge';
        this.showToast('🛡️ AdBlock & Tracker Protection enabled');
      } else {
        this.btnShield.textContent = '🛡️ Shield: Off';
        this.btnShield.className = 'badge offline-badge';
        this.showToast('⚠ AdBlock Protection disabled for this session');
      }
    }
  }

  async recordHistory(url, title) {
    if (!url || url === 'about:blank') return;
    try {
      await this.rpc('history.record', {
        url,
        title: title || url,
        transition: 'typed',
        profile: 'default'
      });
      this.fetchHistory();
    } catch (e) {
      console.warn('Record history error:', e);
    }
  }

  async fetchHistory(search = '') {
    if (!this.historyList) return;
    try {
      const list = await this.rpc('history.list', { profile: 'default', limit: 40, search: search || null }) || [];
      if (list.length === 0) {
        this.historyList.innerHTML = '<span class="empty-hint" style="font-size:12px;">No history records found.</span>';
        return;
      }
      this.historyList.innerHTML = list.map(item => `
        <div class="history-item" style="padding:6px;border-radius:6px;background:var(--bg-card);border:1px solid var(--border-color);display:flex;justify-content:space-between;align-items:center;">
          <div style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin-right:8px;cursor:pointer;" class="hist-title-link">
            <div style="font-size:12px;font-weight:600;color:var(--text-primary);">${this.escapeHtml(item.title || item.url)}</div>
            <div style="font-size:10px;color:var(--text-dim);">${this.escapeHtml(item.url)} &bull; ${new Date(item.started_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</div>
          </div>
          <button class="btn-icon btn-del-hist" data-id="${item.id}" title="Remove" style="font-size:11px;opacity:0.6;cursor:pointer;">×</button>
        </div>
      `).join('');

      this.historyList.querySelectorAll('.hist-title-link').forEach((el, idx) => {
        el.addEventListener('click', () => {
          const item = list[idx];
          if (item) {
            this.omnibox.value = item.url;
            this.handleOmniboxSubmit();
          }
        });
      });

      this.historyList.querySelectorAll('.btn-del-hist').forEach((btn) => {
        btn.addEventListener('click', async (e) => {
          e.stopPropagation();
          const id = btn.getAttribute('data-id');
          await this.rpc('history.delete', { id });
          this.fetchHistory(search);
        });
      });
    } catch (e) {
      console.warn('Fetch history error:', e);
    }
  }

  async clearHistory() {
    try {
      const res = await this.rpc('history.clear', { profile: 'default' });
      this.showToast(`Cleared ${res ? res.cleared_count : 0} history records`);
      this.fetchHistory();
    } catch (e) {
      console.error('Clear history error:', e);
    }
  }

  async fetchModelCatalog() {
    if (!this.modelCatalogList) return;
    try {
      const res = await this.rpc('models.status', {});
      if (!res || !res.models || res.models.length === 0) {
        this.modelCatalogList.innerHTML = '<span class="empty-hint" style="font-size:12px;">No recommended models found.</span>';
        return;
      }
      this.modelCatalogList.innerHTML = res.models.map(m => {
        const isInstalled = m.status && m.status.status === 'installed';
        const sizeMb = isInstalled ? (m.status.size_bytes / (1024 * 1024)).toFixed(0) : (m.status.expected_size_bytes / (1024 * 1024)).toFixed(0);
        const prog = m.progress;
        const isDownloading = prog && !prog.complete && prog.downloaded_bytes > 0;

        let statusBadge = '<span class="warning-alert" style="font-size:10px;">Not downloaded</span>';
        if (isInstalled) {
          statusBadge = '<span class="safe-tag" style="font-size:10px;">✔ Installed</span>';
        } else if (isDownloading) {
          statusBadge = `<span class="focus-badge" style="font-size:10px;">⏳ Downloading ${prog.percent.toFixed(1)}%</span>`;
        }

        let actionButton = '';
        if (isInstalled) {
          actionButton = `<div style="font-size:10px;color:var(--text-dim);">Ready for offline inference in <code>${this.escapeHtml(res.models_dir)}</code></div>`;
        } else if (isDownloading) {
          actionButton = `
            <div style="width:100%;background:var(--bg-main);border-radius:4px;height:6px;overflow:hidden;margin-top:4px;">
              <div style="background:var(--accent-color);height:100%;width:${prog.percent}%;"></div>
            </div>
          `;
        } else {
          actionButton = `<button class="btn-primary btn-download-model" data-id="${m.id}" style="font-size:10px;padding:3px 8px;cursor:pointer;">📥 1-Click Download</button>`;
        }

        return `
          <div style="padding:8px;background:var(--bg-card);border:1px solid var(--border-color);border-radius:6px;">
            <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:4px;">
              <strong style="font-size:12px;color:var(--text-primary);">${this.escapeHtml(m.name)}</strong>
              ${statusBadge}
            </div>
            <div style="font-size:11px;color:var(--text-secondary);margin-bottom:6px;">
              Tier: <code>${m.tier}</code> &bull; Size: ~${sizeMb} MB
            </div>
            ${actionButton}
          </div>
        `;
      }).join('');

      this.modelCatalogList.querySelectorAll('.btn-download-model').forEach(btn => {
        btn.addEventListener('click', () => {
          this.downloadModel(btn.dataset.id);
        });
      });
    } catch (e) {
      console.warn('Fetch model catalog error:', e);
    }
  }

  async downloadModel(id) {
    try {
      this.showToast(`Starting download for ${id}...`);
      const res = await this.rpc('models.download', { id });
      if (res && res.status === 'started') {
        this.showToast(`📥 Downloading model ${id} in background...`);
        // Poll progress periodically
        const interval = setInterval(async () => {
          const progRes = await this.rpc('models.progress', { id });
          const prog = progRes ? progRes.progress : null;
          if (prog && prog.complete) {
            clearInterval(interval);
            this.showToast(`✔ Model ${id} downloaded and verified!`);
            this.fetchModelCatalog();
          } else {
            this.fetchModelCatalog();
          }
        }, 3000);
      } else if (res && res.status === 'already_installed') {
        this.showToast(`Model ${id} is already installed.`);
      }
    } catch (e) {
      console.error('Download model error:', e);
      this.showToast(`Failed to start download: ${e.message || e}`);
    }
  }

  async showPlaywrightModal() {
    if (this.playwrightModal) this.playwrightModal.classList.remove('hidden');
    await this.renderPlaywrightScript();
  }

  async renderPlaywrightScript() {
    const title = (this.agentTaskInput && this.agentTaskInput.value.trim()) || 'Browser Automation Flow';
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    const actions = this.recordedActions.length > 0 ? this.recordedActions : [
      { tool: 'navigate', url: (activeTab ? activeTab.url : 'https://example.com') }
    ];
    const res = await this.rpc('session.export_playwright', {
      title,
      language: this.playwrightLang,
      actions
    });
    if (res && this.playwrightCodePreview) {
      this.playwrightCodePreview.textContent = res.code;
    }
  }

  async toggleReaderMode() {
    if (!this.readerPane) return;
    if (!this.readerPane.classList.contains('hidden')) {
      this.readerPane.classList.add('hidden');
      return;
    }
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    const url = activeTab ? activeTab.url : (this.omnibox ? this.omnibox.value : '');
    const title = activeTab ? activeTab.title : '';

    if (this.readerBody) {
      this.readerBody.innerHTML = '<div style="padding:40px;text-align:center;color:var(--text-secondary);">Extracting distraction-free article...</div>';
    }
    this.readerPane.classList.remove('hidden');

    try {
      const res = await this.rpc('page.reader_mode', { url, title });
      if (res && res.clean_html) {
        this.readerBody.innerHTML = res.clean_html;
      } else {
        this.readerBody.innerHTML = '<p style="text-align:center;color:var(--text-secondary);margin-top:40px;">Unable to extract article text from this page.</p>';
      }
    } catch (e) {
      if (this.readerBody) {
        this.readerBody.innerHTML = `<p style="color:var(--danger-color);text-align:center;">Failed to load reader mode: ${this.escapeHtml(e.message || String(e))}</p>`;
      }
    }
  }

  async forgetCurrentSite() {
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    const url = activeTab ? activeTab.url : (this.omnibox ? this.omnibox.value : '');
    if (!url || url.startsWith('about:') || url.startsWith('browse:')) {
      this.showToast('No active domain to forget');
      return;
    }
    if (!confirm(`Are you sure you want to forget all data for this site? This completely purges history, cached snapshots, and memory for this domain.`)) {
      return;
    }
    try {
      const res = await this.rpc('page.forget_site', { url });
      if (res) {
        this.showToast(`Deleted ${res.deleted_pages} page record(s) for ${res.domain}`);
        await this.fetchHistory();
        await this.fetchMemoryStats();
      }
    } catch (e) {
      this.showToast(`Error forgetting site: ${e.message || e}`);
    }
  }

  showDownloadsTab() {
    this.sidebarPane.classList.remove('collapsed');
    const dlTab = document.querySelector('.sidebar-tab[data-tab="downloads"]');
    if (dlTab) dlTab.click();
    this.fetchDownloads();
  }

  async fetchDownloads() {
    if (!this.downloadsList) return;
    try {
      const items = await this.rpc('downloads.list', { profile: 'default' }) || [];
      if (items.length === 0) {
        this.downloadsList.innerHTML = '<span class="empty-hint" style="font-size:12px;">No downloads recorded yet.</span>';
        return;
      }
      this.downloadsList.innerHTML = items.map(d => {
        const sizeMb = d.total_bytes ? (d.total_bytes / (1024 * 1024)).toFixed(1) : (d.downloaded_bytes / (1024 * 1024)).toFixed(1);
        const dangerBadge = d.danger_level === 'dangerous'
          ? '<span class="warning-alert" style="font-size:10px;background:#ef444422;color:#ef4444;border-color:#ef444455;">⚠️ Dangerous File</span>'
          : d.danger_level === 'suspicious'
          ? '<span class="warning-alert" style="font-size:10px;">⚠️ Suspicious File</span>'
          : '<span class="safe-tag" style="font-size:10px;">✔ Safe</span>';

        const statusText = d.status === 'completed'
          ? '✔ Completed'
          : d.status === 'in_progress'
          ? '⏳ In Progress'
          : d.status;

        const shaHtml = d.sha256 ? `<div style="font-size:10px;color:var(--text-dim);font-family:var(--font-mono);word-break:break-all;margin-top:2px;">SHA-256: ${this.escapeHtml(d.sha256)}</div>` : '';

        return `
          <div style="padding:8px 10px;background:var(--bg-card);border:1px solid var(--border-color);border-radius:6px;display:flex;flex-direction:column;gap:4px;">
            <div style="display:flex;justify-content:space-between;align-items:center;">
              <strong style="font-size:12px;color:var(--text-primary);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:200px;" title="${this.escapeHtml(d.filename)}">${this.escapeHtml(d.filename)}</strong>
              <div style="display:flex;gap:6px;align-items:center;">
                ${dangerBadge}
                <button class="btn-icon" style="font-size:11px;opacity:0.6;" onclick="browseShell.deleteDownload('${this.escapeHtml(d.id)}')" title="Delete from list">×</button>
              </div>
            </div>
            <div style="font-size:11px;color:var(--text-secondary);display:flex;justify-content:space-between;">
              <span>${statusText} &bull; ${sizeMb} MB</span>
              <span style="opacity:0.6;">${new Date(d.started_at).toLocaleTimeString()}</span>
            </div>
            ${shaHtml}
          </div>
        `;
      }).join('');
    } catch (e) {
      console.warn('Fetch downloads error:', e);
    }
  }

  async deleteDownload(id) {
    try {
      await this.rpc('downloads.delete', { id });
      this.fetchDownloads();
    } catch (e) {
      console.error('Delete download error:', e);
    }
  }

  async clearDownloads() {
    try {
      const res = await this.rpc('downloads.clear', { profile: 'default' });
      this.showToast(`Cleared ${res ? res.cleared_count : 0} download record(s)`);
      this.fetchDownloads();
    } catch (e) {
      console.error('Clear downloads error:', e);
    }
  }

  openFind() {
    if (!this.findBar) return;
    this.findBar.classList.remove('hidden');
    if (this.findInput) {
      this.findInput.focus();
      this.findInput.select();
    }
    this.performFind();
  }

  closeFind() {
    if (!this.findBar) return;
    this.findBar.classList.add('hidden');
    this.findMatches = [];
    this.currentFindIndex = -1;
    if (this.findCounter) this.findCounter.textContent = '0 / 0';
  }

  performFind() {
    if (!this.findInput) return;
    const query = this.findInput.value.trim();
    if (!query) {
      this.findMatches = [];
      this.currentFindIndex = -1;
      if (this.findCounter) this.findCounter.textContent = '0 / 0';
      return;
    }

    let textToSearch = '';
    const readerPane = document.getElementById('reader-pane');
    if (readerPane && !readerPane.classList.contains('hidden')) {
      textToSearch = readerPane.innerText || '';
    } else {
      textToSearch = document.body.innerText || '';
    }

    const regexFlags = this.findMatchCase ? 'g' : 'gi';
    const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const regex = new RegExp(escaped, regexFlags);

    const matches = [];
    let match;
    while ((match = regex.exec(textToSearch)) !== null) {
      matches.push(match.index);
    }

    this.findMatches = matches;
    if (matches.length > 0) {
      this.currentFindIndex = 0;
      if (this.findCounter) this.findCounter.textContent = `1 / ${matches.length}`;
    } else {
      this.currentFindIndex = -1;
      if (this.findCounter) this.findCounter.textContent = '0 / 0';
    }
  }

  nextFindMatch() {
    if (this.findMatches.length === 0) return;
    this.currentFindIndex = (this.currentFindIndex + 1) % this.findMatches.length;
    if (this.findCounter) {
      this.findCounter.textContent = `${this.currentFindIndex + 1} / ${this.findMatches.length}`;
    }
  }

  prevFindMatch() {
    if (this.findMatches.length === 0) return;
    this.currentFindIndex = (this.currentFindIndex - 1 + this.findMatches.length) % this.findMatches.length;
    if (this.findCounter) {
      this.findCounter.textContent = `${this.currentFindIndex + 1} / ${this.findMatches.length}`;
    }
  }

  async inspectCookies() {
    if (!this.cookiesDetails) return;
    this.cookiesDetails.classList.remove('hidden');
    if (this.cookiesBreakdown) {
      this.cookiesBreakdown.innerHTML = '<span style="font-size:11px;color:var(--text-secondary);">Auditing cookies and site storage...</span>';
    }

    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    const url = activeTab ? activeTab.url : (this.omnibox ? this.omnibox.value : '');
    let domain = 'example.com';
    try {
      if (url && url.includes('://')) {
        domain = new URL(url).hostname;
      }
    } catch (_) {}

    const cookies = [
      { name: 'session_token', value: 's_9876543210abcdef', domain: domain, path: '/', secure: true, http_only: true },
      { name: 'theme', value: 'dark', domain: domain, path: '/', secure: false, http_only: false },
      { name: '_ga', value: 'GA1.2.11223344', domain: `.${domain}`, path: '/', secure: true, http_only: false },
    ];

    try {
      const summary = await this.rpc('cookies.audit', { domain, cookies });
      if (!summary) return;

      if (this.cookiesBreakdown) {
        this.cookiesBreakdown.innerHTML = `
          <span class="safe-tag" style="font-size:10px;">Necessary: ${summary.strictly_necessary_count}</span>
          <span class="safe-tag" style="font-size:10px;">Functional: ${summary.functional_count}</span>
          <span class="warning-alert" style="font-size:10px;">Analytics: ${summary.analytics_count}</span>
          <span class="warning-alert" style="font-size:10px;">Ad Trackers: ${summary.advertising_count}</span>
        `;
      }

      if (this.cookiesList) {
        this.cookiesList.innerHTML = summary.cookies.map(c => `
          <div style="font-size:11px;padding:4px 6px;background:var(--bg-card);border:1px solid var(--border-color);border-radius:4px;display:flex;justify-content:space-between;align-items:center;">
            <div>
              <strong style="color:var(--text-primary);">${this.escapeHtml(c.name)}</strong>
              <span style="opacity:0.6;font-family:var(--font-mono);margin-left:4px;">${this.escapeHtml(c.value_preview)}</span>
            </div>
            <span style="font-size:10px;opacity:0.8;color:${c.category === 'advertising' || c.category === 'analytics' ? '#f59e0b' : '#10b981'};">${c.category}</span>
          </div>
        `).join('');
      }
    } catch (e) {
      console.warn('Cookie audit error:', e);
    }
  }

  async clearOriginStorage() {
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    const url = activeTab ? activeTab.url : (this.omnibox ? this.omnibox.value : '');
    let domain = 'current origin';
    try {
      if (url && url.includes('://')) {
        domain = new URL(url).hostname;
      }
    } catch (_) {}

    if (!confirm(`Clear all cookies and storage for ${domain}?`)) return;

    try {
      const res = await this.rpc('cookies.clear', { domain });
      this.showToast(res ? res.message : `Storage cleared for ${domain}`);
      if (this.cookiesDetails) this.cookiesDetails.classList.add('hidden');
    } catch (e) {
      this.showToast(`Clear storage failed: ${e.message || e}`);
    }
  }

  async promptSaveCredential() {
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    const origin = activeTab ? activeTab.url : (this.omnibox ? this.omnibox.value : '');
    const username = prompt('Enter username / email:');
    if (!username) return;
    const secret = prompt('Enter password:');
    if (!secret) return;

    try {
      const res = await this.rpc('vault.save', { origin, username, secret });
      if (res) {
        this.showToast(`Saved encrypted credentials for ${res.username}`);
        this.fetchVaultCredentials();
      }
    } catch (e) {
      this.showToast(`Failed to save credential: ${e.message || e}`);
    }
  }

  async fetchVaultCredentials() {
    if (!this.vaultCredsList) return;
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    const url = activeTab ? activeTab.url : (this.omnibox ? this.omnibox.value : '');
    let domain = null;
    try {
      if (url && url.includes('://')) domain = new URL(url).hostname;
    } catch (_) {}

    try {
      const creds = await this.rpc('vault.list', { domain }) || [];
      if (creds.length === 0) {
        this.vaultCredsList.innerHTML = '<div class="empty-hint" style="font-size:11px;">No saved credentials for this domain.</div>';
        return;
      }
      this.vaultCredsList.innerHTML = creds.map(c => `
        <div style="font-size:11px;padding:6px 8px;background:var(--bg-card);border:1px solid var(--border-color);border-radius:6px;display:flex;justify-content:space-between;align-items:center;">
          <div>
            <strong style="color:var(--text-primary);">${this.escapeHtml(c.username)}</strong>
            <span style="opacity:0.6;margin-left:6px;">(${this.escapeHtml(c.domain)})</span>
            <div style="font-family:var(--font-mono);opacity:0.5;font-size:10px;">••••••••••••</div>
          </div>
          <div style="display:flex;gap:4px;">
            <button class="btn-secondary" style="font-size:10px;padding:2px 6px;" onclick="browseShell.revealCredential('${this.escapeHtml(c.id)}')">👁️ Reveal</button>
            <button class="btn-icon" style="font-size:11px;opacity:0.6;" onclick="browseShell.deleteCredential('${this.escapeHtml(c.id)}')">×</button>
          </div>
        </div>
      `).join('');
    } catch (e) {
      console.warn('Fetch vault error:', e);
    }
  }

  async revealCredential(id) {
    try {
      const res = await this.rpc('vault.get', { id });
      if (res && res.secret) {
        alert(`Decrypted Password: ${res.secret}`);
      }
    } catch (e) {
      this.showToast(`Decrypt error: ${e.message || e}`);
    }
  }

  async deleteCredential(id) {
    try {
      await this.rpc('vault.delete', { id });
      this.showToast('Credential deleted from vault');
      this.fetchVaultCredentials();
    } catch (e) {
      console.error('Delete cred error:', e);
    }
  }

  // Command Palette
  openCommandPalette() {
    if (!this.paletteModal) return;
    this.paletteModal.classList.remove('hidden');
    if (this.paletteInput) {
      this.paletteInput.value = '';
      this.paletteInput.focus();
    }
    this.searchCommandPalette();
  }

  closeCommandPalette() {
    if (!this.paletteModal) return;
    this.paletteModal.classList.add('hidden');
    if (this.paletteInput) this.paletteInput.blur();
  }

  async searchCommandPalette() {
    const query = this.paletteInput ? this.paletteInput.value.trim() : '';
    try {
      const items = await this.rpc('palette.search', { query, profile: this.currentProfile }) || [];
      this.paletteItems = items;
      this.selectedPaletteIndex = 0;
      this.renderPaletteResults();
    } catch (e) {
      console.warn('Palette search error:', e);
    }
  }

  renderPaletteResults() {
    if (!this.paletteResults) return;
    if (this.paletteItems.length === 0) {
      this.paletteResults.innerHTML = '<div class="empty-hint" style="padding:16px;text-align:center;font-size:12px;">No matching actions, tabs, bookmarks, or history.</div>';
      return;
    }

    const categoryIcons = {
      action: '⚡',
      tab: '🗂️',
      bookmark: '⭐',
      history: '🕒'
    };

    this.paletteResults.innerHTML = this.paletteItems.map((item, idx) => {
      const icon = categoryIcons[item.category] || '🔍';
      const isSelected = idx === this.selectedPaletteIndex ? 'active' : '';
      return `
        <div class="palette-item ${isSelected}" data-index="${idx}" onclick="browseShell.selectPaletteIndex(${idx})">
          <div class="palette-item-left">
            <span class="palette-item-icon">${icon}</span>
            <div class="palette-item-text">
              <span class="palette-item-title">${this.escapeHtml(item.title)}</span>
              <span class="palette-item-subtitle">${this.escapeHtml(item.subtitle || '')}</span>
            </div>
          </div>
          <span class="palette-category-badge">${this.escapeHtml(item.category)}</span>
        </div>
      `;
    }).join('');

    const activeEl = this.paletteResults.querySelector('.palette-item.active');
    if (activeEl) {
      activeEl.scrollIntoView({ block: 'nearest' });
    }
  }

  selectPaletteIndex(idx) {
    this.selectedPaletteIndex = idx;
    if (this.paletteItems[idx]) {
      this.executePaletteItem(this.paletteItems[idx]);
    }
  }

  handlePaletteKeydown(e) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (this.paletteItems.length > 0) {
        this.selectedPaletteIndex = (this.selectedPaletteIndex + 1) % this.paletteItems.length;
        this.renderPaletteResults();
      }
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (this.paletteItems.length > 0) {
        this.selectedPaletteIndex = (this.selectedPaletteIndex - 1 + this.paletteItems.length) % this.paletteItems.length;
        this.renderPaletteResults();
      }
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (this.paletteItems[this.selectedPaletteIndex]) {
        this.executePaletteItem(this.paletteItems[this.selectedPaletteIndex]);
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      this.closeCommandPalette();
    }
  }

  async executePaletteItem(item) {
    this.closeCommandPalette();
    if (!item) return;

    if (item.category === 'action') {
      switch (item.target) {
        case 'action:reader':
          this.toggleReaderMode();
          break;
        case 'action:focus':
          this.toggleFocusMode();
          break;
        case 'action:new_tab':
          this.createTab('https://example.com');
          break;
        case 'action:downloads': {
          this.sidebarPane.classList.remove('collapsed');
          const tab = document.querySelector('.sidebar-tab[data-tab="downloads"]');
          if (tab) tab.click();
          break;
        }
        case 'action:vault': {
          this.sidebarPane.classList.remove('collapsed');
          const tab = document.querySelector('.sidebar-tab[data-tab="settings"]');
          if (tab) tab.click();
          break;
        }
        case 'action:cookies':
          this.inspectCookies();
          break;
        case 'action:clear_storage':
          this.clearOriginStorage();
          break;
        case 'action:shield':
          this.toggleShield();
          break;
        case 'action:export_vault':
          this.exportMemory();
          break;
        case 'action:find':
          this.openFind();
          break;
        case 'action:sleep_tabs':
          this.sleepBackgroundTabs();
          break;
        case 'action:security':
          this.openSecurityModal();
          break;
        case 'action:toggle_mute':
          if (this.activeTabId) this.toggleMuteTab(this.activeTabId);
          break;
        case 'action:save_session':
        case 'action:sessions':
          this.openSessionsModal();
          break;
        case 'action:restore_crash':
          this.restoreCrashSession();
          break;
        case 'action:zoom_in':
          this.zoomIn();
          break;
        case 'action:zoom_out':
          this.zoomOut();
          break;
        case 'action:zoom_reset':
          this.resetZoom();
          break;
        case 'action:dedup_tabs':
          this.dedupTabs();
          break;
        case 'action:userscripts':
          this.openUserScriptsModal();
          break;
        case 'action:prompts':
          this.openPromptTemplates();
          break;
        case 'action:reading_list':
          this.openReadingListModal();
          break;
        case 'action:add_reading_list':
          this.addCurrentPageToReadingList();
          break;
        case 'action:network_monitor':
          this.openNetworkModal();
          break;
        case 'action:backup_memory':
          this.exportMemory();
          break;
        case 'action:restore_memory':
          if (this.memoryImportFile) this.memoryImportFile.click();
          break;
        default:
          this.showToast(`Action executed: ${item.title}`);
      }
    } else if (item.category === 'tab') {
      this.switchTab(item.target);
    } else if (item.category === 'bookmark' || item.category === 'history') {
      this.navigate(item.target);
    }
  }

  // Profile Management
  updateProfileIndicator() {
    if (this.profileIndicator) {
      this.profileIndicator.textContent = `Profile: ${this.currentProfile}`;
    }
  }

  openProfileModal() {
    if (!this.profileModal) return;
    this.profileModal.classList.remove('hidden');
    this.fetchProfiles();
  }

  closeProfileModal() {
    if (!this.profileModal) return;
    this.profileModal.classList.add('hidden');
  }

  async fetchProfiles() {
    if (!this.profilesList) return;
    try {
      const profiles = await this.rpc('profiles.list', {}) || [];
      if (profiles.length === 0) {
        this.profilesList.innerHTML = '<div class="empty-hint" style="font-size:11px;">No profiles found.</div>';
        return;
      }
      this.profilesList.innerHTML = profiles.map(p => {
        const isActive = p.id === this.currentProfile;
        return `
          <div class="profile-card-item ${isActive ? 'active-profile' : ''}">
            <div style="display:flex;align-items:center;gap:8px;">
              <span style="font-size:14px;">${p.kind === 'agent' ? '🤖' : p.kind === 'private' ? '🕶️' : '👤'}</span>
              <div>
                <strong style="font-size:12px;color:var(--text-main);">${this.escapeHtml(p.name)}</strong>
                <span style="font-size:10px;opacity:0.6;margin-left:6px;text-transform:uppercase;">[${this.escapeHtml(p.kind)}]</span>
              </div>
            </div>
            <div style="display:flex;gap:6px;align-items:center;">
              ${isActive
                ? '<span style="font-size:10px;color:var(--accent);font-weight:600;">ACTIVE</span>'
                : `<button class="btn-secondary" style="font-size:11px;padding:2px 8px;" onclick="browseShell.switchProfile('${this.escapeHtml(p.id)}', '${this.escapeHtml(p.name)}')">Switch</button>`
              }
              ${p.id !== 'default'
                ? `<button class="btn-icon" style="font-size:11px;opacity:0.6;" title="Delete Profile" onclick="browseShell.deleteProfile('${this.escapeHtml(p.id)}')">×</button>`
                : ''
              }
            </div>
          </div>
        `;
      }).join('');
    } catch (e) {
      console.warn('Fetch profiles error:', e);
    }
  }

  async createProfile() {
    const name = this.newProfileName ? this.newProfileName.value.trim() : '';
    const kind = this.newProfileKind ? this.newProfileKind.value : 'user';
    if (!name) return;
    try {
      const prof = await this.rpc('profiles.create', { name, kind });
      if (prof) {
        if (this.newProfileName) this.newProfileName.value = '';
        this.showToast(`Profile "${prof.name}" created`);
        this.fetchProfiles();
      }
    } catch (e) {
      this.showToast(`Error creating profile: ${e.message || e}`);
    }
  }

  async deleteProfile(id) {
    if (!confirm('Are you sure you want to delete this profile and its workspace data?')) return;
    try {
      await this.rpc('profiles.delete', { id });
      this.showToast('Profile deleted');
      if (this.currentProfile === id) {
        this.switchProfile('default', 'Default User');
      } else {
        this.fetchProfiles();
      }
    } catch (e) {
      this.showToast(`Error deleting profile: ${e.message || e}`);
    }
  }

  switchProfile(id, name) {
    this.currentProfile = id;
    localStorage.setItem('browse_active_profile', id);
    if (this.profileIndicator) {
      this.profileIndicator.textContent = `Profile: ${name || id}`;
    }
    this.showToast(`Switched to profile: ${name || id}`);
    this.fetchProfiles();
    this.fetchBookmarks();
    this.fetchHistory();
    this.fetchDownloads();
    this.fetchVaultCredentials();
  }

  // Site Security & Permissions Matrix
  async openSecurityModal() {
    if (!this.securityModal) return;
    this.securityModal.classList.remove('hidden');

    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    const url = activeTab ? activeTab.url : (this.omnibox ? this.omnibox.value : '');
    let origin = 'https://example.com';
    try {
      if (url && url.includes('://')) {
        const u = new URL(url);
        origin = u.origin;
      }
    } catch (_) {}

    if (this.secModalOrigin) this.secModalOrigin.textContent = origin;
    if (this.secModalConn) {
      const isHttps = origin.startsWith('https://');
      this.secModalConn.textContent = isHttps ? '🔒 Connection Encrypted (TLS 1.3)' : '⚠️ Connection Unencrypted (HTTP)';
      this.secModalConn.style.background = isHttps ? 'var(--success-light)' : 'var(--warning-light)';
    }

    await this.fetchSitePermissions(origin);
  }

  closeSecurityModal() {
    if (this.securityModal) this.securityModal.classList.add('hidden');
  }

  async fetchSitePermissions(origin) {
    if (!this.secPermissionsList) return;
    try {
      const res = await this.rpc('permissions.get', { origin });
      const perms = res ? res.permissions : {};
      const standardPerms = [
        { key: 'geolocation', label: '📍 Location' },
        { key: 'camera', label: '📷 Camera' },
        { key: 'microphone', label: '🎙️ Microphone' },
        { key: 'notifications', label: '🔔 Notifications' },
        { key: 'clipboard', label: '📋 Clipboard' },
        { key: 'javascript', label: '⚡ JavaScript' }
      ];

      this.secPermissionsList.innerHTML = standardPerms.map(p => {
        const val = perms[p.key] || 'ask';
        return `
          <div class="perm-row">
            <div class="perm-row-left">
              <span>${p.label}</span>
            </div>
            <select class="perm-select" onchange="browseShell.setSitePermission('${this.escapeHtml(origin)}', '${p.key}', this.value)">
              <option value="allow" ${val === 'allow' ? 'selected' : ''}>Allow</option>
              <option value="block" ${val === 'block' ? 'selected' : ''}>Block</option>
              <option value="ask" ${val === 'ask' ? 'selected' : ''}>Ask</option>
            </select>
          </div>
        `;
      }).join('');
    } catch (e) {
      console.warn('Fetch site perms error:', e);
    }
  }

  async setSitePermission(origin, permission, state) {
    try {
      await this.rpc('permissions.set', { origin, permission, state });
      this.showToast(`Updated ${permission} permission to ${state}`);
    } catch (e) {
      this.showToast(`Failed to update permission: ${e.message || e}`);
    }
  }

  // Tab Sleep / Memory Saver
  async sleepBackgroundTabs() {
    let sleptCount = 0;
    for (const t of this.tabs) {
      if (!t.active && !t.pinned && !t.discarded) {
        t.discarded = true;
        await this.rpc('tabs.discard', { id: t.id });
        sleptCount++;
      }
    }
    this.renderTabs();
    this.showToast(`Put ${sleptCount} background tab(s) to sleep (RAM saved)`);
  }

  // Page Zoom Controls
  zoomIn() {
    this.zoomLevel = Math.min(2.5, Math.round((this.zoomLevel + 0.1) * 10) / 10);
    this.applyZoom();
  }

  zoomOut() {
    this.zoomLevel = Math.max(0.5, Math.round((this.zoomLevel - 0.1) * 10) / 10);
    this.applyZoom();
  }

  resetZoom() {
    this.zoomLevel = 1.0;
    this.applyZoom();
  }

  applyZoom() {
    if (this.zoomLevelBadge) {
      this.zoomLevelBadge.textContent = `${Math.round(this.zoomLevel * 100)}%`;
    }
    if (this.webFrame) {
      this.webFrame.style.transformOrigin = '0 0';
      this.webFrame.style.transform = `scale(${this.zoomLevel})`;
      this.webFrame.style.width = `${100 / this.zoomLevel}%`;
      this.webFrame.style.height = `${100 / this.zoomLevel}%`;
    }
    this.showToast(`Zoom: ${Math.round(this.zoomLevel * 100)}%`);
  }

  // Tab Audio Muting
  async toggleMuteTab(tabId) {
    const tab = this.tabs.find(t => t.id === tabId);
    if (!tab) return;
    const res = await this.rpc('tabs.toggle_mute', { id: tabId });
    if (res && res.found) {
      tab.muted = res.muted;
      this.renderTabs();
      this.showToast(tab.muted ? `🔇 Muted audio on ${tab.title || tab.url}` : `🔊 Unmuted audio on ${tab.title || tab.url}`);
    }
  }

  // Saved Sessions & Crash Recovery
  saveActiveSessionTabs() {
    clearTimeout(this._saveActiveTimer);
    this._saveActiveTimer = setTimeout(() => {
      this.rpc('sessions.save_active', { profile: this.currentProfile || 'user' });
    }, 500);
  }

  openSessionsModal() {
    if (!this.sessionsModal) return;
    this.sessionsModal.classList.remove('hidden');
    if (this.newSessionName) this.newSessionName.value = '';
    this.loadSessionsList();
    this.checkCrashRecoveryAlert();
  }

  closeSessionsModal() {
    if (!this.sessionsModal) return;
    this.sessionsModal.classList.add('hidden');
  }

  async loadSessionsList() {
    if (!this.savedSessionsList) return;
    try {
      const sessions = await this.rpc('sessions.list', { profile: this.currentProfile || 'user' }) || [];
      if (sessions.length === 0) {
        this.savedSessionsList.innerHTML = '<div class="empty-hint" style="padding:12px;text-align:center;font-size:11px;">No saved sessions yet. Save your open tabs above.</div>';
        return;
      }
      this.savedSessionsList.innerHTML = sessions.map(s => {
        let tabCount = 0;
        try {
          const parsed = JSON.parse(s.tabs_json);
          tabCount = Array.isArray(parsed) ? parsed.length : 0;
        } catch (_) {}
        const dateStr = s.created_at ? new Date(s.created_at).toLocaleDateString() : '';
        return `
          <div class="session-item">
            <div class="session-item-left">
              <span class="session-item-title">💾 ${this.escapeHtml(s.name)}</span>
              <span class="session-item-meta">${tabCount} tab(s) · ${dateStr}</span>
            </div>
            <div class="session-item-actions">
              <button class="btn-secondary" style="font-size:11px;padding:3px 8px;" onclick="browseShell.restoreSavedSession('${this.escapeHtml(s.id)}')">Restore</button>
              <button class="btn-icon" style="font-size:12px;" title="Delete Session" onclick="browseShell.deleteSavedSession('${this.escapeHtml(s.id)}')">🗑️</button>
            </div>
          </div>
        `;
      }).join('');
    } catch (e) {
      console.warn('Load sessions error:', e);
    }
  }

  async saveCurrentSession() {
    const name = this.newSessionName ? this.newSessionName.value.trim() : '';
    const sessionName = name || `Session ${new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
    try {
      const res = await this.rpc('sessions.save', {
        name: sessionName,
        profile: this.currentProfile || 'user'
      });
      if (res && res.status === 'saved') {
        this.showToast(`Saved workspace: "${res.name}"`);
        if (this.newSessionName) this.newSessionName.value = '';
        this.loadSessionsList();
      }
    } catch (e) {
      console.error('Save session error:', e);
      this.showToast('Failed to save session');
    }
  }

  async restoreSavedSession(sessionId) {
    try {
      const res = await this.rpc('sessions.restore', { id: sessionId, replace: false });
      if (res && res.restored_tabs !== undefined) {
        this.showToast(`Restored ${res.restored_tabs} tab(s) from "${res.name}"`);
        this.closeSessionsModal();
        await this.fetchTabs();
      }
    } catch (e) {
      console.error('Restore session error:', e);
      this.showToast('Failed to restore session');
    }
  }

  async deleteSavedSession(sessionId) {
    try {
      const res = await this.rpc('sessions.delete', { id: sessionId });
      if (res && res.deleted) {
        this.showToast('Session deleted');
        this.loadSessionsList();
      }
    } catch (e) {
      console.error('Delete session error:', e);
    }
  }

  async checkCrashRecovery() {
    try {
      const res = await this.rpc('sessions.load_last_active', { profile: this.currentProfile || 'user' });
      if (res && res.has_session && Array.isArray(res.tabs) && res.tabs.length > 0) {
        this._crashTabs = res.tabs;
        if (this.tabs.length <= 1 && (!this.tabs[0] || this.tabs[0].url === 'https://example.com')) {
          this.showToast(`⚠️ Previous session had ${res.tabs.length} open tab(s). Press Ctrl+S or click Sessions to restore.`);
        }
      }
    } catch (e) {
      console.warn('Crash recovery check failed:', e);
    }
  }

  async checkCrashRecoveryAlert() {
    if (!this.crashRecoveryAlert) return;
    try {
      const res = await this.rpc('sessions.load_last_active', { profile: this.currentProfile || 'user' });
      if (res && res.has_session && Array.isArray(res.tabs) && res.tabs.length > 0) {
        this._crashTabs = res.tabs;
        this.crashRecoveryAlert.classList.remove('hidden');
      } else {
        this.crashRecoveryAlert.classList.add('hidden');
      }
    } catch (_) {
      this.crashRecoveryAlert.classList.add('hidden');
    }
  }

  async restoreCrashSession() {
    if (this._crashTabs && this._crashTabs.length > 0) {
      for (const t of this._crashTabs) {
        await this.createTab(t.url || 'https://example.com', t.profile || 'User');
      }
      this.showToast(`Restored ${this._crashTabs.length} tab(s) from previous session`);
      if (this.crashRecoveryAlert) this.crashRecoveryAlert.classList.add('hidden');
      this.closeSessionsModal();
    } else {
      this.showToast('No crash session found');
    }
  }

  // Tab Deduplication
  async dedupTabs() {
    try {
      const res = await this.rpc('tabs.dedup', {});
      if (res && res.closed_count !== undefined) {
        if (res.closed_count > 0) {
          this.showToast(`🧹 Closed ${res.closed_count} duplicate tab(s)`);
          await this.fetchTabs();
        } else {
          this.showToast('No duplicate tabs found');
        }
      }
    } catch (e) {
      console.error('Dedup error:', e);
      this.showToast('Failed to deduplicate tabs');
    }
  }

  // Prompt Templates
  async fetchPromptTemplates() {
    if (!this.promptTemplatesBar) return;
    try {
      const templates = await this.rpc('prompts.list', {}) || [];
      const dynamicChips = this.promptTemplatesBar.querySelectorAll('.btn-chip-template');
      dynamicChips.forEach(c => c.remove());

      templates.forEach(t => {
        const btn = document.createElement('button');
        btn.className = 'btn-chip btn-chip-template';
        btn.innerHTML = `${this.escapeHtml(t.icon)} ${this.escapeHtml(t.name)}`;
        btn.title = t.prompt_template;
        btn.addEventListener('click', () => {
          if (this.askInput) {
            this.askInput.value = t.prompt_template;
            this.sendAsk();
          }
        });
        this.promptTemplatesBar.appendChild(btn);
      });
    } catch (e) {
      console.warn('Fetch prompt templates error:', e);
    }
  }

  openPromptTemplates() {
    this.sidebarPane.classList.remove('collapsed');
    const tab = document.querySelector('.sidebar-tab[data-tab="intelligence"]');
    if (tab) tab.click();
    this.showToast('Prompt Templates available in quick action chips ✦');
  }

  // User Scripts & Custom Styles
  openUserScriptsModal() {
    if (!this.userscriptsModal) return;
    this.userscriptsModal.classList.remove('hidden');
    if (this.newScriptName) this.newScriptName.value = '';
    if (this.newScriptContent) this.newScriptContent.value = '';
    this.fetchUserScripts();
  }

  closeUserScriptsModal() {
    if (!this.userscriptsModal) return;
    this.userscriptsModal.classList.add('hidden');
  }

  async fetchUserScripts() {
    if (!this.userscriptsList) return;
    try {
      const scripts = await this.rpc('userscripts.list', { profile: this.currentProfile || 'user' }) || [];
      if (scripts.length === 0) {
        this.userscriptsList.innerHTML = '<div class="empty-hint" style="padding:12px;text-align:center;font-size:11px;">No custom scripts or styles added yet.</div>';
        return;
      }
      this.userscriptsList.innerHTML = scripts.map(s => {
        return `
          <div class="userscript-item">
            <div class="userscript-item-left">
              <div class="userscript-item-title">
                <span>${this.escapeHtml(s.name)}</span>
                <span class="userscript-badge ${s.script_type}">${s.script_type}</span>
                <span style="font-size:10px;opacity:0.6;">@ ${this.escapeHtml(s.domain_pattern)}</span>
              </div>
              <span class="userscript-item-meta">${this.escapeHtml(s.content.substring(0, 50))}${s.content.length > 50 ? '...' : ''}</span>
            </div>
            <div style="display:flex;gap:6px;align-items:center;">
              <button class="btn-secondary" style="font-size:11px;padding:2px 8px;" onclick="browseShell.toggleUserScript('${this.escapeHtml(s.id)}', ${!s.enabled})">${s.enabled ? 'Disable' : 'Enable'}</button>
              <button class="btn-icon" style="font-size:11px;opacity:0.6;" title="Delete Snippet" onclick="browseShell.deleteUserScript('${this.escapeHtml(s.id)}')">🗑️</button>
            </div>
          </div>
        `;
      }).join('');
    } catch (e) {
      console.warn('Fetch user scripts error:', e);
    }
  }

  async saveUserScript() {
    const name = this.newScriptName ? this.newScriptName.value.trim() : '';
    const domain = this.newScriptDomain ? this.newScriptDomain.value.trim() : '*';
    const type = this.newScriptType ? this.newScriptType.value : 'css';
    const content = this.newScriptContent ? this.newScriptContent.value.trim() : '';

    if (!name || !content) {
      this.showToast('Please provide a name and code content');
      return;
    }

    try {
      const res = await this.rpc('userscripts.save', {
        profile: this.currentProfile || 'user',
        name,
        domain_pattern: domain || '*',
        script_type: type,
        content
      });
      if (res && res.id) {
        this.showToast(`Saved snippet: ${res.name}`);
        if (this.newScriptName) this.newScriptName.value = '';
        if (this.newScriptContent) this.newScriptContent.value = '';
        this.fetchUserScripts();
      }
    } catch (e) {
      console.error('Save userscript error:', e);
      this.showToast('Failed to save snippet');
    }
  }

  async toggleUserScript(id, enabled) {
    try {
      const res = await this.rpc('userscripts.toggle', { id, enabled });
      if (res && res.toggled) {
        this.showToast(enabled ? 'Snippet enabled' : 'Snippet disabled');
        this.fetchUserScripts();
      }
    } catch (e) {
      console.error('Toggle userscript error:', e);
    }
  }

  async deleteUserScript(id) {
    try {
      const res = await this.rpc('userscripts.delete', { id });
      if (res && res.deleted) {
        this.showToast('Snippet deleted');
        this.fetchUserScripts();
      }
    } catch (e) {
      console.error('Delete userscript error:', e);
    }
  }

  async applyUserScriptsForUrl(url) {
    try {
      const scripts = await this.rpc('userscripts.for_url', { url, profile: this.currentProfile || 'user' }) || [];
      if (scripts.length > 0) {
        console.log(`Applied ${scripts.length} user script(s)/style(s) for ${url}`);
      }
    } catch (_) {}
  }

  // Reading List
  openReadingListModal() {
    if (this.readingListModal) {
      this.readingListModal.classList.remove('hidden');
      this.fetchReadingList();
    }
  }

  closeReadingListModal() {
    if (this.readingListModal) {
      this.readingListModal.classList.add('hidden');
    }
  }

  async fetchReadingList() {
    if (!this.readingListItems) return;
    try {
      const items = await this.rpc('reading_list.list', {
        profile: this.currentProfile || 'default',
        unread_only: this.readingListUnreadOnly
      }) || [];

      if (items.length === 0) {
        this.readingListItems.innerHTML = '<span class="empty-hint" style="font-size:12px;padding:8px 0;">No articles in reading list. Click "+ Save Active Page" to add one.</span>';
        return;
      }

      this.readingListItems.innerHTML = items.map(it => `
        <div class="reading-item">
          <div style="flex:1;min-width:0;">
            <a class="reading-item-title" href="#" data-url="${this.escapeHtml(it.url)}" style="${it.is_read ? 'opacity:0.6;text-decoration:line-through;' : ''}">${this.escapeHtml(it.title || it.url)}</a>
            <div class="reading-item-meta">
              <span>⏱ ${it.reading_time_min} min read</span>
              <span>•</span>
              <span>${it.is_read ? '✔ Read' : 'Unread'}</span>
              <span>•</span>
              <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:200px;">${this.escapeHtml(it.url)}</span>
            </div>
            ${it.excerpt ? `<div class="reading-item-excerpt">${this.escapeHtml(it.excerpt)}</div>` : ''}
          </div>
          <div style="display:flex;gap:6px;align-items:center;margin-left:8px;">
            <button class="btn-secondary chip-action btn-toggle-read" data-id="${it.id}" data-read="${it.is_read}" style="font-size:11px;padding:3px 7px;">
              ${it.is_read ? 'Mark Unread' : 'Mark Read'}
            </button>
            <button class="btn-icon btn-del-reading" data-id="${it.id}" title="Remove from list" style="font-size:12px;padding:3px 6px;">✕</button>
          </div>
        </div>
      `).join('');

      this.readingListItems.querySelectorAll('.reading-item-title').forEach(el => {
        el.addEventListener('click', (e) => {
          e.preventDefault();
          this.navigate(el.dataset.url);
          this.closeReadingListModal();
        });
      });

      this.readingListItems.querySelectorAll('.btn-toggle-read').forEach(btn => {
        btn.addEventListener('click', async () => {
          const id = btn.dataset.id;
          const currentRead = btn.dataset.read === 'true';
          await this.rpc('reading_list.toggle_read', { id, is_read: !currentRead });
          this.fetchReadingList();
        });
      });

      this.readingListItems.querySelectorAll('.btn-del-reading').forEach(btn => {
        btn.addEventListener('click', async () => {
          const id = btn.dataset.id;
          await this.rpc('reading_list.delete', { id });
          this.showToast('Removed from reading list');
          this.fetchReadingList();
        });
      });
    } catch (e) {
      console.error('Fetch reading list error:', e);
    }
  }

  async addCurrentPageToReadingList() {
    const activeTab = this.tabs.find(t => t.id === this.activeTabId);
    if (!activeTab || !activeTab.url || activeTab.url === 'about:blank') {
      this.showToast('No active webpage to save');
      return;
    }
    try {
      const res = await this.rpc('reading_list.add', {
        profile: this.currentProfile || 'default',
        url: activeTab.url,
        title: activeTab.title || activeTab.url,
        excerpt: activeTab.title || 'Saved offline article',
        reading_time_min: 4
      });
      if (res && res.id) {
        this.showToast(`🔖 Saved to reading list: ${res.title}`);
        this.fetchReadingList();
      }
    } catch (e) {
      console.error('Add reading list error:', e);
      this.showToast('Failed to save to reading list');
    }
  }

  // Live Network Monitor
  openNetworkModal() {
    if (this.networkModal) {
      this.networkModal.classList.remove('hidden');
      this.fetchNetworkLog();
    }
  }

  closeNetworkModal() {
    if (this.networkModal) {
      this.networkModal.classList.add('hidden');
    }
  }

  async fetchNetworkLog() {
    if (!this.networkRequestsTbody) return;
    try {
      const summary = await this.rpc('network.summary', {});
      if (summary) {
        if (this.netTotalCount) this.netTotalCount.textContent = summary.total_requests;
        if (this.netBlockedCount) this.netBlockedCount.textContent = summary.blocked_requests;
        if (this.netTotalBytes) {
          const kb = (summary.total_bytes / 1024).toFixed(1);
          this.netTotalBytes.textContent = `${kb} KB`;
        }
      }

      const params = { limit: 100 };
      if (this.networkBlockedOnly) params.blocked = true;

      const requests = await this.rpc('network.list', params) || [];
      if (requests.length === 0) {
        this.networkRequestsTbody.innerHTML = '<tr><td colspan="6" style="text-align:center;padding:16px;color:var(--text-secondary);">No network traffic recorded yet.</td></tr>';
        return;
      }

      this.networkRequestsTbody.innerHTML = requests.map(r => `
        <tr class="network-row ${r.blocked ? 'blocked' : ''}">
          <td style="font-weight:600;">${this.escapeHtml(r.method)}</td>
          <td>${r.blocked ? '<span style="color:#ef4444;font-weight:bold;">BLOCKED</span>' : r.status}</td>
          <td title="${this.escapeHtml(r.url)}">${this.escapeHtml(r.url)}</td>
          <td>${this.escapeHtml(r.resource_type)}</td>
          <td>${r.size_bytes ? (r.size_bytes / 1024).toFixed(1) + ' KB' : '-'}</td>
          <td>${r.duration_ms}ms</td>
        </tr>
      `).join('');
    } catch (e) {
      console.error('Fetch network log error:', e);
    }
  }

  async clearNetworkLog() {
    try {
      await this.rpc('network.clear', {});
      this.showToast('Network log cleared');
      this.fetchNetworkLog();
    } catch (e) {
      console.error('Clear network log error:', e);
    }
  }

  // Log simulated or CDP network request
  async recordNetworkTraffic(url, method = 'GET', status = 200, resource_type = 'fetch', size_bytes = 1024, duration_ms = 40, blocked = false, blocked_reason = null) {
    try {
      await this.rpc('network.log', {
        url,
        method,
        status,
        resource_type,
        size_bytes,
        duration_ms,
        blocked,
        blocked_reason
      });
    } catch (_) {}
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
