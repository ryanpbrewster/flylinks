const BASE = 'https://flylinks-backend-weathered-pond-9344.fly.dev';

// We store the flylinks namespace in Chrome sync storage.
// Fetch it on initial load, and also set up a listener to catch any further updates.
let namespace = 'default';
chrome.storage.sync.get('namespace', (result) => {
  if (result.namespace) {
    namespace = result.namespace;
  }
  chrome.storage.onChanged.addListener((changes, area) => {
    if (changes.namespace) {
      namespace = changes.namespace.newValue;
    }
  });
});

// Intercept any "main_frame" requests that look like `go/asdf`. Redirect them
// to the flylinks backend (which will in return redirect them to the final
// destination).
chrome.webRequest.onBeforeRequest.addListener(
  (details) => {
    const url = new URL(details.url);
    if (url.hostname === 'go') {
      return { redirectUrl: BASE + '/v1/redirect/' + namespace + url.pathname };
    }
    return {};
  },
  {
    urls: ['*://go/*'],
    types: ['main_frame'],
  },
  ['blocking'],
);

// The popup needs to communicate with us. Handle incoming messages.
chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (msg.getNamespace) {
    return sendResponse(namespace);
  }
  if (msg.getKeys) {
    handleGetKeys()
      .then((resp) => sendResponse(resp))
      .catch(() => sendResponse('failure'));
    return true;
  }
  if (msg.setKey) {
    handleSetKey(msg.setKey.key)
      .then(() => sendResponse('success'))
      .catch(() => sendResponse('failure'));
    return true;
  }
  if (msg.setNamespace) {
    handleSetNamespace(msg.setNamespace.namespace)
      .then(() => sendResponse('success'))
      .catch(() => sendResponse('failure'));
    return true;
  }
  return false;
});

async function handleSetNamespace(namespace) {
  return new Promise((resolve, reject) => {
    chrome.storage.sync.set({ namespace }, () => {
      resolve();
    });
  });
}

async function handleSetKey(key) {
  return new Promise((resolve, reject) => {
    chrome.tabs.query({ active: true, currentWindow: true }, async (tabs) => {
      const url = tabs[0].url;
      console.log(`setting key ${key} = ${url}`);
      const resp = await fetch(BASE + '/v1/links/' + namespace, {
        method: 'POST',
        json: {short_form: key, long_form: url},
      });
      if (resp.status === 200) {
        resolve();
      } else {
        reject();
      }
    });
  });
}

async function handleGetKeys() {
  console.log(`getting keys`);
  return new Promise((resolve, reject) => {
    chrome.tabs.query({ active: true, currentWindow: true }, async (tabs) => {
      const url = tabs[0].url;
      console.log(`getting keys for ${url}`);
      const request = await fetch(BASE + '/v1/reverse_lookup/' + namespace, {
        json: { 'long_form': url },
      });
      const keys = await request.json();
      console.log(`keys for ${url}: `, keys);
      resolve(keys);
    });
  });
}
