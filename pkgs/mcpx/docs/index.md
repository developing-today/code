<!-- id: mcpx-home since: pre-release -->

<style>
  .mcpx-home { max-width: 52rem; margin: 0 auto; }
  .mcpx-eyebrow { margin: 2rem 0 0.35rem; font-size: 0.8rem; letter-spacing: 0.12em; text-transform: uppercase; opacity: 0.7; }
  .mcpx-display { font-size: 2.6rem; line-height: 1.1; margin: 0 0 1rem; border-bottom: 0; }
  .mcpx-chips { display: flex; flex-wrap: wrap; gap: 0.5rem; margin: 0 0 1rem; padding: 0; list-style: none; }
  .mcpx-chips span { border: 1px solid var(--quote-border); border-radius: 999px; padding: 0.15rem 0.7rem; font-size: 0.85rem; }
  .mcpx-lede { font-size: 1.1rem; margin: 0 0 1.5rem; }
  .mcpx-actions { display: flex; flex-wrap: wrap; gap: 0.75rem; align-items: center; margin: 0 0 2rem; }
  .mcpx-actions a { font-weight: 600; }
  .mcpx-video-slot { border: 1px dashed var(--quote-border); border-radius: 6px; padding: 1.5rem; text-align: center; margin: 0 0 2rem; }
  .mcpx-video-slot span { display: block; opacity: 0.7; font-size: 0.9rem; }
  .mcpx-section { margin: 0 0 2.5rem; }
  .mcpx-section h2 { border-bottom: 1px solid var(--quote-border); padding-bottom: 0.3rem; }
  .mcpx-install { position: relative; margin: 0 0 0.9rem; }
  .mcpx-install pre { margin: 0; padding-right: 4.5rem; }
  .mcpx-copy { position: absolute; top: 0.35rem; right: 0.35rem; font-size: 0.8rem; padding: 0.2rem 0.6rem; cursor: pointer; }
  .mcpx-install-note { font-size: 0.85rem; opacity: 0.75; }
  .mcpx-prompt { position: relative; border: 1px solid var(--quote-border); border-radius: 6px; padding: 0.9rem 4.5rem 0.9rem 0.9rem; margin: 0 0 0.9rem; min-height: 3.2rem; }
  .mcpx-prompt-payload { font-family: var(--mono-font); }
  .mcpx-prompt-source { display: block; font-size: 0.8rem; opacity: 0.7; margin-top: 0.35rem; }
  .mcpx-why { display: grid; grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr)); gap: 1rem; margin: 0; padding: 0; list-style: none; }
  .mcpx-why li { border: 1px solid var(--quote-border); border-radius: 6px; padding: 1rem; }
  .mcpx-why strong { display: block; margin-bottom: 0.35rem; }
  .mcpx-next { display: flex; flex-wrap: wrap; gap: 1rem; }
  .mcpx-next a { flex: 1 1 14rem; border: 1px solid var(--quote-border); border-radius: 6px; padding: 1rem; font-size: 1.05rem; font-weight: 600; }
  .mcpx-noscript { border-left: 4px solid var(--quote-border); padding-left: 0.8rem; font-size: 0.9rem; }
  @media (max-width: 40rem) { .mcpx-display { font-size: 2rem; } }
</style>

<div class="mcpx-home">

<section class="mcpx-hero">
<p class="mcpx-eyebrow">mcpx · code mode for MCP</p>

<h1 class="mcpx-display">Every MCP server.<br>One daemon.<br>Scripts instead of schemas.</h1>

<p class="mcpx-chips"><span>code mode</span><span>one MCP config</span><span>share it on your tailnet</span><span>reclaim context</span></p>

<p class="mcpx-lede">
mcpx puts every MCP server you already have behind one local daemon, and gives
your agent a TypeScript client instead of 43 tool schemas. Write a script once,
run it again for free; publish it as a named script when it earns its place.
</p>

<div class="mcpx-actions">
<a href="overview.md">Read the overview →</a>
<a href="quick-start.md">Quick start →</a>
<a href="https://github.com/developing-today/code/releases/tag/mcpx%2Fv0.1.0">Download 0.1.0</a>
</div>
</section>

