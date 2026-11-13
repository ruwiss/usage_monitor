function invoke(cmd, args) {
    const core = window.__TAURI__ && window.__TAURI__.core;
    if (!core) return Promise.reject(new Error('tauri unavailable'));
    return args === undefined ? core.invoke(cmd) : core.invoke(cmd, args);
}

let els;
let statusState = {};
let translations = {};
let textTimerId = null;
let popupPinned = false;
let compactHide = [];
let lastData = null;

/**
 * Translate a key with optional {placeholder} substitution.
 * Falls back to the provided English text (or the key) if missing.
 */
function tr(key, vars, fallback) {
    let text = (translations && translations[key]) || fallback || key;
    if (vars) {
        for (const [name, value] of Object.entries(vars)) {
            text = text.split('{' + name + '}').join(String(value));
        }
    }
    return text;
}

function applyStaticI18n() {
    document.querySelectorAll('[data-i18n]').forEach((el) => {
        const text = translations[el.dataset.i18n];
        if (text) el.textContent = text;
    });
    document.querySelectorAll('[data-i18n-placeholder]').forEach((el) => {
        const text = translations[el.dataset.i18nPlaceholder];
        if (text) el.placeholder = text;
    });
    document.querySelectorAll('[data-i18n-title]').forEach((el) => {
        const text = translations[el.dataset.i18nTitle];
        if (text) el.title = text;
    });
    document.querySelectorAll('[data-i18n-aria]').forEach((el) => {
        const text = translations[el.dataset.i18nAria];
        if (text) el.setAttribute('aria-label', text);
    });
}

/**
 * Apply the current translation map to every piece of visible chrome.
 * Called once at boot and again after a language change, so the popup does
 * not need a reload to switch languages.
 */
function applyTranslations() {
    applyStaticI18n();
    const setText = (id, value) => {
        const el = document.getElementById(id);
        if (el && value != null) el.textContent = value;
    };
    setText('title', translations.title);
    setText('headingAccount', translations.account);
    setText('labelEmail', translations.email);
    setText('labelPlan', translations.plan);
    setText('headingUsage', translations.usage);
    setText('headingExtraUsage', translations.extra_usage);
    setText('headingInfo', translations.credits_heading);
    setText('infoLink', translations.open_dashboard_link);
    // Settings-panel strings set imperatively in JavaScript (not via data-i18n).
    if (typeof applySettingsText === 'function') applySettingsText();
}

/** Assigned inside setupSettingsPanel so it can re-apply its own labels. */
let applySettingsText = null;

function setupProviderUpdateToggle(toggle, status, onChange) {
    let confirmed = null;
    let pending = false;
    let needsFreshRead = false;
    toggle.addEventListener('change', async () => {
        if (pending || confirmed === null) return;
        pending = true;
        toggle.disabled = true;
        onChange(); // Invalidate settings loads that started before this change.
        try {
            const result = await invoke('set_provider_auto_update', { enabled: toggle.checked });
            if (!result || typeof result.enabled !== 'boolean') throw new Error('Invalid settings response');
            confirmed = result.enabled;
            status.textContent = result.error
                ? tr('set_provider_update_unsaved', { error: result.error }, 'Could not save: {error}. The switch shows the setting for this session.')
                : tr('set_saved', null, 'Saved.');
        } catch (err) {
            // The backend may have saved before an IPC response was lost.
            // Reconcile before presenting the switch as a confirmed setting.
            try {
                const actual = await invoke('get_provider_auto_update');
                confirmed = typeof actual === 'boolean' ? actual : null;
            } catch { confirmed = null; }
            needsFreshRead = confirmed === null;
            status.textContent = tr('set_provider_update_error', { error: String(err) }, 'Could not change automatic updates: {error}');
        } finally {
            toggle.checked = confirmed === true;
            toggle.disabled = confirmed === null;
            pending = false;
        }
    });
    return {
        render(enabled, fresh = false) {
            // This dedicated command owns the preference after hydration. Late
            // source/language responses must not undo a successfully saved toggle.
            if (confirmed !== null || pending || typeof enabled !== 'boolean' || (needsFreshRead && !fresh)) return;
            confirmed = enabled;
            needsFreshRead = false;
            toggle.checked = enabled;
            toggle.disabled = false;
        },
    };
}

/**
 * Set CSS custom properties for theme colors and inject translation strings.
 *
 * Called once by Python after the page loads.  Translations are set as
 * textContent on heading elements so the HTML file stays language-neutral.
 *
 * @param {object} config - { colors, t (translations), app_version, data (initial snapshot) }
 */
function init(config) {
    const s = document.documentElement.style;
    for (const [key, value] of Object.entries(config.colors)) {
        s.setProperty(`--${key.replaceAll('_', '-')}`, value);
    }

    translations = config.t;
    compactHide = config.compact_hide || [];
    applyTranslations();
    document.getElementById('closeBtn').addEventListener('click', () => invoke('close_popup'));
    document.getElementById('infoLink').addEventListener('click', () => {
        if (infoUrl) invoke('open_link', { url: infoUrl }).catch(() => {});
    });
    setupRefreshButton();
    setupGroupNav();
    setupPinButton();
    setupSettingsPanel();
    setupPinnedDrag();

    document.getElementById('appVersion').textContent = config.app_version;

    els = {
        accountSection: document.getElementById('accountSection'),
        emailRow: document.getElementById('emailRow'),
        emailValue: document.getElementById('emailValue'),
        planRow: document.getElementById('planRow'),
        planValue: document.getElementById('planValue'),
        groupNav: document.getElementById('groupNav'),
        groupIndex: document.getElementById('groupIndex'),
        usageSection: document.getElementById('usageSection'),
        headingUsage: document.getElementById('headingUsage'),
        usageBars: document.getElementById('usageBars'),
        extraSection: document.getElementById('extraSection'),
        extraSpent: document.getElementById('extraSpent'),
        extraPct: document.getElementById('extraPct'),
        extraBarContainer: document.getElementById('extraBarContainer'),
        extraFill: document.getElementById('extraFill'),
        statusSection: document.getElementById('statusSection'),
        statusText: document.getElementById('statusText'),
    };
    updateData(config.data);
    requestAnimationFrame(() => {
        document.body.classList.add('open');
        reportHeight();
    });
}


