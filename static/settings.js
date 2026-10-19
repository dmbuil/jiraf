/* jiraf Settings screen: every setting in jiraf.toml plus the Jira token, as a form.
 * Loaded after app.js (uses its api, esc, toast). The Rust side validates, checks with Jira and writes
 * the files (src/settings.rs); this file only edits a copy of the config and shows what came back. */

/* "How to fill it in" for every field, keyed like the save errors' paths but without list indexes. */
const HELP = {
  'jira.url': 'Base URL of your Jira, e.g. https://jira.example.com',
  'token': 'Jira > Profile > Personal Access Tokens. Stored in .env in the config folder, never shown again.',
  'jira.cache_seconds': 'How long Jira answers are kept in memory. The ↻ button bypasses it.',
  'fields': 'Custom field ids differ per Jira: GET /rest/api/2/field lists them.',
  'fields.extended_status': 'Single-line text: the follow-up note',
  'fields.activity_type': 'Single-select "Activity Type": decides the tab',
  'fields.parent_link': 'Advanced Roadmaps "Parent Link"',
  'fields.epic_link': '"Epic Link"',
  'fields.target_start': 'Advanced Roadmaps "Target start"',
  'fields.target_end': 'Advanced Roadmaps "Target end": editable, drives the "late" check',
  'follow_up.stale_days': 'An Extended Status older than this counts as "no update"',
  'follow_up.done_days': 'Closed issues stay visible this many days',
  'follow_up.ext_max_len': 'Jira single-line text fields cap at 255',
  'follow_up.hierarchy': 'Issue type names, top of the hierarchy first',
  'follow_up.initiative_type': 'The Initiative level in the hierarchy',
  'follow_up.priority_order': "Display order for priorities (Jira's own order if empty)",
  'tabs.name': 'Label shown on the tab',
  'tabs.activity_types': "Whole Activity Type values; case and accents are ignored. The top-most Activity Type in an item's line wins, so a whole tree stays in one tab.",
  'tabs.include_unset': 'Also holds items with no Activity Type anywhere up their tree',
  'tabs.issues': 'Issue keys pinned to this tab (their descendants follow); wins over everything',
  'bubbles.name': 'Label shown in the top bar',
  'bubbles.jql': 'Any JQL; ORDER BY is ignored. Open issues plus those closed in the last done_days are shown.',
  'bubbles.icon_project': 'A Jira project key whose avatar becomes the bubble icon',
  'bubbles.exclude': 'Subtract these bubbles: (this JQL) AND NOT (theirs)',
};
const FIELD_LABELS = [['extended_status', 'Extended Status'], ['activity_type', 'Activity Type'], ['parent_link', 'Parent Link'],
  ['epic_link', 'Epic Link'], ['target_start', 'Target start'], ['target_end', 'Target end']];

/* S: the screen's state. view = what `settings` returned; cfg = the copy being edited; token = a typed
 * replacement (null: keep the saved one); errors / warnings = messages by path, from the last save. */
let S = null;
const setSec = document.getElementById('settings');

const helpKey = path => path.replace(/\.\d+\./, '.');
const hlp = path => HELP[helpKey(path)] ? ` <i class="cf-hlp" title="${esc(HELP[helpKey(path)])}">ⓘ</i>` : '';
const msgs = path => `<span class="cf-err" data-err="${esc(path)}"></span>`;
// a div, not a label: a click on a label's text would "click" its first control (a chip's ×)
const field = (path, label, input) => `<div class="cf-f"><span class="cf-l">${esc(label)}${hlp(path)}</span>${input}${msgs(path)}</div>`;
const text = (path, v, ph = '') => `<input type="text" data-k="${path}" value="${esc(v)}" placeholder="${esc(ph)}" spellcheck="false">`;
const num = (path, v) => `<input type="number" data-k="${path}" data-num value="${esc(v)}" min="0">`;
function chips(path, arr, order) {
  return `<div class="cf-chips">${arr.map((v, i) => `<span class="cf-chip">${esc(v)}${order
    ? `<button type="button" data-mv="${path}|${i}|-1" title="Move left">‹</button><button type="button" data-mv="${path}|${i}|1" title="Move right">›</button>` : ''
  }<button type="button" data-rm="${path}|${i}" title="Remove">×</button></span>`).join('')}<input type="text" class="cf-add" data-add="${path}" placeholder="Add, then Enter" spellcheck="false"></div>`;
}

