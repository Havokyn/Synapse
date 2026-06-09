const HOST_NAME = "com.synapse.chrome_debugger";
const PROTOCOL_VERSION = 1;
const ERROR_ATTACH_FAILED = "A11Y_CDP_ATTACH_FAILED";
const ERROR_AXTREE_FAILED = "A11Y_CDP_AXTREE_FAILED";
const ERROR_EXTENSION_DETACHED = "A11Y_CDP_EXTENSION_DETACHED";
const ERROR_EXTENSION_UNAVAILABLE = "A11Y_CDP_EXTENSION_UNAVAILABLE";
const DETACH_SURFACE_MS = 5000;

let nativePort = null;
let reconnectTimer = null;
const attachedTabs = new Set();
const intentionalDetachTabs = new Set();
const recentDetachByTab = new Map();

function connectNative() {
  if (nativePort) {
    return;
  }
  try {
    nativePort = chrome.runtime.connectNative(HOST_NAME);
  } catch (error) {
    nativePort = null;
    scheduleReconnect(`connectNative failed: ${errorMessage(error)}`);
    return;
  }
  nativePort.onMessage.addListener((message) => {
    handleCommand(message).catch((error) => {
      postResponse(message?.id ?? "", false, null, errorPayload(error));
    });
  });
  nativePort.onDisconnect.addListener(() => {
    const detail = chrome.runtime.lastError?.message || "native port disconnected";
    nativePort = null;
    scheduleReconnect(detail);
  });
  postNative({
    type: "hello",
    extensionId: chrome.runtime.id,
    version: chrome.runtime.getManifest().version,
    protocolVersion: PROTOCOL_VERSION,
    userAgent: navigator.userAgent
  });
}

function scheduleReconnect(detail) {
  if (reconnectTimer) {
    return;
  }
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectNative();
  }, 1000);
  console.warn(`Synapse native bridge disconnected: ${detail}`);
}

chrome.runtime.onInstalled.addListener(connectNative);
chrome.runtime.onStartup.addListener(connectNative);
chrome.debugger.onDetach.addListener((source, reason) => {
  const tabId = source.tabId;
  const intentional = typeof tabId === "number" && intentionalDetachTabs.has(tabId);
  if (typeof source.tabId === "number") {
    attachedTabs.delete(source.tabId);
    if (intentional) {
      intentionalDetachTabs.delete(source.tabId);
    } else {
      recentDetachByTab.set(source.tabId, Date.now());
    }
  }
  postNative({
    type: "event",
    event: "debuggerDetached",
    tabId: source.tabId ?? null,
    targetId: source.targetId ?? null,
    intentional,
    reason
  });
});

connectNative();

async function handleCommand(command) {
  if (!command || typeof command !== "object") {
    throw bridgeError(ERROR_ATTACH_FAILED, "native command was not an object");
  }
  const { id, kind, params = {} } = command;
  if (!id || typeof id !== "string") {
    throw bridgeError(ERROR_ATTACH_FAILED, "native command id is required");
  }
  try {
    let result;
    if (kind === "snapshot") {
      result = await handleSnapshot(params);
    } else if (kind === "clickNode") {
      result = await handleClickNode(params);
    } else if (kind === "nodeValue") {
      result = await handleNodeValue(params);
    } else {
      throw bridgeError(ERROR_ATTACH_FAILED, `unknown command kind ${String(kind)}`);
    }
    postResponse(id, true, result, null);
  } catch (error) {
    postResponse(id, false, null, errorPayload(error));
  }
}