function setupSettingsPanel() {
    const section = document.getElementById('settingsSection');
    const btn = document.getElementById('settingsBtn');
    const title = document.getElementById('title');
    const status = document.getElementById('settingsStatus');
    const list = document.getElementById('sourceList');
    const testBox = document.getElementById('customTestResult');
    const testStatus = document.getElementById('customTestStatus');
    const fieldsBox = document.getElementById('customFields');
    const allWrap = document.getElementById('customAllWrap');
    const allBox = document.getElementById('customAllFields');
    const suggestedTitle = document.getElementById('customSuggestedTitle');
    const rawBox = document.getElementById('customRaw');
    const pathInput = document.getElementById('customPath');
    let lastTest = null;

    function customPayload() {
        return {
            name: document.getElementById('customName').value,
            url: document.getElementById('customUrl').value,
            header: document.getElementById('customHeader').value,
            token: document.getElementById('customToken').value,
        };
    }

    function selectedFields() {
        return Array.from(testBox.querySelectorAll('input[type="checkbox"]:checked')).map((box) => {
            const name = box.closest('.field-pick')?.querySelector('input.field-name')?.value.trim();
            return {
                path: box.dataset.path,
                key: box.dataset.key,
                label: name || box.dataset.label,
            };
        });
    }

    function fieldRow(field, checked) {
        const row = document.createElement('div');
        row.className = 'field-pick';
        const box = document.createElement('input');
        box.type = 'checkbox';
        box.checked = !!checked;
        box.dataset.path = field.path;
        box.dataset.key = field.key;
        box.dataset.label = field.label;
        const meta = document.createElement('span');
        meta.className = 'field-meta';
        const name = document.createElement('input');
        name.type = 'text';
        name.className = 'field-name';
        name.value = field.label || '';
        name.placeholder = tr('set_display_name', null, 'Display name');
        const path = document.createElement('span');
        path.className = 'field-path';
        path.textContent = field.preview ? `${field.path} · ${field.preview}` : field.path;
        meta.append(name, path);
        row.append(box, meta);
        return row;
    }

    function titleFromPath(path) {
        const leaf = path.split('.').pop().replace(/\[|\]|"/g, '');
        return leaf.replace(/[_-]+/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()) || path;
    }

    function slugFromPath(path) {
        return titleFromPath(path).toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, '') || 'quota';
    }

    function previewFromRaw(path) {
        try {
            const raw = JSON.parse(rawBox.textContent);
            const value = path.split('.').reduce((cur, part) => {
                const index = part.match(/^(\w+)\[(\d+)\]$/);
                if (index) return cur?.[index[1]]?.[Number(index[2])];
                return cur?.[part];
            }, raw);
            if (value == null || typeof value === 'object') return '';
            return String(value);
        } catch {
            return '';
        }
    }

    function resetTest() {
        lastTest = null;
        testBox.hidden = true;
        fieldsBox.replaceChildren();
        allBox.replaceChildren();
        allWrap.hidden = true;
        allWrap.open = false;
        suggestedTitle.hidden = false;
        rawBox.textContent = '';
        testStatus.textContent = '';
        pathInput.value = '';
    }

    function showTest(result) {
        lastTest = result;
        testBox.hidden = false;
        const suggested = result.fields || [];
        const suggestedPaths = new Set(suggested.map((field) => field.path));
        const extras = (result.keys || []).filter((field) => !suggestedPaths.has(field.path));
        suggestedTitle.hidden = !suggested.length;
        if (suggested.length && extras.length) {
            testStatus.textContent = tr('set_suggested_n', { n: suggested.length }, 'Suggested {n}. Rename the display name, then add.');
        } else if (suggested.length) {
            testStatus.textContent = suggested.length === 1
                ? tr('set_found_one', null, 'Found 1 field. Rename if you want.')
                : tr('set_found_n', { n: suggested.length }, 'Found {n} fields. Rename if you want.');
        } else if (extras.length) {
            testStatus.textContent = tr('set_nothing_obvious', null, 'Nothing obvious. Pick keys below or type a path.');
        } else {
            testStatus.textContent = tr('set_no_numeric', null, 'No numeric keys found. Type a path such as quotas.session.');
        }
        fieldsBox.replaceChildren(...suggested.map((field) => fieldRow(field, true)));
        allBox.replaceChildren(...extras.map((field) => fieldRow(field, false)));
        allWrap.hidden = extras.length === 0;
        allWrap.open = suggested.length === 0 && extras.length > 0;
        rawBox.textContent = result.raw || '';
        reportHeight();
    }

    function addTypedPath() {
        const path = pathInput.value.trim();
        if (!path) {
            status.textContent = tr('set_type_path_first', null, 'Type a JSON path first, e.g. quotas.session.used');
            return;
        }
        const existing = testBox.querySelector(`input[data-path="${CSS.escape(path)}"]`);
        if (existing) {
            existing.checked = true;
            existing.closest('.field-pick')?.scrollIntoView({ block: 'nearest' });
            status.textContent = tr('set_already_listed', null, 'Already listed — checked.');
            return;
        }
        const known = [...(lastTest?.fields || []), ...(lastTest?.keys || [])].find((field) => field.path === path);
        const field = known || {
            path,
            key: slugFromPath(path),
            label: titleFromPath(path),
            preview: previewFromRaw(path),
        };
        allWrap.hidden = false;
        allWrap.open = true;
        allBox.append(fieldRow(field, true));
        pathInput.value = '';
        status.textContent = tr('set_added_path', { path: field.path }, 'Added {path}');
        reportHeight();
    }


    let tabBusy = false;
    let pendingTab = null;

    function applyTab(id, reveal) {
        section.querySelectorAll('.settings-tab').forEach((tab) => {
            const on = tab.dataset.tab === id;
            tab.setAttribute('aria-selected', on ? 'true' : 'false');
            tab.tabIndex = on ? 0 : -1;
        });
        const canReveal = reveal
            && section.classList.contains('visible')
            && !document.body.classList.contains('view-fade');
        section.querySelectorAll('.settings-card').forEach((card) => {
            const on = card.dataset.panel === id;
            card.classList.toggle('is-active', on);
            card.classList.toggle('is-in', on && canReveal);
            card.toggleAttribute('hidden', !on);
            card.setAttribute('aria-hidden', on ? 'false' : 'true');
        });
    }

    function currentTabId() {
        return section.querySelector('.settings-tab[aria-selected="true"]')?.dataset.tab || '';
    }

    async function showTab(id, animate) {
        if (!id) {
            return;
        }
        if (animate === undefined) {
            animate = section.classList.contains('visible') && !document.body.classList.contains('view-fade');
        }
        if (!animate || viewBusy) {
            applyTab(id, true);
            reportHeight();
            return;
        }
        if (tabBusy) {
            pendingTab = id;
            section.querySelectorAll('.settings-tab').forEach((tab) => {
                const on = tab.dataset.tab === id;
                tab.setAttribute('aria-selected', on ? 'true' : 'false');
                tab.tabIndex = on ? 0 : -1;
            });
            return;
        }
        if (id === currentTabId() && section.querySelector('.settings-card.is-active.is-in')) {
            return;
        }
        tabBusy = true;
        pauseHeightReports();
        section.querySelectorAll('.settings-tab').forEach((tab) => {
            const on = tab.dataset.tab === id;
            tab.setAttribute('aria-selected', on ? 'true' : 'false');
            tab.tabIndex = on ? 0 : -1;
        });
        const outgoing = section.querySelector('.settings-card.is-active');
        if (outgoing && outgoing.dataset.panel !== id) {
            outgoing.classList.remove('is-in');
            await waitMs(90);
        }
        document.body.classList.add('resizing');
        applyTab(id, false);
        await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        await animateHeightTo(contentHeight());
        document.body.classList.remove('resizing');
        if (pendingTab && pendingTab !== id) {
            const next = pendingTab;
            pendingTab = null;
            heightPaused = false;
            tabBusy = false;
            showTab(next, true);
            return;
        }
        pendingTab = null;
        const incoming = section.querySelector('.settings-card.is-active');
        if (incoming) {
            void incoming.offsetWidth;
            incoming.classList.add('is-in');
        }
        heightPaused = false;
        tabBusy = false;
    }

    const CLAUDE_WEB = '__claude_web__';
    let keyProviders = [];
    let claudeWebSignedIn = false;
    let claudeCliSignedIn = false;
    let claudeCliAvailable = true;
    let claudeCliPending = false;
    let claudeCliPoll = null;
    let languages = [];
    let currentLanguage = '';
    let mutSeq = 0;
    let lastState = null;
    const providerUpdates = setupProviderUpdateToggle(document.getElementById('providerAutoUpdate'), status, () => { mutSeq++; });
    // Do not make the emergency off switch wait for source/network discovery.
    const providerInitialSeq = mutSeq;
    invoke('get_provider_auto_update').then((enabled) => {
        if (providerInitialSeq === mutSeq) providerUpdates.render(enabled, true);
    }).catch(() => {});

    function renderKeyProviders(providers) {
        const select = document.getElementById('keyedProvider');
        if (JSON.stringify(providers) === JSON.stringify(keyProviders)) return;
        keyProviders = providers;
        const current = select.value;
        const claudeOpt = document.createElement('option');
        claudeOpt.value = CLAUDE_WEB;
        claudeOpt.textContent = (translations || {}).keyed_claude_option || 'Claude';
        select.replaceChildren(claudeOpt, ...providers.map((p) => {
            const opt = document.createElement('option');
            opt.value = p.kind;
            opt.textContent = p.name;
            return opt;
        }));
        const keep = current && Array.from(select.options).some((o) => o.value === current);
        select.value = keep ? current : (providers.length ? providers[0].kind : CLAUDE_WEB);
        onKeyProviderChange();
    }

    function selectedKeyProvider() {
        const kind = document.getElementById('keyedProvider').value;
        if (kind === CLAUDE_WEB) return undefined;
        return keyProviders.find((p) => p.kind === kind);
    }

    function onKeyProviderChange() {
        const claude = document.getElementById('keyedProvider').value === CLAUDE_WEB;
        const p = selectedKeyProvider();
        const tr = translations || {};
        document.getElementById('claudeWebPanel').hidden = !claude;
        ['keyedKeyRow', 'keyedNameRow', 'keyedActions', 'keyedHelp'].forEach((id) => {
            document.getElementById(id).hidden = claude;
        });
        document.getElementById('keyedKey').placeholder = p ? p.hint : '';
        document.getElementById('keyedKey').required = !claude && !(p && p.key_optional);
        document.getElementById('keyedHelp').textContent = p && p.key_optional ? (tr.keyed_key_optional_hint || '') : '';
        document.getElementById('keyedGetKey').hidden = !(p && p.key_url);
        renderClaudeWeb();
        reportHeight();
    }

    function renderClaudeWeb() {
        const tr = translations || {};
        const action = document.getElementById('claudeWebAction');
        const cancel = document.getElementById('claudeCliCancel');
        const fallback = document.getElementById('claudeWebFallback');
        const signedIn = claudeCliSignedIn || claudeWebSignedIn;
        const help = document.getElementById('claudeWebHelp');
        if (claudeCliPending) {
            help.textContent = tr.claude_cli_waiting || 'Waiting for the browser…';
        } else if (!claudeCliAvailable) {
            help.textContent = tr.claude_cli_missing || 'Claude CLI not found. Sign in inside the app instead.';
        } else if (claudeCliSignedIn) {
            help.textContent = tr.claude_cli_help || 'Signed in through the Claude CLI.';
        } else if (claudeWebSignedIn) {
            help.textContent = tr.claude_web_help || '';
        } else {
            help.textContent = tr.claude_cli_help || '';
        }
        document.getElementById('claudeWebBadge').hidden = !signedIn;
        document.getElementById('claudeWebBadge').textContent = tr.signin_badge || 'Signed in';
        action.hidden = claudeCliPending;
        action.textContent = signedIn
            ? (tr.claude_cli_sign_out || tr.claude_web_sign_out || 'Sign out')
            : (tr.claude_cli_sign_in || 'Sign in with browser');
        action.classList.toggle('primary', !signedIn);
        action.disabled = !claudeCliAvailable && !signedIn;
        cancel.hidden = !claudeCliPending;
        cancel.textContent = tr.claude_cli_cancel || 'Cancel';
        // The in-app web sign-in stays as a fallback; hide it once a CLI login is present.
        fallback.hidden = claudeCliPending || claudeCliSignedIn;
        fallback.textContent = claudeWebSignedIn
            ? (tr.claude_web_sign_out_app || 'Sign out of the app')
            : (tr.claude_cli_use_app || 'Sign in inside the app instead');
    }

    function applyKeyedText() {
        const tr = translations || {};
        const set = (id, key) => { if (tr[key]) document.getElementById(id).textContent = tr[key]; };
        set('keyedTitle', 'keyed_add_heading');
        set('keyedProviderLabel', 'keyed_provider');
        set('keyedKeyLabel', 'keyed_api_key');
        set('keyedNameLabel', 'keyed_name_optional');
        set('keyedGetKey', 'keyed_get_key');
        set('keyedSubmit', 'keyed_test_add');
        const claudeOpt = document.querySelector('#keyedProvider option[value="' + CLAUDE_WEB + '"]');
        if (claudeOpt && tr.keyed_claude_option) claudeOpt.textContent = tr.keyed_claude_option;
        renderClaudeWeb();
    }
    applySettingsText = () => {
        applyStaticI18n();
        applyKeyedText();
        renderLanguageSelect();
    };

    function render(state) {
        mutSeq++;
        renderState(state);
    }

    function renderState(state) {
        lastState = state;
        providerUpdates.render(state.provider_auto_update);
        renderKeyProviders(state.key_providers || []);
        claudeWebSignedIn = (state.sources || []).some((item) => item.id === 'claude-web');
        claudeCliSignedIn = (state.sources || []).some((item) => item.id === 'claude');
        renderClaudeWeb();
        currentLanguage = state.language || '';
        const langSelect = document.getElementById('languageSelect');
        if (langSelect && !langSelect.disabled) langSelect.value = currentLanguage;
        document.getElementById('ninerouterUrl').value = state.ninerouter_url || 'http://localhost:20128';
        const remaining = !!state.show_remaining;
        document.getElementById('quotaUsed').setAttribute('aria-pressed', remaining ? 'false' : 'true');
        document.getElementById('quotaRemaining').setAttribute('aria-pressed', remaining ? 'true' : 'false');
        list.replaceChildren(...(state.sources || []).map((item) => {
            const row = document.createElement('div');
            row.className = 'saved-row';
            const check = document.createElement('label');
            check.className = 'source-check';
            const box = document.createElement('input');
            box.type = 'checkbox';
            box.checked = item.visible !== false;
            box.addEventListener('change', () => {
                invoke('set_source_visible', { id: item.id, visible: box.checked }).then(render).catch((err) => {
                    box.checked = item.visible !== false;
                    status.textContent = String(err);
                });
            });
            const meta = document.createElement('span');
            meta.className = 'saved-meta';
            const name = document.createElement('span');
            name.className = 'saved-name';
            name.textContent = String(item.label || '').replace(/^(OMP|9Router|Custom)\s*(?:\u00b7|Â·|·)\s*/, '') || item.label;
            meta.append(name);
            if (item.detail) {
                const detail = document.createElement('span');
                detail.className = 'saved-url';
                detail.textContent = item.detail;
                meta.append(detail);
            }
            const kindNames = {
                omp: 'OMP',
                '9router': '9Router',
                custom: tr('set_tab_custom', null, 'Custom'),
                keyed: tr('set_kind_key', null, 'Key'),
                'claude-web': tr('set_kind_web', null, 'Web'),
            };
            if (kindNames[item.kind]) {
                const kind = document.createElement('span');
                kind.className = 'source-kind';
                kind.textContent = kindNames[item.kind];
                meta.append(kind);
            }
            check.append(box, meta);
            row.append(check);
            if (item.removable) {
                const del = document.createElement('button');
                del.type = 'button';
                del.textContent = tr('set_remove', null, 'Remove');
                del.addEventListener('click', () => {
                    invoke(item.kind === 'keyed' ? 'remove_keyed' : 'remove_custom', { id: item.id }).then(render);
                });
                row.append(del);
            }
            return row;
        }));
    }

    let navTimers = [];
    let viewBusy = false;

    function clearNavEnter() {
        navTimers.forEach((id) => clearTimeout(id));
        navTimers = [];
        section.classList.remove('nav-enter');
        section.querySelectorAll('.settings-tab, .settings-card').forEach((el) => {
            el.classList.remove('is-in');
        });
    }

    function playNavEnter() {
        clearNavEnter();
        const tabs = Array.from(section.querySelectorAll('.settings-tab')).reverse();
        const card = section.querySelector('.settings-card.is-active');
        void section.offsetWidth;
        tabs.forEach((tab, i) => {
            navTimers.push(setTimeout(() => tab.classList.add('is-in'), 10 + i * 35));
        });
        if (card) {
            navTimers.push(setTimeout(() => card.classList.add('is-in'), 20));
        }
    }

    async function setOpen(open) {
        if (viewBusy || open === document.body.classList.contains('settings-open')) {
            return;
        }
        viewBusy = true;
        pauseHeightReports();
        clearNavEnter();
        document.body.classList.add('view-fade');
        await waitMs(120);
        document.body.classList.toggle('settings-open', open);
        section.classList.toggle('visible', open);
        btn.classList.toggle('open', open);
        btn.setAttribute('aria-pressed', open ? 'true' : 'false');
        title.textContent = open ? tr('menu_settings', null, 'Settings') : translations.title;
        try {
            if (open) {
                // Show the panel right away; source discovery (disk, credential manager, OMP,
                // 9Router) runs in the backend and fills the list when it returns.
                if (lastState) renderState(lastState);
                const selected = section.querySelector('.settings-tab[aria-selected="true"]');
                showTab(selected ? selected.dataset.tab : 'display');
                const seq = mutSeq;
                invoke('get_provider_auto_update').then((enabled) => {
                    if (seq === mutSeq) providerUpdates.render(enabled, true);
                }).catch(() => {});
                invoke('load_settings').then((state) => {
                    if (seq === mutSeq) renderState(state);
                }).catch((err) => { status.textContent = String(err); });
                await settleHeight();
                document.body.classList.remove('view-fade');
                playNavEnter();
            } else if (lastData) {
                updateData(lastData);
                await settleHeight();
                document.body.classList.remove('view-fade');
            } else {
                await settleHeight();
                document.body.classList.remove('view-fade');
            }
        } catch {
            await settleHeight();
            document.body.classList.remove('view-fade');
            if (open) {
                playNavEnter();
            }
        }
        viewBusy = false;
    }

    btn.addEventListener('click', () => {
        setOpen(!document.body.classList.contains('settings-open'));
    });

    document.getElementById('routerForm').addEventListener('submit', (event) => {
        event.preventDefault();
        invoke('save_ninerouter', { url: document.getElementById('ninerouterUrl').value }).then((state) => {
            status.textContent = tr('set_saved', null, 'Saved.');
            render(state);
        }).catch((err) => {
            status.textContent = String(err);
        });
    });

    function setRemaining(remaining) {
        invoke('set_show_remaining', { remaining }).then((state) => {
            status.textContent = remaining
                ? tr('set_showing_remaining', null, 'Showing remaining.')
                : tr('set_showing_used', null, 'Showing used.');
            render(state);
        }).catch((err) => {
            status.textContent = String(err);
        });
    }

    document.getElementById('quotaUsed').addEventListener('click', () => setRemaining(false));
    document.getElementById('quotaRemaining').addEventListener('click', () => setRemaining(true));

    const languageSelect = document.getElementById('languageSelect');

    function renderLanguageSelect() {
        const tr = translations || {};
        const opts = [{ code: '', label: tr.set_language_auto || 'System' }]
            .concat(languages.map(([code, name]) => ({ code, label: name })));
        languageSelect.replaceChildren(...opts.map((o) => {
            const opt = document.createElement('option');
            opt.value = o.code;
            opt.textContent = o.label;
            return opt;
        }));
        languageSelect.value = currentLanguage;
    }

    function loadLanguages() {
        return invoke('list_languages').then((list) => {
            languages = Array.isArray(list) ? list : [];
            renderLanguageSelect();
        }).catch(() => {});
    }

    function applyLanguage(code) {
        currentLanguage = code || '';
        invoke('translations_for', { code: currentLanguage }).then((map) => {
            translations = map || {};
            applyTranslations();
            renderLanguageSelect();
            reportHeight();
        }).catch(() => {});
    }

    languageSelect.addEventListener('change', () => {
        const code = languageSelect.value;
        languageSelect.disabled = true;
        invoke('set_language', { code }).then((state) => {
            status.textContent = tr('set_saved', null, 'Saved.');
            render(state);
            applyLanguage(code);
        }).catch((err) => {
            status.textContent = String(err);
            renderLanguageSelect();
        }).finally(() => {
            languageSelect.disabled = false;
        });
    });
    loadLanguages();

    section.querySelectorAll('.settings-tab').forEach((tab) => {
        tab.addEventListener('click', () => showTab(tab.dataset.tab));
    });

    ['customUrl', 'customHeader', 'customToken'].forEach((id) => {
        document.getElementById(id).addEventListener('input', resetTest);
    });

    document.getElementById('customTest').addEventListener('click', () => {
        const payload = customPayload();
        testBox.hidden = false;
        testStatus.textContent = tr('keyed_testing', null, 'Testing…');
        fieldsBox.replaceChildren();
        allBox.replaceChildren();
        rawBox.textContent = '';
        invoke('test_custom', { payload }).then((result) => {
            showTest(result);
            status.textContent = '';
        }).catch((err) => {
            lastTest = { error: true };
            testStatus.textContent = String(err);
            status.textContent = String(err);
        });
    });

    document.getElementById('customPathAdd').addEventListener('click', addTypedPath);
    pathInput.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') {
            event.preventDefault();
            addTypedPath();
        }
    });

    applyKeyedText();
    invoke('claude_cli_available').then((ok) => {
        claudeCliAvailable = !!ok;
        renderClaudeWeb();
    }).catch(() => {});
    document.getElementById('keyedProvider').addEventListener('change', onKeyProviderChange);
    document.getElementById('keyedGetKey').addEventListener('click', () => {
        const p = selectedKeyProvider();
        if (p && p.key_url) invoke('open_link', { url: p.key_url }).catch(() => {});
    });
    document.getElementById('keyedForm').addEventListener('submit', (event) => {
        event.preventDefault();
        const tr = translations || {};
        const submit = document.getElementById('keyedSubmit');
        const keyInput = document.getElementById('keyedKey');
        submit.disabled = true;
        status.textContent = tr.keyed_testing || 'Testing...';
        invoke('add_keyed', {
            provider: document.getElementById('keyedProvider').value,
            key: keyInput.value,
            name: document.getElementById('keyedName').value,
        }).then((state) => {
            keyInput.value = '';
            document.getElementById('keyedName').value = '';
            status.textContent = tr.keyed_added || 'Provider added.';
            render(state);
        }).catch((err) => {
            status.textContent = String(err);
        }).finally(() => {
            submit.disabled = false;
        });
    });

    let claudeWebPoll = null;
    function claudeWebSignIn() {
        const tr = translations || {};
        invoke('claude_web_sign_in').then(() => {
            status.textContent = tr.claude_web_waiting || 'Complete the sign-in in the Claude window.';
            clearInterval(claudeWebPoll);
            let tries = 0;
            claudeWebPoll = setInterval(() => {
                invoke('load_settings').then((state) => {
                    render(state);
                    if ((state.sources || []).some((item) => item.id === 'claude-web') || ++tries > 450) {
                        clearInterval(claudeWebPoll);
                        if (tries <= 450) status.textContent = tr.claude_web_signed_in || 'Signed in to Claude (web).';
                    }
                }).catch(() => {});
            }, 2000);
        }).catch((err) => { status.textContent = String(err); });
    }
    function claudeWebSignOut() {
        const tr = translations || {};
        clearInterval(claudeWebPoll);
        invoke('claude_web_sign_out').then((state) => {
            render(state);
            status.textContent = tr.claude_web_signed_out || 'Signed out of Claude (web).';
        }).catch((err) => { status.textContent = String(err); });
    }

    function claudeCliStopWaiting() {
        claudeCliPending = false;
        clearInterval(claudeCliPoll);
        renderClaudeWeb();
        reportHeight();
    }

    function claudeCliSignIn() {
        const tr = translations || {};
        invoke('claude_cli_sign_in').then(() => {
            claudeCliPending = true;
            status.textContent = tr.claude_cli_waiting || 'Waiting for the browser…';
            renderClaudeWeb();
            reportHeight();
            // The `claude-login://done` event ends this, but poll as a backstop.
            clearInterval(claudeCliPoll);
            let tries = 0;
            claudeCliPoll = setInterval(() => {
                invoke('load_settings').then((state) => {
                    render(state);
                    if ((state.sources || []).some((item) => item.id === 'claude') || ++tries > 150) {
                        claudeCliStopWaiting();
                    }
                }).catch(() => {});
            }, 2000);
        }).catch((err) => {
            claudeCliStopWaiting();
            status.textContent = String(err);
        });
    }

    function claudeCliSignOut() {
        const tr = translations || {};
        invoke('claude_cli_sign_out').then((state) => {
            render(state);
            status.textContent = tr.claude_cli_signed_out || 'Signed out of Claude.';
        }).catch((err) => { status.textContent = String(err); });
    }

    const eventApi = window.__TAURI__ && window.__TAURI__.event;
    if (eventApi && eventApi.listen) {
        eventApi.listen('claude-login://done', (e) => {
            const tr = translations || {};
            const payload = (e && e.payload) || {};
            claudeCliStopWaiting();
            if (payload.ok) {
                status.textContent = tr.claude_cli_signed_in || 'Signed in through the Claude CLI.';
                invoke('load_settings').then(render).catch(() => {});
            } else if (payload.error) {
                status.textContent = String(payload.error);
            }
        });
    }

    document.getElementById('claudeWebAction').addEventListener('click', () => {
        const signedIn = claudeCliSignedIn || claudeWebSignedIn;
        if (signedIn) {
            if (claudeCliSignedIn) claudeCliSignOut(); else claudeWebSignOut();
        } else if (claudeCliAvailable) {
            claudeCliSignIn();
        } else {
            claudeWebSignIn();
        }
    });
    document.getElementById('claudeCliCancel').addEventListener('click', () => {
        invoke('claude_cli_cancel').catch(() => {});
        claudeCliStopWaiting();
    });
    document.getElementById('claudeWebFallback').addEventListener('click', () => {
        if (claudeWebSignedIn) claudeWebSignOut(); else claudeWebSignIn();
    });
    onKeyProviderChange();

    document.getElementById('customForm').addEventListener('submit', (event) => {
        event.preventDefault();
        const payload = customPayload();
        if (!lastTest) {
            status.textContent = tr('set_test_first', null, 'Test the URL first, then pick the fields to keep.');
            return;
        }
        if (lastTest.error) {
            status.textContent = tr('set_fix_error', null, 'Fix the test error before adding the source.');
            return;
        }
        const fields = selectedFields();
        if (!fields.length) {
            status.textContent = tr('set_pick_one', null, 'Pick at least one field, or add a custom path.');
            return;
        }
        payload.fields = fields;
        invoke('add_custom', { payload }).then((state) => {
            document.getElementById('customForm').reset();
            resetTest();
            status.textContent = tr('set_source_added', null, 'Source added.');
            render(state);
            showTab('sources');
        }).catch((err) => {
            status.textContent = String(err);
        });
    });
}

