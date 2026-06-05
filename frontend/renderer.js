const apiBase = 'http://127.0.0.1:3000';
const historyBody = document.getElementById('history-body');
const addModal = document.getElementById('add-modal');
const checkAll = document.getElementById('check-all');

const confirmModal = document.getElementById('confirm-modal-overlay');
const confirmTitle = document.getElementById('confirm-title');
const confirmText = document.getElementById('confirm-text');
const confirmProceed = document.getElementById('confirm-proceed');
const confirmCancel = document.getElementById('confirm-cancel');

// Modal elements
const urlInput = document.getElementById('download-url');
const savePathInput = document.getElementById('save-path');
const btnBrowse = document.getElementById('btn-browse');
const btnStartDownload = document.getElementById('btn-start-download');
const fileInfoSection = document.getElementById('file-info-section');
const detectNameText = document.getElementById('detect-name-text');
const detectTypeBadge = document.getElementById('detect-type-badge');
const detectSizeText = document.getElementById('detect-size-text');
const detectIconBox = document.getElementById('detect-icon-box');
const qualitySection = document.getElementById('quality-section');
const formatSelect = document.getElementById('format-select');
const customNameInput = document.getElementById('custom-name');

let currentTasks = [];
let selectedTaskIds = new Set();
let lastClickedTaskId = null;
let currentFilter = 'all';
let currentInspectType = null;

// ─── Helpers ──────────────────────────────────────────────────────────────────

