{ ... }:
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
  ];
}
