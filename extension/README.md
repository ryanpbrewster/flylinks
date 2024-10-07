# Permissions

We request these permissions:
```
  "permissions": [
    "activeTab",
    "declarativeNetRequestFeedback",
    "declarativeNetRequestWithHostAccess",
    "nativeMessaging",
    "storage"
  ],
```

- `activeTab` is so that we can get the current URL of the active tab (for setting/fetching the short links for that URL).
- `declarativeNetRequestFeedback` is purely for debugging declarative net requests; it only works on unpacked extensions
- `declarativeNetRequestWithHostAccess` is for redirecting `go/blah` style requests
- `nativeMessaging` is for communicating between the popup and the service worker
- `storage` is for accessing local storage to set the namespace