async function handleSnapshot(params) {
  const selected = await selectPageTarget(params);
  return await withAttached(selected, async (debuggee) => {
    await sendCdp(debuggee, "Accessibility.enable", {});
    const tree = await sendCdp(debuggee, "Accessibility.getFullAXTree", {});
    const axNodes = Array.isArray(tree.nodes) ? tree.nodes : [];
    const byAxId = new Map(axNodes.map((node) => [String(node.nodeId), node]));
    const maxNodes = Math.max(0, Number(params.maxNodes ?? 200));
    const domNodes = [];
    let totalAxNodes = 0;
    for (const node of axNodes) {
      if (node.ignored) {
        continue;
      }
      totalAxNodes += 1;
      const backend = Number(node.backendDOMNodeId);
      if (!Number.isFinite(backend)) {
        continue;
      }
      const role = axValueString(node.role);
      if (!role) {
        continue;
      }
      const bbox = domNodes.length < maxNodes
        ? await boxForBackend(debuggee, backend)
        : null;
      domNodes.push({
        backend_node_id: backend,
        parent_backend_node_id: nearestBackendAncestor(node, byAxId),
        role,
        name: axValueString(node.name),
        value: nonEmptyOrNull(axValueString(node.value)),
        bbox,
        child_count: Array.isArray(node.childIds) ? node.childIds.length : 0,
        enabled: !axBoolProperty(node, "disabled"),
        focused: axBoolProperty(node, "focused")
      });
    }
    return {
      extension_id: chrome.runtime.id,
      nodes: domNodes,
      total_ax_nodes: totalAxNodes,
      page_url: selected.url || "",
      target_id: selected.target.id,
      session_id: `tab-${selected.tabId}`,
      target_candidate_count: selected.targetCandidateCount,
      target_selection_reason: selected.selectionReason
    };
  });
}

async function handleClickNode(params) {
  const backendNodeId = requiredNumber(params.backendNodeId, "backendNodeId");
  const selected = await selectPageTarget(params);
  return await withAttached(selected, async (debuggee) => {
    await sendCdp(debuggee, "DOM.scrollIntoViewIfNeeded", { backendNodeId });
    const bbox = await boxForBackend(debuggee, backendNodeId);
    if (!bbox || bbox.w <= 0 || bbox.h <= 0) {
      throw bridgeError(ERROR_AXTREE_FAILED, `backendNodeId ${backendNodeId} has no clickable box model`);
    }
    const point = {
      x: bbox.x + bbox.w / 2,
      y: bbox.y + bbox.h / 2
    };
    const button = normalizeButton(params.button);
    const clickCount = Math.max(1, Number(params.clickCount ?? 1));
    await sendCdp(debuggee, "Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: point.x,
      y: point.y,
      button: "none",
      buttons: 0,
      clickCount: 0
    });
    await sendCdp(debuggee, "Input.dispatchMouseEvent", {
      type: "mousePressed",
      x: point.x,
      y: point.y,
      button,
      buttons: buttonMask(button),
      clickCount
    });
    await sendCdp(debuggee, "Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x: point.x,
      y: point.y,
      button,
      buttons: 0,
      clickCount
    });
    return { x: point.x, y: point.y, target_id: selected.target.id };
  });
}

async function handleNodeValue(params) {
  const backendNodeId = requiredNumber(params.backendNodeId, "backendNodeId");
  const selected = await selectPageTarget(params);
  return await withAttached(selected, async (debuggee) => {
    const resolved = await sendCdp(debuggee, "DOM.resolveNode", { backendNodeId });
    const objectId = resolved?.object?.objectId;
    if (!objectId) {
      throw bridgeError(ERROR_AXTREE_FAILED, `DOM.resolveNode returned no objectId for backendNodeId ${backendNodeId}`);
    }
    const readback = await sendCdp(debuggee, "Runtime.callFunctionOn", {
      objectId,
      returnByValue: true,
      silent: true,
      functionDeclaration: `function() {
        if (this === null || this === undefined) { return ""; }
        if ("value" in this) { return String(this.value ?? ""); }
        if ("checked" in this) { return String(Boolean(this.checked)); }
        if (this.textContent !== null && this.textContent !== undefined) {
          return String(this.textContent);
        }
        return "";
      }`
    });
    return {
      value: String(readback?.result?.value ?? ""),
      target_id: selected.target.id
    };
  });
}

