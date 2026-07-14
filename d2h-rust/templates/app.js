// --- LOGIKA APLIKACE ---
let chunkCache = {};
let chunkIndex = null;
let fullSearchIndex = null;
let dirPaths = {};
let rootName = '{{ROOT_NAME}}';
// {{LANG_JSON}}

let currentSortColumn = 'name';
let currentSortOrder = 'asc';
let currentFolderData = [];
let currentSearchResults = [];
let currentView = 'folder';
let currentFolderId = '0';
let lastFolderId = '0';
let searchResultsDisplayed = 500;
let savedColWidths = null; // {size: px, modified: px} persisted during session
var statsMode = false; // false = Tree (file listing), true = Stats
var currentStatsData = null; // last rendered stats data (for toggleCategoryDetail)
const SEARCH_PAGE_SIZE = 500;

function jsEscape(s) {
    if (!s) return '';
    return String(s)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;');
}

function decompressB64(b64string) {
    if (!b64string) return null;
    try {
        const binaryString = atob(b64string);
        const len = binaryString.length;
        const bytes = new Uint8Array(len);
        for (let i = 0; i < len; i++) {
            bytes[i] = binaryString.charCodeAt(i);
        }
        const inflated = pako.inflate(bytes, { to: 'string' });
        return JSON.parse(inflated);
    } catch (e) {
        console.error("Failed to decompress data:", e);
        return null;
    }
}

async function init() {
    try {
        if (useCompression) {
            chunkIndex = decompressB64(compressedIndexB64);
        } else {
            chunkIndex = JSON.parse(atob(compressedIndexB64));
        }

        if (!chunkIndex) throw new Error("Could not load chunk index.");
        await loadTree();
        showFolder('0', true);

        // Search index loaded on-demand when user searches (lazy load)
    } catch (e) {
        console.error(e);
        var treeEl = document.getElementById('tree'); treeEl.textContent = ''; var errDiv = document.createElement('div'); errDiv.className = 'loading'; errDiv.textContent = LANG.loadError; treeEl.appendChild(errDiv);
    }
}

function loadDirData(dirId) {
    const chunkId = chunkIndex[dirId];
    if (chunkId === undefined) return null;
    if (!chunkCache[chunkId]) {
        const b64data = compressedChunksB64[chunkId];
        if (!b64data) return null;
        if (useCompression) {
            chunkCache[chunkId] = decompressB64(b64data);
        } else {
            chunkCache[chunkId] = JSON.parse(atob(b64data));
        }
    }
    return chunkCache[chunkId] ? chunkCache[chunkId][dirId] : null;
}

function loadDirStats(dirId) {
    var chunkId = chunkIndex[dirId];
    if (chunkId === undefined) return null;
    if (!chunkCache[chunkId]) {
        var b64data = compressedChunksB64[chunkId];
        if (!b64data) return null;
        if (useCompression) {
            chunkCache[chunkId] = decompressB64(b64data);
        } else {
            chunkCache[chunkId] = JSON.parse(atob(b64data));
        }
    }
    return chunkCache[chunkId] ? chunkCache[chunkId][dirId + '_stats'] : null;
}

function decodeData(compactData) {
    if (!compactData) return [];
    return compactData.map(item => {
        if (item.length === 5) return { title: item[0], key: item[1], isFolder: true, isLazy: item[2] === 1, modified: item[3], size: item[4] };
        if (item.length === 4) return { title: item[0], key: item[1], isFolder: true, isLazy: item[2] === 1, modified: item[3], size: 0 };
        return { title: item[0], size: item[1], modified: item[2] };
    });
}

async function loadTree() {
    dirPaths['0'] = { name: rootName, path: rootName, parent: null, isLazy: true };
    const treeEl = document.getElementById('tree');
    treeEl.innerHTML = '<ul>' + createTreeNodeHtml('0') + '</ul>';
    const rootNode = treeEl.querySelector('li[data-id="0"]');
    await expandNode(rootNode, true);
}

function createTreeNodeHtml(folderId) {
    const folder = dirPaths[folderId];
    const toggler = folder.isLazy ? '<span class="toggler" onclick="event.stopPropagation(); toggleNode(this.parentElement.parentElement)">+</span>' : '<span class="toggler empty">-</span>';
    const displayName = folder.name;
    return '<li data-id="' + folderId + '" data-loaded="false">' +
        '<div class="tree-node" onclick="showFolder(\'' + folderId + '\')">' +
            toggler +
            '<span class="folder-icon"></span>' +
            '<span class="tree-node-name" title="' + jsEscape(folder.name) + '">' + jsEscape(displayName) + '</span>' +
        '</div>' +
        '<ul class="nested"></ul>' +
    '</li>';
}

async function toggleNode(liElement) {
    if (!liElement) return;
    const isLoaded = liElement.dataset.loaded === 'true';
    if (!isLoaded) {
        await expandNode(liElement, true);
    } else {
        const nestedUl = liElement.querySelector('.nested');
        const toggler = liElement.querySelector('.toggler');
        nestedUl.classList.toggle('active');
        toggler.textContent = nestedUl.classList.contains('active') ? '-' : '+';
    }
}

function expandNode(liElement, forceOpen) {
    if (forceOpen === undefined) forceOpen = false;
    return new Promise(function(resolve) {
        const folderId = liElement.dataset.id;
        const childrenData = loadDirData(folderId);
        const decodedChildren = decodeData(childrenData);

        const currentPath = dirPaths[folderId].path;
        decodedChildren.filter(function(item) { return item.isFolder; }).forEach(function(item) {
            if (!dirPaths[item.key]) {
                dirPaths[item.key] = {
                    name: item.title, path: currentPath + '/' + item.title,
                    parent: folderId, isLazy: item.isLazy
                };
            }
        });

        const folders = decodedChildren.filter(function(item) { return item.isFolder; }).sort(function(a,b) { return a.title.localeCompare(b.title); });
        let childrenHtml = '';
        folders.forEach(function(item) {
            childrenHtml += createTreeNodeHtml(item.key);
        });

        const nestedUl = liElement.querySelector('.nested');
        const toggler = liElement.querySelector('.toggler');
        nestedUl.innerHTML = childrenHtml;
        liElement.dataset.loaded = 'true';

        if (forceOpen || !nestedUl.classList.contains('active')) {
            nestedUl.classList.add('active');
            if (toggler) toggler.textContent = '-';
        }
        resolve();
    });
}

function buildBreadcrumb(folderId) {
    let path = [];
    let currentId = folderId;
    while (currentId !== null && dirPaths[currentId]) {
        path.unshift({ id: currentId, name: dirPaths[currentId].name });
        currentId = dirPaths[currentId].parent;
    }

    let html = '';
    path.forEach(function(item, index) {
        html += '<span class="breadcrumb-item">';
        if (index === path.length - 1) {
            html += '<span class="breadcrumb-current">' + jsEscape(item.name) + '</span>';
        } else {
            html += '<a href="#" onclick="event.preventDefault(); showFolder(\'' + item.id + '\');">' + jsEscape(item.name) + '</a>';
        }
        if (index < path.length - 1) {
            html += '<span class="breadcrumb-separator">\u25B6</span>';
        }
        html += '</span>';
    });
    document.getElementById('breadcrumb-path').innerHTML = html;
}