function setupGroupNav() {
    document.getElementById('groupPrev').addEventListener('click', () => {
        invoke('cycle_group_account', { direction: -1 }).catch(() => {});
    });
    document.getElementById('groupNext').addEventListener('click', () => {
        invoke('cycle_group_account', { direction: 1 }).catch(() => {});
    });
}

let infoUrl = '';
let refreshSpinTimer = 0;

function setRefreshSpinning(on) {
    const btn = document.getElementById('refreshBtn');
    if (btn) btn.classList.toggle('spinning', on);
}

function setupRefreshButton() {
    const btn = document.getElementById('refreshBtn');
    const label = translations.refresh_now || '';
    btn.setAttribute('aria-label', label);
    btn.title = label;
    btn.addEventListener('click', () => {
        if (btn.classList.contains('spinning')) return;
        setRefreshSpinning(true);
        // Stopped by the next usage update; the timer is a safety net for failed refreshes.
        clearTimeout(refreshSpinTimer);
        refreshSpinTimer = setTimeout(() => setRefreshSpinning(false), 15000);
        invoke('refresh_now').catch(() => setRefreshSpinning(false));
    });
}

function setupPinButton() {
    const pinBtn = document.getElementById('pinBtn');

    function render() {
        document.body.classList.toggle('pinned', popupPinned);
        pinBtn.classList.toggle('pinned', popupPinned);
        pinBtn.setAttribute('aria-pressed', popupPinned ? 'true' : 'false');
        pinBtn.setAttribute('aria-label', popupPinned ? translations.unpin_popup : translations.pin_popup);
        pinBtn.title = popupPinned ? translations.unpin_popup : translations.pin_popup;
    }

    pinBtn.addEventListener('click', () => {
        const nextPinned = !popupPinned;
        popupPinned = nextPinned;
        render();
        reapplyData();
        invoke('set_pinned', { pinned: nextPinned }).then((applied) => {
            popupPinned = !!applied;
            render();
            reapplyData();
        }).catch(() => {
            popupPinned = !nextPinned;
            render();
            reapplyData();
        });
    });

    const event = window.__TAURI__ && window.__TAURI__.event;
    if (event && event.listen) {
        event.listen('popup://reset-pin', () => {
            popupPinned = false;
            render();
            reapplyData();
        });
    }

    render();
}

