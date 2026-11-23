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

## Build and release

GitHub Actions builds the app on every push to `main` that touches it
(`.github/workflows/pm-console-android.yml`), and by hand:

```bash
gh workflow run pm-console-android.yml
bash pm-stack/console/android/build.sh        # here: needs JAVA_HOME (17 or 21) and ANDROID_HOME
```

A build names its own version from its commit (`build.sh`): a code that
rises with the commit's time, so each build installs over the one
before, and a name that says the day and the commit.

What happens next depends on who holds the signing key.

**GitHub holds it.** The build is signed in the workflow, checked
against the certificate in `signing-fingerprint.txt`, and released by
`release.sh` with nobody involved, twice:

- `pm-console-android-v<code>`: that build. Once made it is never
  changed.
- `pm-console-android`: one address that always holds the newest build
  and is never taken back to an older one. The site hands this one out.

Each carries `pm-console.apk`, `pm-console.apk.sha256` and a
`version.json` (name, code, commit, sha256, size, when, certificate).

**Only the laptop holds it** (how it starts). The workflow's build is
unsigned, kept as the artifact `pm-console-apk-unsigned`, and the run
says nothing was released. It can be signed and put in place by hand:

```bash
gh run download <run-id> -n pm-console-apk-unsigned -D /tmp/pm-apk
nix shell nixpkgs#apksigner -c bash pm-stack/console/android/sign.sh /tmp/pm-apk/app-release-unsigned.apk
install -m 644 /tmp/pm-apk/pm-console.apk ~/.local/state/pm-console/android/pm-console.apk
```

### Giving GitHub the key

The key is on the pm-stack host at
`~/.local/state/pm-console/android/release.p12`, with its password in
`keystore-password` beside it. The workflow signs once the
`pm-console-android` environment of the repository holds two secrets:

- `PM_CONSOLE_ANDROID_KEYSTORE_B64`: the keystore, base64;
- `PM_CONSOLE_ANDROID_KEYSTORE_PASSWORD`: its password.

Putting them there is the owner's to do, by hand, with `gh secret set
--env pm-console-android` or in the repository's settings. No script,
workflow or agent here holds or moves the key, on purpose. The
environment is made by the workflow's first run; limiting it to the
`main` branch (Settings, Environments) keeps a workflow on any other
branch from reading the key.

A build signed by any other key fails in `build.sh` before anything is
released. Losing the key means a new key, a new fingerprint and a
reinstall; nothing else depends on it.

## Install

On the phone, open `https://pm.security.cab/console/`, sign in, and go to
More, Phones, "Download the app" (`/console/pm-console.apk`, owner
only). What that is, in order:

1. the build GitHub released, fetched from the release and checked
   against its `version.json`; this needs nothing of the laptop;
2. otherwise the laptop's signed build
   (`~/.local/state/pm-console/android/pm-console.apk`), or the copy of
   it kept at Cloudflare when the laptop is away.

The page says which it is. Later builds install over it.

The native parts are checked by compiling the build and reading its
merged manifest; the site's side of each (the Ask box taking handed-over
words, linking a phone, the summary) is tested in `test/`. They have not
been exercised by an automated run on a device.
