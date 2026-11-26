{
  runCommand,
  python3,
}:
runCommand "alignment-handler-0.1.0"
  {
    meta = {
      description = "Fleet #alignment request handler (code#6651)";
      mainProgram = "alignment-handler";
    };
  }
  ''
    mkdir -p $out/bin $out/libexec/alignment-handler
    cp ${./handler.py} $out/libexec/alignment-handler/handler.py
    cp ${./launcher.sh} $out/bin/alignment-handler
    substituteInPlace $out/bin/alignment-handler \
      --replace-fail "@python@" "${python3}/bin/python3" \
      --replace-fail "@handlerPy@" "$out/libexec/alignment-handler/handler.py"
    chmod +x $out/bin/alignment-handler
  ''