/**
 * Return true if a section or usage bar is hidden by the pinned compact view.
 *
 * Hiding only applies while the popup is pinned; unpinned it always shows
 * everything.  `key` is a section key (account, extra_usage, claude_code,
 * status) or a usage field name (e.g. seven_day_opus).
 */
function compactHidden(key) {
    return popupPinned && compactHide.includes(key);
}

// Re-render the last snapshot so compact hiding takes effect on pin toggle.
function reapplyData() {
    if (lastData) {
        updateData(lastData);
    }
}

function setupPinnedDrag() {
    const header = document.querySelector('header');
    let dragging = false;

    function setDragging(active) {
        dragging = active;
        header.classList.toggle('dragging', active);
    }

    header.addEventListener('mousedown', (event) => {
        if (!popupPinned || event.button !== 0 || event.target.closest('button')) {
            return;
        }
        event.preventDefault();
        setDragging(true);
        invoke('begin_drag').then((started) => {
            setDragging(!!started);
        }).catch(() => {
            setDragging(false);
        });
    });

    document.addEventListener('mousemove', (event) => {
        if (!dragging) {
            return;
        }
        // No button held (e.g. released outside the window): stop dragging.
        if (event.buttons === 0) {
            setDragging(false);
            invoke('end_drag');
            return;
        }
        invoke('drag').catch(() => {});
    });

    document.addEventListener('mouseup', () => {
        if (!dragging) {
            return;
        }
        setDragging(false);
        invoke('end_drag');
    });
}