/* path helpers over S.cfg: "bubbles.2.jql" */
function getPath(path) { return path.split('.').reduce((o, k) => o?.[k], S.cfg); }
function setPath(path, v) {
  const ks = path.split('.'), last = ks.pop(), o = ks.reduce((o, k) => o[k], S.cfg);
  o[last] = v;
}

function itemHead(list, i, n, title) {
  return `<div class="cf-ih"><b>${esc(title)}</b>
    <button type="button" class="tog" data-cmv="${list}|${i}|-1" ${i ? '' : 'disabled'} title="Move up">↑</button>
    <button type="button" class="tog" data-cmv="${list}|${i}|1" ${i < n - 1 ? '' : 'disabled'} title="Move down">↓</button>
    <button type="button" class="tog" data-crm="${list}|${i}">Delete</button></div>`;
}

function renderSettings() {
  const c = S.cfg, v = S.view, fu = c.follow_up;
  const banner = v.parseError
    ? `<div class="banner">jiraf.toml couldn't be read: ${esc(v.parseError)}. Saving replaces it; the unreadable file is kept as jiraf.toml.broken in ${esc(v.configDir)}.</div>`
    : v.firstRun ? `<div class="banner cf-info">Welcome! Fill in your Jira details and at least one bubble, then Save.</div>` : '';
  const token = v.tokenSet && S.token === null
    ? `<div class="cf-row"><span class="cf-dots">••••••••••••</span><button type="button" class="tog" id="cfTokRep">Replace</button></div>`
    : `<input type="password" id="cfTok" value="${esc(S.token || '')}" placeholder="Paste your Jira token" autocomplete="off">`;
  const tabs = c.tabs.map((t, i) => `<div class="cf-item">${itemHead('tabs', i, c.tabs.length, t.name || 'New tab')}
    ${field(`tabs.${i}.name`, 'Name', text(`tabs.${i}.name`, t.name))}
    ${field(`tabs.${i}.activity_types`, 'Activity Types', chips(`tabs.${i}.activity_types`, t.activity_types))}
    <label class="cf-f cf-check"><input type="checkbox" data-k="tabs.${i}.include_unset" data-bool ${t.include_unset ? 'checked' : ''}> Also holds items with no Activity Type${hlp(`tabs.${i}.include_unset`)}</label>
    ${field(`tabs.${i}.issues`, 'Pinned issues', chips(`tabs.${i}.issues`, t.issues))}</div>`).join('');
  const bubbles = c.bubbles.map((b, i) => {
    const others = c.bubbles.filter((o, j) => j !== i && o.id);
    const ex = others.length
      ? others.map(o => `<label class="cf-check"><input type="checkbox" data-ex="${i}" value="${esc(o.id)}" ${(b.exclude || []).includes(o.id) ? 'checked' : ''}> ${esc(o.name)}</label>`).join('')
      : '<span class="cf-muted">No other saved bubbles</span>';
    return `<div class="cf-item">${itemHead('bubbles', i, c.bubbles.length, b.name || 'New bubble')}
      ${field(`bubbles.${i}.name`, 'Name', text(`bubbles.${i}.name`, b.name))}
      ${field(`bubbles.${i}.jql`, 'JQL', `<textarea data-k="bubbles.${i}.jql" spellcheck="false" placeholder="Paste the JQL you worked out in Jira">${esc(b.jql)}</textarea>`)}
      <div class="cf-2">${field(`bubbles.${i}.icon_project`, 'Icon project', `<input type="text" data-k="bubbles.${i}.icon_project" data-opt value="${esc(b.icon_project || '')}" placeholder="e.g. ABC" spellcheck="false">`)}
      <div class="cf-f"><span class="cf-l">Exclude${hlp(`bubbles.${i}.exclude`)}</span><div class="cf-chips">${ex}</div>${msgs(`bubbles.${i}.exclude`)}</div></div></div>`;
  }).join('');
  setSec.innerHTML = `${banner}
    <section class="card"><h2>Jira</h2>
      ${field('jira.url', 'URL', text('jira.url', c.jira.url, 'https://jira.example.com'))}
      ${field('token', 'Token', token)}
      ${v.tokenFromEnv ? '<p class="cf-muted cf-pad">JIRA_TOKEN is set in the environment and wins over the one saved here.</p>' : ''}
      ${field('jira.cache_seconds', 'Cache (seconds)', num('jira.cache_seconds', c.jira.cache_seconds))}
    </section>
    <section class="card"><h2>Fields${hlp('fields')}</h2><div class="cf-2">
      ${FIELD_LABELS.map(([k, l]) => field(`fields.${k}`, l, text(`fields.${k}`, c.fields[k], 'customfield_…'))).join('')}
    </div></section>
    <section class="card"><h2>Follow-up</h2><div class="cf-2">
      ${field('follow_up.stale_days', 'Stale after (days)', num('follow_up.stale_days', fu.stale_days))}
      ${field('follow_up.done_days', 'Closed stay visible (days)', num('follow_up.done_days', fu.done_days))}
      ${field('follow_up.ext_max_len', 'Extended Status max length', num('follow_up.ext_max_len', fu.ext_max_len))}
      ${field('follow_up.initiative_type', 'Initiative type', text('follow_up.initiative_type', fu.initiative_type))}</div>
      ${field('follow_up.hierarchy', 'Hierarchy', chips('follow_up.hierarchy', fu.hierarchy, true))}
      ${field('follow_up.priority_order', 'Priority order', chips('follow_up.priority_order', fu.priority_order, true))}
    </section>
    <section class="card"><h2>Tabs</h2>${msgs('tabs')}${tabs}<div class="cf-pad"><button type="button" class="tog" id="cfAddTab">+ Add tab</button></div></section>
    <section class="card"><h2>Bubbles</h2>${msgs('bubbles')}${bubbles}<div class="cf-pad"><button type="button" class="tog" id="cfAddBubble">+ Add bubble</button></div></section>
    <div class="cf-bar"><span id="cfNote" class="cf-muted"></span><span class="spacer"></span>
      <button type="button" class="tog" id="cfBack" hidden>Back to dashboard</button>
      ${v.firstRun ? '' : '<button type="button" class="tog" id="cfCancel">Cancel</button>'}
      <button type="button" class="tog on" id="cfSave">Save</button></div>`;
  showProblems();
}

