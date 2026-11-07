// Executed once in Firefox's own chrome context during local profile startup.
// Uses native browsing-context configuration; never patches web Date/Intl or webdriver.
function nodecloakRegionWindow() {
  const win = Services.wm.getMostRecentWindow('navigator:browser');
  if (!win?.gBrowser || win.gBrowserInit?.delayedStartupFinished !== true) return null;
  // A tab exists before Firefox consumes its command-line URI. In existing
  // profiles that URI can also wait on session startup. Let its own callbacks
  // finish before navigating, otherwise their about:blank replaces our gate.
  const initial = win.gBrowserInit.uriToLoadPromise;
  if (initial && typeof initial.then === 'function') {
    const key = Symbol.for('nodecloak.region.initial-uri');
    let state = win[key];
    if (!state) {
      state = win[key] = {ready:false,failed:false};
      Promise.resolve(initial).then(() => {
        Services.tm.dispatchToMainThread(() => {state.ready = true;});
      }, () => {state.failed = true;});
    }
    if (state.failed) throw Error('NATIVE_REGION_INITIAL_URI_FAILED');
    if (!state.ready) return null;
  }
  return win;
}

function nodecloakConfigureRegion(config) {
  if (!nodecloakRegionWindow()) return 'pending';
  const nativeSetter = (value, key) => {
    for (let proto = Object.getPrototypeOf(value); proto; proto = Object.getPrototypeOf(proto)) {
      const descriptor = Object.getOwnPropertyDescriptor(proto, key);
      if (descriptor) return typeof descriptor.get === 'function' && typeof descriptor.set === 'function';
    }
    return false;
  };
  const capable = bc => nativeSetter(bc, 'languageOverride') &&
    (config.timezone === null || nativeSetter(bc, 'timezoneOverride')) &&
    (!config.width || nativeSetter(bc, 'overrideDPPX'));
  let checked = false;
  for (const win of Services.wm.getEnumerator('navigator:browser')) {
    if (!win.gBrowser) continue; // RDP can become ready before browser chrome.
    for (const browser of win.gBrowser.browsers) {
      if (!capable(browser.browsingContext)) throw Error('NATIVE_REGION_UNSUPPORTED');
      if (config.width && typeof browser.ownerDocument?.synchronouslyUpdateRemoteBrowserDimensions !== 'function') {
        throw Error('NATIVE_REGION_UNSUPPORTED');
      }
      checked = true;
    }
  }
  if (!checked) return 'pending';
  const apply = bc => {
    if (!bc.isContent || bc.parent || bc.isDiscarded) return;
    if (!capable(bc)) throw Error('NATIVE_REGION_UNSUPPORTED');
    bc.languageOverride = config.locale;
    if (config.timezone !== null) bc.timezoneOverride = config.timezone;
    if (bc.languageOverride !== config.locale || (config.timezone !== null && bc.timezoneOverride !== config.timezone)) {
      throw Error('NATIVE_REGION_NOT_APPLIED');
    }
    if (config.width > 0) {
      bc.overrideDPPX = 1;
      const browser = bc.embedderElement;
      if (browser) {
        browser.style.setProperty('width', `${config.width}px`);
        browser.style.setProperty('height', `${config.height}px`);
        browser.ownerDocument.synchronouslyUpdateRemoteBrowserDimensions(true);
      }
    }
  };
  const tab = event => {
    const browser = event.target.linkedBrowser;
    if (browser?.browsingContext) apply(browser.browsingContext);
  };
  const windows = new Set();
  const watchWindow = win => {
    if (!win.gBrowser || windows.has(win)) return;
    windows.add(win);
    win.gBrowser.tabContainer.addEventListener('TabOpen', tab);
    win.gBrowser.tabContainer.addEventListener('TabRemotenessChange', tab);
    win.addEventListener('unload', () => windows.delete(win), {once:true});
    for (const browser of win.gBrowser.browsers) apply(browser.browsingContext);
  };
  const opened = {observe(win) {
    win.addEventListener('load', () => watchWindow(win), {once:true});
  }};
  const attached = {observe:apply};
  Services.prefs.setCharPref('intl.accept_languages', config.language);
  Services.prefs.setCharPref('intl.locale.requested', config.locale);
  Services.prefs.setBoolPref('intl.regional_prefs.use_os_locales', false);
  if (config.geoUri !== null) {
    Services.prefs.setCharPref('geo.provider.network.url', config.geoUri);
    Services.prefs.setBoolPref('geo.provider.testing', true);
    Services.prefs.setBoolPref('geo.provider.network.scan', false);
    Services.prefs.setIntPref('geo.provider.network.timeToWaitBeforeSending', 100);
  }
  Services.obs.addObserver(attached, 'browsing-context-attached');
  Services.obs.addObserver(attached, 'browsing-context-did-set-embedder');
  Services.obs.addObserver(opened, 'domwindowopened');
  for (const win of Services.wm.getEnumerator('navigator:browser')) watchWindow(win);
  const cleanup = {observe() {
    Services.obs.removeObserver(attached, 'browsing-context-attached');
    Services.obs.removeObserver(attached, 'browsing-context-did-set-embedder');
    Services.obs.removeObserver(opened, 'domwindowopened');
    Services.obs.removeObserver(cleanup, 'quit-application');
    for (const win of windows) {
      win.gBrowser.tabContainer.removeEventListener('TabOpen', tab);
      win.gBrowser.tabContainer.removeEventListener('TabRemotenessChange', tab);
    }
    windows.clear();
  }};
  Services.obs.addObserver(cleanup, 'quit-application');
  return 'configured';
}