/**
 * Update all popup sections with fresh data from Python.
 *
 * @param {object} data - Pre-formatted snapshot from _snapshot_to_dict().
 */
function updateData(data) {
    lastData = data;
    if (!data.status || !data.status.refreshing) {
        clearTimeout(refreshSpinTimer);
        setRefreshSpinning(false);
    }

    const hasProfile = !!data.profile;
    const group = data.group;
    const accountVisible = hasProfile && !compactHidden('account');
    els.accountSection.classList.toggle('visible', accountVisible);
    if (hasProfile) {
        els.emailValue.textContent = data.profile.email;
        els.emailRow.style.display = data.profile.email ? '' : 'none';
        els.planValue.textContent = data.profile.plan;
        els.planRow.style.display = data.profile.plan ? '' : 'none';
    }

    els.groupNav.classList.toggle('visible', !!group);
    if (group) {
        els.groupIndex.textContent = `${group.index}/${group.count}`;
    }

    const usage = (data.usage || []).filter((entry) => !compactHidden(entry.key));
    const hasUsage = !!usage.length;
    els.usageSection.classList.toggle('visible', hasUsage);
    if (hasUsage) {
        updateUsageBars(usage);
    }

    const hasExtra = !!data.extra;
    const extraVisible = hasExtra && !compactHidden('extra_usage');
    els.extraSection.classList.toggle('visible', extraVisible);
    if (hasExtra) {
        els.extraSpent.textContent = data.extra.spent_text;
        els.extraPct.style.display = data.extra.has_limit ? '' : 'none';
        els.extraPct.textContent = data.extra.pct_text;
        els.extraBarContainer.style.display = data.extra.has_limit ? '' : 'none';
        els.extraFill.style.width = `${data.extra.fill_pct * 100}%`;
    }

    updateInfo(data.info);

    const navVisible = els.groupNav.classList.contains('visible');
    els.headingUsage.style.display = (hasUsage && !accountVisible && !extraVisible && !navVisible) ? 'none' : '';

    updateStatus(data.status);
    if (!document.body.classList.contains('settings-open')) {
        reportHeight();
    }
}