function renderCurrentFolderContent() {
    const folders = currentFolderData.filter(function(item) { return item.isFolder; });
    const files = currentFolderData.filter(function(item) { return !item.isFolder; });

    const sortFn = function(a, b) {
        const isAsc = currentSortOrder === 'asc';
        const multiplier = isAsc ? 1 : -1;

        const nameA = a.title.toLowerCase();
        const nameB = b.title.toLowerCase();

        if (currentSortColumn === 'name') {
            if (a.isFolder && !b.isFolder) return -1 * multiplier;
            if (!a.isFolder && b.isFolder) return 1 * multiplier;
            if (nameA < nameB) return -1 * multiplier;
            if (nameA > nameB) return 1 * multiplier;
            return 0;
        }

        let valA, valB;
        if (currentSortColumn === 'size') {
            valA = a.size || 0;
            valB = b.size || 0;
        } else {
            valA = a.modified || 0;
            valB = b.modified || 0;
        }

        if (valA < valB) return -1 * multiplier;
        if (valA > valB) return 1 * multiplier;
        if (nameA < nameB) return -1;
        if (nameA > nameB) return 1;
        return 0;
    };

    folders.sort(sortFn);
    files.sort(sortFn);

    const foldersSize = folders.reduce(function(acc, f) { return acc + (f.size || 0); }, 0);
    const filesSize = files.reduce(function(acc, f) { return acc + (f.size || 0); }, 0);
    const totalCurrentSize = foldersSize + filesSize;

    let footerRightParts = [];
    if (folders.length > 0) footerRightParts.push(folders.length + ' ' + LANG.folders + ' (' + formatSize(foldersSize) + ')');
    if (files.length > 0) footerRightParts.push(files.length + ' ' + LANG.files + ' (' + formatSize(filesSize) + ')');

    const footerEl = document.getElementById('content-footer');
    if (folders.length > 0 || files.length > 0) {
        footerEl.textContent = '';
        const spanSize = document.createElement('span');
        spanSize.textContent = LANG.size + ': ' + formatSize(totalCurrentSize);
        const spanDetails = document.createElement('span');
        spanDetails.textContent = footerRightParts.join(', ');
        footerEl.appendChild(spanSize);
        footerEl.appendChild(spanDetails);
    } else {
        footerEl.textContent = '';
    }

    const getArrow = function(column) {
        if (currentSortColumn === column) {
            return '<div class="sort-arrow"><span>' + (currentSortOrder === 'asc' ? '\u25B2' : '\u25BC') + '</span></div>';
        }
        return '<div class="sort-arrow"><span>\u2195</span></div>';
    };

    const isMobile = window.innerWidth <= 768;
    const sizeW = savedColWidths ? savedColWidths.size : 80;
    const modW = savedColWidths ? savedColWidths.modified : 150;
    const colgroupHtml = isMobile
        ? '<colgroup><col style="width: 58%;"><col style="width: 18%;"><col style="width: 24%;"></colgroup>'
        : '<colgroup><col><col style="width: ' + sizeW + 'px;"><col style="width: ' + modW + 'px;"></colgroup>';

    let html = '<table>' + colgroupHtml +
        '<thead><tr>' +
        '<th onclick="setSort(\'name\')"><div class="th-content"><span>' + LANG.name + '</span>' + getArrow('name') + '</div><div class="col-resize" data-col="1"></div></th>' +
        '<th onclick="setSort(\'size\')"><div class="th-content"><span>' + LANG.size + '</span>' + getArrow('size') + '</div><div class="col-resize" data-col="2"></div></th>' +
        '<th onclick="setSort(\'modified\')"><div class="th-content"><span>' + LANG.modified + '</span>' + getArrow('modified') + '</div></th>' +
        '</tr></thead><tbody>';

    if (currentFolderId !== '0' && dirPaths[currentFolderId] && dirPaths[currentFolderId].parent !== null) {
        const parentId = dirPaths[currentFolderId].parent;
        html += '<tr onclick="showFolder(\'' + parentId + '\')" style="cursor:pointer">' +
            '<td><div class="item-name-container"><span class="folder-icon"></span><a href="#" class="item-name">[..]</a></div></td>' +
            '<td class="size"></td><td class="date"></td></tr>';
    }

    folders.forEach(function(item) {
        html += '<tr onclick="showFolder(\'' + item.key + '\')" style="cursor:pointer">' +
            '<td><div class="item-name-container"><span class="folder-icon"></span><a href="#" class="item-name" title="' + jsEscape(item.title) + '">' + jsEscape(item.title) + '</a></div></td>' +
            '<td class="size">' + (item.size > 0 ? formatSize(item.size) : '-') + '</td>' +
            '<td class="date">' + formatDate(item.modified) + '</td></tr>';
    });

    files.forEach(function(item) {
        html += '<tr>' +
            '<td><div class="item-name-container"><span class="file-icon"></span><span class="item-name" title="' + jsEscape(item.title) + '">' + jsEscape(item.title) + '</span></div></td>' +
            '<td class="size">' + formatSize(item.size) + '</td>' +
            '<td class="date">' + formatDate(item.modified) + '</td></tr>';
    });

    if (folders.length === 0 && files.length === 0) {
        html += '<tr><td colspan="3">' +
            '<div class="empty-folder">' +
                '<div class="empty-folder-icon">\uD83D\uDCC2</div>' +
                '<div class="empty-folder-text">' + LANG.emptyFolder + '</div>' +
            '</div>' +
        '</td></tr>';
    }

    html += '</tbody></table>';
    document.getElementById('content-table').innerHTML = html;
}

var colResizing = false;
function setSort(column) {
    if (colResizing) return;
    if (column === currentSortColumn) {
        currentSortOrder = currentSortOrder === 'asc' ? 'desc' : 'asc';
    } else {
        currentSortColumn = column;
        currentSortOrder = 'asc';
    }

    if (currentView === 'statistics') return;
    if (currentView === 'search') renderSearchResults();
    else renderCurrentFolderContent();
    document.getElementById('content-table-wrapper').scrollTop = 0;
}

async function expandTreeToFolder(folderId) {
    let pathIds = [];
    let currentId = folderId;
    while (currentId !== null && dirPaths[currentId]) {
        pathIds.unshift(currentId);
        currentId = dirPaths[currentId].parent;
    }

    for (const id of pathIds) {
        const node = document.querySelector('.tree li[data-id="' + id + '"]');
        if (node && node.dataset.loaded === 'false') {
            await expandNode(node, true);
        } else if (node) {
             const nestedUl = node.querySelector('.nested');
             const toggler = node.querySelector('.toggler');
             if(nestedUl && !nestedUl.classList.contains('active')){
                nestedUl.classList.add('active');
                if (toggler) toggler.textContent = '-';
             }
        }
    }
}

function ensurePathExists(folderId) {
    return new Promise(async function(resolve) {
        if (dirPaths[folderId]) return resolve();

        if (!fullSearchIndex) initSearch();
        if (!fullSearchIndex) return resolve();

        const searchIndexMap = new Map();
        fullSearchIndex.forEach(function(item) { if(item.id) searchIndexMap.set(String(item.id), item); });

        if (!searchIndexMap.has(String(folderId)) && String(folderId) !== '0') return resolve();

        let itemsToBuild = [];
        let currentId = String(folderId);

        while (currentId && !dirPaths[currentId]) {
            const item = searchIndexMap.get(currentId);
            if (item) {
                itemsToBuild.unshift(item);
                currentId = item.parent_id;
            } else break;
        }

        for (const item of itemsToBuild) {
            const parentDir = dirPaths[item.parent_id];
            if (parentDir && !dirPaths[item.id]) {
                const itemName = item.path.split('/').pop();
                dirPaths[item.id] = {
                    name: itemName,
                    path: parentDir.path + '/' + itemName,
                    parent: item.parent_id,
                    isLazy: item.is_dir
                };
            }
        }
        resolve();
    });
}

