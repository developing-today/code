{...}: {
  maps.normal = {
    "<C-s>" = ":w<CR>";
    "<esc>" = {
      action = ":noh<CR>";
      silent = true;
    };
  };
}