function updateInfo(info) {
    const section = document.getElementById('infoSection');
    const rowsEl = document.getElementById('infoRows');
    const link = document.getElementById('infoLink');
    const visible = !!info && !compactHidden('info') && ((info.rows && info.rows.length) || info.dashboard_url);
    section.classList.toggle('visible', !!visible);
    if (!visible) return;
    rowsEl.replaceChildren(...(info.rows || []).map((row) => {
        const div = document.createElement('div');
        const dt = document.createElement('dt');
        dt.textContent = row.label;
        const dd = document.createElement('dd');
        dd.textContent = row.value;
        div.append(dt, dd);
        return div;
    }));
    infoUrl = info.dashboard_url || '';
    link.hidden = !infoUrl;
}

/**
 * Update the status footer with live timer data or static text.
 *
 * Live mode (has last_success_time): starts a 1-second interval for
 * the text counter.  Static mode (has text): shows plain text.
 */
function updateStatus(status) {
    if (textTimerId) {
        clearInterval(textTimerId);
        textTimerId = null;
    }

    if (!status) {
        els.statusSection.classList.remove('visible');
        return;
    }

    // Keep the live timer running even when the footer is hidden in compact
    // view, so the stale-dimming of the usage bars still updates.
    els.statusSection.classList.toggle('visible', !compactHidden('status'));

    if (status.last_success_time !== undefined) {
        statusState = {
            lastSuccessTime: status.last_success_time,
            nextPollTime: status.next_poll_time,
            refreshing: status.refreshing,
            error: status.error,
        };
        els.statusSection.classList.toggle('error', !!status.error);
        tickStatusText();
        textTimerId = setInterval(tickStatusText, 1000);
    } else {
        statusState = {};
        els.statusText.textContent = status.text || '';
        els.statusText.title = status.is_error ? (status.text || '') : '';
        els.statusSection.classList.toggle('error', !!status.is_error);
    }
}

