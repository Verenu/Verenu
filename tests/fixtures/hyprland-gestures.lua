local time, timers, bindings, actions = 0, {}, {}, {}
local keyboard_handlers = {}
local function keyboard(...)
  for _, handler in ipairs(keyboard_handlers) do handler(...) end
end
local window_rules = {}
hl = {
  window_rule = function(rule) window_rules[#window_rules + 1] = rule end,
  dsp = {
    global = function(id) return id end,
    exec_cmd = function(command) return command end,
    submap = function() return "reset" end,
  },
  dispatch = function(action) actions[#actions + 1] = action end,
  bind = function(keys, callback)
    local handle = { removed = false }
    function handle:remove() self.removed = true end
    local previous = bindings[keys]
    bindings[keys] = function()
      if previous then previous() end
      if not handle.removed then callback() end
    end
    return handle
  end,
  on = function(_, callback) keyboard_handlers[#keyboard_handlers + 1] = callback end,
  timer = function(callback, options)
    timers[#timers + 1] = { at = time + options.timeout, callback = callback }
  end,
}
local function advance(target)
  while true do
    local first
    for i, timer in ipairs(timers) do
      if timer.at <= target and (not first or timer.at < timers[first].at) then first = i end
    end
    if not first then break end
    local timer = table.remove(timers, first)
    time = timer.at
    timer.callback()
  end
  time = target
end
local function count(action)
  local total = 0
  for _, dispatched in ipairs(actions) do
    if dispatched == action then total = total + 1 end
  end
  return total
end

local function reload_bindings()
__VERENU_GENERATED_BINDINGS__()
end

-- Hyprland can reload its config without destroying Lua event listeners.
reload_bindings()
reload_bindings()

__VERENU_GENERATED_WINDOW_RULES__()
assert(#window_rules == 2, "fresh setup did not install both window policies")
assert(window_rules[1].match.class == "^(com\\.verenu\\.app|[Vv]erenu)$", "window policy depends on executable filename")
assert(window_rules[1].float and window_rules[1].center and window_rules[1].min_size[1] == 1100 and window_rules[1].min_size[2] == 700, "main window size policy missing")
assert(window_rules[2].float and window_rules[2].pin and window_rules[2].no_initial_focus, "pill can tile or steal initial focus")
assert(not window_rules[2].no_focus, "pill buttons cannot receive clicks")

bindings["CTRL + Super_L"]()
advance(312)
keyboard(133, nil, 0)
advance(475)
bindings["CTRL + Super_L"]()
advance(555)
keyboard(133, nil, 0)
advance(590)
assert(count("app:dictate") == 1, "double-tap started two captures")
assert(count("handsfree") == 1, "double-tap failed to enter hands-free")
assert(count("app:cancel") == 0, "double-tap dispatched Escape cancellation")
assert(count("app:cancel-chord") == 0, "double-tap rejected its own chord")

-- A second tap's release must not arm another double tap, even when the
-- next press arrives within the double-tap window.
bindings["CTRL + Super_L"]()
assert(count("handsfree") == 1, "second release armed a spurious hands-free toggle")
advance(610)
keyboard(133, nil, 0)
advance(2000)

advance(2500)
bindings["SUPER + Control_L"]()
advance(2550)
keyboard(37, nil, 0)
keyboard(133, nil, 0)
advance(2600)
assert(count("release") == 4, "simultaneous modifier release dispatched twice")
assert(count("app:cancel") == 0, "mouse chord cancelled hands-free")

advance(3000)
bindings["CTRL + Super_L"]()
keyboard(38, nil, 1)
keyboard(39, nil, 1)
assert(count("app:cancel-chord") == 1, "combo rejection did not fire exactly once")
assert(count("app:cancel") == 0, "combo rejection used Escape cancellation")
keyboard(133, nil, 0)
advance(3050)
bindings["CTRL + Super_L"]()
assert(count("app:dictate") == 4, "combo follow-up started another dictation")
assert(count("handsfree") == 1, "combo follow-up entered hands-free")
advance(4000)

bindings["CTRL + Super_L"]()
local cancelled_before = count("app:cancel-chord")
local released_before = count("release")
for _, keycode in ipairs({121, 122, 123, 171, 172, 173, 174, 208, 209, 215, 232, 233, 236, 237, 238, 251, 252, 256}) do
  keyboard(keycode, nil, 1)
  keyboard(keycode, nil, 0)
end
assert(count("app:cancel-chord") == cancelled_before, "media control cancelled held dictation")
assert(count("release") == released_before, "media control ended held dictation")
keyboard(133, nil, 0)
advance(5000)
assert(count("release") == released_before + 1, "modifier release no longer finishes dictation")

bindings["CTRL + Super_L"]()
cancelled_before = count("app:cancel-chord")
released_before = count("release")
keyboard(191, nil, 1)
advance(6500)
keyboard(191, nil, 2)
assert(count("app:cancel-chord") == cancelled_before, "mouse F13 companion cancelled dictation")
assert(count("release") == released_before, "mouse hold ended before its modifiers were released")
keyboard(191, nil, 0)
keyboard(37, nil, 0)
keyboard(133, nil, 0)
advance(6600)
assert(count("release") == released_before + 1, "mouse release must finish exactly once")

bindings["CTRL + Super_L"]()
keyboard(38, nil, 1)
assert(count("app:cancel-chord") == cancelled_before + 1, "initial letter shortcut was not rejected")
keyboard(133, nil, 0)
advance(7500)
bindings["CTRL + Super_L"]()
advance(7560)
keyboard(191, nil, 1)
assert(count("app:cancel-chord") == cancelled_before + 2, "later F13 shortcut was not rejected")
keyboard(133, nil, 0)
advance(8000)

-- Holding the chord must deliver one release when either modifier lifts.
local starts_before = count("app:dictate")
released_before = count("release")
local toggles_before = count("handsfree")
bindings["CTRL + Super_L"]()
advance(9000)
keyboard(133, nil, 0)
keyboard(37, nil, 0)
advance(9050)
assert(count("app:dictate") == starts_before + 1, "held chord did not start")
assert(count("release") == released_before + 1, "held chord did not release exactly once")
assert(count("handsfree") == toggles_before, "held chord entered hands-free")

released_before = count("release")
bindings["CTRL + Super_L"]()
advance(10000)
reload_bindings()
keyboard(133, nil, 0)
advance(10050)
assert(count("release") == released_before + 1, "reload during hold failed to release exactly once")

released_before = count("release")
bindings["CTRL + Super_L"]()
advance(11000)
keyboard(133, nil, 0)
reload_bindings()
advance(11050)
assert(count("release") == released_before + 1, "reload during pending release dispatched twice")

released_before = count("release")
starts_before = count("app:dictate")
bindings["CTRL + Super_L"]()
reload_bindings()
bindings["CTRL + Super_L"]()
assert(count("release") == released_before, "reload released before the portal press could arrive")
assert(count("app:dictate") == starts_before + 1, "reload accepted a press while retiring the previous chord")
advance(11075)
assert(count("release") == released_before + 1, "immediate reload did not deliver the delayed release")
print("Generated Hyprland gesture regressions passed")
