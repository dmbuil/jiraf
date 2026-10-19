// jiraf front-end: renders /api/data and sends edits back through the jiraf server (which talks to Jira).
'use strict';

/* ---------- data (filled by load()) ---------- */
let D, FU, TODAY, today, TYPE_ORDER, PRI_RANK = {}, PRI_ICON = {}, BUB = {}, STREAMS = [];
let items = [], byKey = {}, children = {};
const ALL = '*';
const edited = {};          // key -> Set of fields changed in this session (visual marker only)

/* ---------- helpers ---------- */
const isDone = i => i.sc === 'done';
const blocked = i => /blocked/i.test(i.st);
const days = d => Math.round((today - new Date(d + 'T00:00:00')) / 864e5);
const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const title = s => s ? s.toLowerCase().replace(/(^|[\s(/-])\S/g, c => c.toUpperCase()) : '';
const fmt = d => d ? new Date(d + 'T00:00:00').toLocaleDateString('en-GB', { day: '2-digit', month: 'short', year: '2-digit' }) : '';
const ddmm = d => String(d.getDate()).padStart(2, '0') + '/' + String(d.getMonth() + 1).padStart(2, '0');
// Freshness of the Extended Status = last time the field itself changed (changelog), whatever the text says.
const extAge = i => i.extAt ? days(i.extAt) : null;
const freshness = i => isDone(i) ? 0 : (extAge(i) ?? (i.upd ? days(i.upd) : 0));
const late = i => !isDone(i) && i.te && days(i.te) > 0;
const overdue = i => !isDone(i) && i.due && days(i.due) > 0;
const stale = i => !isDone(i) && freshness(i) > FU.stale_days;
const recentDone = i => isDone(i) && i.res && days(i.res) <= FU.done_days;
const stClass = (name, cat) => /blocked/i.test(name) ? 's-block' : cat === 'done' ? 's-done' : cat === 'indeterminate' ? 's-prog' : 's-todo';
const ST_RANK = i => blocked(i) ? 0 : i.sc === 'indeterminate' ? 1 : i.sc === 'new' ? 2 : 3;
const typeIcon = t => D.types[t] ? `<img class="ti" src="${esc(D.types[t])}" alt="${esc(t)}" title="${esc(t)}">` : `<span class="ti" title="${esc(t)}">${esc(t[0] || '?')}</span>`;
const priIcon = p => PRI_ICON[p] ? `<img class="pi" src="${esc(PRI_ICON[p])}" alt="">` : '';
const bubIcon = b => b.icon ? `<img src="${esc(b.icon)}" alt="">` : `<span class="pav ini">${esc(b.name.slice(0, 2).toUpperCase())}</span>`;
function ancestors(i) { const c = []; let p = byKey[i.p]; const seen = new Set(); while (p && !seen.has(p.k)) { seen.add(p.k); c.unshift(p); p = byKey[p.p]; } return c; }
const mark = (i, f) => (edited[i.k] ??= new Set()).add(f);
const isEd = (i, f) => edited[i.k]?.has(f) ? 'edited' : '';
const keyLink = i => `<a class="key" href="${esc(D.jiraBase)}/browse/${esc(i.k)}" target="_blank">${esc(i.k)}</a>`;

// Case- and accent-insensitive form ("Evolucion tecnologica" = "Evolución Tecnológica"): Activity Types and the filter box.
const norm = v => (v || '').normalize('NFD').replace(/\p{M}/gu, '').trim().toLowerCase();
function streamOf(i) {
  // root first: the top-most Activity Type wins, so a whole tree (Initiative and below) lives in one tab
  const line = [...ancestors(i), i];
  const pinned = STREAMS.find(s => line.some(a => s.issues.includes(a.k)));
  if (pinned) return pinned.id;
  const at = norm(line.find(a => a.at)?.at);
  for (const s of STREAMS) if (at ? s.activity_types.includes(at) : s.include_unset) return s.id;
  return 'other';
}
// Plain Jira labels under the summary: the first LBL_MAX as chips, the rest behind a "+N" chip with a hover list.
const LBL_MAX = 2;
// Each label chip opens a Jira search for `labels = "<label>"` in a new tab.
const lblJql = l => `labels = "${l.replace(/["\\]/g, '\\$&')}"`;
const lblChip = l => `<a class="lbl" href="${esc(D.jiraBase)}/issues/?jql=${encodeURIComponent(lblJql(l))}" target="_blank" title="Search Jira: ${esc(lblJql(l))}">${esc(l)}</a>`;
const labels = i => !i.lb?.length ? '' : `<span class="lbls">${i.lb.slice(0, LBL_MAX).map(lblChip).join('')}${i.lb.length > LBL_MAX ? `<span class="lbl more" data-lb="${esc(i.k)}" tabindex="0">+${i.lb.length - LBL_MAX}</span>` : ''}</span>`;
const isTreeTab = t => t !== 'att';

/* ---------- backend calls (Tauri commands; errors arrive as their message) ---------- */
const api = (cmd, args = {}) => window.__TAURI__.core.invoke(cmd, args)
  .catch(e => { throw new Error(typeof e === 'string' ? e : e?.message || String(e)); });

/* ---------- state ---------- */
let state = { proj: ALL, tab: null, q: '', types: new Set(), done: false, sort: null, collapsed: new Set() };
const scopeItems = () => state.proj === ALL ? items : items.filter(i => i.bubbles.includes(state.proj));
function readHash() {
  const p = new URLSearchParams(location.hash.slice(1)), w = p.get('p'), t = p.get('t');
  state.proj = w && (BUB[w] || w === ALL) ? w : ALL;
  state.tab = t && (t === 'att' || t === 'other' || STREAMS.some(s => s.id === t)) ? t : STREAMS[0].id;
}
function writeHash() { history.replaceState(null, '', `#p=${encodeURIComponent(state.proj)}${state.tab !== STREAMS[0].id ? '&t=' + state.tab : ''}`); }

/* ---------- cells ---------- */
function statusCell(i) {
  const pill = `<span class="st ${stClass(i.st, i.sc)}">${esc(i.st)}</span>`;
  return i.ctx ? pill : `<button class="ed tight ${isEd(i, 'status')}" data-menu="status" data-k="${esc(i.k)}" title="Change status">${pill}</button>`;
}
function priCell(i) {
  if (!i.pr) return '';
  return i.ctx ? `<span class="pr" title="${esc(i.pr)}">${priIcon(i.pr)}</span>`
    : `<button class="ed ico ${isEd(i, 'priority')}" data-menu="priority" data-k="${esc(i.k)}" title="${esc(i.pr)}. Click to change"><span class="pr">${priIcon(i.pr) || esc(i.pr)}</span></button>`;
}
function tgtCell(i) {
  if (i.ctx) return '';
  const t = i.te;
  // the "Nd late" warning goes on its own line under the date
  const lab = t ? `<span class="tgt ${late(i) ? 'bad' : ''}">${fmt(t)}${late(i) ? `<span class="late">${days(t)}d late</span>` : ''}</span>` : '<span class="tgt none">—</span>';
  return `<button class="ed ${isEd(i, 'target')}" data-menu="target" data-k="${esc(i.k)}" title="Target end${i.ts ? ` (start ${fmt(i.ts)})` : ''}">${lab}<span class="caret">▼</span></button>`;
}
// Jira's own Due date: editable, red once it has passed on an open issue.
function dueCell(i) {
  if (i.ctx) return '';
  const lab = i.due ? `<span class="tgt ${overdue(i) ? 'bad' : ''}">${fmt(i.due)}</span>` : '<span class="tgt none">—</span>';
  return `<button class="ed ${isEd(i, 'due')}" data-menu="due" data-k="${esc(i.k)}" title="Due date${overdue(i) ? `: ${days(i.due)}d overdue` : ''}">${lab}<span class="caret">▼</span></button>`;
}
function ageChip(i) {
  if (isDone(i)) return `<span class="age a-fresh">closed ${fmt(i.res)}</span>`;
  if (!i.ext) return `<span class="age a-none" title="Empty. Last Jira update ${fmt(i.upd)}">—</span>`;
  const a = extAge(i);
  if (a == null) return `<span class="age a-none" title="Never edited since issue creation">orig.</span>`;
  return `<span class="age ${a <= 7 ? 'a-fresh' : a <= FU.stale_days ? 'a-warn' : 'a-old'}" title="Edited ${fmt(i.extAt)}${i.extBy ? ` by ${esc(title(i.extBy))}` : ''}">${a === 0 ? 'today' : a + 'd'}</span>`;
}
function extCell(i) {
  if (i.ctx) return '';
  return `<div class="ext ${isEd(i, 'ext')}" data-edit="${esc(i.k)}" title="Click to edit Extended Status">${ageChip(i)}<span class="txt">${i.ext ? esc(i.ext) : '<i style="color:var(--muted)">No status note</i>'}</span><span class="pen">✎</span></div>`;
}
// Avatar of a user: Jira's picture when Jira hosts it, else initials on a colour derived from the name.
function avatar(name, url) {
  if (!name) return `<span class="av none">?</span>`;
  if (url) return `<img class="av" src="${esc(url)}" alt="">`;
  const ini = title(name).split(/\s+/).filter(Boolean).slice(0, 2).map(w => w[0]).join('').toUpperCase();
  const hue = [...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);
  return `<span class="av ini" style="--h:${hue}">${esc(ini)}</span>`;
}
function whoCell(i) {
  const who = i.a ? title(i.a) : 'Unassigned';
  return i.ctx ? `<span class="who-av" title="${esc(who)}">${avatar(i.a, i.av)}</span>`
    : `<button class="ed ico ${isEd(i, 'assignee')}" data-menu="assignee" data-k="${esc(i.k)}" title="${esc(who)}: click to change" aria-label="Assignee: ${esc(who)}">${avatar(i.a, i.av)}</button>`;
}
const cmBtn = i => i.ctx ? '' : i.nc
  ? `<button class="cm" data-cm="${esc(i.k)}" aria-label="${i.nc} comments, show the last one">💬<span>${i.nc}</span></button>`
  : `<span class="cm none" title="No comments">💬</span>`;

/* ---------- bubbles + header ---------- */
function renderTeam() {
  const chip = (k, label, its, av) => {
    const o = its.filter(x => !isDone(x)).length, b = its.filter(blocked).length, s = its.filter(stale).length;
    return `<button class="member ${k === state.proj ? 'on' : ''}" data-p="${esc(k)}">${av}<span class="nm">${esc(label)}</span>
      <span class="badge b-grey" title="open">${o}</span>${b ? `<span class="badge b-red" title="blocked">${b}</span>` : ''}${s ? `<span class="badge b-amber" title="Extended Status not updated in ${FU.stale_days}+ days">${s}</span>` : ''}</button>`;
  };
  document.getElementById('team').innerHTML = chip(ALL, 'All', items, '<span class="pav all">∗</span>')
    + D.bubbles.map(b => chip(b.id, b.name, items.filter(i => i.bubbles.includes(b.id)), bubIcon(b))).join('');
  document.querySelectorAll('.member').forEach(b => b.onclick = () => select(b.dataset.p));
  updateScrl();   // first: showing the arrows narrows the strip
  // centre the selected bubble, so it never sits under an edge fade
  const on = document.querySelector('.member.on');
  if (on) team.scrollLeft = on.offsetLeft - team.offsetLeft - (team.clientWidth - on.offsetWidth) / 2;
  updateScrl();
}
/* ‹ › buttons and mouse wheel for the bubble strip (horizontal scrolling is awkward with a plain mouse) */
const team = document.getElementById('team');
function updateScrl() {
  const max = team.scrollWidth - team.clientWidth;
  document.getElementById('scrl').hidden = max <= 1;
  document.getElementById('scrlL').disabled = team.scrollLeft <= 1;
  document.getElementById('scrlR').disabled = team.scrollLeft >= max - 1;
  team.classList.toggle('more-l', team.scrollLeft > 1);
  team.classList.toggle('more-r', team.scrollLeft < max - 1);
}
const scrollTeam = dir => team.scrollBy({ left: dir * Math.max(120, team.clientWidth * 0.7), behavior: 'smooth' });
document.getElementById('scrlL').onclick = () => scrollTeam(-1);
document.getElementById('scrlR').onclick = () => scrollTeam(1);
team.addEventListener('scroll', updateScrl, { passive: true });
addEventListener('resize', updateScrl);
team.addEventListener('wheel', e => {
  if (Math.abs(e.deltaY) <= Math.abs(e.deltaX) || team.scrollWidth <= team.clientWidth) return;
  e.preventDefault(); team.scrollLeft += e.deltaY;
}, { passive: false });
function buckets() {
  const its = scopeItems(), open = its.filter(i => !isDone(i));
  return {
    open,
    B: open.filter(blocked).sort((a, b) => freshness(b) - freshness(a)),
    L: open.filter(i => late(i) && !blocked(i)).sort((a, b) => days(b.te) - days(a.te)),
    S: open.filter(i => stale(i) && !blocked(i) && !late(i)).sort((a, b) => freshness(b) - freshness(a)),
    D: its.filter(recentDone),
  };
}
function renderHead() {
  const { open, B, L, S, D: done } = buckets(), b = BUB[state.proj];
  const jql = b ? b.jql : D.bubbles.map(x => `(${x.jql})`).join(' OR ');
  document.getElementById('who').innerHTML = (b ? `${bubIcon(b)}${esc(b.name)}` : 'All')
    + ` <a class="jql" href="${esc(D.jiraBase)}/issues/?jql=${encodeURIComponent(jql)}" target="_blank" title="${esc(jql)}">JQL ↗</a>`;
  document.getElementById('kpis').innerHTML = [
    ['Open', open.length, '', 'tree'], ['Blocked', B.length, 'red', 'att-B'], ['Late', L.length, 'amber', 'att-L'],
    [`No update ${FU.stale_days}d+`, S.length, 'amber', 'att-S'], [`Closed ${FU.done_days}d`, done.length, 'green', 'att-D'],
  ].map(([l, v, c, t]) => `<button class="kpi ${c}" data-t="${t}"><span class="v">${v}</span><span class="l">${l}</span></button>`).join('');
  document.querySelectorAll('.kpi').forEach(k => k.onclick = () => {
    const t = k.dataset.t;
    if (t === 'tree') { if (!isTreeTab(state.tab)) setTab(STREAMS[0].id); return; }
    setTab('att'); document.getElementById(t)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  });
  document.getElementById('nAtt').textContent = B.length + L.length + S.length;
  const nOther = open.filter(i => i.stream === 'other').length;
  document.getElementById('treeTabs').innerHTML = STREAMS.map(s =>
    `<button class="tab ${state.tab === s.id ? 'on' : ''}" data-tab="${esc(s.id)}">${esc(s.name)} <span class="badge b-grey">${open.filter(i => i.stream === s.id).length}</span></button>`).join('')
    + (nOther || state.tab === 'other' ? `<button class="tab ${state.tab === 'other' ? 'on' : ''}" data-tab="other" title="Activity Type matches no tab in jiraf.toml">Other <span class="badge b-grey">${nOther}</span></button>` : '');
  document.querySelectorAll('.tab').forEach(t => { t.classList.toggle('on', t.dataset.tab === state.tab); t.onclick = () => setTab(t.dataset.tab); });
}

/* ---------- tree tabs ---------- */
const COLS = () => [
  { id: 'issue', label: 'Issue', w: 'auto', sort: i => +i.k.split('-')[1] },
  { id: 'st', label: 'Status', w: '100px', cls: 'c-st', sort: ST_RANK },
  { id: 'pr', label: 'P', title: 'Priority', w: '44px', cls: 'c-pr', sort: i => PRI_RANK[i.pr] ?? 99 },
  { id: 'who', label: 'Who', title: 'Assignee', w: '52px', cls: 'c-who', sort: i => i.a || '~' },
  { id: 'tgt', label: 'Target end', w: '104px', cls: 'c-tgt', sort: i => i.te || '9' },
  { id: 'due', label: 'Due', w: '84px', cls: 'c-due', sort: i => i.due || '9' },
  { id: 'ext', label: 'Extended status', w: '36%', sort: i => -freshness(i) },
  { id: 'cm', label: '', w: '64px', cls: 'c-cm' },
];
function matches(i) {
  if (!state.types.has(i.t) && TYPE_ORDER.includes(i.t)) return false;
  if (isDone(i) && !(state.done && recentDone(i))) return false;
  return !state.q || norm(i.k + ' ' + i.s + ' ' + i.ext + ' ' + (i.a || '') + ' ' + (i.lb || []).join(' ')).includes(state.q);
}
function buildTree() {
  // shown = matching items + their ancestors (ancestors of a hidden type are skipped; children attach higher up)
  const shown = new Set();
  for (const i of scopeItems()) if (i.stream === state.tab && matches(i)) {
    shown.add(i.k);
    for (const a of ancestors(i)) if (state.types.has(a.t) || !TYPE_ORDER.includes(a.t)) shown.add(a.k);
  }
  const parentOf = i => { let p = byKey[i.p]; const seen = new Set(); while (p && !shown.has(p.k) && !seen.has(p.k)) { seen.add(p.k); p = byKey[p.p]; } return p && shown.has(p.k) ? p : null; };
  const kids = {}, roots = [];
  for (const k of shown) { const i = byKey[k], p = parentOf(i); if (p) (kids[p.k] ??= []).push(i); else roots.push(i); }
  return { shown, kids, roots };
}
// In "All", roots are grouped under a bubble: their own, or (for context-only parents) a descendant's.
function bubbleOf(i, kids) {
  if (i.bubbles?.length) return i.bubbles[0];
  for (const c of kids[i.k] || []) { const b = bubbleOf(c, kids); if (b) return b; }
  return null;
}
function renderScope() {
  if (!D) return;
  const cols = COLS();
  document.getElementById('cols').innerHTML = cols.map(c => `<col style="width:${c.w}">`).join('');
  document.getElementById('thead').innerHTML = cols.map(c => `<th class="${c.cls || ''}" ${c.sort ? `data-s="${c.id}"` : ''} title="${c.title || ''}">${c.label}${state.sort?.id === c.id ? ` ${state.sort.dir > 0 ? '↑' : '↓'}` : ''}</th>`).join('');
  document.querySelectorAll('th[data-s]').forEach(th => th.onclick = () => { const id = th.dataset.s; state.sort = state.sort?.id === id ? (state.sort.dir > 0 ? { id, dir: -1 } : null) : { id, dir: 1 }; renderScope(); });
  const { shown, kids, roots } = buildTree();
  const col = state.sort && cols.find(c => c.id === state.sort.id);
  const tIdx = t => { const n = TYPE_ORDER.indexOf(t); return n < 0 ? 99 : n; };
  const cmp = col ? ((a, b) => { const x = col.sort(a), y = col.sort(b); return (x > y ? 1 : x < y ? -1 : 0) * state.sort.dir; })
    : ((a, b) => tIdx(a.t) - tIdx(b.t) || ST_RANK(a) - ST_RANK(b) || freshness(b) - freshness(a));
  const row = (i, d) => {
    const ch = kids[i.k] || [], open = !state.collapsed.has(i.k), total = (children[i.k] || []).length;
    let h = `<tr class="r ${i.ctx ? 'ctx' : ''}">${cols.map(c => `<td class="${c.cls || ''}">${
      c.id === 'issue' ? `<div class="issue" style="--d:${d}"><button class="chev ${ch.length && open ? 'open' : ''}" data-tg="${esc(i.k)}" ${ch.length ? '' : 'disabled'} title="${ch.length ? (open ? 'Collapse' : 'Expand') + ` (${ch.length} shown${total > ch.length ? ` of ${total}` : ''})` : 'No children'}">▶</button>${typeIcon(i.t)}${keyLink(i)}<span class="sum">${esc(i.s)}${labels(i)}</span>${ch.length && !open ? `<span class="kids">+${ch.length}</span>` : ''}</div>`
      : c.id === 'st' ? statusCell(i) : c.id === 'pr' ? priCell(i) : c.id === 'who' ? whoCell(i)
      : c.id === 'tgt' ? tgtCell(i) : c.id === 'due' ? dueCell(i) : c.id === 'ext' ? extCell(i) : cmBtn(i)}</td>`).join('')}</tr>`;
    if (open) h += ch.sort(cmp).map(c => row(c, d + 1)).join('');
    return h;
  };
  let html = '';
  if (state.proj === ALL) {
    for (const b of D.bubbles) {
      const rs = roots.filter(r => bubbleOf(r, kids) === b.id); if (!rs.length) continue;
      const pk = 'bubble:' + b.id, open = !state.collapsed.has(pk);
      const cnt = [...shown].filter(k => byKey[k].bubbles?.includes(b.id) && !byKey[k].ctx).length;
      html += `<tr class="pg"><td colspan="${cols.length}"><div class="issue"><button class="chev ${open ? 'open' : ''}" data-tg="${esc(pk)}">▶</button>${bubIcon(b)}${esc(b.name)}<span class="cnt">${cnt}</span></div></td></tr>`;
      if (open) html += rs.sort(cmp).map(r => row(r, 0)).join('');
    }
  } else html = roots.sort(cmp).map(r => row(r, 0)).join('');
  document.getElementById('tbody').innerHTML = html || `<tr><td colspan="${cols.length}" class="empty">No items match.</td></tr>`;
}
document.getElementById('tbody').addEventListener('click', e => {
  const b = e.target.closest('[data-tg]'); if (!b || b.disabled) return;
  const k = b.dataset.tg; state.collapsed.has(k) ? state.collapsed.delete(k) : state.collapsed.add(k); renderScope();
});
document.getElementById('expAll').onclick = () => { state.collapsed.clear(); renderScope(); };
document.getElementById('colAll').onclick = () => { const { kids } = buildTree(); state.collapsed = new Set([...Object.keys(kids), ...D.bubbles.map(b => 'bubble:' + b.id)]); renderScope(); };

/* ---------- Needs attention ---------- */
function attRow(i) {
  const anc = ancestors(i);
  return `<div class="it">
    <div class="l1"><div class="issue">${typeIcon(i.t)}${keyLink(i)}<span class="sum">${esc(i.s)}</span></div>${priCell(i)}${statusCell(i)}</div>
    <div class="ctxl">${anc.length ? `<span>↳ ${anc.map(a => esc(a.s)).join(' › ')}</span>` : ''}${state.proj === ALL ? `<span>${esc(i.bubbles.map(b => BUB[b]?.name).join(', '))}</span>` : ''}<span>${whoCell(i)}</span><span>Target ${tgtCell(i)}</span></div>
    <div class="l3">${extCell(i)}${cmBtn(i)}</div></div>`;
}
function renderAtt() {
  if (!D) return;
  const { B, L, S, D: done } = buckets();
  const sec = (id, label, list, empty) => `<section class="card" id="${id}"><h2>${label}<span class="n">${list.length}</span></h2>${list.length ? list.map(attRow).join('') : `<div class="empty">${empty}</div>`}</section>`;
  document.getElementById('v-att').innerHTML =
    sec('att-B', '🚧 Blocked', B, 'Nothing blocked.') + sec('att-L', '⏰ Past target end', L, 'Nothing late.') +
    sec('att-S', `💤 Extended Status not updated in ${FU.stale_days}+ days`, S, 'Everything has a recent update.') +
    sec('att-D', `✅ Closed in the last ${FU.done_days} days`, done, 'Nothing closed recently.');
}

/* ---------- edits: jiraf writes to Jira and returns the issue as Jira now has it ---------- */
async function save(i, field, path, body, el, okMsg) {
  el?.classList.add('busy');
  try {
    const fresh = await api(path === 'transition' ? 'transition' : `set_${path}`, { key: i.k, ...body });
    Object.assign(i, fresh, { bubbles: i.bubbles, ctx: i.ctx });
    i.stream = streamOf(i);
    mark(i, field); rerender(); toast(`${esc(i.k)}: ${okMsg}`);
  } catch (e) {
    el?.classList.remove('busy'); toast(`${esc(i.k)}: ${esc(e.message)}`, true);
  }
}

// Extended Status: free text, single line. No date prefix is forced; the age comes from the changelog.
document.addEventListener('click', e => {
  const el = e.target.closest('[data-edit]'); if (!el || e.target.closest('a,button')) return;
  const i = byKey[el.dataset.edit], max = FU.ext_max_len;
  el.outerHTML = `<div class="editor" data-editor="${esc(i.k)}">
    <textarea maxlength="${max}">${esc(i.ext)}</textarea>
    <div class="bar"><button class="btn primary" data-save>Save to Jira</button><button class="btn" data-cancel>Cancel</button>
    <button class="btn" data-stamp title="Prepend today's date">+ ${ddmm(today)}</button>
    <span>⌘↵ save · Esc cancel</span><span class="spacer"></span><span class="cnt"></span></div></div>`;
  const ed = document.querySelector(`[data-editor="${CSS.escape(i.k)}"]`), ta = ed.querySelector('textarea'), cnt = ed.querySelector('.cnt');
  const upd = () => { cnt.textContent = `${ta.value.length}/${max}`; cnt.classList.toggle('over', ta.value.length >= max); };
  upd(); ta.focus(); ta.select(); ta.oninput = upd;
  ed.querySelector('[data-stamp]').onclick = () => { ta.value = (`${ddmm(today)}: ` + ta.value.replace(/^\d{1,2}\/\d{1,2}(\/\d{2,4})?\s*:\s*/, '')).slice(0, max); upd(); ta.focus(); ta.setSelectionRange(7, ta.value.length); };
  const go = () => {
    const v = ta.value.replace(/\s+/g, ' ').trim();
    if (v === i.ext) return rerender();
    save(i, 'ext', 'ext', { value: v }, ed, 'Extended Status saved');
  };
  ed.querySelector('[data-save]').onclick = go; ed.querySelector('[data-cancel]').onclick = rerender;
  ta.onkeydown = ev => { if (ev.key === 'Escape') rerender(); if (ev.key === 'Enter' && (ev.metaKey || ev.ctrlKey)) { ev.preventDefault(); go(); } };
});

// dropdown menus for Status / Priority / Assignee / Target end
const menu = document.getElementById('menu');
function openMenu(btn, html) {
  menu.innerHTML = html; menu.classList.add('on'); placeMenu(btn);
  (menu.querySelector('input') || menu.querySelector('.opt.cur') || menu.querySelector('.opt'))?.focus();
}
function placeMenu(btn) {
  const r = btn.getBoundingClientRect(), mw = menu.offsetWidth, mh = menu.offsetHeight;
  menu.style.left = Math.max(8, Math.min(r.left, innerWidth - mw - 8)) + 'px';
  // open downward and scroll inside; flip up only when there's little room below
  const below = innerHeight - r.bottom - 12, up = below < 220 && r.top > below;
  menu.style.maxHeight = (up ? r.top - 12 : below) + 'px';
  menu.style.top = (up ? Math.max(8, r.top - Math.min(mh, r.top - 12) - 4) : r.bottom + 4) + 'px';
}
const closeMenu = () => menu.classList.remove('on');
document.addEventListener('click', async e => {
  const b = e.target.closest('[data-menu]');
  if (!b) { if (!e.target.closest('#menu')) closeMenu(); return; }
  const i = byKey[b.dataset.k], kind = b.dataset.menu;
  if (kind === 'status') {
    openMenu(b, `<div class="mh">Move ${esc(i.k)} to</div><div class="empty">Loading transitions…</div>`);
    let ts;
    try { ts = await api('transitions', { key: i.k }); } catch (err) { menu.innerHTML = `<div class="empty">${esc(err.message)}</div>`; return; }
    menu.innerHTML = `<div class="mh">Move ${esc(i.k)} to</div>` + (ts.length ? ts.map(t => `<button class="opt ${t.to === i.st ? 'cur' : ''}" data-id="${esc(t.id)}" data-to="${esc(t.to)}"><span class="st ${stClass(t.to, t.cat)}">${esc(t.to)}</span>${t.name !== t.to ? `<span style="color:var(--muted);font-size:12px">${esc(t.name)}</span>` : ''}</button>`).join('') : '<div class="empty">No transitions available.</div>');
    placeMenu(b); menu.querySelector('.opt')?.focus();
    menu.querySelectorAll('.opt').forEach(o => o.onclick = () => { closeMenu(); save(i, 'status', 'transition', { id: o.dataset.id }, b, `status → ${esc(o.dataset.to)}`); });
  }
  if (kind === 'priority') {
    // only the priorities this issue's project allows (priority schemes), in the configured order
    openMenu(b, `<div class="mh">Priority</div><div class="empty">Loading…</div>`);
    let allowed;
    try { allowed = new Set(await api('priorities', { key: i.k })); } catch (err) { menu.innerHTML = `<div class="empty">${esc(err.message)}</div>`; return; }
    const opts = D.priorities.filter(p => allowed.has(p.name));
    menu.innerHTML = `<div class="mh">Priority</div>` + (opts.map(p => `<button class="opt ${p.name === i.pr ? 'cur' : ''}" data-v="${esc(p.name)}">${priIcon(p.name)}${esc(p.name)}</button>`).join('') || '<div class="empty">No priorities available.</div>');
    placeMenu(b); menu.querySelector('.opt')?.focus();
    menu.querySelectorAll('.opt').forEach(o => o.onclick = () => { closeMenu(); const n = o.dataset.v; if (n === i.pr) return; save(i, 'priority', 'priority', { name: n }, b, `priority → ${esc(n)}`); });
  }
  if (kind === 'assignee') {
    openMenu(b, `<div class="mh">Assign ${esc(i.k)}</div><div style="padding:0 4px 4px"><input type="search" placeholder="Search people…" style="width:100%;padding:6px 8px;border:1px solid var(--line);border-radius:6px;background:var(--bg);color:var(--ink);font:inherit"></div><div class="lst"><div class="empty">Loading…</div></div>`);
    const inp = menu.querySelector('input'), lst = menu.querySelector('.lst');
    let seq = 0, timer;
    const search = async q => {
      const my = ++seq;
      let users;
      try { users = await api('assignable', { key: i.k, q }); } catch (err) { lst.innerHTML = `<div class="empty">${esc(err.message)}</div>`; return; }
      if (my !== seq) return;   // a newer search is on its way
      const opts = [...(q && !'unassigned'.includes(q.toLowerCase()) ? [] : [{ name: '', displayName: null }]), ...users];
      lst.innerHTML = opts.map(u => `<button class="opt ${(u.name || null) === (i.an || null) ? 'cur' : ''}" data-v="${esc(u.name)}" data-d="${esc(u.displayName || '')}">${avatar(u.displayName, u.avatar)}${u.displayName ? esc(title(u.displayName)) : '<i>Unassigned</i>'}</button>`).join('') || '<div class="empty">Nobody matches.</div>';
      lst.querySelectorAll('.opt').forEach(o => o.onclick = () => {
        closeMenu(); const n = o.dataset.v || null; if (n === (i.an || null)) return;
        save(i, 'assignee', 'assignee', { name: n }, b, n ? `assigned to ${esc(title(o.dataset.d))}` : 'unassigned');
      });
    };
    search('');
    inp.oninput = () => { clearTimeout(timer); timer = setTimeout(() => search(inp.value.trim()), 250); };
    inp.onkeydown = ev => { if (ev.key === 'Enter') lst.querySelector('.opt')?.click(); };
  }
  if (kind === 'target' || kind === 'due') {
    const [cur, name] = kind === 'target' ? [i.te, 'Target end'] : [i.due, 'Due date'];
    openMenu(b, `<div class="mh">${name}${kind === 'target' && i.ts ? ` · start ${fmt(i.ts)}` : ''}</div><div style="padding:0 4px"><input type="date" value="${esc(cur || '')}"></div>
      <div class="row"><button class="btn primary" data-ok>Save</button><button class="btn" data-clr>Clear</button><button class="btn" data-x>Cancel</button></div>`);
    const inp = menu.querySelector('input');
    const set = v => { closeMenu(); if ((v || null) === (cur || null)) return; save(i, kind, kind, { date: v || null }, b, v ? `${name.toLowerCase()} → ${fmt(v)}` : `${name.toLowerCase()} cleared`); };
    menu.querySelector('[data-ok]').onclick = () => set(inp.value);
    menu.querySelector('[data-clr]').onclick = () => set(null);
    menu.querySelector('[data-x]').onclick = closeMenu;
    inp.onkeydown = ev => { if (ev.key === 'Enter') set(inp.value); };
  }
});
document.addEventListener('keydown', e => { if (e.key === 'Escape') closeMenu(); });

/* ---------- last-comment popover (comment comes with the data) ---------- */
const pop = document.getElementById('pop'); let hideT;
function showPop(btn) {
  const i = byKey[btn.dataset.cm || btn.dataset.lb]; if (!i || (btn.dataset.cm && !i.c)) return;
  clearTimeout(hideT);
  pop.classList.toggle('lb', !!btn.dataset.lb);
  pop.innerHTML = btn.dataset.lb ? `<div class="h"><b>${i.lb.length} labels</b> · ${esc(i.k)}</div><div class="lbls">${i.lb.map(lblChip).join('')}</div>` : `<div class="h"><b>${esc(title(i.c.a))}</b> · ${fmt(i.c.d)} · last of ${i.nc} comment${i.nc > 1 ? 's' : ''}</div>
    <div class="b">${esc(i.c.b)}</div>
    <div class="f"><a href="${esc(D.jiraBase)}/browse/${esc(i.k)}#comment-tabpanel" target="_blank">All comments in Jira ↗</a></div>`;
  pop.classList.add('on');
  const r = btn.getBoundingClientRect(), pw = pop.offsetWidth, ph = pop.offsetHeight;
  pop.style.left = Math.max(12, Math.min(r.right - pw, innerWidth - pw - 12)) + 'px';
  let y = r.bottom + 6; if (y + ph > innerHeight - 12) y = Math.max(12, r.top - ph - 6); pop.style.top = y + 'px';
}
const hidePop = () => { hideT = setTimeout(() => pop.classList.remove('on'), 180); };
document.addEventListener('mouseover', e => { const b = e.target.closest('[data-cm],[data-lb]'); if (b) showPop(b); else if (e.target.closest('#pop')) clearTimeout(hideT); });
document.addEventListener('mouseout', e => { if (e.target.closest('[data-cm],[data-lb]') || e.target.closest('#pop')) hidePop(); });
document.addEventListener('focusin', e => { const b = e.target.closest('[data-cm],[data-lb]'); if (b) showPop(b); });
addEventListener('scroll', () => { pop.classList.remove('on'); closeMenu(); }, { passive: true });

function toast(html, err) {
  const t = document.getElementById('toast'); t.innerHTML = html; t.classList.toggle('err', !!err); t.classList.add('on');
  clearTimeout(t._t); t._t = setTimeout(() => t.classList.remove('on'), err ? 9000 : 4000);
}

/* ---------- wiring ---------- */
function setTab(t) {
  state.tab = t; writeHash();
  document.querySelectorAll('.tab').forEach(b => b.classList.toggle('on', b.dataset.tab === t));
  document.getElementById('v-scope').hidden = !isTreeTab(t); document.getElementById('v-att').hidden = isTreeTab(t);
  if (D) { renderHead(); if (isTreeTab(t)) renderScope(); }
}
function rerender() { if (!D) return; renderTeam(); renderHead(); renderScope(); renderAtt(); }
function select(p) { state.proj = p; state.sort = null; writeHash(); rerender(); window.scrollTo({ top: 0 }); }
document.getElementById('doneTog').onclick = e => { state.done = !state.done; e.currentTarget.classList.toggle('on'); renderScope(); };
document.getElementById('q').oninput = e => { state.q = norm(e.target.value); renderScope(); };
document.addEventListener('keydown', e => {
  if (!D || /INPUT|TEXTAREA/.test(e.target.tagName) || menu.classList.contains('on')) return;
  const order = [ALL, ...D.bubbles.map(b => b.id)], i = order.indexOf(state.proj);
  if (e.key === 'ArrowRight') select(order[(i + 1) % order.length]);
  if (e.key === 'ArrowLeft') select(order[(i - 1 + order.length) % order.length]);
});
document.getElementById('refresh').onclick = () => load(true);

/* ---------- theme: Auto (follow the OS) → Light → Dark. A per-browser preference, kept in localStorage. ---------- */
const THEMES = { auto: ['◐', 'Theme: auto (follows the system). Click for light'], light: ['☀', 'Theme: light. Click for dark'], dark: ['☾', 'Theme: dark. Click for auto'] };
function applyTheme(t) {
  if (!THEMES[t]) t = 'auto';
  if (t === 'auto') document.documentElement.removeAttribute('data-theme'); else document.documentElement.dataset.theme = t;
  const b = document.getElementById('theme'); b.textContent = THEMES[t][0]; b.title = THEMES[t][1];
  try { localStorage.setItem('jiraf.theme', t); } catch { /* private mode: just don't remember */ }
}
document.getElementById('theme').onclick = () => {
  const cur = document.documentElement.dataset.theme || 'auto';
  applyTheme({ auto: 'light', light: 'dark', dark: 'auto' }[cur]);
};
// (falls back to the key used before the rename to jiraf)
try { applyTheme(localStorage.getItem('jiraf.theme') || localStorage.getItem('gira.theme') || 'auto'); } catch { applyTheme('auto'); }
// Follow #p=…&t=… edits/links without a reload (our own writeHash uses replaceState, which doesn't fire this).
addEventListener('hashchange', () => { if (!D) return; readHash(); state.sort = null; setTab(state.tab); rerender(); });

async function load(refresh) {
  const dot = document.getElementById('dot'), now = document.getElementById('now');
  dot.className = 'dot busy'; now.textContent = 'loading…';
  if (!D) document.getElementById('tbody').innerHTML = '<tr><td class="empty">Loading from Jira…</td></tr>';
  let data;
  try { data = await api('data', { refresh: !!refresh }); }
  catch (e) {
    dot.className = 'dot err'; now.textContent = 'error';
    toast(`Could not load from Jira: ${esc(e.message)}`, true);
    if (!D) document.getElementById('tbody').innerHTML = `<tr><td class="empty failed"><img class="oops" src="jiraf-error.png" alt="">Could not load from Jira: ${esc(e.message)}</td></tr>`;
    return;
  }
  const first = !D;
  D = data; FU = D.followUp; TODAY = D.today; today = new Date(TODAY + 'T00:00:00');
  TYPE_ORDER = FU.hierarchy;
  PRI_RANK = Object.fromEntries(D.priorities.map((p, n) => [p.name, n]));
  PRI_ICON = Object.fromEntries(D.priorities.map(p => [p.name, p.icon]));
  BUB = Object.fromEntries(D.bubbles.map(b => [b.id, b]));
  STREAMS = D.tabs.map(t => ({ ...t, activity_types: (t.activity_types || []).map(norm), issues: t.issues || [] }));
  byKey = Object.fromEntries(D.issues.map(i => [i.k, i]));
  items = D.issues.filter(i => !i.ctx);
  children = {}; for (const i of D.issues) if (i.p && byKey[i.p]) (children[i.p] ??= []).push(i);
  for (const i of D.issues) i.stream = streamOf(i);
  if (first) {
    state.types = new Set(TYPE_ORDER);
    document.getElementById('typeTogs').innerHTML = TYPE_ORDER.map(t => `<button class="tog on" data-k="${esc(t)}" title="Show/hide ${esc(t)}">${typeIcon(t)}${esc(t)}</button>`).join('');
    document.querySelectorAll('#typeTogs .tog').forEach(b => b.onclick = () => { const k = b.dataset.k; state.types.has(k) ? state.types.delete(k) : state.types.add(k); b.classList.toggle('on'); renderScope(); });
    document.getElementById('doneTog').textContent = `+ closed ${FU.done_days}d`;
    readHash();
    if (state.proj !== ALL && !BUB[state.proj]) state.proj = ALL;
  }
  dot.className = 'dot'; now.textContent = D.fetchedAt;
  document.getElementById('sync').title = `Loaded from Jira at ${D.fetchedAt}. ← → switches bubble.`;
  setTab(state.tab); rerender();
}
// macOS draws the window buttons over the top bar (overlay title bar): see `.mac .top` in index.html.
if (navigator.userAgent.includes('Mac')) document.documentElement.classList.add('mac');

/* ---------- Jira links open in the default browser, never inside the jiraf window ---------- */
document.addEventListener('click', e => {
  const a = e.target.closest('a[target=_blank]'); if (!a) return;
  e.preventDefault();
  api('open_external', { url: a.href }).catch(err => toast(esc(err.message), true));
}, true);

/* ---------- startup: wait for the config/token/Jira checks, then the dashboard or the error screen ---------- */
const startup = document.getElementById('startup');
function showStartupError(s) {
  document.getElementById('startupMsg').textContent = s.error;
  document.getElementById('startupDir').textContent = s.configDir;
  document.getElementById('retry').disabled = false; document.getElementById('retry').textContent = 'Retry';
  document.body.classList.add('failed'); startup.hidden = false;
}
async function boot() {
  const s = await api('app_status');
  if (s.state === 'starting') return setTimeout(boot, 250);
  if (s.state === 'failed') return showStartupError(s);
  load(false);
}
// Re-read jiraf.toml and .env and run the checks again (Retry button, Reload config menu).
async function retryFlow() {
  const b = document.getElementById('retry');
  b.disabled = true; b.textContent = 'Retrying…';
  document.getElementById('dot').className = 'dot busy'; document.getElementById('now').textContent = 'loading…';
  const s = await api('retry');
  if (s.state === 'ready') location.reload(); else showStartupError(s);
}
document.getElementById('retry').onclick = retryFlow;
window.__TAURI__.event.listen('reload-config', retryFlow);
document.getElementById('openDir').onclick = () => api('open_config_dir').catch(e => toast(esc(e.message), true));
boot();