<!-- Hero video slot (#1874): the asset is produced separately. The pattern is
     poster + controls + a text track, so swapping the source in is the whole
     change. Kept as a labelled slot rather than a broken <video> element. -->
<div class="mcpx-video-slot" data-video-slot="hero">
<strong>A one-minute demo will live here.</strong>
<span>The interactive example below is already real — it needs no video and no install.</span>
</div>

<section class="mcpx-section" id="install">
<h2>Install</h2>

<p>The installer downloads a published release tarball, verifies its checksum,
and puts <code>mcpx</code> on your <code>PATH</code>. Per-user by default:
<code>~/.local/bin</code>, no sudo.</p>

<div class="mcpx-install">
<pre><code>curl -fsSL https://raw.githubusercontent.com/developing-today/code/main/pkgs/mcpx/scripts/install.sh | sh</code></pre>
<button type="button" class="mcpx-copy" data-copy-target="install">Copy</button>
</div>

<p class="mcpx-install-note">System-wide (installs into <code>/usr/local/bin</code>; the script escalates only the copy with sudo):</p>

<div class="mcpx-install">
<pre><code>curl -fsSL https://raw.githubusercontent.com/developing-today/code/main/pkgs/mcpx/scripts/install.sh | sh -s -- --system</code></pre>
<button type="button" class="mcpx-copy" data-copy-target="install">Copy</button>
</div>

<p class="mcpx-install-note">
Never <code>sudo $(curl … | sh)</code>: that runs the installer unprivileged and
then executes its <em>stdout</em> as root. Prefer nix or the container? See
<a href="install.md">every channel and its real status</a>.
</p>
</section>

<section class="mcpx-section" id="agent-prompt">
<h2>Or hand it to your agent</h2>

<p>One prompt is enough. The button copies the prompt together with the agent
entry point, so the agent knows where to read next.</p>

<div class="mcpx-prompt" id="mcpx-agent-prompt">
<code class="mcpx-prompt-payload" data-prompt-index="0">hey please one-shot setup mcpx right now</code>
<span class="mcpx-prompt-source">Agent entry point: <a href="https://github.com/developing-today/code/blob/main/pkgs/mcpx/AGENTS.md">pkgs/mcpx/AGENTS.md</a></span>
<button type="button" class="mcpx-copy" data-copy-target="prompt">Copy</button>
</div>

<noscript>
<p class="mcpx-noscript">Rotation needs JavaScript. Every prompt is listed here so
this page works without it — select one and paste it with the agent entry point
above:</p>
<ul>
<li><code>hey please one-shot setup mcpx right now</code></li>
<li><code>hey install this mcpx thing and comment out all my mcp config</code></li>
<li><code>replace mcp with mcpx codemode</code></li>
<li><code>guided install mcpx now plz</code></li>
</ul>
</noscript>
</section>

<section class="mcpx-section" id="example">
<h2>Try it without installing anything</h2>

{{#include home-example/component.html}}

<p><a href="home-example.md">How this example works, and what it deliberately cannot do →</a></p>
</section>

<section class="mcpx-section" id="why">
<h2>Why you would install it</h2>

<ul class="mcpx-why">
<li><strong>Code mode</strong>Tool calls become one script. The script is shareable,
reusable, and costs nothing to run again — keep it as a named script under
<code>.mcpx/scripts/</code> and run it with <code>mcpx run</code>.</li>
<li><strong>Reclaim context</strong>Schemas are pulled on demand:
<code>mcpx ls</code> over seven namespaces is about 171 tokens, and the story's
worked task spends about 4,600 tokens total instead of loading every tool's
schema.</li>
<li><strong>Tame MCP</strong>One config for CLI, plugin, <code>/v1</code> and
<code>/mcp</code>; bind the daemon to your tailnet address to share it; and
declare scripts, APIs and binaries as tools that live in the same daemon.</li>
</ul>
</section>

<section class="mcpx-section">
<h2>Where next</h2>
<div class="mcpx-next">
<a href="overview.md">Overview — what mcpx is, in one page</a>
<a href="quick-start.md">Quick start — from zero to a script</a>
</div>
<p class="mcpx-install-note">The numbers on this page come from
<a href="story.md">the story</a>; the install commands match
<code>scripts/install.sh</code>. Nothing here is a mock except the example,
which says so on its label.</p>
</section>

</div>

<script>
(function () {
  const prompts = [
    "hey please one-shot setup mcpx right now",
    "hey install this mcpx thing and comment out all my mcp config",
    "replace mcp with mcpx codemode",
    "guided install mcpx now plz",
  ];
  const entry = "https://github.com/developing-today/code/blob/main/pkgs/mcpx/AGENTS.md";
  const promptBox = document.getElementById("mcpx-agent-prompt");
  let index = 0;

  const payloadFor = (box) => {
    const prompt = box.querySelector(".mcpx-prompt-payload").textContent.trim();
    return prompt + "\n\n" + "Agent entry point: " + entry;
  };

  document.querySelectorAll(".mcpx-copy").forEach((button) => {
    button.addEventListener("click", async () => {
      const kind = button.dataset.copyTarget;
      let text;
      if (kind === "prompt") {
        text = payloadFor(button.closest(".mcpx-prompt"));
      } else {
        text = button.closest(".mcpx-install").querySelector("code").textContent.trim();
      }
      try {
        await navigator.clipboard.writeText(text);
        button.textContent = "Copied";
      } catch {
        button.textContent = "Select the text";
      }
      setTimeout(() => { button.textContent = "Copy"; }, 2000);
    });
  });

  if (promptBox) {
    setInterval(() => {
      index = (index + 1) % prompts.length;
      const payload = promptBox.querySelector(".mcpx-prompt-payload");
      payload.textContent = prompts[index];
      payload.dataset.promptIndex = String(index);
    }, 8000);
  }
})();
</script>
