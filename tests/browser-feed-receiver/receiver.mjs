// SPDX-License-Identifier: MIT OR Apache-2.0
// TEST ONLY: a disposable receiving host, not a production browser client.
import init, * as core from './wasm/booknook_wasm.js';
import {BookNookFeedImportReviewV1, validateFeedConfigurationV1} from './sdk/feeds.js';
const bytes = value => new TextEncoder().encode(value);
const element = id => document.getElementById(id);
let db, pending, busy = false, sequence = 0, lastRaw = null, lastPreview = null;
let commitBarrier, writes = 0;
const testState = {ready: false, error: null, result: null};
const canonical = raw => core.validate_feed_configuration_v1(bytes(raw));
function displayError(error) {
  const text = error instanceof Error ? error.message : String(error);
  testState.error = text; element('error').textContent = text;
}
function controls() {
  element('request-feeds').disabled = busy || !testState.ready;
  element('confirm-import').disabled = busy || !pending || pending.review.preview().candidate === null;
  element('resolve-conflicts').disabled = busy || !pending || pending.review.preview().conflicts.length === 0;
  element('cancel-import').disabled = !pending && !busy;
}
function closeReview() { pending?.review.close(); pending = undefined; lastPreview = null; element('preview').textContent = ''; element('conflicts').replaceChildren(); }
function readRow() {
  return new Promise((resolve, reject) => {
    const tx = db.transaction('state', 'readonly'), request = tx.objectStore('state').get('configuration');
    let row;
    request.onsuccess = () => { row = request.result; };
    tx.onabort = tx.onerror = () => reject(tx.error || Error('Storage read failed'));
    tx.oncomplete = () => {
      try {
        if (row === undefined) { resolve(null); return; }
        if (!row || typeof row.raw !== 'string' || typeof row.token !== 'string') throw Error('Saved configuration is malformed; it was not reset');
        validateFeedConfigurationV1(core, bytes(row.raw)); resolve({...row});
      } catch (error) { reject(error); }
    };
  });
}
// Actual asynchronous IndexedDB configuration transaction. It does not pretend
// to implement FeedCacheStoreV1's separate synchronous cache-commit interface.
function compareAndSave(expected, candidateRaw) {
  canonical(candidateRaw);
  return new Promise((resolve, reject) => {
    const tx = db.transaction('state', 'readwrite'), store = tx.objectStore('state'), request = store.get('configuration');
    let result = {accepted: false, changed: false};
    request.onsuccess = () => {
      const actual = request.result;
      const matches = expected === null ? actual === undefined : actual?.token === expected.token && actual?.raw === expected.raw;
      if (!matches) return;
      if (actual && canonical(actual.raw) === canonical(candidateRaw)) { result = {accepted: true, changed: false}; return; }
      store.put({raw: candidateRaw, token: crypto.randomUUID()}, 'configuration');
      result = {accepted: true, changed: true};
    };
    tx.onabort = tx.onerror = () => reject(tx.error || Error('Atomic configuration save failed'));
    tx.oncomplete = () => { if (result.changed) writes++; resolve(result); };
  });
}
async function renderSaved() {
  const row = await readRow(); element('saved').textContent = row?.raw ?? 'No saved configuration'; return row;
}
function renderReview() {
  const preview = pending.review.preview(); lastPreview = structuredClone(preview);
  element('preview').textContent = JSON.stringify(preview.candidate, null, 2);
  element('conflicts').replaceChildren();
  for (const conflict of preview.conflicts) {
    const label = document.createElement('label'), caption = document.createElement('span'), select = document.createElement('select');
    caption.textContent = `Conflicting ${conflict.target}: ${conflict.id}
Current: ${JSON.stringify(conflict.current)}
Incoming: ${JSON.stringify(conflict.incoming)}`;
    select.dataset.target = conflict.target; select.dataset.id = conflict.id;
    for (const [value, title] of [['', 'Choose explicitly'], ['current', 'Keep current'], ['incoming', 'Use incoming']]) {
      const option = document.createElement('option'); option.value = value; option.textContent = title; select.appendChild(option);
    }
    label.append(caption, select); element('conflicts').appendChild(label);
  }
  controls();
}
async function receive(raw) {
  if (busy) throw Error('Another receiver operation is active');
  if (typeof raw !== 'string' || bytes(raw).length === 0 || bytes(raw).length > 262144) throw Error('Expected bounded original UTF-8 settings');
  busy = true; const own = ++sequence; closeReview(); lastRaw = raw; testState.error = null; element('error').textContent = ''; controls();
  try {
    const row = await readRow(); if (own !== sequence) throw Error('Import canceled');
    if (!row) throw Error('Local configuration is unavailable');
    const incoming = bytes(raw); // Preserve this exact array through resolve/confirm.
    const review = BookNookFeedImportReviewV1.prepare(core, bytes(row.raw), incoming);
    pending = {review, incoming, row, sequence: own};
    renderReview(); element('status').textContent = 'Review received settings. Nothing saved or activated.';
    return review.preview();
  } catch (error) { displayError(error); throw error; }
  finally { busy = false; controls(); }
}
async function resolveChoices(choices) {
  if (busy || !pending) throw Error('No available import review');
  busy = true; const held = pending; controls();
  try {
    const row = await readRow();
    if (held !== pending || held.sequence !== sequence || !row || row.token !== held.row.token || row.raw !== held.row.raw) throw Error('Storage changed; start a fresh review');
    const review = held.review.resolve(bytes(row.raw), held.incoming, bytes(JSON.stringify(choices)));
    held.review.close(); pending = {...held, review}; renderReview();
    return review.preview();
  } catch (error) { displayError(error); throw error; }
  finally { busy = false; controls(); }
}
async function confirmImport() {
  if (busy || !pending) throw Error('No available import review');
  busy = true; const held = pending; controls();
  try {
    const row = await readRow();
    if (held !== pending || held.sequence !== sequence || !row || row.token !== held.row.token || row.raw !== held.row.raw) throw Error('Storage changed; start a fresh review');
    const candidate = held.review.confirm(bytes(row.raw), held.incoming);
    if (commitBarrier) { const barrier = commitBarrier; barrier.entered(); await barrier.release; }
    if (held !== pending || held.sequence !== sequence) throw Error('Import canceled');
    const result = await compareAndSave(row, JSON.stringify(candidate));
    if (!result.accepted) throw Error('Storage changed; start a fresh review');
    testState.result = result; closeReview(); await renderSaved();
    element('status').textContent = result.changed ? 'Saved locally. All sources remain inactive.' : 'Settings already match; no write or activation.';
    return result;
  } catch (error) { displayError(error); throw error; }
  finally { busy = false; controls(); }
}
function cancelImport() { sequence++; closeReview(); element('status').textContent = 'Import canceled. Saved settings unchanged.'; controls(); }
element('request-feeds').addEventListener('click', () => {
  if (busy) return;
  const provider = window.castaliaWallet;
  if (!provider?.selectFeeds || !provider?.getCapabilities) { displayError(Error('Selected feed capability is unavailable')); return; }
  busy = true; const own = ++sequence; closeReview(); controls(); testState.error = null; element('error').textContent = '';
  void (async () => {
    const capabilities = await provider.getCapabilities();
    if (own !== sequence) throw Error('Import canceled');
    if (!capabilities.includes('selected_feeds_v1')) throw Error('Selected feed capability is unavailable');
    const raw = await provider.selectFeeds();
    if (own !== sequence) throw Error('Import canceled');
    busy = false; return receive(raw);
  })().catch(displayError).finally(() => {busy = false; controls();});
});
element('confirm-import').addEventListener('click', () => { void confirmImport().catch(() => {}); });
element('resolve-conflicts').addEventListener('click', () => {
  const choices = [...element('conflicts').querySelectorAll('select')].map(select => ({target: select.dataset.target, id: select.dataset.id, choice: select.value}));
  void resolveChoices(choices).catch(() => {});
});
element('cancel-import').addEventListener('click', cancelImport);
window.addEventListener('pagehide', () => { cancelImport(); db?.close(); });
const ready = (async () => {
  await init({module_or_path: new URL('./wasm/booknook_wasm_bg.wasm', import.meta.url)});
  db = await new Promise((resolve, reject) => {
    const request = indexedDB.open('booknook-selected-feed-receiver-fixture-v1', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('state');
    request.onerror = () => reject(request.error); request.onblocked = () => reject(Error('Fixture database is blocked'));
    request.onsuccess = () => { const connection = request.result; connection.onversionchange = () => connection.close(); resolve(connection); };
  });
  const row = await readRow();
  if (!row) {
    const result = await compareAndSave(null, core.empty_feed_configuration_v1());
    if (!result.accepted && !(await readRow())) throw Error('Initial configuration was not retained');
  }
  await renderSaved(); testState.ready = true; element('status').textContent = 'Ready. No source has been contacted.'; controls();
})();
ready.catch(displayError);
// Explicitly test-only diagnostics and deterministic race barrier. Never package
// this module as a production client or expose these hooks in a runtime app.
window.feedReceiverTest = {
  ready,
  get state() {return structuredClone({...testState, raw: lastRaw, preview: lastPreview, writes, pending: !!pending, activations: [], providerInstalled: false});},
  read: async () => {await ready; const row = await readRow(); return {...row, config: row ? JSON.parse(row.raw) : null};},
  receive: async raw => {await ready; return receive(raw);}, resolve: resolveChoices, confirm: confirmImport, cancel: cancelImport,
  seedConfiguration: async raw => {await ready; const row = await readRow(); const result = await compareAndSave(row, canonical(raw)); if (!result.accepted) throw Error('Fixture seed CAS conflicted'); await renderSaved(); return result;},
  armCommitBarrier() {
    if (commitBarrier) throw Error('Barrier already armed');
    let entered, release; const reached = new Promise(resolve => {entered = resolve;});
    const waiting = new Promise(resolve => {release = resolve;});
    commitBarrier = {entered, reached, release: waiting, releaseNow: release};
  },
  waitCommitBarrier: () => {if (!commitBarrier) throw Error('No barrier'); return commitBarrier.reached;},
  releaseCommitBarrier() {if (!commitBarrier) throw Error('No barrier'); const held = commitBarrier; commitBarrier = undefined; held.releaseNow();},
};