async function showFolder(folderId, isInitialLoad) {
    if (isInitialLoad === undefined) isInitialLoad = false;
    if (!isInitialLoad) {
        showLoading(LANG.loading + '...');
    }

    await ensurePathExists(folderId);

    currentView = 'folder';
    currentFolderId = folderId;
    if (!isInitialLoad) document.getElementById('search_text').value = '';

    await expandTreeToFolder(folderId);

    document.querySelectorAll('.tree .tree-node.active').forEach(function(f) { f.classList.remove('active'); });
    const activeNode = document.querySelector('.tree li[data-id="' + folderId + '"] > .tree-node');
    if (activeNode) {
        activeNode.classList.add('active');
        if(!isInitialLoad) activeNode.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }

    buildBreadcrumb(folderId);

    const data = loadDirData(folderId);
    currentFolderData = decodeData(data);

    const currentPath = dirPaths[folderId] ? dirPaths[folderId].path : rootName;
    currentFolderData.filter(function(item) { return item.isFolder; }).forEach(function(item) {
        if (!dirPaths[item.key]) {
            dirPaths[item.key] = {
                name: item.title, path: currentPath + '/' + item.title,
                parent: folderId, isLazy: item.isLazy
            };
        }
    });

    if (statsMode) {
        showDirStats(folderId);
    } else {
        renderCurrentFolderContent();
    }
    hideLoading();
    document.getElementById('content-table-wrapper').scrollTop = 0;
}

// --- Search Functionality ---
function initSearch() {
    if (fullSearchIndex) return;

    if(useCompression){
        fullSearchIndex = decompressB64(compressedSearchB64);
    } else {
        fullSearchIndex = JSON.parse(atob(compressedSearchB64));
    }

    if (!fullSearchIndex) console.error("Failed to load search index");
}

function wildcardToRegex(pattern) {
    let escaped = pattern.replace(/([.+^${}()|[\]\/])/g, '\\$1');
    escaped = escaped.replace(/\*/g, '.*').replace(/\?/g, '.');
    return new RegExp('^' + escaped + '$', 'i');
}

function removeDiacritics(s) {
    return s.normalize('NFD').replace(/[\u0300-\u036f]/g, '');
}

