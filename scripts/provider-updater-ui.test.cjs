const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

function fixture(invoke) {
    const context = vm.createContext({
        window: { __TAURI__: { core: { invoke } } },
        document: { readyState: 'loading', body: {}, addEventListener() {} },
        ResizeObserver: class { observe() {} },
        console,
        setTimeout,
        clearTimeout,
    });
    vm.runInContext(fs.readFileSync(path.join(__dirname, '../ui/popup.js'), 'utf8'), context);
    const events = {};
    const toggle = { checked: false, disabled: true, addEventListener(name, handler) { events[name] = handler; } };
    const status = { textContent: '' };
    let mutations = 0;
    const control = context.setupProviderUpdateToggle(toggle, status, () => { mutations++; });
    return { toggle, status, control, change: () => events.change(), mutations: () => mutations };
}

test('loads the persisted switch and saves one click; stale snapshots cannot undo it', async () => {
    let complete;
    let calls = 0;
    const f = fixture((name, args) => {
        calls++;
        assert.equal(name, 'set_provider_auto_update');
        assert.equal(args.enabled, false);
        return new Promise((resolve) => { complete = resolve; });
    });
    assert.equal(f.toggle.disabled, true);
    f.control.render(true);
    assert.equal(f.toggle.checked, true);
    f.toggle.checked = false;
    const pending = f.change();
    assert.equal(f.toggle.disabled, true);
    f.control.render(true);
    await f.change(); // Duplicate event while a save is pending.
    assert.equal(calls, 1);
    complete({ enabled: false, error: null });
    await pending;
    f.control.render(true); // A late response from another settings mutation.
    assert.equal(f.toggle.checked, false);
    assert.equal(f.toggle.disabled, false);
    assert.equal(f.mutations(), 1);
});

test('a disk failure keeps disabling effective and explains session-only behavior', async () => {
    const f = fixture(async () => ({ enabled: false, error: 'read-only settings' }));
    f.control.render(true);
    f.toggle.checked = false;
    await f.change();
    assert.equal(f.toggle.checked, false);
    assert.match(f.status.textContent, /read-only settings/);
    assert.match(f.status.textContent, /session/);
});

test('failed enabling reflects the backend state instead of claiming success', async () => {
    const f = fixture(async () => ({ enabled: false, error: 'invalid settings JSON' }));
    f.control.render(false);
    f.toggle.checked = true;
    await f.change();
    assert.equal(f.toggle.checked, false);
    assert.match(f.status.textContent, /invalid settings JSON/);
});

test('a lost save response is reconciled with a fresh backend read', async () => {
    const calls = [];
    const f = fixture(async (name) => {
        calls.push(name);
        if (name === 'set_provider_auto_update') throw new Error('response lost');
        return false;
    });
    f.control.render(true);
    f.toggle.checked = false;
    await f.change();
    assert.deepEqual(calls, ['set_provider_auto_update', 'get_provider_auto_update']);
    assert.equal(f.toggle.checked, false);
    assert.equal(f.toggle.disabled, false);
    assert.match(f.status.textContent, /response lost/);
});

test('an unconfirmed state stays disabled until settings can be loaded again', async () => {
    const f = fixture(async () => { throw new Error('IPC unavailable'); });
    f.control.render(true);
    f.toggle.checked = false;
    await f.change();
    assert.equal(f.toggle.disabled, true);
    f.control.render(true); // Cached source settings cannot confirm an unknown state.
    assert.equal(f.toggle.disabled, true);
    f.control.render(false, true);
    assert.equal(f.toggle.disabled, false);
    assert.equal(f.toggle.checked, false);
});