async function selectPageTarget(params) {
  const targets = await chrome.debugger.getTargets();
  const pages = targets.filter((target) => target.type === "page" && typeof target.tabId === "number");
  if (pages.length === 0) {
    throw bridgeError(ERROR_ATTACH_FAILED, "chrome.debugger.getTargets returned no page targets");
  }
  const targetIdHint = String(params.targetIdHint || "").trim();
  const urlHint = String(params.foregroundUrlHint || "").trim();
  const titleHint = String(params.foregroundTitle || "").trim();
  const selectedById = targetIdHint
    ? pages.find((target) => target.id === targetIdHint)
    : null;
  if (selectedById) {
    return selectedPage(selectedById, pages.length, "target_id_hint");
  }
  if (targetIdHint) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `targetIdHint ${targetIdHint} was not found in chrome.debugger.getTargets`
    );
  }
  if (urlHint) {
    const matches = pages.filter((target) => urlMatchesHint(target.url || "", urlHint));
    if (matches.length === 1) {
      return selectedPage(matches[0], pages.length, "url_hint");
    }
    if (matches.length > 1) {
      throw bridgeError(ERROR_AXTREE_FAILED, `url hint matched ${matches.length} page targets`);
    }
  }
  if (titleHint) {
    const matches = pages.filter((target) => {
      const title = target.title || "";
      return title && (titleHint.includes(title) || title.includes(titleHint));
    });
    if (matches.length === 1) {
      return selectedPage(matches[0], pages.length, "foreground_title");
    }
    if (matches.length > 1) {
      throw bridgeError(ERROR_AXTREE_FAILED, `title hint matched ${matches.length} page targets`);
    }
  }
  return selectedPage(pages[0], pages.length, "fallback_first_page");
}

function selectedPage(target, targetCandidateCount, selectionReason) {
  return {
    target,
    tabId: target.tabId,
    url: target.url || "",
    title: target.title || "",
    targetCandidateCount,
    selectionReason
  };
}

async function withAttached(selected, operation) {
  const debuggee = await attachForCommand(selected);
  try {
    return await operation(debuggee);
  } finally {
    await detachAfterCommand(debuggee, selected.tabId);
  }
}

async function attachForCommand(selected) {
  const tabId = selected.tabId;
  const recentDetachAt = recentDetachByTab.get(tabId);
  if (recentDetachAt && Date.now() - recentDetachAt < DETACH_SURFACE_MS) {
    throw bridgeError(
      ERROR_EXTENSION_DETACHED,
      `debugger detached from tab ${tabId}; reason was surfaced and command refused`
    );
  }
  const debuggee = { tabId };
  try {
    await chrome.debugger.attach(debuggee, "1.3");
  } catch (error) {
    if (!await existingAttachStillUsable(debuggee, error)) {
      throw bridgeError(ERROR_ATTACH_FAILED, `chrome.debugger.attach tab ${tabId}: ${errorMessage(error)}`);
    }
  }
  attachedTabs.add(tabId);
  await sendCdp(debuggee, "Runtime.enable", {});
  await sendCdp(debuggee, "DOM.enable", {});
  await sendCdp(debuggee, "Target.setAutoAttach", {
    autoAttach: true,
    waitForDebuggerOnStart: false,
    flatten: true,
    filter: [{ type: "iframe", exclude: false }]
  });
  return debuggee;
}

async function detachAfterCommand(debuggee, tabId) {
  if (!attachedTabs.has(tabId)) {
    return;
  }
  intentionalDetachTabs.add(tabId);
  try {
    await chrome.debugger.detach(debuggee);
  } catch (error) {
    intentionalDetachTabs.delete(tabId);
    throw bridgeError(ERROR_EXTENSION_DETACHED, `chrome.debugger.detach tab ${tabId}: ${errorMessage(error)}`);
  } finally {
    attachedTabs.delete(tabId);
  }
}