/* Messages from the last save, under their fields. */
function showProblems() {
  setSec.querySelectorAll('[data-err]').forEach(el => {
    const p = el.dataset.err, e = S.errors[p] || [], w = S.warnings[p] || [];
    el.textContent = [...e, ...w].join(' · ');
    el.classList.toggle('warn', !e.length && w.length > 0);
  });
}
const byPath = list => list.reduce((m, p) => ((m[p.at] ||= []).push(p.message), m), {});

async function openSettings() {
  let view;
  try { view = await api('settings'); } catch (e) { return toast(esc(e.message), true); }
  // missing lists come back absent (they aren't written when empty)
  for (const b of view.config.bubbles) b.exclude ||= [];
  S = { view, cfg: view.config, token: null, errors: {}, warnings: {} };
  document.body.classList.add('set-open');
  setSec.hidden = false;
  renderSettings();
  window.scrollTo(0, 0);
}
function closeSettings() {
  S = null;
  setSec.hidden = true; setSec.innerHTML = '';
  document.body.classList.remove('set-open');
}

async function saveSettings() {
  // a value typed into a list's box but not confirmed with Enter still counts
  setSec.querySelectorAll('[data-add]').forEach(el => { const v = el.value.trim(); if (v) getPath(el.dataset.add).push(v); el.value = ''; });
  const btn = document.getElementById('cfSave');
  btn.disabled = true; btn.textContent = 'Saving…';
  let r;
  try { r = await api('save_settings', { config: S.cfg, token: S.token || null }); }
  catch (e) { btn.disabled = false; btn.textContent = 'Save'; return toast(esc(e.message), true); }
  S.errors = byPath(r.errors); S.warnings = byPath(r.warnings);
  if (!r.saved) {
    renderSettings();
    setSec.querySelector('.cf-err:not(:empty)')?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    return toast('Not saved: fix the marked fields', true);
  }
  if (r.status?.state === 'failed' || (!r.warnings.length && !r.unverified)) return location.reload();
  // saved, but with something to read: stay, with the saved config (new ids) and the warnings
  const keep = { errors: S.errors, warnings: S.warnings };
  await openSettings();
  Object.assign(S, keep);
  renderSettings();
  document.getElementById('cfNote').textContent = r.unverified ? "Saved, but jiraf couldn't verify it with Jira." : 'Saved. Some bubbles have JQL that Jira rejects (see below).';
  document.getElementById('cfBack').hidden = false;
  document.getElementById('cfCancel')?.setAttribute('hidden', '');
  setSec.querySelector('.cf-err.warn')?.scrollIntoView({ behavior: 'smooth', block: 'center' });
}