function nodecloakQueryRegion(urls, key) {
  const state = globalThis[Symbol.for(key)] = {status:'pending'};
  (async () => {
    try {
      let body = null; let last = '';
      for (const url of urls) {
        try {
          const response = await fetch(url, {cache:'no-store', signal:AbortSignal.timeout(6000)});
          if (!response.ok) throw Error('LOOKUP_STATUS');
          const reader = response.body.getReader();
          const chunks = []; let size = 0;
          try {
            while (true) {
              const next = await reader.read();
              if (next.done) break;
              size += next.value.length;
              if (size > 65536) throw Error('LOOKUP_SIZE');
              chunks.push(next.value);
            }
          } finally { await reader.cancel(); }
          const bytes = new Uint8Array(size); let offset = 0;
          for (const chunk of chunks) {bytes.set(chunk, offset); offset += chunk.length;}
          const value = JSON.parse(new TextDecoder().decode(bytes));
          if (value.success !== true) throw Error('LOOKUP_REJECTED');
          // Return only the fixed provider schema, bounded before crossing RDP.
          const timezone = value.timezone && typeof value.timezone === 'object' ? value.timezone.id : value.timezone;
          body = JSON.stringify({success:true, ip:value.ip, country_code:value.country_code,
            city:value.city, latitude:value.latitude, longitude:value.longitude, timezone:{id:timezone}});
          if (body.length > 900) throw Error('LOOKUP_SHAPE');
          break;
        } catch (error) { last = url + ': ' + String(error); body = null; }
      }
      if (body === null) throw Error('LOOKUP_FAILED ' + last);
      state.body = body; state.status = 'ready';
    } catch (error) { state.error = String(error).slice(0, 600); state.status = 'failed'; }
  })();
  return 'pending';
}

function nodecloakFinishRegion(url, restore, resumeSession = false) {
  // Close listeners before loading content. Closing may also close this RDP
  // connection, so the independent web startup gate confirms successful handoff.
  const api = ChromeUtils.importESModule(
    'resource://devtools/shared/loader/DistinctSystemPrincipalLoader.sys.mjs', {global:'shared'});
  const owner = {};
  const loader = api.useDistinctSystemPrincipalLoader(owner);
  const {DevToolsServer} = loader.require('resource://devtools/server/devtools-server.js');
  // Closing DevTools retires the evaluation actor itself. Queue the browser's
  // handoff first so it survives that teardown, but cannot run until this
  // synchronous stack has closed the listening sockets.
  const win = nodecloakRegionWindow();
  if (!win) throw Error('NATIVE_REGION_STARTUP_PENDING');
  const browser = win.gBrowser.selectedBrowser;
  const uri = Services.io.newURI(url);
  const principal = Services.scriptSecurityManager.getSystemPrincipal();
  Services.tm.dispatchToMainThread(() => {
    for (const [name, value] of restore) {
      if (value === null) Services.prefs.clearUserPref(name);
      else if (typeof value === 'boolean') Services.prefs.setBoolPref(name, value);
      else throw Error('INVALID_RESTORE');
    }
    if(!resumeSession) browser.loadURI(uri, {triggeringPrincipal:principal});
    win.focus();
    browser.focus();
  });
  DevToolsServer.closeAllSocketListeners();
  api.releaseDistinctSystemPrincipalLoader(owner);
  return 'finished';
}
