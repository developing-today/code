//! `id world new`: the scaffold for a new world app (main.roc, README.md,
//! build.sh). Split out of `commands/world.rs` to keep that file under the
//! production-line limit (#13057).

use anyhow::{Context, Result};

/// Scaffold a Roc world app in `dir`.
///
/// The directory must be absent or empty. Writes `main.roc`, `build.sh`
/// (executable), and `README.md`, and prints the directory path.
///
/// # Errors
///
/// A bad name, a non-empty directory, or a filesystem failure.
pub(super) fn scaffold_world(dir: &std::path::Path, name: &str) -> Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "the app name may contain letters, digits, '-' and '_'"
    );
    if dir.exists() {
        anyhow::ensure!(
            std::fs::read_dir(dir)?.next().is_none(),
            "{} is not empty",
            dir.display()
        );
    } else {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(dir.join("main.roc"), template_main(name))
        .with_context(|| format!("write {}", dir.join("main.roc").display()))?;
    std::fs::write(dir.join("README.md"), template_readme(name))
        .with_context(|| format!("write {}", dir.join("README.md").display()))?;
    let build = dir.join("build.sh");
    std::fs::write(&build, template_build(name))
        .with_context(|| format!("write {}", build.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&build, std::fs::Permissions::from_mode(0o755))?;
    }
    println!("{}", dir.display());
    Ok(())
}

fn template_main(name: &str) -> String {
    format!(
        r#"app [program] {{ pf: platform "../roc-world/platform.roc" }}

import pf.Screen

# {name}: a scaffolded world app. Replace the model, update, and view with your
# own. Build with ./build.sh, then install with `id world install` or compile the
# source with `id world compile`.

Model : {{ count : U64 }}

program = {{ init, update, view, layout, records, wants, snapshot, restore }}

init : U64 -> Model
init = |_seed| {{ count: 0 }}

update : Model, U64, Str -> Model
update = |model, _pid, _ev| {{ count: model.count + 1 }}

view : Model -> Str
view = |model| {{
    screen = Screen.boxed(Screen.new(24, 3), 0, 0, 24, 3, "{name}")
    drawn = Screen.put_text(screen, 2, 1, "count=${{model.count.to_str()}}", {{ fg: 2, bg: 0, bold: Bool.True }})
    Screen.render_json(drawn)
}}

## Render the same view at any viewport.
layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |model| "{{\"count\":${{model.count.to_str()}}}}"

wants : Model -> Str
wants = |_model| "{{}}"

snapshot : Model -> Str
snapshot = |model| model.count.to_str()

restore : Str -> Model
restore = |text| match U64.from_str(text) {{
    Ok(count) => {{ count: count }}
    Err(_) => {{ count: 0 }}
}}
"#
    )
}

fn template_readme(name: &str) -> String {
    format!(
        r#"# {name}

A world app scaffolded with `id world new`.

## Build

```bash
./build.sh
```

The script expects a checkout of the world platform at `../roc-world` and
produces `{name}.wasm`, validated import-free.

## Run

```bash
id serve --data-dir ./data --world --world-admin-token "$ADMIN" \
  --world-module "$PWD/{name}.wasm" --world-ssh-port 2223 --web --port 8787
```

## Next steps

- Edit `update` to react to input events and `view` to draw a portable screen.
- Ask for host capabilities through `wants`; grant them with `id world caps`.
- Keep `snapshot`/`restore` faithful so the host can compact the journal.
"#
    )
}

fn template_build(name: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${{BASH_SOURCE[0]}}")"
ROC="${{ROC:-roc}}"

if [ -z "${{WASM_TOOLS:-}}" ]; then
  if command -v wasm-tools >/dev/null 2>&1; then
    WASM_TOOLS="wasm-tools"
  else
    STORE_TOOL=$(ls -d /nix/store/*-wasm-tools-*/bin/wasm-tools 2>/dev/null | head -n1 || true)
    if [ -n "$STORE_TOOL" ] && [ -x "$STORE_TOOL" ]; then
      WASM_TOOLS="$STORE_TOOL"
    else
      WASM_TOOLS="nix run nixpkgs#wasm-tools --"
    fi
  fi
fi

(cd ../roc-world && ./build-host.sh >/dev/null)
"$ROC" build main.roc --target=wasm32 --debug --wasm-memory=8454144 --output={name}.wasm
$WASM_TOOLS validate {name}.wasm
echo "Built {name}.wasm (import-free Roc world module)"
"#
    )
}
