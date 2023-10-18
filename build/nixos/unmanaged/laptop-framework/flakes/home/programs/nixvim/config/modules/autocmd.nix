{...}: {
  autoCmd = [
    {
      event = "FileType";
      pattern = "nix";
      command = "setlocal tabstop=2 shiftwidth=2";
    }
  ];
}
