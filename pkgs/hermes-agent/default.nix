{
  lib,
  python313Packages,
  fetchurl,
}:

# Hermes, NousResearch's self-improving agent harness.
#   https://hermes-agent.nousresearch.com  /  github:NousResearch/hermes-agent
#
# Packaging notes:
#   * Not in nixpkgs (only unrelated `hermes-*` hits: hermes-nvim, hermes-json,
#     hermes-wireshare). No open packaging PR exists.
#   * Upstream publishes a pure-Python wheel to PyPI, so we install that rather
#     than building from the git tree.
#   * requires-python is ">=3.11,<3.14", so this must NOT use the default
#     python3 (currently 3.14) — hence python313Packages.
#   * Upstream pins every core dependency with `==`. Those exact versions do not
#     exist in nixpkgs, so we relax them and use nixpkgs' versions instead.
#   * Only the base dependency set is wired up; the many optional extras
#     (messaging, matrix, voice, web, ...) are intentionally omitted.
python313Packages.buildPythonApplication rec {
  pname = "hermes-agent";
  version = "0.19.0";
  format = "wheel";

  src = fetchurl {
    url = "https://files.pythonhosted.org/packages/e5/30/c85be8290e9565dc3c7a9720e93f3e59e09b1b163487be4946c3aa848f80/hermes_agent-${version}-py3-none-any.whl";
    hash = "sha256-vQusASruOKYIlHgfRZfcKe577bNEhUAkmSHxDTvvMn8=";
  };

  # Upstream pins exact versions; use whatever nixpkgs ships.
  pythonRelaxDeps = true;

  nativeBuildInputs = with python313Packages; [ pythonRelaxDepsHook ];

  dependencies = with python313Packages; [
    openai
    certifi
    python-dotenv
    fire
    httpx
    rich
    tenacity
    pyyaml
    ruamel-yaml
    requests
    jinja2
    pydantic
    prompt-toolkit
    croniter
    packaging
    markdown
    pyjwt
    urllib3
    cryptography
    psutil
    websockets
    pathspec
    fastapi
    uvicorn
    python-multipart
    ptyprocess
    pillow
  ];

  # The wheel ships no test suite.
  doCheck = false;

  # Console scripts are hermes (hermes_cli.main), hermes-acp (acp_adapter.entry)
  # and hermes-agent (run_agent). Note upstream also installs very generic
  # top-level packages (agent, tools, providers, plugins, gateway, cron) into
  # site-packages, so avoid putting this in a shared python environment.
  pythonImportsCheck = [ "hermes_cli" ];

  meta = {
    description = "Self-improving AI agent harness that creates skills from experience";
    homepage = "https://hermes-agent.nousresearch.com";
    downloadPage = "https://github.com/NousResearch/hermes-agent";
    license = lib.licenses.mit;
    sourceProvenance = with lib.sourceTypes; [ fromSource ];
    mainProgram = "hermes";
    platforms = lib.platforms.unix;
  };
}
