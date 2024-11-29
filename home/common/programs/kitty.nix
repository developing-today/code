{
  enable = true;
  themeFile = "VibrantInk"; # matches file here except without ".conf": https://github.com/kovidgoyal/kitty-themes/tree/master/themes
  font.name = "Terminess Nerd Font";
  settings = {
    clipboard_control = "write-clipboard read-clipboard write-primary read-primary";
    confirm_os_window_close = -0;
    copy_on_select = true;
    enable_audio_bell = false;
    input_delay = -0;
    repaint_delay = 2;
    scrollback_fill_enlarged_window = true;
    scrollback_lines = 10000;
    scrollback_pager_history_size = 4096;
    sync_to_monitor = true;
    update_check_interval = 0;
    wayland_enable_ime = false;
    hide_window_decorations = true;
    window_border_width = 0;
    draw_minimal_borders = true;
    single_window_margin_width = -1;
    window_padding_width = 0;
    single_window_padding_width = -1;
    remember_window_size = false;
    placement_strategy = "top";
    #placement_strategy = "top-left";
    #linux_display_server = "x11"; # xwayland, but should work
    #resize_in_steps = true; # does not wayland
  };
}