/**
 * Build and display the status text from current state.
 *
 * < 60s:  "Updated Xs ago"
 * >= 60s: "Updated Xm ago · Next update in Ym"
 * + refreshing or error appended with · separator
 */
function tickStatusText() {
    if (!statusState.lastSuccessTime) return;

    const now = Date.now() / 1000;
    const secondsAgo = Math.max(0, Math.floor(now - statusState.lastSuccessTime));
    const isStale = !!statusState.nextPollTime && (now > statusState.nextPollTime + 30);
    els.usageSection.classList.toggle('stale', isStale);
    els.extraSection.classList.toggle('stale', isStale);

    const parts = [formatDuration(secondsAgo)];

    if (statusState.refreshing) {
        parts.push(translations.status_refreshing);
    } else if (statusState.error) {
        parts.push(statusState.error);
    } else if (secondsAgo >= 60 && statusState.nextPollTime) {
        const secondsUntil = Math.max(0, Math.floor(statusState.nextPollTime - now));
        if (secondsUntil > 0) {
            parts.push(translations.status_next_update.replace('{duration}', formatCountdown(secondsUntil)));
        }
    }

    els.statusText.textContent = parts.join(' \u00b7 ');
    // Errors are raw API messages that can overflow; reveal the full text on hover.
    els.statusText.title = statusState.error ? els.statusText.textContent : '';
}

/**
 * Format seconds into a localized "Updated Xs ago" / "Updated Xm ago" string.
 */
function formatDuration(totalSeconds) {
    if (totalSeconds < 60) {
        return translations.status_updated_s.replace('{s}', totalSeconds);
    }

    const totalMin = Math.floor(totalSeconds / 60);
    const hours = Math.floor(totalMin / 60);
    const mins = totalMin % 60;

    let duration;
    if (hours > 0) {
        duration = translations.duration_hm.replace('{h}', hours).replace('{m}', mins);
    } else {
        duration = translations.duration_m.replace('{m}', totalMin);
    }
    return translations.status_updated.replace('{duration}', duration);
}

/**
 * Format a countdown in seconds into a localized duration string.
 */
function formatCountdown(totalSeconds) {
    if (totalSeconds < 60) {
        return translations.duration_s.replace('{s}', totalSeconds);
    }

    const totalMin = Math.ceil(totalSeconds / 60);
    const hours = Math.floor(totalMin / 60);
    const mins = totalMin % 60;

    if (hours > 0) {
        return translations.duration_hm.replace('{h}', hours).replace('{m}', mins);
    }
    return translations.duration_m.replace('{m}', totalMin);
}

function updateUsageBars(entries) {
    // An open rename editor owns its label; let the next update re-render.
    if (els.usageBars.querySelector('.editing')) {
        return;
    }
    // Never render the same quota key twice, whatever the source sent.
    const seen = new Set();
    entries = entries.filter((entry) => !seen.has(entry.key) && seen.add(entry.key));
    // Rebuild whenever the field set changes, not only the count - after an
    // account switch the same number of bars can carry different quotas, and
    // an in-place update would show the new values under the old labels.
    const bars = els.usageBars.children;
    const sameFields = entries.length === bars.length
        && entries.every((entry, i) => bars[i].dataset.key === entry.key);
    if (!sameFields) {
        els.usageBars.replaceChildren(...entries.map((entry) => createBarElement(entry)));
        requestAnimationFrame(() => {
            for (let i = 0; i < entries.length; i++) {
                const fill = els.usageBars.children[i].querySelector('.bar-fill');
                if (fill) {
                    fill.style.width = `${entries[i].fill_pct * 100}%`;
                }
            }
        });
    } else {
        for (let i = 0; i < entries.length; i++) {
            updateBarElement(els.usageBars.children[i], entries[i]);
            applyBarState(els.usageBars.children[i], entries[i]);
        }
    }
}

// The tray badge marks the quota the tray icon shows; the backend picks it automatically.
function applyBarState(div, entry) {
    const isTrayActive = entry.kind !== 'text' && entry.tray_active === true;
    div.classList.toggle('icon-active', isTrayActive);
    let badge = div.querySelector('.tray-badge');
    const header = div.querySelector('.bar-header');
    if (isTrayActive && header) {
        if (!badge) {
            badge = document.createElement('span');
            badge.className = 'tray-badge';
            badge.textContent = '\u25cf';
            badge.title = translations.bar_tray_active || '';
            const pct = div.querySelector('.bar-pct');
            header.insertBefore(badge, pct || null);
        }
    } else if (badge) {
        badge.remove();
    }
}

function startBarRename(div, labelSpan, key, fallbackLabel) {
    if (div.classList.contains('editing')) {
        return;
    }
    div.classList.add('editing');
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'bar-rename-input';
    input.value = labelSpan.textContent;
    input.placeholder = fallbackLabel;
    labelSpan.hidden = true;
    labelSpan.after(input);
    input.focus();
    input.select();

    let done = false;
    function finish(commit) {
        if (done) return;
        done = true;
        const name = input.value.trim();
        input.removeEventListener('keydown', onKey);
        input.removeEventListener('blur', onBlur);
        labelSpan.hidden = false;
        div.classList.remove('editing');
        input.remove();
        if (!commit || !name || name === labelSpan.textContent) {
            return;
        }
        labelSpan.textContent = name;
        invoke('rename_bar', { key, name }).catch(() => {
            labelSpan.textContent = fallbackLabel;
        });
    }
    function onKey(event) {
        if (event.key === 'Enter') {
            event.preventDefault();
            finish(true);
        } else if (event.key === 'Escape') {
            event.preventDefault();
            finish(false);
        }
        event.stopPropagation();
    }
    function onBlur() {
        finish(true);
    }
    input.addEventListener('keydown', onKey);
    input.addEventListener('blur', onBlur);
}

