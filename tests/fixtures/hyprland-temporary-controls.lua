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
__VERENU_INITIAL_CONTROL__()
local old = _verenu_escape_binding
assert(old.enabled and old.key == "F8")
__VERENU_CHANGED_CONTROL__()
local current = _verenu_escape_binding
assert(current ~= old and current.key == "ESCAPE" and current.enabled,
  "changed key retained the old function-key binding")
assert(not old.enabled, "old function-key binding remained enabled")
__VERENU_UNAVAILABLE_CONTROL__()
assert(not current.enabled, "unavailable control left its binding enabled")
assert(personal.enabled, "disabling Verenu modified a personal binding")
__VERENU_RESTORED_CONTROL__()
assert(_verenu_escape_binding == current and current.enabled,
  "restoring the same control created a duplicate binding")
__VERENU_RECONNECTED_CONTROL__()
assert(not current.enabled and _verenu_escape_binding.action == "new:cancel",
  "portal reconnect retained the stale action")
__VERENU_DISCONNECTED_CONTROL__()
assert(not _verenu_escape_binding.enabled, "disconnected control stayed enabled")
assert(personal.enabled, "portal disconnect modified a personal binding")
print("Temporary control lifecycle regressions passed")
