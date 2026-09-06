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
    document.getElementById('title').textContent = translations.title;
    document.getElementById('headingAccount').textContent = translations.account;
    document.getElementById('labelEmail').textContent = translations.email;
    document.getElementById('labelPlan').textContent = translations.plan;
    document.getElementById('headingUsage').textContent = translations.usage;
    document.getElementById('headingExtraUsage').textContent = translations.extra_usage;
    document.getElementById('closeBtn').addEventListener('click', () => invoke('close_popup'));
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
        name.placeholder = 'Display name';
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
            testStatus.textContent = `Suggested ${suggested.length}. Rename the display name, then add.`;
        } else if (suggested.length) {
            testStatus.textContent = `Found ${suggested.length} field${suggested.length === 1 ? '' : 's'}. Rename if you want.`;
        } else if (extras.length) {
            testStatus.textContent = 'Nothing obvious. Pick keys below or type a path.';
        } else {
            testStatus.textContent = 'No numeric keys found. Type a path such as quotas.session.';
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
            status.textContent = 'Type a JSON path first, e.g. quotas.session.used';
            return;
        }
        const existing = testBox.querySelector(`input[data-path="${CSS.escape(path)}"]`);
        if (existing) {
            existing.checked = true;
            existing.closest('.field-pick')?.scrollIntoView({ block: 'nearest' });
            status.textContent = 'Already listed — checked.';
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
        status.textContent = `Added ${field.path}`;
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
            await waitMs(180);
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

    function render(state) {
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
            const kindNames = { omp: 'OMP', '9router': '9Router', custom: 'Custom' };
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
                del.textContent = 'Remove';
                del.addEventListener('click', () => {
                    invoke('remove_custom', { id: item.id }).then(render);
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
            navTimers.push(setTimeout(() => tab.classList.add('is-in'), 40 + i * 120));
        });
        if (card) {
            navTimers.push(setTimeout(() => card.classList.add('is-in'), 60));
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
        await waitMs(200);
        document.body.classList.toggle('settings-open', open);
        section.classList.toggle('visible', open);
        btn.classList.toggle('open', open);
        btn.setAttribute('aria-pressed', open ? 'true' : 'false');
        title.textContent = open ? 'Settings' : translations.title;
        try {
            if (open) {
                const state = await invoke('load_settings');
                render(state);
                const selected = section.querySelector('.settings-tab[aria-selected="true"]');
                showTab(selected ? selected.dataset.tab : 'display');
                await settleHeight();
                document.body.classList.remove('view-fade');
                await waitMs(40);
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
            status.textContent = 'Saved.';
            render(state);
        }).catch((err) => {
            status.textContent = String(err);
        });
    });

    function setRemaining(remaining) {
        invoke('set_show_remaining', { remaining }).then((state) => {
            status.textContent = remaining ? 'Showing remaining.' : 'Showing used.';
            render(state);
        }).catch((err) => {
            status.textContent = String(err);
        });
    }

    document.getElementById('quotaUsed').addEventListener('click', () => setRemaining(false));
    document.getElementById('quotaRemaining').addEventListener('click', () => setRemaining(true));
    section.querySelectorAll('.settings-tab').forEach((tab) => {
        tab.addEventListener('click', () => showTab(tab.dataset.tab));
    });

    ['customUrl', 'customHeader', 'customToken'].forEach((id) => {
        document.getElementById(id).addEventListener('input', resetTest);
    });

    document.getElementById('customTest').addEventListener('click', () => {
        const payload = customPayload();
        testBox.hidden = false;
        testStatus.textContent = 'Testing…';
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

    document.getElementById('customForm').addEventListener('submit', (event) => {
        event.preventDefault();
        const payload = customPayload();
        if (!lastTest) {
            status.textContent = 'Test the URL first, then pick the fields to keep.';
            return;
        }
        if (lastTest.error) {
            status.textContent = 'Fix the test error before adding the source.';
            return;
        }
        const fields = selectedFields();
        if (!fields.length) {
            status.textContent = 'Pick at least one field, or add a custom path.';
            return;
        }
        payload.fields = fields;
        invoke('add_custom', { payload }).then((state) => {
            document.getElementById('customForm').reset();
            resetTest();
            status.textContent = 'Source added.';
            render(state);
            showTab('sources');
        }).catch((err) => {
            status.textContent = String(err);
        });
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

    const hasProfile = !!data.profile;
    const accountVisible = hasProfile && !compactHidden('account');
    els.accountSection.classList.toggle('visible', accountVisible);
    if (hasProfile) {
        els.emailValue.textContent = data.profile.email;
        els.emailRow.style.display = data.profile.email ? '' : 'none';
        els.planValue.textContent = data.profile.plan;
        els.planRow.style.display = data.profile.plan ? '' : 'none';
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

    els.headingUsage.style.display = (hasUsage && !accountVisible && !extraVisible) ? 'none' : '';

    updateStatus(data.status);
    if (!document.body.classList.contains('settings-open')) {
        reportHeight();
    }
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
    // Rebuild whenever the field set changes, not only the count - after an
    // account switch the same number of bars can carry different quotas, and
    // an in-place update would show the new values under the old labels.
    const bars = els.usageBars.children;
    const sameFields = entries.length === bars.length
        && entries.every((entry, i) => bars[i].dataset.key === entry.key);

    if (!sameFields) {
        els.usageBars.replaceChildren(...entries.map(createBarElement));
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
        }
    }
}

function createBarElement(entry) {
    const div = document.createElement('div');
    div.className = entry.kind === 'text' ? 'usage-entry usage-note' : 'usage-entry';
    div.dataset.key = entry.key;

    const header = document.createElement('div');
    header.className = 'bar-header';
    const label = document.createElement('span');
    label.textContent = entry.label;
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

    return div;
}

function updateBarElement(div, entry) {
    div.querySelector('.bar-pct').textContent = entry.pct_text;

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
        const duration = 240;
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