async function existingAttachStillUsable(debuggee, attachError) {
  if (!/another debugger/i.test(errorMessage(attachError))) {
    return false;
  }
  try {
    await chrome.debugger.sendCommand(debuggee, "Runtime.enable", {});
    return true;
  } catch (_) {
    return false;
  }
}

async function sendCdp(debuggee, method, params) {
  try {
    return await chrome.debugger.sendCommand(debuggee, method, params);
  } catch (error) {
    const message = errorMessage(error);
    if (/detached|not attached/i.test(message)) {
      throw bridgeError(ERROR_EXTENSION_DETACHED, `${method}: ${message}`);
    }
    throw bridgeError(ERROR_AXTREE_FAILED, `${method}: ${message}`);
  }
}

async function boxForBackend(debuggee, backendNodeId) {
  try {
    const response = await sendCdp(debuggee, "DOM.getBoxModel", { backendNodeId });
    return rectFromQuad(response?.model?.content || []);
  } catch (_) {
    return null;
  }
}

function nearestBackendAncestor(node, byAxId) {
  let parentId = node.parentId;
  let guard = 256;
  while (parentId && guard > 0) {
    const parent = byAxId.get(String(parentId));
    if (!parent) {
      return null;
    }
    const backend = Number(parent.backendDOMNodeId);
    if (Number.isFinite(backend)) {
      return backend;
    }
    parentId = parent.parentId;
    guard -= 1;
  }
  return null;
}

function rectFromQuad(quad) {
  if (!Array.isArray(quad) || quad.length < 8) {
    return null;
  }
  const xs = [quad[0], quad[2], quad[4], quad[6]].map(Number);
  const ys = [quad[1], quad[3], quad[5], quad[7]].map(Number);
  if (![...xs, ...ys].every(Number.isFinite)) {
    return null;
  }
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  return {
    x: Math.round(minX),
    y: Math.round(minY),
    w: Math.max(0, Math.round(maxX - minX)),
    h: Math.max(0, Math.round(maxY - minY))
  };
}

function axValueString(value) {
  if (!value || value.value === undefined || value.value === null) {
    return "";
  }
  return String(value.value);
}

function axBoolProperty(node, name) {
  const properties = Array.isArray(node.properties) ? node.properties : [];
  const found = properties.find((property) => property.name === name);
  return Boolean(found?.value?.value);
}

function nonEmptyOrNull(value) {
  return value ? value : null;
}

function urlMatchesHint(url, hint) {
  return url === hint || url.startsWith(hint) || hint.startsWith(url);
}

function requiredNumber(value, name) {
  const number = Number(value);
  if (!Number.isFinite(number)) {
    throw bridgeError(ERROR_ATTACH_FAILED, `${name} must be a finite number`);
  }
  return number;
}

function normalizeButton(button) {
  const normalized = String(button || "left").toLowerCase();
  if (["left", "right", "middle"].includes(normalized)) {
    return normalized;
  }
  throw bridgeError(ERROR_ATTACH_FAILED, `unsupported mouse button ${button}`);
}

function buttonMask(button) {
  if (button === "left") {
    return 1;
  }
  if (button === "right") {
    return 2;
  }
  if (button === "middle") {
    return 4;
  }
  return 0;
}

function postResponse(id, ok, result, error) {
  postNative({
    type: "response",
    id,
    ok,
    result,
    error
  });
}

function postNative(message) {
  if (!nativePort) {
    console.warn("Synapse native bridge unavailable; message dropped");
    return;
  }
  nativePort.postMessage(message);
}

function bridgeError(code, detail) {
  const error = new Error(detail);
  error.code = code;
  return error;
}

function errorPayload(error) {
  return {
    code: error?.code || ERROR_ATTACH_FAILED,
    detail: errorMessage(error)
  };
}

function errorMessage(error) {
  if (!error) {
    return "unknown error";
  }
  if (typeof error === "string") {
    return error;
  }
  return error.message || String(error);
}