function formatBytes(bytes, decimals = 2) {
    if (!bytes || bytes === 0) return '0 Bytes';
    const k = 1024, sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(decimals))} ${sizes[i]}`;
}

function formatSpeed(bps) {
    if (!bps || bps === 0) return '-';
    return `${formatBytes(bps)}/s`;
}

function formatETA(downloaded, total, speed) {
    if (!speed || speed === 0 || !total || total === 0) return '-';
    const secs = Math.round((total - downloaded) / speed);
    if (secs < 0) return '0s';
    if (secs > 3600) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
    if (secs > 60) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    return `${secs}s`;
}

// ─── Main Refresh Loop ─────────────────────────────────────────────────────────

async function refreshHistory() {
    try {
        const res = await fetch(`${apiBase}/history`);
        currentTasks = await res.json();
        renderTasks();
    } catch (e) { console.error('Fetch error:', e); }
}

// ─── Smart DOM Renderer (no flicker) ──────────────────────────────────────────

function renderTasks() {
    const filtered = currentTasks.filter(t => currentFilter === 'all' || t.status === currentFilter);

    // Remove stale rows
    Array.from(historyBody.querySelectorAll('tr')).forEach(row => {
        if (!filtered.find(t => t.id === row.dataset.id)) row.remove();
    });

    filtered.forEach(task => {
        const isCompleted = task.status === 'completed';
        const progress = task.total_bytes > 0 
            ? Math.round((task.downloaded_bytes / task.total_bytes) * 100) 
            : (isCompleted ? 100 : 0);
        const isDownloading = task.status === 'downloading';
        const isSelected = selectedTaskIds.has(task.id);

        let row = historyBody.querySelector(`tr[data-id="${task.id}"]`);

        if (!row) {
            row = document.createElement('tr');
            row.dataset.id = task.id;
            row.innerHTML = `
                <td style="padding-left:24px;"><input type="checkbox" class="task-check" data-id="${task.id}"></td>
                <td class="task-name-cell">
                  <div class="task-name-text" style="font-weight:600;color:#f8fafc;" title="${task.file_name}">${task.file_name}</div>
                </td>
                <td class="task-size-cell" style="color:var(--text-muted);font-size:12px;">${formatBytes(task.total_bytes)}</td>
                <td class="progress-cell">
                    <div class="progress-bg"><div class="progress-fill" style="width:${Math.min(progress,100)}%"></div></div>
                    <div class="progress-info"><span class="pct">${progress}%</span><span class="dl-bytes">${formatBytes(task.downloaded_bytes)}</span></div>
                </td>
                <td>
                    <span class="badge badge-${task.status}" title="${task.status === 'failed' && task.error_message ? task.error_message : task.status}">
                        ${task.status === 'failed' ? '⚠ FAILED' : task.status}
                    </span>
                </td>
                <td class="speed-cell" style="font-family:monospace;color:var(--accent);">${isDownloading ? formatSpeed(task.speed) : '-'}</td>
                <td class="eta-cell" style="color:var(--text-muted);">${isDownloading ? formatETA(task.downloaded_bytes, task.total_bytes, task.speed) : '-'}</td>
                <td class="actions-cell" style="padding-right:24px;">
                    <div class="inline-actions" style="display:flex;justify-content:flex-end;gap:6px;">
                        <div class="btn-group-main"></div>
                        <button class="action-btn" data-action="folder" title="Open Folder" style="background:transparent;border:1px solid transparent;color:var(--text-muted);padding:6px;border-radius:8px;cursor:pointer;transition:all 0.2s;">
                            <i data-lucide="folder-open" style="width:14px;height:14px;"></i>
                        </button>
                        <button class="action-btn action-delete" data-action="delete" title="Delete" style="background:transparent;border:1px solid transparent;color:var(--text-muted);padding:6px;border-radius:8px;cursor:pointer;transition:all 0.2s;">
                            <i data-lucide="trash-2" style="width:14px;height:14px;"></i>
                        </button>
                    </div>
                </td>`;

            row.onclick = (e) => {
                if (e.target.type === 'checkbox') return;
                const btn = e.target.closest('.action-btn');
                if (btn) {
                    const action = btn.dataset.action;
                    if (action === 'folder') window.electron.openFolder(`${task.save_path}/${task.file_name}`);
                    else if (action === 'delete') triggerAction(task.id, 'delete');
                    return;
                }
                handleRowClick(task.id, e.shiftKey, e.ctrlKey || e.metaKey);
            };
            row.querySelector('.task-check').onchange = (e) => {
                if (e.target.checked) selectedTaskIds.add(task.id);
                else selectedTaskIds.delete(task.id);
                updateSelectionUI();
            };

            historyBody.appendChild(row);
            lucide.createIcons({ parent: row });
        }

        // Smart in-place updates
        if (isSelected) row.classList.add('selected'); else row.classList.remove('selected');
        row.querySelector('.task-check').checked = isSelected;
        row.querySelector('.task-name-text').textContent = task.file_name;
        row.querySelector('.task-name-text').title = task.file_name;
        row.querySelector('.task-size-cell').textContent = (task.total_bytes > 0) ? formatBytes(task.total_bytes) : (isCompleted ? 'Unknown' : '0 Bytes');
        row.querySelector('.progress-fill').style.width = `${Math.min(progress, 100)}%`;
        row.querySelector('.pct').textContent = `${progress}%`;
        row.querySelector('.dl-bytes').textContent = isCompleted ? 'Completed' : formatBytes(task.downloaded_bytes);

        const badge = row.querySelector('.badge');
        const badgeLabel = task.status === 'failed' ? '⚠ FAILED' : task.status;
        const badgeTitle = task.status === 'failed' && task.error_message ? task.error_message : task.status;
        if (badge.textContent.trim() !== badgeLabel) {
            badge.textContent = badgeLabel;
            badge.className = `badge badge-${task.status}`;
        }
        badge.title = badgeTitle;

        row.querySelector('.speed-cell').textContent = isDownloading ? formatSpeed(task.speed) : '-';
        row.querySelector('.eta-cell').textContent = isDownloading ? formatETA(task.downloaded_bytes, task.total_bytes, task.speed) : '-';

        // Play/Pause button toggle
        const btnGroup = row.querySelector('.btn-group-main');
        if (!isCompleted) {
            const icon = isDownloading ? 'pause' : 'play';
            const action = isDownloading ? 'pause' : 'resume';
            const existingIcon = btnGroup.querySelector('i');
            if (!existingIcon || existingIcon.getAttribute('data-lucide') !== icon) {
                btnGroup.innerHTML = `<button class="action-btn" data-action="${action}" title="${action}" style="background:transparent;border:1px solid transparent;color:var(--text-muted);padding:6px;border-radius:8px;cursor:pointer;transition:all 0.2s;"><i data-lucide="${icon}" style="width:14px;height:14px;"></i></button>`;
                lucide.createIcons({ parent: btnGroup });
            }
        } else {
            btnGroup.innerHTML = '';
        }
    });

    updateSelectionUI();
}

// ─── Confirm Modal ─────────────────────────────────────────────────────────────

function showConfirm(title, text) {
    return new Promise(resolve => {
        confirmTitle.textContent = title;
        confirmText.textContent = text;
        confirmModal.style.display = 'flex';
        confirmProceed.onclick = () => { confirmModal.style.display = 'none'; resolve(true); };
        confirmCancel.onclick = () => { confirmModal.style.display = 'none'; resolve(false); };
    });
}

async function triggerAction(id, action) {
    if (action === 'delete') {
        const ok = await showConfirm('Delete Download?', 'The task record will be removed. The file on disk will remain.');
        if (!ok) return;
    }
    await fetch(`${apiBase}/${action}/${id}`, { method: action === 'delete' ? 'DELETE' : 'POST' });
    if (action === 'delete') selectedTaskIds.delete(id);
    refreshHistory();
}

// ─── Row Selection ─────────────────────────────────────────────────────────────

function handleRowClick(id, shift, ctrl) {
    const ids = currentTasks.map(t => t.id);
    if (shift && lastClickedTaskId) {
        const a = ids.indexOf(lastClickedTaskId), b = ids.indexOf(id);
        ids.slice(Math.min(a, b), Math.max(a, b) + 1).forEach(rid => selectedTaskIds.add(rid));
    } else if (ctrl) {
        if (selectedTaskIds.has(id)) selectedTaskIds.delete(id); else selectedTaskIds.add(id);
    } else {
        selectedTaskIds.clear(); selectedTaskIds.add(id);
    }
    lastClickedTaskId = id;
    renderTasks();
}

function updateSelectionUI() {
    const task = currentTasks.find(t => t.id === lastClickedTaskId) || currentTasks.find(t => selectedTaskIds.has(t.id));
    if (task) updateDetailsPanel(task);
    else {
        document.getElementById('detail-name').textContent = 'No selection';
        ['detail-url', 'detail-path', 'detail-date', 'detail-type'].forEach(id => document.getElementById(id).textContent = '-');
    }
    checkAll.checked = currentTasks.length > 0 && selectedTaskIds.size === currentTasks.length;
}

function updateDetailsPanel(task) {
    document.getElementById('detail-name').textContent = task.file_name;
    document.getElementById('detail-url').textContent = task.url;
    document.getElementById('detail-path').textContent = `${task.save_path}/${task.file_name}`;
    document.getElementById('detail-type').textContent = task.file_name.includes('.') ? task.file_name.split('.').pop().toUpperCase() : 'Unknown';
    document.getElementById('detail-date').textContent = new Date(task.created_at * 1000).toLocaleString();
}

// ─── Toolbar Bulk Actions ──────────────────────────────────────────────────────

checkAll.onchange = () => {
    if (checkAll.checked) currentTasks.forEach(t => selectedTaskIds.add(t.id)); else selectedTaskIds.clear();
    renderTasks();
};

document.getElementById('btn-pause-selected').onclick = () => bulkAction('pause');
document.getElementById('btn-resume-selected').onclick = () => bulkAction('resume');
document.getElementById('btn-delete-selected').onclick = () => bulkAction('delete');
document.getElementById('btn-open-folder').onclick = () => {
    const task = currentTasks.find(t => selectedTaskIds.has(t.id));
    if (task) window.electron.openFolder(`${task.save_path}/${task.file_name}`);
};

async function bulkAction(action) {
    if (selectedTaskIds.size === 0) return;
    if (action === 'delete') {
        const ok = await showConfirm(`Delete ${selectedTaskIds.size} task(s)?`, 'Records will be removed. Files on disk remain.');
        if (!ok) return;
    }
    const ids = Array.from(selectedTaskIds).filter(id => {
        const t = currentTasks.find(x => x.id === id);
        if (!t) return false;
        if ((action === 'pause' || action === 'resume') && t.status === 'completed') return false;
        return true;
    });
    for (const id of ids) {
        await fetch(`${apiBase}/${action}/${id}`, { method: action === 'delete' ? 'DELETE' : 'POST' }).catch(() => {});
    }
    if (action === 'delete') selectedTaskIds.clear();
    refreshHistory();
}

// ─── Sidebar Filters ───────────────────────────────────────────────────────────

document.querySelectorAll('.sidebar .nav-item').forEach(item => {
    item.onclick = () => {
        document.querySelectorAll('.nav-item').forEach(i => i.classList.remove('active'));
        item.classList.add('active');
        currentFilter = item.dataset.filter;
        renderTasks();
    };
});

// ─── New Download Modal ────────────────────────────────────────────────────────

function openAddModal() {
    addModal.style.display = 'flex';
    urlInput.value = '';
    savePathInput.value = '';
    customNameInput.value = '';
    fileInfoSection.style.display = 'none';
    qualitySection.style.display = 'none';
    btnStartDownload.disabled = true;
    btnStartDownload.innerHTML = '<i data-lucide="arrow-down-to-line"></i> Start Download';
    lucide.createIcons({ parent: btnStartDownload });
    currentInspectType = null;
    urlInput.focus();
}

function closeAddModal() {
    addModal.style.display = 'none';
}

document.getElementById('open-add-modal').onclick = openAddModal;
document.getElementById('close-add-modal').onclick = closeAddModal;
document.getElementById('btn-cancel-add').onclick = closeAddModal;

btnBrowse.onclick = async () => {
    const path = await window.electron.selectFolder();
    if (path) savePathInput.value = path;
};

// ─── Smart Link Inspection ─────────────────────────────────────────────────────

const ICON_MAP = { video: 'video', torrent: 'magnet', file: 'file' };
const BADGE_MAP = { video: 'badge-downloading', torrent: 'badge-paused', file: 'badge-completed' };
const LABEL_MAP = { video: 'VIDEO', torrent: 'TORRENT / MAGNET', file: 'DIRECT FILE' };

async function inspectLink(url) {
    if (!url) return;
    const urlLower = url.toLowerCase();
    const isMagnet = urlLower.startsWith('magnet:');
    const isTorrentFile = urlLower.endsWith('.torrent') || urlLower.includes('.torrent?');
    const isHttp = urlLower.startsWith('http://') || urlLower.startsWith('https://');
    
    if (!isMagnet && !isHttp) return;

    // Show card immediately with loading state
    fileInfoSection.style.display = 'block';
    detectNameText.textContent = 'Analysing source...';
    detectTypeBadge.textContent = 'WAITING';
    detectTypeBadge.className = 'badge badge-queued';
    detectSizeText.textContent = 'Fetching metadata';
    detectIconBox.innerHTML = '<i data-lucide="loader-2" class="spin" style="width:24px;height:24px;"></i>';
    lucide.createIcons({ parent: detectIconBox });
    btnStartDownload.disabled = true;
    btnStartDownload.innerHTML = '<i data-lucide="loader-2" class="spin" style="width:16px;"></i> Analyzing...';
    lucide.createIcons({ parent: btnStartDownload });

    try {
        const res = await fetch(`${apiBase}/inspect?url=${encodeURIComponent(url)}`);
        if (!res.ok) throw new Error('Inspection failed');
        const data = await res.json();
        const type = data.type || 'file';
        currentInspectType = type;

        detectNameText.textContent = data.title || data.filename || url.split('/').pop().split('?')[0] || 'Unknown';
        detectTypeBadge.textContent = LABEL_MAP[type] || type.toUpperCase();
        detectTypeBadge.className = `badge ${BADGE_MAP[type] || 'badge-queued'}`;
        const finalSize = data.filesize || data.filesize_approx;
        detectSizeText.textContent = finalSize ? formatBytes(finalSize) : (type === 'torrent' ? 'Connecting to peers...' : 'Size unknown');
        detectIconBox.innerHTML = `<i data-lucide="${ICON_MAP[type]}" style="width:24px;height:24px;"></i>`;
        lucide.createIcons({ parent: detectIconBox });

        // Show quality selector only for videos
        if (type === 'video') {
            qualitySection.style.display = 'block';
            const formats = (data.formats || []).filter(f => f.vcodec !== 'none' && f.acodec !== 'none');
            formatSelect.innerHTML = '<option value="best">Auto (Best Quality)</option>';
            formats.sort((a, b) => (b.height || 0) - (a.height || 0)).forEach(f => {
                const opt = document.createElement('option');
                opt.value = f.format_id;
                const res = f.height ? `${f.height}p` : 'Unknown';
                const fSize = f.filesize || f.filesize_approx;
                const size = fSize ? ` (~${formatBytes(fSize)})` : '';
                opt.textContent = `${f.ext?.toUpperCase() || 'MP4'} – ${res}${size}`;
                formatSelect.appendChild(opt);
            });
        } else {
            qualitySection.style.display = 'none';
        }

        btnStartDownload.disabled = false;
        btnStartDownload.innerHTML = '<i data-lucide="arrow-down-to-line"></i> Start Download';
        lucide.createIcons({ parent: btnStartDownload });
    } catch (err) {
        detectNameText.textContent = 'Could not analyse link';
        detectTypeBadge.textContent = 'ERROR';
        detectTypeBadge.className = 'badge badge-failed';
        detectIconBox.innerHTML = '<i data-lucide="alert-circle" style="width:24px;height:24px;color:var(--danger);"></i>';
        lucide.createIcons({ parent: detectIconBox });
        
        btnStartDownload.disabled = false;
        btnStartDownload.innerHTML = '<i data-lucide="arrow-down-to-line"></i> Start Anyway';
        lucide.createIcons({ parent: btnStartDownload });
    }
}

let inspectTimer = null;
urlInput.addEventListener('input', () => {
    clearTimeout(inspectTimer);
    const val = urlInput.value.trim();
    if (!val) { 
        fileInfoSection.style.display = 'none'; 
        btnStartDownload.disabled = true; 
        btnStartDownload.innerHTML = '<i data-lucide="arrow-down-to-line"></i> Start Download';
        lucide.createIcons({ parent: btnStartDownload });
        return; 
    }
    inspectTimer = setTimeout(() => inspectLink(val), 600);
});
urlInput.addEventListener('paste', () => {
    clearTimeout(inspectTimer);
    setTimeout(() => inspectLink(urlInput.value.trim()), 100);
});

// ─── Start Download ────────────────────────────────────────────────────────────

btnStartDownload.onclick = async () => {
    const url = urlInput.value.trim();
    if (!url) return;
    const save_path = savePathInput.value.trim() || null;
    const file_name = customNameInput.value.trim() || null;
    const format_id = (currentInspectType === 'video' && formatSelect.value) ? formatSelect.value :
                      (currentInspectType === 'torrent' ? 'torrent' : null);

    btnStartDownload.disabled = true;
    btnStartDownload.textContent = 'Starting...';

    try {
        const res = await fetch(`${apiBase}/download`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ url, save_path, format_id, file_name })
        });
        if (res.ok) {
            closeAddModal();
            setTimeout(refreshHistory, 500);
        } else {
            btnStartDownload.textContent = 'Failed — Retry';
            btnStartDownload.disabled = false;
        }
    } catch (e) {
        btnStartDownload.textContent = 'Error — Retry';
        btnStartDownload.disabled = false;
    }
};

// ─── Init ──────────────────────────────────────────────────────────────────────

window.onload = () => {
    refreshHistory();
    setInterval(refreshHistory, 1000);
};
