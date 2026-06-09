# Synapse Chrome Debugger Bridge

This unpacked MV3 extension lets the Synapse daemon inspect and control the
user's normal Chrome profile through `chrome.debugger` plus Chrome Native
Messaging. It is the existing-profile CDP path for Chrome 136+ when the default
profile has no raw `--remote-debugging-port`.

Stable extension ID: `leoocgnkjnplbfdbklajepahofecgfbk`

Native host name: `com.synapse.chrome_debugger`

Install the native host registration with:

```powershell
scripts\install-synapse-chrome-debugger.ps1
```

Then load this directory as an unpacked extension from `chrome://extensions`.
The extension keeps one `runtime.connectNative()` port open and sends real CDP
commands only after the daemon asks through the local authenticated bridge.
