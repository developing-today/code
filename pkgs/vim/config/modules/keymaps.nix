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
        mode = ["n" "i"];  # Both normal and insert modes
        key = "<C-1>";
        action = "<C-j>";
        options = {
          silent = true;
        };
      }
      {
        mode = ["n" "i"];  # Both normal and insert modes
        key = "<C-2>";
        action = "<C-k>";
        options = {
          silent = true;
        };
      }
      {
        mode = ["n" "i"];  # Both normal and insert modes
        key = "<C-3>";
        action = "<C-l>";
        options = {
          silent = true;
        };
      }
      {
        mode = ["n" "i"];  # Both normal and insert modes
        key = "<C-4>";
        action = "<C-m>";
        options = {
          silent = true;
        };
      }
  ];
}
