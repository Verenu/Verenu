/linux hotkey: portal press/ { presses++; next }
presses == 1 && /lifecycle: recording -> stopping gen=[0-9]+/ {
    if (generation != "") { ambiguous = 1; next }
    generation = $0
    sub(/^.*lifecycle: recording -> stopping gen=/, "", generation)
    sub(/[^0-9].*$/, "", generation)
    next
}
presses == 1 && generation != "" && /pipeline: completed gen=[0-9]+/ {
    completed = $0
    sub(/^.*pipeline: completed gen=/, "", completed)
    sub(/[^0-9].*$/, "", completed)
    if (completed == generation) found = 1
}
END { exit !(found && presses == 1 && !ambiguous) }