function createBarElement(entry) {
    const div = document.createElement('div');
    div.className = entry.kind === 'text' ? 'usage-entry usage-note' : 'usage-entry';
    div.dataset.key = entry.key;

    const header = document.createElement('div');
    header.className = 'bar-header';
    const label = document.createElement('span');
    label.className = 'bar-label';
    label.textContent = entry.label;
    label.title = translations.bar_dblclick_rename || '';
    label.addEventListener('dblclick', (event) => {
        event.stopPropagation();
        startBarRename(div, label, entry.key, entry.label);
    });
    const pct = document.createElement('span');
    pct.className = 'bar-pct';
    pct.textContent = entry.pct_text;
    header.append(label, pct);
    div.append(header);

    if (entry.kind !== 'text') {
        const container = document.createElement('div');
        container.className = 'bar-container';
        const fill = document.createElement('div');
        fill.className = 'bar-fill';
        fill.classList.toggle('warn', entry.warn);
        fill.style.width = '0%';
        container.appendChild(fill);

        for (const pos of entry.dividers) {
            const d = document.createElement('div');
            d.className = 'bar-divider';
            d.style.left = `calc(${pos * 100}% - 1px)`;
            container.appendChild(d);
        }

        if (entry.marker_rel !== null && entry.marker_rel !== undefined) {
            const marker = document.createElement('div');
            marker.className = 'bar-marker';
            marker.style.left = `calc(${entry.marker_rel * 100}% - 1px)`;
            container.appendChild(marker);
        }

        div.append(container);
    }

    if (entry.reset_text) {
        const reset = document.createElement('div');
        reset.className = 'reset-text';
        reset.textContent = entry.reset_text;
        div.appendChild(reset);
    }

    if (entry.runout_text) {
        const runout = document.createElement('div');
        runout.className = 'runout-text';
        runout.textContent = entry.runout_text;
        runout.classList.toggle('urgent', !!entry.runout_urgent);
        div.appendChild(runout);
    }

    // Runs last: needs the header to exist for the tray badge.
    applyBarState(div, entry);
    return div;
}

function updateBarElement(div, entry) {
    div.querySelector('.bar-pct').textContent = entry.pct_text;

    const label = div.querySelector('.bar-label');
    if (label && label.textContent !== entry.label) {
        label.textContent = entry.label;
    }

    const fill = div.querySelector('.bar-fill');
    if (fill) {
        fill.style.width = `${entry.fill_pct * 100}%`;
        fill.classList.toggle('warn', entry.warn);
    }

    const container = div.querySelector('.bar-container');
    if (container) {
        let marker = container.querySelector('.bar-marker');
        if (entry.marker_rel !== null && entry.marker_rel !== undefined) {
            if (!marker) {
                marker = document.createElement('div');
                marker.className = 'bar-marker';
                container.appendChild(marker);
            }
            marker.style.left = `calc(${entry.marker_rel * 100}% - 1px)`;
        } else if (marker) {
            marker.remove();
        }

        for (const d of container.querySelectorAll('.bar-divider')) d.remove();
        for (const pos of entry.dividers) {
            const d = document.createElement('div');
            d.className = 'bar-divider';
            d.style.left = `calc(${pos * 100}% - 1px)`;
            container.appendChild(d);
        }
    }

    let resetEl = div.querySelector('.reset-text');
    if (entry.reset_text) {
        if (!resetEl) {
            resetEl = document.createElement('div');
            resetEl.className = 'reset-text';
            div.appendChild(resetEl);
        }
        resetEl.textContent = entry.reset_text;
    } else if (resetEl) {
        resetEl.remove();
    }

    let runoutEl = div.querySelector('.runout-text');
    if (entry.runout_text) {
        if (!runoutEl) {
            runoutEl = document.createElement('div');
            runoutEl.className = 'runout-text';
            div.appendChild(runoutEl);
        }
        runoutEl.textContent = entry.runout_text;
        runoutEl.classList.toggle('urgent', !!entry.runout_urgent);
    } else if (runoutEl) {
        runoutEl.remove();
    }
}
function maxPopupHeight() {
    const screenH = window.screen.availHeight || window.screen.height || 800;
    return Math.max(160, Math.floor(screenH * 0.9));
}

function contentHeight() {
    const html = document.documentElement;
    const body = document.body;
    const prevHtmlHeight = html.style.height;
    const prevBodyMax = body.style.maxHeight;
    measuringHeight = true;
    html.style.height = 'auto';
    body.style.maxHeight = 'none';
    let bottom = 0;
    for (const el of body.children) {
        if (!(el instanceof HTMLElement)) {
            continue;
        }
        if (getComputedStyle(el).display === 'none') {
            continue;
        }
        bottom = Math.max(bottom, el.offsetTop + el.offsetHeight);
    }
    const pad = parseFloat(getComputedStyle(body).paddingBottom) || 0;
    const height = Math.ceil(bottom + pad);
    html.style.height = prevHtmlHeight;
    body.style.maxHeight = prevBodyMax;
    measuringHeight = false;
    return height;
}

let heightPaused = false;
let heightFrame = 0;
let measuringHeight = false;

function publishHeight() {
    const natural = contentHeight();
    const max = maxPopupHeight();
    document.body.classList.toggle('is-scrollable', natural > max);
    invoke('report_height', { height: Math.min(natural, max) }).catch(() => {});
}

function reportHeight() {
    if (heightPaused || heightFrame || measuringHeight) {
        return;
    }
    heightFrame = requestAnimationFrame(() => {
        heightFrame = 0;
        if (!heightPaused) {
            publishHeight();
        }
    });
}

function pauseHeightReports() {
    heightPaused = true;
    if (heightFrame) {
        cancelAnimationFrame(heightFrame);
        heightFrame = 0;
    }
}

function waitMs(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

function animateHeightTo(target) {
    const start = window.innerHeight || contentHeight();
    const end = Math.min(Math.max(1, target), maxPopupHeight());
    document.body.classList.add('resizing');
    document.body.classList.toggle('is-scrollable', target > maxPopupHeight());
    if (Math.abs(end - start) < 2) {
        invoke('report_height', { height: end }).catch(() => {});
        return Promise.resolve();
    }
    return new Promise((resolve) => {
        const duration = 160;
        const origin = performance.now();
        function frame(now) {
            const t = Math.min(1, (now - origin) / duration);
            const eased = 1 - ((1 - t) ** 3);
            invoke('report_height', { height: Math.round(start + (end - start) * eased) }).catch(() => {});
            if (t < 1) {
                requestAnimationFrame(frame);
            } else {
                document.body.classList.remove('resizing');
                resolve();
            }
        }
        requestAnimationFrame(frame);
    });
}

function settleHeight() {
    return new Promise((resolve) => {
        requestAnimationFrame(() => {
            requestAnimationFrame(() => {
                heightPaused = false;
                publishHeight();
                requestAnimationFrame(resolve);
            });
        });
    });
}

new ResizeObserver(() => {
    if (!measuringHeight) {
        reportHeight();
    }
}).observe(document.body);

async function boot() {
    try {
        const config = await invoke('get_popup_init');
        init(config);
        const event = window.__TAURI__ && window.__TAURI__.event;
        if (event && event.listen) {
            event.listen('usage://update', (e) => updateData(e.payload));
        }
        document.addEventListener('keydown', (e) => {
            if (e.key === 'Escape' && !popupPinned) {
                invoke('close_popup');
            }
        });
        if (!/Windows/i.test(navigator.userAgent)) {
            window.addEventListener('blur', () => {
                if (!popupPinned) {
                    invoke('close_popup');
                }
            });
        }
    } catch (err) {
        console.error(err);
    }
}
if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
} else {
    boot();
}
