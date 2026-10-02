-- Execute the generated production bindings without changing the desktop.
local time, timers, bindings, actions = 0, {}, {}, {}
local keyboard
hl = {
  dsp = {
    global = function(id) return id end,
    exec_cmd = function(command) return command end,
    submap = function() return "reset" end,
  },
  dispatch = function(action) actions[#actions + 1] = action end,
  bind = function(keys, callback) bindings[keys] = callback end,
  on = function(_, callback) keyboard = callback end,
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

-- GENERATED_BINDINGS

-- Replay the reported 312 ms first tap and 163 ms gap while Ctrl stays down.
bindings["CTRL + Super_L"]()
advance(312)
keyboard(133, nil, 0)
advance(475)
bindings["CTRL + Super_L"]()
advance(555)
keyboard(133, nil, 0)
advance(1500)
assert(count("app:dictate") == 1, "double-tap started two captures")
assert(count("handsfree") == 1, "double-tap failed to enter hands-free")
assert(count("app:cancel") == 0, "double-tap dispatched Escape cancellation")
assert(count("app:cancel-chord") == 0, "double-tap rejected its own chord")

-- A mouse macro releases both modifiers in the same input frame.
bindings["SUPER + Control_L"]()
advance(1550)
keyboard(37, nil, 0)
keyboard(133, nil, 0)
advance(1600)
assert(count("release") == 3, "simultaneous modifier release dispatched twice")
assert(count("app:cancel") == 0, "mouse chord cancelled hands-free")

-- Other keys while the chord is held may discard the hold prefix, but must
-- use the separate action that the backend ignores during hands-free.
advance(2500)
bindings["CTRL + Super_L"]()
keyboard(38, nil, 1)
keyboard(39, nil, 1)
assert(count("app:cancel-chord") == 1, "combo rejection did not fire exactly once")
assert(count("app:cancel") == 0, "combo rejection used Escape cancellation")
keyboard(133, nil, 0)
advance(2550)
bindings["CTRL + Super_L"]()
assert(count("app:dictate") == 3, "combo follow-up started another dictation")
assert(count("handsfree") == 1, "combo follow-up entered hands-free")
advance(4000)
print("Generated Hyprland gesture regressions passed")