function formatNumber(n) {
    return n.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

function renderSearchResults() {
    const sortFn = function(a, b) {
        const isAsc = currentSortOrder === 'asc';
        const multiplier = isAsc ? 1 : -1;

        const nameA = a.path.split('/').pop().toLowerCase();
        const nameB = b.path.split('/').pop().toLowerCase();

        if (currentSortColumn === 'name') {
            if (a.is_dir && !b.is_dir) return -1 * multiplier;
            if (!a.is_dir && b.is_dir) return 1 * multiplier;
            if (nameA < nameB) return -1 * multiplier;
            if (nameA > nameB) return 1 * multiplier;
            return 0;
        }

        let valA, valB;
        if (currentSortColumn === 'folder') {
            valA = a.path.substring(0, a.path.lastIndexOf('/'));
            valB = b.path.substring(0, b.path.lastIndexOf('/'));
        } else if (currentSortColumn === 'size') {
            valA = a.is_dir ? (a.dirsize || 0) : (a.size || 0);
            valB = b.is_dir ? (b.dirsize || 0) : (b.size || 0);
        } else {
            valA = a.modified || 0;
            valB = b.modified || 0;
        }

        if (valA < valB) return -1 * multiplier;
        if (valA > valB) return 1 * multiplier;
        if (nameA < nameB) return -1;
        if (nameA > nameB) return 1;
        return 0;
    };

    currentSearchResults.sort(sortFn);

    const totalResults = currentSearchResults.length;
    const displayCount = Math.min(searchResultsDisplayed, totalResults);
    const displayedResults = currentSearchResults.slice(0, displayCount);

    // Stats from ALL results (not just displayed)
    const numFolders = currentSearchResults.filter(function(r) { return r.is_dir; }).length;
    const numFiles = totalResults - numFolders;
    const foldersSize = currentSearchResults.filter(function(r) { return r.is_dir; }).reduce(function(acc, r) { return acc + (r.dirsize || 0); }, 0);
    const filesSize = currentSearchResults.filter(function(r) { return !r.is_dir; }).reduce(function(acc, r) { return acc + (r.size || 0); }, 0);
    const totalSearchSize = foldersSize + filesSize;

    let footerLeftParts = [];
    if (displayCount < totalResults) {
        footerLeftParts.push(LANG.showingResults + ' ' + formatNumber(displayCount) + ' / ' + formatNumber(totalResults));
    }
    footerLeftParts.push(LANG.size + ': ' + formatSize(totalSearchSize));

    let footerRightParts = [];
    if (numFolders > 0) footerRightParts.push(numFolders + ' ' + LANG.folders + ' (' + formatSize(foldersSize) + ')');
    if (numFiles > 0) footerRightParts.push(numFiles + ' ' + LANG.files + ' (' + formatSize(filesSize) + ')');

    const footerEl = document.getElementById('content-footer');
    if (totalResults > 0) {
        footerEl.textContent = '';
        const spanSize = document.createElement('span');
        spanSize.textContent = footerLeftParts.join(' | ');
        const spanFound = document.createElement('span');
        spanFound.textContent = LANG.found + ': ' + footerRightParts.join(', ');
        footerEl.appendChild(spanSize);
        footerEl.appendChild(spanFound);
    } else {
        footerEl.textContent = '';
    }

    const getArrow = function(column) {
        if (currentSortColumn === column) return '<div class="sort-arrow"><span>' + (currentSortOrder === 'asc' ? '\u25B2' : '\u25BC') + '</span></div>';
        return '<div class="sort-arrow"><span>\u2195</span></div>';
    };

    const isMobile = window.innerWidth <= 768;
    const sizeW = savedColWidths ? savedColWidths.size : 80;
    const modW = savedColWidths ? savedColWidths.modified : 150;
    const colgroupHtml = isMobile
        ? '<colgroup><col style="width: 32%;"><col style="width: 28%;"><col style="width: 18%;"><col style="width: 22%;"></colgroup>'
        : '<colgroup><col><col><col style="width: ' + sizeW + 'px;"><col style="width: ' + modW + 'px;"></colgroup>';

    let html = '<table>' + colgroupHtml +
        '<thead><tr>' +
            '<th onclick="setSort(\'name\')"><div class="th-content"><span>' + LANG.name + '</span>' + getArrow('name') + '</div><div class="col-resize" data-col="1"></div></th>' +
            '<th onclick="setSort(\'folder\')"><div class="th-content"><span>' + LANG.folder + '</span>' + getArrow('folder') + '</div><div class="col-resize" data-col="2"></div></th>' +
            '<th onclick="setSort(\'size\')"><div class="th-content"><span>' + LANG.size + '</span>' + getArrow('size') + '</div><div class="col-resize" data-col="3"></div></th>' +
            '<th onclick="setSort(\'modified\')"><div class="th-content"><span>' + LANG.modified + '</span>' + getArrow('modified') + '</div></th>' +
        '</tr></thead><tbody>';

    html += '<tr onclick="showFolder(\'' + lastFolderId + '\')" style="cursor:pointer">' +
                '<td colspan="4" style="border-bottom: 2px solid #e0c090;"><div class="item-name-container"><span class="folder-icon"></span><a href="#" class="item-name">[..] ' + LANG.searchBack + '</a></div></td>' +
            '</tr>';

    if (totalResults === 0) {
        html += '<tr><td colspan="4" style="text-align: center; padding: 20px;">' + LANG.noResults + '</td></tr>';
    } else {
        displayedResults.forEach(function(item) {
            const itemName = item.path.split('/').pop();
            const itemPath = item.path.substring(0, item.path.lastIndexOf('/'));
            const parentFolderId = item.parent_id || '0';
            const iconClass = item.is_dir ? 'folder-icon' : 'file-icon';

            const nameLink = item.is_dir
                ? '<a href="#" onclick="event.preventDefault(); showFolder(\'' + item.id + '\');">' + jsEscape(itemName) + '</a>'
                : jsEscape(itemName);

            const sizeStr = item.is_dir ? (item.dirsize > 0 ? formatSize(item.dirsize) : '-') : formatSize(item.size);

            const folderLink = itemPath
                ? '<a href="#" onclick="event.preventDefault(); showFolder(\'' + parentFolderId + '\');">' + jsEscape(itemPath) + '</a>'
                : '<span style="color:#999">/</span>';

            html += '<tr>' +
                '<td><div class="item-name-container"><span class="' + iconClass + '"></span> ' + nameLink + '</div></td>' +
                '<td>' + folderLink + '</td>' +
                '<td class="size">' + sizeStr + '</td>' +
                '<td class="date">' + formatDate(item.modified) + '</td>' +
            '</tr>';
        });

        // "Show more" row when results are truncated
        if (displayCount < totalResults) {
            const remaining = totalResults - displayCount;
            const nextBatch = Math.min(remaining, SEARCH_PAGE_SIZE);
            html += '<tr><td colspan="4" style="text-align: center; padding: 12px;">' +
                '<a href="#" onclick="event.preventDefault(); showMoreSearchResults();" ' +
                'style="color: #006699; font-weight: bold;">Show ' + formatNumber(nextBatch) + ' more (' + formatNumber(remaining) + ' remaining)</a>' +
                '</td></tr>';
        }
    }
    html += '</tbody></table>';
    document.getElementById('content-table').innerHTML = html;
}

function showMoreSearchResults() {
    searchResultsDisplayed += SEARCH_PAGE_SIZE;
    renderSearchResults();
}

const searchInput = document.getElementById('search_text');
function handleSearchLogic() {
    const searchTerm = searchInput.value.trim();
    if (searchTerm.length >= 3) {
        doSearch();
    } else if (searchTerm.length === 0) {
        if (currentView === 'search') showFolder(lastFolderId);
    }
}

function doSearch() {
    const searchTerm = document.getElementById('search_text').value.trim();
    if (currentView === 'folder') lastFolderId = currentFolderId;

    if (!searchTerm) return showFolder(lastFolderId);

    showLoading(LANG.search + '...');

    if (!fullSearchIndex) {
        document.getElementById('content-table').innerHTML = '<div class="loading"><span class="loading-spinner"></span>' + LANG.searchBuilding + '</div>';
        initSearch();
    }

    if (!fullSearchIndex) {
        hideLoading();
        document.getElementById('content-table').innerHTML = '<div class="loading">' + LANG.searchError + '</div>';
        return;
    }

    currentView = 'search';
    searchResultsDisplayed = SEARCH_PAGE_SIZE;

    try {
        // Always search within current folder scope
        // "path/to/query" = search in full paths, "query" = search filename only
        const isPathSearch = searchTerm.includes('/');

        // Normalize: strip diacritics for accent-insensitive matching
        const normalizedTerm = removeDiacritics(searchTerm);
        const searchPattern = (normalizedTerm.includes('*') || normalizedTerm.includes('?'))
            ? normalizedTerm
            : '*' + normalizedTerm + '*';
        const searchRegex = wildcardToRegex(searchPattern);

        // Build scope prefix from current folder
        let scopePrefix = '';
        if (dirPaths[lastFolderId]) {
            const folderPath = dirPaths[lastFolderId].path;
            const rootPrefix = rootName + '/';
            if (folderPath.startsWith(rootPrefix)) {
                scopePrefix = folderPath.substring(rootPrefix.length) + '/';
            }
        }

        currentSearchResults = fullSearchIndex.filter(function(item) {
            // Scope filter: only items within current folder
            if (scopePrefix && !item.path.startsWith(scopePrefix)) {
                return false;
            }

            // Match against filename or full path (with diacritics stripped)
            const target = isPathSearch ? item.path : item.path.split('/').pop();
            return searchRegex.test(removeDiacritics(target));
        });
    } catch (e) {
        console.error("Invalid search pattern:", e);
        currentSearchResults = [];
    }

    currentSortColumn = 'name';
    currentSortOrder = 'asc';

    // Breadcrumb: full path to scope folder + search indicator
    let pathHtml = '';
    let pathItems = [];
    let crumbId = lastFolderId;
    while (crumbId !== null && dirPaths[crumbId]) {
        pathItems.unshift({ id: crumbId, name: dirPaths[crumbId].name });
        crumbId = dirPaths[crumbId].parent;
    }
    pathItems.forEach(function(item) {
        pathHtml += '<span class="breadcrumb-item">';
        pathHtml += '<a href="#" onclick="event.preventDefault(); showFolder(\'' + item.id + '\');">' + jsEscape(item.name) + '</a>';
        pathHtml += '<span class="breadcrumb-separator">\u25B6</span>';
        pathHtml += '</span>';
    });
    pathHtml += '<span class="breadcrumb-item"><span class="breadcrumb-current">' +
        LANG.search + ': "' + jsEscape(searchTerm) + '" (' + formatNumber(currentSearchResults.length) + ')' +
        '</span></span>';
    document.getElementById('breadcrumb-path').innerHTML = pathHtml;

    document.querySelectorAll('.tree .tree-node.active').forEach(function(f) { f.classList.remove('active'); });
    renderSearchResults();
    hideLoading();
}

// --- Statistics View ---
var FILE_CATEGORIES = {
    'photos': ['jpg','jpeg','png','gif','bmp','tiff','tif','webp','heic','heif','svg','ico'],
    'rawphotos': ['cr2','cr3','nef','nrw','arw','orf','rw2','dng','raf','pef','srw','x3f'],
    'video': ['mp4','mov','avi','mkv','wmv','m4v','mts','m2ts','mpg','mpeg','webm','flv','3gp'],
    'audio': ['mp3','wav','flac','aac','ogg','wma','m4a','aiff','alac','opus'],
    'documents': ['pdf','doc','docx','xls','xlsx','ppt','pptx','odt','ods','odp','txt','rtf','csv','pages','numbers','keynote'],
    'archives': ['zip','rar','7z','tar','gz','bz2','xz','iso','dmg','cab'],
    'email': ['pst','ost','eml','mbox','msg'],
    'databases': ['mdb','accdb','sql','sqlite','db','dbf'],
    'web': ['html','htm','css','js','json','xml','php','asp'],
    'sourcecode': ['py','rs','c','cpp','h','java','go','ts','rb','swift','kt'],
    'executables': ['exe','dll','sys','app','msi','deb','rpm']
};

var CATEGORY_NAMES = {
    'photos': LANG.catPhotos, 'rawphotos': LANG.catRawPhotos,
    'video': LANG.catVideo, 'audio': LANG.catAudio,
    'documents': LANG.catDocuments, 'archives': LANG.catArchives,
    'email': LANG.catEmail, 'databases': LANG.catDatabases,
    'web': LANG.catWeb, 'sourcecode': LANG.catSourceCode,
    'executables': LANG.catExecutables, 'other': LANG.other
};

var CATEGORY_COLORS = {
    'photos': '#4CAF50',
    'rawphotos': '#8BC34A',
    'video': '#2196F3',
    'audio': '#9C27B0',
    'documents': '#FF9800',
    'archives': '#795548',
    'email': '#00BCD4',
    'databases': '#607D8B',
    'web': '#E91E63',
    'sourcecode': '#3F51B5',
    'executables': '#F44336',
    'other': '#9E9E9E'
};

function showStatistics() {
    statsMode = true;
    updateToggleButtons();
    if (currentView === 'search') {
        showFolder(lastFolderId || '0');
        return;
    }
    showDirStats(currentFolderId);
}

function showTree() {
    statsMode = false;
    updateToggleButtons();
    if (currentView === 'folder' || currentView === 'statistics') {
        currentView = 'folder';
        renderCurrentFolderContent();
    }
}

function updateToggleButtons() {
    var treeBtn = document.getElementById('toggle-tree-btn');
    var statsBtn = document.getElementById('toggle-stats-btn');
    if (treeBtn && statsBtn) {
        treeBtn.classList.toggle('active', !statsMode);
        statsBtn.classList.toggle('active', statsMode);
    }
}

function showDirStats(dirId) {
    currentView = 'folder';
    var rawStats = loadDirStats(dirId);
    if (!rawStats) {
        // Fallback for root: use global statsData
        if (dirId === '0' && statsData) {
            renderStatistics({
                total_files: statsData.total_files,
                total_size: statsData.total_size,
                total_dirs: undefined,
                extensions: statsData.extensions
            });
            return;
        }
        var ct = document.getElementById('content-table');
        ct.textContent = '';
        var msg = document.createElement('div');
        msg.className = 'loading';
        msg.textContent = 'No statistics available for this folder.';
        ct.appendChild(msg);
        return;
    }
    var dirStatsData = {
        total_files: rawStats.tf,
        total_size: rawStats.ts,
        total_dirs: rawStats.td,
        extensions: rawStats.ext.map(function(e) {
            return { ext: e[0], count: e[1], size: e[2] };
        })
    };
    renderStatistics(dirStatsData);
}

function renderStatistics(data) {
    if (!data) data = statsData;
    if (!data) return;
    currentStatsData = data;

    var extMap = {};
    data.extensions.forEach(function(e) {
        extMap[e.ext] = { count: e.count, size: e.size };
    });

    // Group by category
    var categories = {};
    var categorized = {};
    for (var cat in FILE_CATEGORIES) {
        categories[cat] = { count: 0, size: 0, extensions: [] };
        FILE_CATEGORIES[cat].forEach(function(ext) {
            if (extMap[ext]) {
                categories[cat].count += extMap[ext].count;
                categories[cat].size += extMap[ext].size;
                categories[cat].extensions.push({ ext: ext, count: extMap[ext].count, size: extMap[ext].size });
                categorized[ext] = true;
            }
        });
        categories[cat].extensions.sort(function(a, b) { return b.size - a.size; });
    }

    // Other category
    categories['other'] = { count: 0, size: 0, extensions: [] };
    data.extensions.forEach(function(e) {
        if (!categorized[e.ext]) {
            categories['other'].count += e.count;
            categories['other'].size += e.size;
            categories['other'].extensions.push({ ext: e.ext, count: e.count, size: e.size });
        }
    });
    categories['other'].extensions.sort(function(a, b) { return b.size - a.size; });

    // Sort categories by size desc, filter out empty
    var sortedCats = Object.keys(categories).filter(function(c) { return categories[c].count > 0; });
    sortedCats.sort(function(a, b) { return categories[b].size - categories[a].size; });

    var maxCatSize = sortedCats.length > 0 ? categories[sortedCats[0]].size : 1;
    var uniqueExts = data.extensions.length;

    // Build stats view using DOM for safety
    var container = document.createElement('div');
    container.className = 'stats-container';

    // Overview section
    var overview = document.createElement('div');
    overview.className = 'stats-overview';
    var overviewItems = [
        { value: formatNumber(data.total_files), label: LANG.totalFiles },
        { value: formatSize(data.total_size), label: LANG.totalSize },
        { value: formatNumber(uniqueExts), label: LANG.fileTypes },
        { value: formatNumber(sortedCats.length), label: LANG.categories }
    ];
    if (data.total_dirs !== undefined) {
        overviewItems.splice(1, 0, { value: formatNumber(data.total_dirs), label: LANG.totalFolders });
    }
    overviewItems.forEach(function(item) {
        var div = document.createElement('div');
        div.className = 'stats-overview-item';
        var valDiv = document.createElement('div');
        valDiv.className = 'stats-overview-value';
        valDiv.textContent = item.value;
        var lblDiv = document.createElement('div');
        lblDiv.className = 'stats-overview-label';
        lblDiv.textContent = item.label;
        div.appendChild(valDiv);
        div.appendChild(lblDiv);
        overview.appendChild(div);
    });
    container.appendChild(overview);

    // Category bars section
    var catSection = document.createElement('div');
    catSection.className = 'stats-section';
    var catTitle = document.createElement('h3');
    catTitle.className = 'stats-section-title';
    catTitle.textContent = LANG.categoriesBySize;
    catSection.appendChild(catTitle);

    sortedCats.forEach(function(cat) {
        var c = categories[cat];
        var pct = data.total_size > 0 ? (c.size / data.total_size * 100) : 0;
        var barWidth = maxCatSize > 0 ? (c.size / maxCatSize * 100) : 0;
        var color = CATEGORY_COLORS[cat] || '#9E9E9E';

        var catDiv = document.createElement('div');
        catDiv.className = 'stats-category';
        catDiv.dataset.cat = cat;

        var clickArea = document.createElement('div');
        clickArea.className = 'stats-category-click';
        clickArea.onclick = (function(catName) { return function() { toggleCategoryDetail(catName); }; })(cat);

        var header = document.createElement('div');
        header.className = 'stats-category-header';
        var toggler = document.createElement('span');
        toggler.className = 'stats-category-toggler';
        toggler.textContent = '\u25B8';
        var nameSpan = document.createElement('span');
        nameSpan.className = 'stats-category-name';
        nameSpan.textContent = CATEGORY_NAMES[cat] || cat;
        var infoSpan = document.createElement('span');
        infoSpan.className = 'stats-category-info';
        infoSpan.textContent = formatNumber(c.count) + ' ' + LANG.files + ' \u00B7 ' + formatSize(c.size) + ' (' + pct.toFixed(1) + '%)';
        header.appendChild(toggler);
        header.appendChild(nameSpan);
        header.appendChild(infoSpan);

        var bar = document.createElement('div');
        bar.className = 'stats-bar';
        var fill = document.createElement('div');
        fill.className = 'stats-bar-fill';
        fill.style.width = barWidth.toFixed(1) + '%';
        fill.style.background = color;
        bar.appendChild(fill);

        clickArea.appendChild(header);
        clickArea.appendChild(bar);
        catDiv.appendChild(clickArea);

        // Detail panel (hidden by default)
        var detail = document.createElement('div');
        detail.className = 'stats-category-detail';
        detail.id = 'stats-detail-' + cat.replace(/\s+/g, '-');
        catDiv.appendChild(detail);

        catSection.appendChild(catDiv);
    });
    container.appendChild(catSection);

    var contentEl = document.getElementById('content-table');
    contentEl.textContent = '';
    contentEl.appendChild(container);

    // Footer
    var footerEl = document.getElementById('content-footer');
    footerEl.textContent = '';
    var spanLeft = document.createElement('span');
    spanLeft.textContent = LANG.total + ': ' + formatSize(data.total_size);
    var spanRight = document.createElement('span');
    spanRight.textContent = formatNumber(data.total_files) + ' ' + LANG.files + ', ' + formatNumber(uniqueExts) + ' ' + LANG.fileTypes;
    footerEl.appendChild(spanLeft);
    footerEl.appendChild(spanRight);
}

var openCategoryDetail = null;
var openExtensionFiles = null;

function toggleCategoryDetail(catName) {
    var detailId = 'stats-detail-' + catName.replace(/\s+/g, '-');
    var detail = document.getElementById(detailId);
    if (!detail) return;

    // Close previously open category
    if (openCategoryDetail && openCategoryDetail !== catName) {
        var prevId = 'stats-detail-' + openCategoryDetail.replace(/\s+/g, '-');
        var prev = document.getElementById(prevId);
        if (prev) {
            prev.textContent = '';
            prev.classList.remove('open');
            prev.style.maxHeight = '';
            var prevCat = prev.closest('.stats-category');
            if (prevCat) prevCat.querySelector('.stats-category-toggler').textContent = '\u25B8';
        }
        openExtensionFiles = null;
    }

    var toggler = detail.closest('.stats-category').querySelector('.stats-category-toggler');

    if (detail.classList.contains('open')) {
        detail.textContent = '';
        detail.classList.remove('open');
        detail.style.maxHeight = '';
        toggler.textContent = '\u25B8';
        openCategoryDetail = null;
        openExtensionFiles = null;
        return;
    }

    // Build detail content
    var extMap = {};
    currentStatsData.extensions.forEach(function(e) {
        extMap[e.ext] = { count: e.count, size: e.size };
    });

    var exts = [];
    var categorized = {};
    if (FILE_CATEGORIES[catName]) {
        FILE_CATEGORIES[catName].forEach(function(ext) {
            if (extMap[ext]) {
                exts.push({ ext: ext, count: extMap[ext].count, size: extMap[ext].size });
                categorized[ext] = true;
            }
        });
    }
    if (catName === 'other') {
        for (var cat2 in FILE_CATEGORIES) {
            FILE_CATEGORIES[cat2].forEach(function(ext) { categorized[ext] = true; });
        }
        currentStatsData.extensions.forEach(function(e) {
            if (!categorized[e.ext]) exts.push({ ext: e.ext, count: e.count, size: e.size });
        });
    }
    exts.sort(function(a, b) { return b.size - a.size; });

    var totalCatSize = exts.reduce(function(s, e) { return s + e.size; }, 0);
    var totalCatCount = exts.reduce(function(s, e) { return s + e.count; }, 0);
    var segmentColors = getSegmentColors(exts.length);

    // Layout: donut left, legend right
    var row = document.createElement('div');
    row.className = 'stats-detail-row';

    // Donut chart
    var chartWrap = document.createElement('div');
    chartWrap.className = 'stats-donut-wrap';
    var svgSize = 170;
    var svg = buildDonutSVG(exts, totalCatSize, segmentColors, svgSize);
    chartWrap.appendChild(svg);
    // Center label
    var centerLabel = document.createElement('div');
    centerLabel.className = 'stats-donut-center';
    var centerVal = document.createElement('div');
    centerVal.className = 'stats-donut-center-value';
    centerVal.textContent = formatSize(totalCatSize);
    var centerSub = document.createElement('div');
    centerSub.className = 'stats-donut-center-sub';
    centerSub.textContent = formatNumber(totalCatCount) + ' ' + LANG.files;
    centerLabel.appendChild(centerVal);
    centerLabel.appendChild(centerSub);
    chartWrap.appendChild(centerLabel);
    row.appendChild(chartWrap);

    // Legend (clickable items → show extension files)
    var legend = document.createElement('div');
    legend.className = 'stats-detail-legend';
    exts.forEach(function(e, i) {
        var pct = totalCatSize > 0 ? (e.size / totalCatSize * 100) : 0;
        var item = document.createElement('div');
        item.className = 'stats-legend-item';
        item.dataset.ext = e.ext;
        item.onclick = (function(extName) { return function() { showExtensionFiles(extName); }; })(e.ext);
        var dot = document.createElement('span');
        dot.className = 'stats-ext-dot';
        dot.style.background = segmentColors[i] || '#9E9E9E';
        var extLabel = document.createElement('span');
        extLabel.className = 'stats-legend-ext';
        extLabel.textContent = '.' + (e.ext || LANG.noExt);
        var info = document.createElement('span');
        info.className = 'stats-legend-info';
        info.textContent = formatNumber(e.count) + ' ' + LANG.files + ' \u00B7 ' + formatSize(e.size) + ' (' + pct.toFixed(1) + '%)';
        item.appendChild(dot);
        item.appendChild(extLabel);
        item.appendChild(info);
        legend.appendChild(item);
    });
    row.appendChild(legend);

    detail.textContent = '';
    detail.appendChild(row);
    detail.classList.add('open');
    toggler.textContent = '\u25BE';
    openCategoryDetail = catName;
}

var EXT_FILES_PAGE_SIZE = 500;
var extFilesDisplayed = 0;
var extFilesSortColumn = 'size';
var extFilesSortOrder = 'desc';
var extFilesCurrentResults = null;
var extFilesCurrentPanel = null;
var extFilesCurrentExt = null;

function showExtensionFiles(ext) {
    var detail = document.querySelector('.stats-category-detail.open');
    if (!detail) return;

    var existingPanel = detail.querySelector('.stats-ext-files');

    // Toggle off if same ext
    if (openExtensionFiles === ext && existingPanel) {
        existingPanel.remove();
        detail.style.maxHeight = '';
        openExtensionFiles = null;
        // Remove active from legend items
        detail.querySelectorAll('.stats-legend-item.active').forEach(function(el) { el.classList.remove('active'); });
        return;
    }

    // Remove existing panel
    if (existingPanel) existingPanel.remove();
    detail.querySelectorAll('.stats-legend-item.active').forEach(function(el) { el.classList.remove('active'); });

    // Highlight clicked legend item
    var legendItem = detail.querySelector('.stats-legend-item[data-ext="' + ext + '"]');
    if (legendItem) legendItem.classList.add('active');

    // Load search index if needed
    if (!fullSearchIndex) {
        initSearch();
    }
    if (!fullSearchIndex) return;

    // Filter by extension + folder scope
    var scopePrefix = '';
    var scopeId = currentFolderId;
    if (dirPaths[scopeId]) {
        var folderPath = dirPaths[scopeId].path;
        var rootPrefix = rootName + '/';
        if (folderPath.startsWith(rootPrefix)) {
            scopePrefix = folderPath.substring(rootPrefix.length) + '/';
        }
    }

    var extLower = ext.toLowerCase();
    var results = fullSearchIndex.filter(function(item) {
        if (item.is_dir) return false;
        if (scopePrefix && !item.path.startsWith(scopePrefix)) return false;
        var name = item.path.split('/').pop().toLowerCase();
        var dotIdx = name.lastIndexOf('.');
        var itemExt = dotIdx >= 0 ? name.substring(dotIdx + 1) : '';
        return itemExt === extLower;
    });

    extFilesDisplayed = EXT_FILES_PAGE_SIZE;
    extFilesSortColumn = 'size';
    extFilesSortOrder = 'desc';
    extFilesCurrentResults = results;
    extFilesCurrentExt = ext;
    openExtensionFiles = ext;

    var panel = document.createElement('div');
    panel.className = 'stats-ext-files';
    extFilesCurrentPanel = panel;
    renderExtensionFilesTable(panel, ext, results);
    detail.appendChild(panel);
    detail.style.maxHeight = 'none';
}

function setExtFilesSort(column) {
    if (column === extFilesSortColumn) {
        extFilesSortOrder = extFilesSortOrder === 'asc' ? 'desc' : 'asc';
    } else {
        extFilesSortColumn = column;
        extFilesSortOrder = column === 'name' || column === 'folder' ? 'asc' : 'desc';
    }
    extFilesDisplayed = EXT_FILES_PAGE_SIZE;
    if (extFilesCurrentPanel && extFilesCurrentResults) {
        extFilesCurrentPanel.textContent = '';
        renderExtensionFilesTable(extFilesCurrentPanel, extFilesCurrentExt, extFilesCurrentResults);
    }
}

function renderExtensionFilesTable(panel, ext, results) {
    // Sort results
    var sortedResults = results.slice();
    sortedResults.sort(function(a, b) {
        var isAsc = extFilesSortOrder === 'asc';
        var multiplier = isAsc ? 1 : -1;
        var nameA = a.path.split('/').pop().toLowerCase();
        var nameB = b.path.split('/').pop().toLowerCase();

        var valA, valB;
        if (extFilesSortColumn === 'name') {
            valA = nameA; valB = nameB;
        } else if (extFilesSortColumn === 'folder') {
            valA = a.path.substring(0, a.path.lastIndexOf('/')).toLowerCase();
            valB = b.path.substring(0, b.path.lastIndexOf('/')).toLowerCase();
        } else if (extFilesSortColumn === 'size') {
            valA = a.size || 0; valB = b.size || 0;
        } else {
            valA = a.modified || 0; valB = b.modified || 0;
        }

        if (valA < valB) return -1 * multiplier;
        if (valA > valB) return 1 * multiplier;
        if (nameA < nameB) return -1;
        if (nameA > nameB) return 1;
        return 0;
    });

    var displayCount = Math.min(extFilesDisplayed, sortedResults.length);
    var displayed = sortedResults.slice(0, displayCount);
    var totalSize = results.reduce(function(s, r) { return s + (r.size || 0); }, 0);
    var displayExt = ext ? ('.' + ext) : LANG.noExt;

    var getArrow = function(col) {
        if (extFilesSortColumn === col) return ' ' + (extFilesSortOrder === 'asc' ? '\u25B2' : '\u25BC');
        return ' \u2195';
    };

    // Header
    var header = document.createElement('div');
    header.className = 'stats-ext-files-header';
    header.textContent = displayExt + ' \u2014 ' + formatNumber(results.length) + ' ' + LANG.files + ' (' + formatSize(totalSize) + ')';

    // Table
    var table = document.createElement('table');
    table.className = 'stats-detail-table stats-ext-files-table';

    var colgroup = document.createElement('colgroup');
    var col1 = document.createElement('col');
    var col2 = document.createElement('col');
    var col3 = document.createElement('col'); col3.style.width = '80px';
    var col4 = document.createElement('col'); col4.style.width = '150px';
    colgroup.appendChild(col1);
    colgroup.appendChild(col2);
    colgroup.appendChild(col3);
    colgroup.appendChild(col4);
    table.appendChild(colgroup);

    var thead = document.createElement('thead');
    var headTr = document.createElement('tr');
    var columns = [
        { label: LANG.name, key: 'name' },
        { label: LANG.folder, key: 'folder' },
        { label: LANG.size, key: 'size' },
        { label: LANG.modified, key: 'modified' }
    ];
    columns.forEach(function(c) {
        var th = document.createElement('th');
        th.style.cursor = 'pointer';
        th.style.userSelect = 'none';
        th.textContent = c.label + getArrow(c.key);
        th.onclick = (function(key) { return function() { setExtFilesSort(key); }; })(c.key);
        headTr.appendChild(th);
    });
    thead.appendChild(headTr);
    table.appendChild(thead);

    var tbody = document.createElement('tbody');

    if (results.length === 0) {
        var tr = document.createElement('tr');
        var td = document.createElement('td');
        td.setAttribute('colspan', '4');
        td.style.textAlign = 'center';
        td.style.padding = '20px';
        td.textContent = LANG.noResults;
        tr.appendChild(td);
        tbody.appendChild(tr);
    } else {
        displayed.forEach(function(item) {
            var tr = document.createElement('tr');
            var itemName = item.path.split('/').pop();
            var itemPath = item.path.substring(0, item.path.lastIndexOf('/'));
            var parentId = item.parent_id || '0';

            var tdName = document.createElement('td');
            var nameContainer = document.createElement('div');
            nameContainer.className = 'item-name-container';
            var icon = document.createElement('span');
            icon.className = 'file-icon';
            nameContainer.appendChild(icon);
            var nameLink = document.createElement('a');
            nameLink.href = '#';
            nameLink.className = 'item-name';
            nameLink.textContent = itemName;
            nameLink.title = itemName;
            nameLink.onclick = (function(fid) { return function(e) { e.preventDefault(); statsMode = false; updateToggleButtons(); showFolder(fid); }; })(parentId);
            nameContainer.appendChild(nameLink);
            tdName.appendChild(nameContainer);

            var tdFolder = document.createElement('td');
            if (itemPath) {
                var folderLink = document.createElement('a');
                folderLink.href = '#';
                folderLink.textContent = itemPath;
                folderLink.title = itemPath;
                folderLink.onclick = (function(fid) { return function(e) { e.preventDefault(); statsMode = false; updateToggleButtons(); showFolder(fid); }; })(parentId);
                tdFolder.appendChild(folderLink);
            } else {
                tdFolder.innerHTML = '<span style="color:#999">/</span>';
            }

            var tdSize = document.createElement('td');
            tdSize.className = 'size';
            tdSize.textContent = formatSize(item.size);

            var tdDate = document.createElement('td');
            tdDate.className = 'date';
            tdDate.textContent = formatDate(item.modified);

            tr.appendChild(tdName);
            tr.appendChild(tdFolder);
            tr.appendChild(tdSize);
            tr.appendChild(tdDate);
            tbody.appendChild(tr);
        });

        if (displayCount < sortedResults.length) {
            var remaining = sortedResults.length - displayCount;
            var nextBatch = Math.min(remaining, EXT_FILES_PAGE_SIZE);
            var moreTr = document.createElement('tr');
            var moreTd = document.createElement('td');
            moreTd.setAttribute('colspan', '4');
            moreTd.style.textAlign = 'center';
            moreTd.style.padding = '12px';
            var moreLink = document.createElement('a');
            moreLink.href = '#';
            moreLink.style.color = '#006699';
            moreLink.style.fontWeight = 'bold';
            moreLink.textContent = 'Show ' + formatNumber(nextBatch) + ' more (' + formatNumber(remaining) + ' remaining)';
            moreLink.onclick = function(e) {
                e.preventDefault();
                extFilesDisplayed += EXT_FILES_PAGE_SIZE;
                panel.textContent = '';
                renderExtensionFilesTable(panel, ext, results);
            };
            moreTd.appendChild(moreLink);
            moreTr.appendChild(moreTd);
            tbody.appendChild(moreTr);
        }
    }

    table.appendChild(tbody);

    panel.textContent = '';
    panel.appendChild(header);
    panel.appendChild(table);
}

function buildDonutSVG(exts, totalSize, shades, size) {
    var ns = 'http://www.w3.org/2000/svg';
    var svg = document.createElementNS(ns, 'svg');
    svg.setAttribute('width', size);
    svg.setAttribute('height', size);
    svg.setAttribute('viewBox', '0 0 ' + size + ' ' + size);

    var cx = size / 2;
    var cy = size / 2;
    var strokeWidth = 30;
    var radius = (size - strokeWidth) / 2 - 2;
    var circumference = 2 * Math.PI * radius;

    if (totalSize === 0 || exts.length === 0) return svg;

    var offset = 0;
    exts.forEach(function(e, i) {
        var fraction = e.size / totalSize;
        var dashLen = fraction * circumference;
        var gap = circumference - dashLen;

        var circle = document.createElementNS(ns, 'circle');
        circle.setAttribute('cx', cx);
        circle.setAttribute('cy', cy);
        circle.setAttribute('r', radius);
        circle.setAttribute('fill', 'none');
        circle.setAttribute('stroke', shades[i] || '#ccc');
        circle.setAttribute('stroke-width', strokeWidth);
        circle.setAttribute('stroke-dasharray', dashLen.toFixed(2) + ' ' + gap.toFixed(2));
        circle.setAttribute('stroke-dashoffset', (-offset).toFixed(2));
        circle.setAttribute('transform', 'rotate(-90 ' + cx + ' ' + cy + ')');
        circle.setAttribute('pointer-events', 'stroke');
        circle.style.cursor = 'pointer';
        circle.onclick = (function(extName) { return function() { showExtensionFiles(extName); }; })(e.ext);
        svg.appendChild(circle);

        offset += dashLen;
    });

    return svg;
}

var DONUT_PALETTE = [
    '#4e79a7','#f28e2b','#e15759','#76b7b2','#59a14f',
    '#edc948','#b07aa1','#ff9da7','#9c755f','#bab0ac',
    '#5fa2ce','#d4a6c8'
];

function getSegmentColors(count) {
    var colors = [];
    for (var i = 0; i < count; i++) {
        colors.push(DONUT_PALETTE[i % DONUT_PALETTE.length]);
    }
    return colors;
}

function findCategory(ext) {
    for (var cat in FILE_CATEGORIES) {
        if (FILE_CATEGORIES[cat].indexOf(ext) !== -1) return cat;
    }
    return 'other';
}

// --- Utility Functions ---
function formatSize(bytes) {
    if (bytes === undefined || bytes === null || isNaN(bytes)) return '0 B';
    if (bytes < 1) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    if (i === 0) return Math.round(bytes) + ' B';
    return (bytes / Math.pow(k, i)).toFixed(1) + ' ' + sizes[i];
}

function formatDate(timestamp) {
    if (!timestamp) return '-';
    const d = new Date(timestamp * 1000);
    return window.innerWidth <= 768 ? d.toLocaleDateString() : d.toLocaleString();
}

// --- Initialisation ---
// {{AUTO_INIT}}

// --- Mobile Menu Functions ---
function toggleMobileMenu() {
    const tree = document.getElementById('tree');
    const backdrop = document.getElementById('tree-backdrop');
    tree.classList.toggle('open');
    backdrop.classList.toggle('active');
    document.body.style.overflow = tree.classList.contains('open') ? 'hidden' : '';
}

function closeMobileMenu() {
    const tree = document.getElementById('tree');
    const backdrop = document.getElementById('tree-backdrop');
    tree.classList.remove('open');
    backdrop.classList.remove('active');
    document.body.style.overflow = '';
}

// Auto-close mobile menu when folder is selected
const originalShowFolder = showFolder;
showFolder = async function(folderId, isInitialLoad) {
    if (isInitialLoad === undefined) isInitialLoad = false;
    if (window.innerWidth <= 768 && !isInitialLoad) {
        closeMobileMenu();
    }
    return originalShowFolder(folderId, isInitialLoad);
};

// --- Loading Indicator Functions ---
function showLoading(message) {
    if (message === undefined) message = LANG.loading + '...';
    const wrapper = document.getElementById('content-table-wrapper');
    if (!wrapper.querySelector('.loading-overlay')) {
        const overlay = document.createElement('div');
        overlay.className = 'loading-overlay';
        const spinner = document.createElement('span');
        spinner.className = 'loading-spinner';
        overlay.appendChild(spinner);
        overlay.appendChild(document.createTextNode(message));
        wrapper.style.position = 'relative';
        wrapper.appendChild(overlay);
    }
}

function hideLoading() {
    const overlay = document.querySelector('.loading-overlay');
    if (overlay) overlay.remove();
}

// --- Search-as-you-type ---
let searchTimer;
searchInput.form.addEventListener('submit', function(e) {
    e.preventDefault();
    clearTimeout(searchTimer);
    handleSearchLogic();
});

searchInput.addEventListener('input', function() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(handleSearchLogic, 300);
});