/* ---------- editing ---------- */
setSec.addEventListener('input', e => {
  const el = e.target;
  if (el.id === 'cfTok') { S.token = el.value; return; }
  const k = el.dataset.k; if (!k) return;
  let v = el.value;
  if ('bool' in el.dataset) v = el.checked;
  else if ('num' in el.dataset) v = parseInt(v, 10) || 0;
  else if ('opt' in el.dataset) v = v.trim() || null;
  setPath(k, v);
});
setSec.addEventListener('change', e => {
  const el = e.target;
  if (el.dataset.ex !== undefined) {
    const b = S.cfg.bubbles[+el.dataset.ex], set = new Set(b.exclude || []);
    el.checked ? set.add(el.value) : set.delete(el.value);
    b.exclude = [...set];
  }
});
setSec.addEventListener('keydown', e => {
  const el = e.target;
  if (e.key !== 'Enter' || !el.dataset.add) return;
  e.preventDefault();
  const v = el.value.trim(); if (!v) return;
  getPath(el.dataset.add).push(v);
  renderSettings();
  setSec.querySelector(`[data-add="${el.dataset.add}"]`)?.focus();
});
setSec.addEventListener('click', e => {
  const b = e.target.closest('button'); if (!b) return;
  const d = b.dataset, swap = (arr, i, j) => { if (j >= 0 && j < arr.length) [arr[i], arr[j]] = [arr[j], arr[i]]; };
  if (b.id === 'cfSave') return saveSettings();
  if (b.id === 'cfCancel') return closeSettings();
  if (b.id === 'cfBack') return location.reload();
  if (b.id === 'cfTokRep') { S.token = ''; renderSettings(); return document.getElementById('cfTok').focus(); }
  if (b.id === 'cfAddTab') S.cfg.tabs.push({ id: '', name: '', activity_types: [], include_unset: false, issues: [] });
  else if (b.id === 'cfAddBubble') S.cfg.bubbles.push({ id: '', name: '', jql: '', icon_project: null, exclude: [] });
  else if (d.rm) { const [p, i] = d.rm.split('|'); getPath(p).splice(+i, 1); }
  else if (d.mv) { const [p, i, by] = d.mv.split('|'); swap(getPath(p), +i, +i + +by); }
  else if (d.crm) {
    const [list, i] = d.crm.split('|'), [gone] = S.cfg[list].splice(+i, 1);
    // a deleted bubble can't stay excluded by the others
    if (list === 'bubbles' && gone.id) for (const o of S.cfg.bubbles) o.exclude = (o.exclude || []).filter(x => x !== gone.id);
  }
  else if (d.cmv) { const [list, i, by] = d.cmv.split('|'); swap(S.cfg[list], +i, +i + +by); }
  else return;
  // indexes moved: old messages would point at the wrong items
  S.errors = {}; S.warnings = {};
  renderSettings();
});

window.__TAURI__.event.listen('open-settings', () => { if (!S) openSettings(); });
document.getElementById('settingsBtn').onclick = () => { if (!S) openSettings(); };
