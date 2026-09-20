-- Hyprland config, Lua format.
--
-- Migrated from the old hyprlang `hyprland.conf`. Hyprland removed legacy
-- hyprlang config support on master ("config: remove legacy config support",
-- #15539); `~/.config/hypr/hyprland.lua` is now the only config that is read.
-- Reference: https://wiki.hypr.land/configuring/
--
-- This file is installed by home-manager via
-- `wayland.windowManager.hyprland.extraConfig` with `configType = "lua"`.
-- hyprctl clients

------------------
---- MONITORS ----
------------------

hl.monitor({
  output = "",
  mode = "preferred",
  position = "auto",
  scale = 1,
})

-----------------------
---- LOOK AND FEEL ----
-----------------------

hl.config({
  input = {
    follow_mouse = 1,
    kb_layout = "us",
    kb_model = "",
    kb_options = "ctrl:nocaps",
    kb_variant = "",
    sensitivity = 0, -- -1.0 - 1.0, 0 means no modification.
    touchpad = {
      natural_scroll = true,
    },
  },

  general = {
    allow_tearing = true,
    border_size = 0,
    col = {
      active_border = { colors = { "rgba(33ccffee)", "rgba(00ff99ee)" }, angle = 45 },
      inactive_border = "rgba(595959aa)",
    },
    gaps_in = 0,
    gaps_out = 0,
    layout = "dwindle",
  },

  decoration = {
    blur = {
      enabled = false,
      size = 3,
      passes = 1,
    },
    shadow = {
      enabled = false,
    },
    rounding = 0, -- 10
  },

  animations = {
    enabled = false,
  },

  dwindle = {
    -- `pseudotile` was removed in 0.55.0 ("it wasn't doing anything").
    -- Pseudotiling is still available per-window via the `hl.dsp.window.pseudo`
    -- dispatcher, bound to mainMod + P below.
    preserve_split = true, -- you probably want this
  },

  master = {
    new_status = "master",
  },
})

----------------
---- GESTURES --
----------------

hl.gesture({ fingers = 3, direction = "horizontal", action = "workspace" })
hl.gesture({ fingers = 4, direction = "pinch", action = "special", workspace_name = "magic" })

-----------------------
---- WINDOW RULES ----
-----------------------

hl.window_rule({
  match = { title = "^.*(Element).*$" },
  workspace = "10 silent",
})

hl.window_rule({
  match = { title = "^.*(Zulip).*$" },
  workspace = "9 silent",
})

hl.window_rule({
  match = { class = ".*" },
  immediate = true,
})

hl.window_rule({
  -- You'll probably like this.
  match = { class = ".*" },
  suppress_event = "maximize",
})

-- windowrule = workspace 3 silent, match:class ^(zen-beta)$
-- windowrule = workspace 9 silent, match:title ^(YouTube)(.*)$
-- windowrule = workspace 2 silent, match:class ^(dev.zed.Zed)$

-------------------
---- AUTOSTART ----
-------------------

-- Move already-open zen tabs onto dedicated workspaces. This replaces the old
-- `hyprctl clients -j | jq ... | xargs hyprctl dispatch movetoworkspacesilent`
-- pipelines: `hyprctl dispatch` now takes Lua, and doing it natively avoids
-- shelling out entirely.
local function move_windows(initial_class, needle, workspace)
  needle = needle:lower()
  for _, w in ipairs(hl.get_windows()) do
    if w.initial_class == initial_class and w.title ~= nil and w.title:lower():find(needle, 1, true) then
      hl.dispatch(hl.dsp.window.move({ window = w, workspace = workspace, follow = false }))
    end
  end
end

-- The old config alternated between the `zen` and `zen-beta` initial classes,
-- and sent YouTube to workspace 8 on the first pass but 9 on every later pass.
-- Normalized to `zen-beta` and workspace 9 throughout.
local ZEN_CLASS = "zen-beta"

local function sort_zen_tabs()
  move_windows(ZEN_CLASS, "- YouTube —", "9")
  move_windows(ZEN_CLASS, "T3 Chat —", "7")
end

-- If no zen window landed on workspace 3, open one there and come back.
local function ensure_zen_on_workspace_3()
  for _, w in ipairs(hl.get_windows()) do
    if w.initial_class == ZEN_CLASS and w.workspace ~= nil and w.workspace.id == 3 then
      return
    end
  end

  local current = hl.get_active_workspace()
  hl.dispatch(hl.dsp.focus({ workspace = 3 }))
  hl.exec_cmd("zen")
  hl.timer(function()
    hl.dispatch(hl.dsp.focus({ workspace = current.id }))
  end, { timeout = 200, type = "oneshot" })
end

hl.on("hyprland.start", function()
  -- REDUNDANT since home-manager was given ownership of hyprland.lua.
  -- This file is now fed through `wayland.windowManager.hyprland.extraConfig`,
  -- and home-manager prepends its own hyprland.start hook which runs
  -- dbus-update-activation-environment with a superset of these variables
  -- (adds HYPRLAND_INSTANCE_SIGNATURE, XDG_CURRENT_DESKTOP, XDG_SESSION_TYPE)
  -- and then starts hyprland-session.target.
  --
  -- Previously ~/.config/hypr/hyprland.lua was a verbatim link to this file
  -- (xdg.configFile."hypr" recursive, now commented out in
  -- home/common/default.nix), which shadowed the generated config, so that
  -- hook never ran and this line was the manual stand-in. It now runs twice,
  -- which is harmless, and is safe to delete.
  hl.exec_cmd("dbus-update-activation-environment --systemd DISPLAY WAYLAND_DISPLAY")
  hl.exec_cmd("hyprpaper")
  -- hl.exec_cmd("swaync")
  hl.exec_cmd("mako")
  -- hl.exec_cmd("dunst")
  hl.exec_cmd("touch /tmp/waybar_autotoggle")
  -- NOTE: hyprland-session.target now actually starts (see above). If any of
  -- the processes launched here are also bound to that target as systemd user
  -- services, they can start twice -- waybar is the likely candidate. If you
  -- see duplicates, remove the exec_cmd here rather than reverting the
  -- home-manager ownership change.
  hl.exec_cmd("GTK_THEME=Adapta waybar")
  -- hl.exec_cmd("nm-applet")
  hl.exec_cmd("sleep 4 ; wpa_gui -t")

  hl.dispatch(hl.dsp.exec_cmd("element-desktop", { workspace = "10 silent" }))
  hl.dispatch(hl.dsp.exec_cmd("zulip", { workspace = "9 silent" }))

  -- zen zen-beta etc.
  hl.dispatch(hl.dsp.focus({ workspace = 3 }))
  hl.exec_cmd("zen")
  hl.timer(function()
    hl.dispatch(hl.dsp.focus({ workspace = 2 }))
  end, { timeout = 300, type = "oneshot" })

  sort_zen_tabs()
  for delay = 1, 5 do
    hl.timer(sort_zen_tabs, { timeout = delay * 1000, type = "oneshot" })
  end

  hl.timer(ensure_zen_on_workspace_3, { timeout = 3000, type = "oneshot" })

  hl.dispatch(hl.dsp.exec_cmd("zeditor", { workspace = "2 silent" }))
  hl.dispatch(hl.dsp.exec_cmd("ghostty -e 'curl http://wttr.in ; exec $SHELL'", { workspace = "1 silent" }))
  -- hl.dispatch(hl.dsp.exec_cmd("kitty", { workspace = "1 silent" }))

  hl.exec_cmd([[dconf write /org/gnome/desktop/interface/gtk-theme "'Adwaita'"]])
  hl.exec_cmd([[dconf write /org/gnome/desktop/interface/icon-theme "'Flat-Remix-Red-Dark'"]])
  hl.exec_cmd([[dconf write /org/gnome/desktop/interface/document-font-name "'Noto Sans Medium 11'"]])
  hl.exec_cmd([[dconf write /org/gnome/desktop/interface/font-name "'Noto Sans Medium 11'"]])
  hl.exec_cmd([[dconf write /org/gnome/desktop/interface/monospace-font-name "'Noto Sans Mono Medium 11'"]])
end)

-------------------------------
---- ENVIRONMENT VARIABLES ----
-------------------------------

-- hl.env("XCURSOR_SIZE", "24")
-- hl.env("XCURSOR_THEME", "Vanilla-DMZ")
-- hl.env("HYPRCURSOR_THEME", "Vanilla-DMZ")
-- hl.env("HYPRCURSOR_THEME", "rose-pine-hyprcursor")
-- hl.env("HYPRCURSOR_SIZE", "24")

-----------------------------------------------------
-----------------------------------------------------
-- TODO: hyprctl setcursor ____
-- TODO: check out https://sr.ht/~emersion/kanshi/
-----------------------------------------------------
-----------------------------------------------------
-- GROUPS BY KEY AFTER HERE ARE SORTED ##############
-----------------------------------------------------
-----------------------------------------------------

local mainMod = "SUPER"

-----------------------------------------------------
-----------------------------------------------------

-- TOGGLE WAYBAR
hl.bind(
  mainMod .. " + B",
  hl.dsp.exec_cmd(
    "if [ -e /tmp/waybar_autotoggle ]; then rm /tmp/waybar_autotoggle; else touch /tmp/waybar_autotoggle; fi"
  )
)
hl.bind("SUPER + R", hl.dsp.exec_cmd("pkill -SIGUSR1 waybar"))
hl.bind("SUPER_L", hl.dsp.exec_cmd("if [ -e /tmp/waybar_autotoggle ]; then pkill -SIGUSR1 waybar; fi"))
hl.bind(
  "SUPER + SUPER_L",
  hl.dsp.exec_cmd("if [ -e /tmp/waybar_autotoggle ]; then pkill -SIGUSR1 waybar; fi"),
  { release = true, transparent = true }
)
hl.bind(
  "SUPER + SHIFT + R",
  hl.dsp.exec_cmd("hyprctl reload & (pkill waybar && GTK_THEME=Adapta waybar && sleep 0.5 && pkill -SIGUSR1 waybar) &")
)

-- Assorted
-- todo how to target forward slash rm /tmp/waybar_autotogglekey?
-- hl.bind(mainMod .. " + slash", hl.dsp.layout("togglesplit")) -- dwindle
hl.bind(mainMod .. " + Z", hl.dsp.layout("togglesplit")) -- dwindle
hl.bind(mainMod .. " + C", hl.dsp.window.close())
hl.bind(mainMod .. " + E", hl.dsp.exec_cmd("zeditor ~/code"))
-- hl.bind(mainMod .. " + F", hl.dsp.exec_cmd("firefox"))
hl.bind(mainMod .. " + F", hl.dsp.exec_cmd("zen"))
hl.bind(mainMod .. " + d", hl.dsp.exec_cmd("dolphin"))
-- `.` is not an xkb keysym name; the keysym for it is `period`.
hl.bind(mainMod .. " + period", hl.dsp.exec_cmd("emote"))
hl.bind(mainMod .. " + M", hl.dsp.exit())
hl.bind(mainMod .. " + P", hl.dsp.window.pseudo()) -- dwindle
hl.bind(mainMod .. " + SHIFT + Q", hl.dsp.window.move({ workspace = "e-1" }))
hl.bind(mainMod .. " + SHIFT + W", hl.dsp.window.move({ workspace = "e+1" }))
hl.bind(mainMod .. " + Q", hl.dsp.exec_cmd("ghostty")) -- kitty # alacritty
hl.bind(mainMod .. " + RETURN", hl.dsp.exec_cmd("ghostty")) -- kitty # alacritty
-- hl.bind(mainMod .. " + R", hl.dsp.exec_cmd("wofi --show drun"))
-- hl.bind(mainMod .. " + R", hl.dsp.exec_cmd("fuzzel"))
hl.bind(mainMod .. " + SPACE", hl.dsp.exec_cmd("fuzzel"))
hl.bind(mainMod .. " + V", hl.dsp.window.float())

-- Print
hl.bind("SHIFT + Print", hl.dsp.exec_cmd("hyprshot -m region"))
hl.bind("Print", hl.dsp.exec_cmd("hyprshot -m output"))

-- F11
hl.bind("ALT + F11", hl.dsp.exec_cmd("hyprshot -m region"))
hl.bind("CTRL + F11", hl.dsp.exec_cmd("hyprshot -m region"))
hl.bind("SHIFT + F11", hl.dsp.exec_cmd("hyprshot -m active"))
hl.bind("SUPER + F11", hl.dsp.window.fullscreen_state({ internal = -1, client = 2 }))
-- S
-- hl.bind("ALT + SHIFT + S", hl.dsp.exec_cmd("hyprshot -m active"))
-- hl.bind("SUPER + ALT + S", hl.dsp.exec_cmd("hyprshot -m output"))
-- hl.bind("SUPER + SHIFT + S", hl.dsp.exec_cmd("hyprshot -m region"))

-- MOUSE
hl.bind(mainMod .. " + mouse_down", hl.dsp.focus({ workspace = "e+1" }))
hl.bind(mainMod .. " + mouse_up", hl.dsp.focus({ workspace = "e-1" }))
-- Move/resize windows with mainMod + LMB/RMB and dragging
hl.bind(mainMod .. " + mouse:272", hl.dsp.window.drag(), { mouse = true })
hl.bind(mainMod .. " + mouse:273", hl.dsp.window.resize(), { mouse = true })

-- XF86
hl.bind("XF86MonBrightnessDown", hl.dsp.exec_cmd("brightnessctl set 5%-")) -- brillo -q -A 5
hl.bind("XF86MonBrightnessUp", hl.dsp.exec_cmd("brightnessctl set +5%")) -- brillo -q -U 5
hl.bind(
  "XF86AudioLowerVolume",
  hl.dsp.exec_cmd("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"),
  { repeating = true, locked = true }
)
hl.bind(
  "XF86AudioRaiseVolume",
  hl.dsp.exec_cmd("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+"),
  { repeating = true, locked = true }
)
hl.bind("XF86AudioMute", hl.dsp.exec_cmd("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"), { locked = true })
hl.bind("XF86AudioNext", hl.dsp.exec_cmd("playerctl next"), { locked = true })
hl.bind("XF86AudioPlay", hl.dsp.exec_cmd("playerctl play-pause"), { locked = true })
hl.bind("XF86AudioPrev", hl.dsp.exec_cmd("playerctl previous"), { locked = true })

-- 1234567890
for i = 1, 10 do
  local key = i % 10 -- 10 maps to key 0
  hl.bind(mainMod .. " + " .. key, hl.dsp.focus({ workspace = i }))
  hl.bind(mainMod .. " + SHIFT + " .. key, hl.dsp.window.move({ workspace = i }))
end

-- UP DOWN LEFT RIGHT
hl.bind(mainMod .. " + ALT + CTRL + down", hl.dsp.window.move({ workspace = 4 }))
hl.bind(mainMod .. " + ALT + CTRL + left", hl.dsp.window.move({ workspace = 1 }))
hl.bind(mainMod .. " + ALT + CTRL + right", hl.dsp.window.move({ workspace = 2 }))
hl.bind(mainMod .. " + ALT + CTRL + up", hl.dsp.window.move({ workspace = 3 }))
-- TODO: allow moving floating windows
hl.bind(mainMod .. " + ALT + down", hl.dsp.window.move({ direction = "d" }))
hl.bind(mainMod .. " + ALT + left", hl.dsp.window.move({ direction = "l" }))
hl.bind(mainMod .. " + ALT + right", hl.dsp.window.move({ direction = "r" }))
hl.bind(mainMod .. " + ALT + up", hl.dsp.window.move({ direction = "u" }))
hl.bind(mainMod .. " + CTRL + SHIFT + down", hl.dsp.window.move({ workspace = "e+2" }))
hl.bind(mainMod .. " + CTRL + SHIFT + left", hl.dsp.window.move({ workspace = "e-1" }))
hl.bind(mainMod .. " + CTRL + SHIFT + right", hl.dsp.window.move({ workspace = "e+1" }))
hl.bind(mainMod .. " + CTRL + SHIFT + up", hl.dsp.window.move({ workspace = "e-2" }))
hl.bind(mainMod .. " + CTRL + down", hl.dsp.focus({ workspace = "e+2" }))
hl.bind(mainMod .. " + CTRL + left", hl.dsp.focus({ workspace = "e-1" }))
hl.bind(mainMod .. " + CTRL + right", hl.dsp.focus({ workspace = "e+1" }))
hl.bind(mainMod .. " + CTRL + up", hl.dsp.focus({ workspace = "e-2" }))
hl.bind(mainMod .. " + SHIFT + down", hl.dsp.window.resize({ x = 0, y = 10, relative = true }))
hl.bind(mainMod .. " + SHIFT + left", hl.dsp.window.resize({ x = -10, y = 0, relative = true }))
hl.bind(mainMod .. " + SHIFT + right", hl.dsp.window.resize({ x = 10, y = 0, relative = true }))
hl.bind(mainMod .. " + SHIFT + up", hl.dsp.window.resize({ x = 0, y = -10, relative = true }))
hl.bind(mainMod .. " + down", hl.dsp.focus({ direction = "d" }))
hl.bind(mainMod .. " + left", hl.dsp.focus({ direction = "l" }))
hl.bind(mainMod .. " + right", hl.dsp.focus({ direction = "r" }))
hl.bind(mainMod .. " + up", hl.dsp.focus({ direction = "u" }))

-- TAB,ASDF
hl.bind(mainMod .. " + TAB", hl.dsp.focus({ workspace = "m+1" }))
hl.bind(mainMod .. " + SHIFT + TAB", hl.dsp.focus({ workspace = "m-1" }))
hl.bind(mainMod .. " + A", hl.dsp.focus({ workspace = "m-1" }))
hl.bind(mainMod .. " + S", hl.dsp.focus({ workspace = "m+1" }))
hl.bind(mainMod .. " + SHIFT + A", hl.dsp.focus({ monitor = "r" }))
hl.bind(mainMod .. " + SHIFT + S", hl.dsp.focus({ monitor = "l" }))
-- hl.bind(mainMod .. " + ALT + SHIFT + A", hl.dsp.window.move({ workspace = 1, follow = false }))
-- hl.bind(mainMod .. " + ALT + SHIFT + D", hl.dsp.window.move({ workspace = 3, follow = false }))
-- hl.bind(mainMod .. " + ALT + SHIFT + F", hl.dsp.window.move({ workspace = 4, follow = false }))
-- hl.bind(mainMod .. " + ALT + SHIFT + S", hl.dsp.window.move({ workspace = 2, follow = false }))
-- hl.bind(mainMod .. " + ALT + A", hl.dsp.window.move({ direction = "l" }))
-- hl.bind(mainMod .. " + ALT + D", hl.dsp.window.move({ direction = "u" }))
-- hl.bind(mainMod .. " + ALT + F", hl.dsp.window.move({ direction = "d" }))
-- hl.bind(mainMod .. " + ALT + S", hl.dsp.window.move({ direction = "r" }))
-- hl.bind(mainMod .. " + CTRL + A", hl.dsp.focus({ workspace = 5 }))
-- hl.bind(mainMod .. " + CTRL + D", hl.dsp.focus({ workspace = 7 }))
-- hl.bind(mainMod .. " + CTRL + F", hl.dsp.focus({ workspace = 8 }))
-- hl.bind(mainMod .. " + CTRL + S", hl.dsp.focus({ workspace = 6 }))
-- hl.bind(mainMod .. " + SHIFT + A", hl.dsp.window.move({ workspace = 1 }))
-- hl.bind(mainMod .. " + SHIFT + D", hl.dsp.window.move({ workspace = 3 }))
-- hl.bind(mainMod .. " + SHIFT + F", hl.dsp.window.move({ workspace = 4 }))
-- hl.bind(mainMod .. " + SHIFT + S", hl.dsp.window.move({ workspace = 2 }))
-- hl.bind(mainMod .. " + D", hl.dsp.focus({ workspace = 3 }))
-- hl.bind(mainMod .. " + F", hl.dsp.focus({ workspace = 4 }))

-- HJKL
hl.bind(mainMod .. " + ALT + CTRL + SHIFT + H", hl.dsp.window.move({ workspace = 1, follow = false }))
hl.bind(mainMod .. " + ALT + CTRL + SHIFT + J", hl.dsp.window.move({ workspace = 2, follow = false }))
hl.bind(mainMod .. " + ALT + CTRL + SHIFT + K", hl.dsp.window.move({ workspace = 3, follow = false }))
hl.bind(mainMod .. " + ALT + CTRL + SHIFT + L", hl.dsp.window.move({ workspace = 4, follow = false }))
hl.bind(mainMod .. " + ALT + CTRL + H", hl.dsp.focus({ workspace = 5 }))
hl.bind(mainMod .. " + ALT + CTRL + J", hl.dsp.focus({ workspace = 6 }))
hl.bind(mainMod .. " + ALT + CTRL + K", hl.dsp.focus({ workspace = 7 }))
hl.bind(mainMod .. " + ALT + CTRL + L", hl.dsp.focus({ workspace = 8 }))
hl.bind(mainMod .. " + ALT + SHIFT + H", hl.dsp.window.move({ workspace = 5, follow = false }))
hl.bind(mainMod .. " + ALT + SHIFT + J", hl.dsp.window.move({ workspace = 6, follow = false }))
hl.bind(mainMod .. " + ALT + SHIFT + K", hl.dsp.window.move({ workspace = 7, follow = false }))
hl.bind(mainMod .. " + ALT + SHIFT + L", hl.dsp.window.move({ workspace = 8, follow = false }))
hl.bind(mainMod .. " + ALT + h", hl.dsp.window.move({ direction = "l" }))
hl.bind(mainMod .. " + ALT + j", hl.dsp.window.move({ direction = "d" }))
hl.bind(mainMod .. " + ALT + k", hl.dsp.window.move({ direction = "u" }))
hl.bind(mainMod .. " + ALT + l", hl.dsp.window.move({ direction = "r" }))
hl.bind(mainMod .. " + CTRL + SHIFT + H", hl.dsp.window.move({ workspace = 1 }))
hl.bind(mainMod .. " + CTRL + SHIFT + J", hl.dsp.window.move({ workspace = 2 }))
hl.bind(mainMod .. " + CTRL + SHIFT + K", hl.dsp.window.move({ workspace = 3 }))
hl.bind(mainMod .. " + CTRL + SHIFT + L", hl.dsp.window.move({ workspace = 4 }))
hl.bind(mainMod .. " + CTRL + H", hl.dsp.focus({ workspace = 1 }))
hl.bind(mainMod .. " + CTRL + J", hl.dsp.focus({ workspace = 2 }))
hl.bind(mainMod .. " + CTRL + K", hl.dsp.focus({ workspace = 3 }))
hl.bind(mainMod .. " + CTRL + L", hl.dsp.focus({ workspace = 4 }))
hl.bind(mainMod .. " + SHIFT + H", hl.dsp.window.move({ workspace = 5 }))
hl.bind(mainMod .. " + SHIFT + J", hl.dsp.window.move({ workspace = 6 }))
hl.bind(mainMod .. " + SHIFT + K", hl.dsp.window.move({ workspace = "e-1" }))
hl.bind(mainMod .. " + SHIFT + L", hl.dsp.window.move({ workspace = "e+1" }))
hl.bind(mainMod .. " + h", hl.dsp.focus({ direction = "l" }))
hl.bind(mainMod .. " + j", hl.dsp.focus({ direction = "d" }))
hl.bind(mainMod .. " + k", hl.dsp.focus({ direction = "u" }))
hl.bind(mainMod .. " + l", hl.dsp.focus({ direction = "r" }))

-- G
-- Left in the old hyprlang syntax on purpose: these used the `mouse_down` /
-- `mouse_up` dispatchers, which no longer exist as dispatchers. Their intent
-- is ambiguous, so they are not translated rather than guessed at.
-- bind = $mainMod ALT CTRL SHIFT, G, mouse_down, ,1
-- bind = $mainMod ALT CTRL, G, mouse_down, ,5g
-- bind = $mainMod ALT SHIFT, G, mouse_down, ,20
-- bind = $mainMod ALT, G, mouse_down, ,10
-- bind = $mainMod CTRL SHIFT, G, mouse_up, ,1
-- bind = $mainMod CTRL, G, mouse_up, ,5
-- bind = $mainMod SHIFT, G, mouse_up, ,20
-- bind = $mainMod, G, mouse_up, ,10

--[[
hypr-dynamic-cursors plugin config, carried over from the old hyprland.conf.
It was already commented out there. The plugin is loaded via the
`plugins` option in home/common/default.nix, and plugin options now go
through `hl.config({ plugin = { ["dynamic-cursors"] = { ... } } })` rather
than a `plugin:dynamic-cursors { }` hyprlang block, so this needs rewriting
before it can be re-enabled.

    enabled = true

    sets the cursor behaviour, supports these values:
    tilt    - tilt the cursor based on x-velocity
    rotate  - rotate the cursor based on movement direction
    stretch - stretch the cursor shape based on direction and velocity
    none    - do not change the cursors behaviour
    mode = tilt / stretch

    threshold = 1
      minimum angle difference in degrees after which the shape is changed
      smaller values are smoother, but more expensive for hw cursors

    shaperule = <shape-name>, <mode> (optional), <property>: <value>, ...
      override the mode behaviour per shape
      shaperule = text, rotate:offset: 90
      shaperule = grab, stretch, stretch:limit: 1000
      shaperule = clientside, none

    rotate { length = 20, offset = 0.0 }
      length: length in px of the simulated stick used to rotate the cursor
      offset: clockwise offset applied to the angle in degrees

    tilt { limit = 100, function = negative_quadratic }
      limit: speed (px/s) at which the full tilt is reached
      function: linear / quadratic / negative_quadratic

    stretch { limit = 100, function = negative_quadratic }
      limit: speed (px/s) at which the full stretch is reached

    shake { enabled, nearest, threshold, base, speed, influence, limit,
            timeout, effects, ipc }
      magnifies the cursor if it is being shaken

    hyprcursor { nearest, enabled, resolution, fallback }
      use hyprcursor to get a higher resolution texture when magnified
--]]
