{
  keymaps = [
    {
      mode = "n";
      key = "<C-s>";
      action = ":w<CR>";
    }
    {
      mode = "n";
      key = "<esc>";
      options = {
        silent = true;
      };
      action = ":noh<CR>";
    }
    {
      mode = "t";
      key = "<Esc>";
      action = "<C-\\><C-n>";
    }
  ];
}