let resizeTimer;
window.addEventListener('resize', function() {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(function() {
        if (currentView === 'folder') renderCurrentFolderContent();
        else if (currentView === 'search') renderSearchResults();
        else if (currentView === 'statistics') renderStatistics();
    }, 200);
});

// --- Column Resize ---
document.getElementById('content-table-wrapper').addEventListener('mousedown', function(e) {
    var handle = e.target.closest('.col-resize');
    if (!handle) return;
    e.preventDefault();
    e.stopPropagation();
    colResizing = true;
    var th = handle.parentElement;
    var table = th.closest('table');
    var cols = table.querySelectorAll('colgroup col');
    var colIdx = parseInt(handle.dataset.col, 10);
    var col = cols[colIdx];
    var startX = e.clientX;
    var startW = col.offsetWidth || parseInt(col.style.width, 10) || th.offsetWidth;
    handle.classList.add('active');

    function onMove(ev) {
        // Handle sits on right edge of left column; dragging left = right column grows
        var newW = Math.max(50, startW - (ev.clientX - startX));
        col.style.width = newW + 'px';
    }
    function onUp() {
        handle.classList.remove('active');
        document.removeEventListener('mousemove', onMove);
        document.removeEventListener('mouseup', onUp);
        // Save widths from current colgroup
        var allCols = table.querySelectorAll('colgroup col');
        var sizeIdx = currentView === 'search' ? 2 : 1;
        var modIdx = currentView === 'search' ? 3 : 2;
        savedColWidths = {
            size: allCols[sizeIdx].offsetWidth || parseInt(allCols[sizeIdx].style.width, 10),
            modified: allCols[modIdx].offsetWidth || parseInt(allCols[modIdx].style.width, 10)
        };
        setTimeout(function() { colResizing = false; }, 50);
    }
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
});

// --- Tree Panel Resize ---
(function() {
    var handle = document.getElementById('tree-resize');
    var tree = document.getElementById('tree');
    if (!handle || !tree) return;

    handle.addEventListener('mousedown', function(e) {
        e.preventDefault();
        handle.classList.add('active');
        var startX = e.clientX;
        var startW = tree.offsetWidth;

        function onMove(ev) {
            var newW = Math.max(120, Math.min(startW + (ev.clientX - startX), window.innerWidth * 0.5));
            tree.style.width = newW + 'px';
        }
        function onUp() {
            handle.classList.remove('active');
            document.removeEventListener('mousemove', onMove);
            document.removeEventListener('mouseup', onUp);
        }
        document.addEventListener('mousemove', onMove);
        document.addEventListener('mouseup', onUp);
    });
})();
