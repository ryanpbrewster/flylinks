const BASE = 'https://flylinks-backend-weathered-pond-9344.fly.dev';

chrome.runtime.onInstalled.addListener((details) => {
  console.log("[ON_INSTALLED]", details);
});

chrome.declarativeNetRequest.onRuleMatchedDebug.addListener((details) => {
  console.log("[ON_RULE_MATCHED", details);
});

let namespace = null;
function configureNamespace(newNamespace) {
  console.log(`configuring namespace=${newNamespace}`);
  namespace = newNamespace;
  chrome.declarativeNetRequest.getDynamicRules(previousRules => {
    console.log("[GET_DYNAMIC_RULES]", previousRules);
    const newRules = [{
      "id": 1,
      "priority": 1,
      "action": {
        "type": "redirect",
        "redirect": {
          "regexSubstitution": BASE + "/v1/redirect/" + newNamespace + "/\\1",
        },
      },
      "condition": {
        "regexFilter": "^https?://go/(.+)",
        "resourceTypes": [ "main_frame" ],
      },
    }];
    console.log({previousRules, newRules});
    chrome.declarativeNetRequest.updateDynamicRules({
      removeRuleIds: previousRules.map((rule) => rule.id),
      addRules: newRules
    }).then((ok) => console.log("OK", ok), (err) => console.error("ERR", err));
  });
}


// We store the flylinks namespace in Chrome sync storage.
// Fetch it on initial load, and also set up a listener to catch any further updates.
chrome.storage.sync.get('namespace', (result) => {
  configureNamespace(result.namespace || 'default');
  chrome.storage.onChanged.addListener((changes, area) => {
    if (changes.namespace) {
      configureNamespace(changes.namespace.newValue);
    }
  });
});

// The popup needs to communicate with us. Handle incoming messages.
chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  console.log("chrome.runtime.onMessage", msg, sender);
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
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({short_form: key, long_form: url}),
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
        method: 'POST',
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ 'long_form': url }),
      });
      const keys = await request.json();
      console.log(`keys for ${url}: `, keys);
      resolve(keys);
    });
  });
}
