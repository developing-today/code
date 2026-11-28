# Getting the exact bytes

Read this in step 1. The transcript has to be the bytes as the user delivered them, with no harness prefix, no added trailing newline and no tidying. A model that types text back out is retyping it, even inside a tool call, and retyping drifts. So copy bytes with programs and type nothing yourself.

## Pick the source, best first

1. **A file the user named or attached.** Set `SRC=<path>`. Done.
2. **An issue, PR or comment body.** Note that `gh ... --jq .body` adds a newline, so write the field with Python instead:
   ```sh
   gh api repos/<owner>/<repo>/issues/<n> | python3 -I -c 'import json,sys; sys.stdout.write(json.load(sys.stdin)["body"])' > "$WORK/input.txt"
   SRC="$WORK/input.txt"
   ```
3. **Output of another program** (clipboard, export, speech-to-text tool): pipe it straight into a file, e.g. `wl-paste --no-newline > "$WORK/input.txt"`.
4. **A message that exists only in this conversation.** Look for the harness's own record of it, such as a session export or a transcript log under the harness's data directory, and extract the message field with a script, as in 2.
5. **Nothing else works.** Write the text once with the file-writing tool. Then:
   - pass `--source "retyped by agent from chat; bytes not verified"`;
   - add a Question: "send the original file or its sha256 -> proceeding with the retyped copy".

   Don't present a retyped copy as exact.

## Hashes and provenance

- `--expect-sha256` takes the hash the user, the issue or an earlier copy gave. A mismatch means you have different bytes, so stop: Blocked on user, showing both hashes.
- A copy kept elsewhere (another repository, a note with a prefix like `in code pull then do this : `) is a different byte string. Record where it lives in `--source`, and don't make it the transcript unless the user says so.
- After `init`, `rw verify-transcript --package "$PKG" --against "$SRC"` proves the stored file equals the source.

## Privacy

The repository is public. Before committing:

- **Secrets:** scan for API keys, tokens, passwords and private keys (the repository's secret scanner, or `grep -nEi 'api[_-]?key|secret|token|passw' "$SRC"`, then read each hit). Any real secret means Blocked on user. Never commit it, not even redacted, unless the user tells you how.
- **Other people's private material** (names with contact details, private repository content): ask before committing. Your default is to keep the transcript out of this repository and record only its hash and where it lives (`--visibility private`).
