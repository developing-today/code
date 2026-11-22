# PM console for Android

The screens are the site itself: the app opens
`https://pm.security.cab/console/` in Chrome without browser chrome (a
Trusted Web Activity). So it does everything the site does, signs in the
way the site does (Chrome's own session), and needs no release when the
console changes.

Around the site, the app uses what only an installed app has:

| What                 | How it is used                                                                                                            | Code                                  |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------- |
| Home-screen widget   | In progress, queued next, needs me, and the task being worked on. Each number opens its screen                            | `StatusWidget`, `Fleet`               |
| Dictation            | Long-press the icon and choose Dictate, or tap the "Ask the fleet" quick-settings tile: the words land in the Ask box     | `DictateActivity`, `AskTileService`   |
| Share and selection  | Share text from any app, or select text and choose "Ask the fleet": it lands in the Ask box                               | `ShareActivity`                       |
| Long-press shortcuts | Dictate, Ask, Needs me, Now. They move the open app to that screen and do not reload it                                   | `OpenActivity`, `res/xml/shortcuts`   |
| Notifications        | The site's notifications arrive as the app's own, with its mark in the status bar; the app asks for the permission itself | the helper library's activities       |
| Links                | `https://pm.security.cab/...` links open in the app                                                                       | the launcher activity's intent filter |

Words handed to the site (shared, selected or dictated) travel after the
`#` of the address, which a browser never sends to a server; they sit in
the Ask box until the owner presses Send.

### The widget's own sign-in

A widget cannot use the browser's session, so the phone is linked once:

1. Add the "PM status" widget to the home screen and tap it.
2. The app opens at More, Phones, showing "Link this phone?". Approve.
3. The widget fills in. It reads again every half hour, when the app is
   opened, and when its refresh mark is tapped.

The phone makes a random value and shows the site only its hash; after
the approval it presents the value itself and is given a token that can
read the widget's summary and nothing else. The owner can remove a phone
under More, Phones, and the widget then goes back to "Tap to link".

## How the app and the site trust each other

- The app names the site in its manifest (`asset_statements`).
- The site names the app in `/.well-known/assetlinks.json`: package
  `today.developing.pmconsole`, signed by the certificate in
  [`signing-fingerprint.txt`](signing-fingerprint.txt). `pm-console`
  serves that file from `PM_CONSOLE_ANDROID_SHA256`.

If the two disagree the app still works, with a browser bar at the top.

## Build

```bash
gh workflow run pm-console-android.yml        # builds the APK in CI
bash pm-stack/console/android/build.sh        # locally: needs JAVA_HOME (17 or 21) and ANDROID_HOME
bash pm-stack/console/android/sign.sh app-release-unsigned.apk   # sign a CI build with the key on this machine
```

The signing key is on the pm-stack host at
`~/.local/state/pm-console/android/release.p12`, and it stays there. The
workflow builds an unsigned APK and keeps it as the artifact
`pm-console-apk-unsigned`; `sign.sh` on the host signs it, checks the
certificate against `signing-fingerprint.txt`, and the signed
`pm-console.apk` is uploaded to the release:

```bash
gh run download <run-id> -n pm-console-apk-unsigned -D /tmp/pm-apk
nix shell nixpkgs#apksigner -c bash pm-stack/console/android/sign.sh /tmp/pm-apk/app-release-unsigned.apk
gh release upload pm-console-android /tmp/pm-apk/pm-console.apk --clobber
```

If the repository secrets `PM_CONSOLE_ANDROID_KEYSTORE_B64` and
`PM_CONSOLE_ANDROID_KEYSTORE_PASSWORD` are ever set, the workflow signs
and publishes by itself instead, and fails unless the certificate
matches. Losing the key means a new key, a new fingerprint and a
reinstall; nothing else depends on it.

## Install

On the phone, open `https://pm.security.cab/console/`, sign in, and go to
More, Phones, "Download the app". That downloads the signed build from
the host (`/console/pm-console.apk`, owner only); open it to install.
Later builds install over it.

The native parts are checked by compiling the build and reading its
merged manifest; the site's side of each (the Ask box taking handed-over
words, linking a phone, the summary) is tested in `test/`. They have not
been exercised by an automated run on a device.

The console serves whatever signed build is at
`~/.local/state/pm-console/android/pm-console.apk` on the host, so
putting a new build in place is the whole release:

```bash
install -m 644 /tmp/pm-apk/pm-console.apk ~/.local/state/pm-console/android/pm-console.apk
```

The same file can also be attached to the GitHub release
`pm-console-android` for a copy outside the host.
