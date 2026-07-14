// --- External file loader for multi-file mode ---

async function loadExternalJSON(path) {
    const response = await fetch(path);
    if (!response.ok) {
        throw new Error('HTTP ' + response.status + ' for ' + path);
    }
    if (path.endsWith('.gz')) {
        const arrayBuffer = await response.arrayBuffer();
        const decompressed = pako.inflate(new Uint8Array(arrayBuffer), { to: 'string' });
        return JSON.parse(decompressed);
    } else {
        return response.json();
    }
}

// Try .gz first, then fallback to plain JSON
async function loadExternalJSONWithFallback(basePath) {
    try {
        return await loadExternalJSON(basePath + '.gz');
    } catch (e) {
        console.log('Trying plain JSON for:', basePath);
        return await loadExternalJSON(basePath);
    }
}

// Override init for multi-file mode
async function init() {
    try {
        chunkIndex = await loadExternalJSONWithFallback('data/chunk_index.json');
        window.useCompression = true;

        // Preload first chunk
        await loadExternalChunk(0);

        await loadTree();
        await showFolder('0', true);

        // Search index loaded on-demand when user searches (lazy load)
    } catch (e) {
        console.error('Init error:', e);
        var tree = document.getElementById('tree');
        tree.textContent = '';
        var errDiv = document.createElement('div');
        errDiv.className = 'loading';
        errDiv.textContent = LANG.loadError + ': ' + e.message;
        tree.appendChild(errDiv);
    }
}

async function loadExternalChunk(chunkId) {
    if (chunkCache[chunkId]) return;
    try {
        const data = await loadExternalJSONWithFallback('data/chunk_' + chunkId + '.json');
        if (data) {
            chunkCache[chunkId] = data;
        }
    } catch (e) {
        console.error('Error loading chunk:', chunkId, e);
    }
}

// Helper: ensure chunk for a dir is loaded
async function ensureChunkLoaded(dirId) {
    const chunkId = chunkIndex[dirId];
    if (chunkId !== undefined && !chunkCache[chunkId]) {
        await loadExternalChunk(chunkId);
    }
}

// Override loadDirData to support async chunk loading (fallback for sync callers)
var _origLoadDirData = loadDirData;
window.loadDirData = function(dirId) {
    const chunkId = chunkIndex[dirId];
    if (chunkId === undefined) return null;

    if (!chunkCache[chunkId]) {
        loadExternalChunk(chunkId).then(function() {
            if (currentView === 'folder' && currentFolderId === dirId) {
                const data = chunkCache[chunkId] ? chunkCache[chunkId][dirId] : null;
                if (data) {
                    currentFolderData = decodeData(data);
                    renderCurrentFolderContent();
                }
            }
        });
        return null;
    }
    return chunkCache[chunkId] ? chunkCache[chunkId][dirId] : null;
};

// Override loadDirStats for multi-file mode (async chunk loading)
var _origLoadDirStats = loadDirStats;
window.loadDirStats = function(dirId) {
    var chunkId = chunkIndex[dirId];
    if (chunkId === undefined) return null;
    if (!chunkCache[chunkId]) {
        loadExternalChunk(chunkId).then(function() {
            if (statsMode && currentFolderId === dirId) {
                showDirStats(dirId);
            }
        });
        return null;
    }
    return chunkCache[chunkId] ? chunkCache[chunkId][dirId + '_stats'] : null;
};

// Override expandNode: preload chunk, then call original
var _origExpandNode = expandNode;
window.expandNode = async function(liElement, forceOpen) {
    const folderId = liElement.dataset.id;
    await ensureChunkLoaded(folderId);
    return _origExpandNode.call(this, liElement, forceOpen);
};

// Override showFolder: preload chunk, then call original
var _origShowFolder = showFolder;
window.showFolder = async function(folderId, isInitialLoad) {
    await ensureChunkLoaded(folderId);
    return _origShowFolder(folderId, isInitialLoad);
};

// Override initSearch for external files
window.initSearch = async function() {
    if (fullSearchIndex) return;
    try {
        fullSearchIndex = await loadExternalJSONWithFallback('full_index.json');
    } catch (e) {
        console.error("Failed to load search index:", e);
    }
};

// Start initialization when DOM is ready
if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
} else {
    init();
}
