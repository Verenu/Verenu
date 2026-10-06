-- Run the production snippets against handles that retain their trigger.
local handles = {}
hl = {
  dsp = { global = function(id) return id end },
  bind = function(key, action)
    local handle = { key = key, action = action, enabled = true }
    function handle:set_enabled(enabled) self.enabled = enabled end
    handles[#handles + 1] = handle
    return handle
  end,
}
local personal = hl.bind("F8", "personal")
-- INITIAL_CONTROL
local old = _verenu_escape_binding
assert(old.enabled and old.key == "F8")
-- CHANGED_CONTROL
local current = _verenu_escape_binding
assert(current ~= old and current.key == "ESCAPE" and current.enabled,
  "changed key retained the old function-key binding")
assert(not old.enabled, "old function-key binding remained enabled")
-- UNAVAILABLE_CONTROL
assert(not current.enabled, "unavailable control left its binding enabled")
assert(personal.enabled, "disabling Verenu modified a personal binding")
-- RESTORED_CONTROL
assert(_verenu_escape_binding == current and current.enabled,
  "restoring the same control created a duplicate binding")
-- RECONNECTED_CONTROL
assert(not current.enabled and _verenu_escape_binding.action == "new:cancel",
  "portal reconnect retained the stale action")
-- DISCONNECTED_CONTROL
assert(not _verenu_escape_binding.enabled, "disconnected control stayed enabled")
assert(personal.enabled, "portal disconnect modified a personal binding")
print("Temporary control lifecycle regressions passed")
